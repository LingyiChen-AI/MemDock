//! Models and slash commands each CLI offers (what `/model` and `/` show),
//! without sending a message. One CLI run yields both:
//! - Claude Code: the `initialize` control response of a stream-json process
//!   carries `models` ({value, displayName, description, supportedEffortLevels})
//!   and `commands` ({name, description, argumentHint, builtin?}).
//! - Codex: `model/list` and `skills/list` on `codex app-server`; when that fails,
//!   models come from the CLI's own `~/.codex/models_cache.json` (entries with
//!   `visibility: "list"`) and there are no commands.
//!
//! Results are cached in memory for an hour.
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use super::Agent;
use crate::extract::runner::{find_program, path_dirs};
use crate::proc;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelOption {
    /// What to pass as `--model` / `model` (`None`-like default is `"default"` for Claude).
    pub value: String,
    pub display_name: String,
    pub description: String,
    /// The CLI's default choice.
    pub is_default: bool,
    /// Supported thinking effort levels (`low`, `medium`...), empty = no effort choice.
    pub efforts: Vec<String>,
    /// The model's default effort (Codex only).
    pub default_effort: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    Skill,
    Command,
}

/// One entry of the `/` menu.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommandOption {
    /// Without the leading slash.
    pub name: String,
    pub description: String,
    pub argument_hint: String,
    pub kind: CommandKind,
}

/// Everything one probe of a CLI returns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalog {
    pub models: Vec<ModelOption>,
    pub commands: Vec<CommandOption>,
}

const TTL: Duration = Duration::from_secs(3600);
const QUERY_TIMEOUT: Duration = Duration::from_secs(25);

static CACHE: Mutex<Option<HashMap<&'static str, (Instant, Catalog)>>> = Mutex::new(None);
/// One probe at a time, so `live_models` and `live_commands` asked together share one CLI run.
static PROBE: Mutex<()> = Mutex::new(());

fn key(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "claude",
        Agent::Codex => "codex",
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

fn first_nonempty(xs: impl IntoIterator<Item = String>) -> Option<String> {
    xs.into_iter().find(|x| !x.is_empty())
}

/// `models` of the Claude `initialize` response.
pub fn parse_claude_models(models: &Value) -> Vec<ModelOption> {
    models
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    let value = s(m, "value");
                    (!value.is_empty()).then(|| ModelOption {
                        is_default: value == "default",
                        display_name: Some(s(m, "displayName")).filter(|d| !d.is_empty()).unwrap_or_else(|| value.clone()),
                        description: s(m, "description"),
                        efforts: m
                            .get("supportedEffortLevels")
                            .and_then(Value::as_array)
                            .map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect())
                            .unwrap_or_default(),
                        default_effort: None,
                        value,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `result.data` of Codex `model/list` (camelCase) or `models` of `models_cache.json` (snake_case).
pub fn parse_codex_models(list: &Value) -> Vec<ModelOption> {
    let Some(a) = list.as_array() else { return vec![] };
    a.iter()
        .filter(|m| !m.get("hidden").and_then(Value::as_bool).unwrap_or(false))
        .filter(|m| m.get("visibility").and_then(Value::as_str).is_none_or(|v| v == "list"))
        .filter_map(|m| {
            let value = first_nonempty([s(m, "model"), s(m, "slug"), s(m, "id")])?;
            let name = first_nonempty([s(m, "displayName"), s(m, "display_name")]).unwrap_or_else(|| value.clone());
            let levels = m
                .get("supportedReasoningEfforts")
                .or_else(|| m.get("supported_reasoning_levels"))
                .and_then(Value::as_array);
            let efforts = levels
                .map(|l| {
                    l.iter()
                        .filter_map(|e| {
                            e.as_str()
                                .map(str::to_string)
                                .or_else(|| first_nonempty([s(e, "reasoningEffort"), s(e, "effort")]))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let default_effort = first_nonempty([s(m, "defaultReasoningEffort"), s(m, "default_reasoning_level")]);
            Some(ModelOption {
                is_default: m.get("isDefault").and_then(Value::as_bool).unwrap_or(false),
                display_name: name,
                description: s(m, "description"),
                efforts,
                default_effort,
                value,
            })
        })
        .collect()
}

/// Whether a Claude command without the `builtin` flag looks like a skill: a
/// user / project / plugin / synced skill (by its description suffix) or a
/// namespaced plugin entry (`plugin:name`).
fn looks_like_skill(name: &str, description: &str) -> bool {
    let d = description.trim_end();
    name.contains(':')
        || d.ends_with("(user)")
        || d.ends_with("(project)")
        || d.ends_with("(claude.ai sync)")
        || d.rsplit_once('(').is_some_and(|(_, tail)| tail.starts_with("plugin") && tail.ends_with(')'))
}

/// `commands` of the Claude `initialize` response. Entries flagged `builtin`
/// are commands; when the CLI sends that flag at all, the rest are skills,
/// otherwise the description / name heuristic decides. Internal `__x` entries
/// are dropped.
pub fn parse_claude_commands(commands: &Value) -> Vec<CommandOption> {
    let Some(a) = commands.as_array() else { return vec![] };
    let flagged = a.iter().any(|c| c.get("builtin").is_some());
    let mut seen = HashSet::new();
    a.iter()
        .filter_map(|c| {
            let name = s(c, "name").trim_start_matches('/').to_string();
            if name.is_empty() || name.starts_with("__") || !seen.insert(name.clone()) {
                return None;
            }
            let description = s(c, "description");
            let builtin = c.get("builtin").and_then(Value::as_bool).unwrap_or(false);
            let kind = if builtin {
                CommandKind::Command
            } else if flagged || looks_like_skill(&name, &description) {
                CommandKind::Skill
            } else {
                CommandKind::Command
            };
            Some(CommandOption {
                argument_hint: first_nonempty([s(c, "argumentHint"), s(c, "argument_hint")]).unwrap_or_default(),
                name,
                description,
                kind,
            })
        })
        .collect()
}

/// `result.data` of Codex `skills/list` ([{cwd, skills: [{name, description,
/// shortDescription?, interface?, enabled}]}]): enabled skills, deduplicated.
pub fn parse_codex_skills(data: &Value) -> Vec<CommandOption> {
    let mut seen = HashSet::new();
    let mut out = vec![];
    for entry in data.as_array().into_iter().flatten() {
        for sk in entry.get("skills").and_then(Value::as_array).into_iter().flatten() {
            if !sk.get("enabled").and_then(Value::as_bool).unwrap_or(true) {
                continue;
            }
            let name = s(sk, "name");
            if name.is_empty() || !seen.insert(name.clone()) {
                continue;
            }
            let iface = sk.get("interface").cloned().unwrap_or(Value::Null);
            let description =
                first_nonempty([s(&iface, "shortDescription"), s(sk, "shortDescription"), s(sk, "description")]).unwrap_or_default();
            out.push(CommandOption { name, description, argument_hint: String::new(), kind: CommandKind::Skill });
        }
    }
    out
}

/// Runs `program args`, writes `input` lines and feeds each stdout JSON line to
/// `take` until it returns `true` (done) or the timeout passes.
fn query(program: &Path, args: &[&str], input: &[Value], mut take: impl FnMut(&Value) -> bool) {
    let dirs = path_dirs();
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(dirs::home_dir().unwrap_or_else(std::env::temp_dir))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_remove("CLAUDECODE");
    if cfg!(target_os = "macos") {
        if let Ok(p) = std::env::join_paths(&dirs) {
            cmd.env("PATH", p);
        }
    }
    let Ok(mut child) = proc::spawn_hidden(&mut cmd) else { return };
    let Some(stdout) = child.stdout.take() else {
        proc::kill_tree(&mut child);
        return;
    };
    let (tx, rx) = mpsc::channel::<Value>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if tx.send(v).is_err() {
                    break;
                }
            }
        }
    });
    if let Some(mut stdin) = child.stdin.take() {
        for v in input {
            let _ = writeln!(stdin, "{v}");
        }
        let _ = stdin.flush();
        // Keep stdin open (closing it ends a stream-json process early); dropped with the child.
        std::mem::forget(stdin);
    }
    let deadline = Instant::now() + QUERY_TIMEOUT;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(v) => {
                if take(&v) {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    proc::kill_tree(&mut child);
}

fn claude_catalog() -> Result<Catalog, String> {
    let program = find_program(OsStr::new("claude"), &path_dirs()).ok_or("未找到 Claude Code 命令行（claude）")?;
    let init = json!({"type": "control_request", "request_id": "memdock-models", "request": {"subtype": "initialize"}});
    let mut found = None;
    query(
        &program,
        &["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"],
        &[init],
        |v| {
            if v.get("type").and_then(Value::as_str) != Some("control_response") {
                return false;
            }
            let r = &v["response"]["response"];
            found = Some(Catalog { models: parse_claude_models(&r["models"]), commands: parse_claude_commands(&r["commands"]) });
            true
        },
    );
    found.filter(|c| !c.models.is_empty()).ok_or_else(|| "没有从 Claude Code 读到模型列表".to_string())
}

fn codex_catalog(codex_home: &Path) -> Result<Catalog, String> {
    let mut cat = Catalog::default();
    if let Some(program) = find_program(OsStr::new("codex"), &path_dirs()) {
        let input = [
            json!({"id": 1, "method": "initialize", "params": {"clientInfo": {"name": "memdock", "title": "MemDock", "version": env!("CARGO_PKG_VERSION")}}}),
            json!({"method": "initialized"}),
            json!({"id": 2, "method": "model/list", "params": {}}),
            json!({"id": 3, "method": "skills/list", "params": {}}),
        ];
        let (mut got_models, mut got_skills) = (false, false);
        query(&program, &["app-server"], &input, |v| {
            match v.get("id").and_then(Value::as_i64) {
                Some(2) => {
                    cat.models = parse_codex_models(&v["result"]["data"]);
                    got_models = true;
                }
                Some(3) => {
                    cat.commands = parse_codex_skills(&v["result"]["data"]);
                    got_skills = true;
                }
                _ => {}
            }
            got_models && got_skills
        });
    }
    if cat.models.is_empty() {
        let text = std::fs::read_to_string(codex_home.join("models_cache.json")).map_err(|_| "没有从 Codex 读到模型列表".to_string())?;
        let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        cat.models = parse_codex_models(&v["models"]);
        if cat.models.is_empty() {
            return Err("没有从 Codex 读到模型列表".into());
        }
    }
    Ok(cat)
}

/// Cached catalog of `agent`; `force` asks the CLI again.
pub fn catalog(agent: Agent, codex_home: &Path, force: bool) -> Result<Catalog, String> {
    let cached = || {
        CACHE
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|c| c.get(key(agent)))
            .filter(|(at, _)| at.elapsed() < TTL)
            .map(|(_, c)| c.clone())
    };
    if !force {
        if let Some(c) = cached() {
            return Ok(c);
        }
    }
    let started = Instant::now();
    let _probe = PROBE.lock().unwrap_or_else(|e| e.into_inner());
    // Another caller may have probed while we waited.
    if let Some((at, c)) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(key(agent))).cloned() {
        if at >= started || (!force && at.elapsed() < TTL) {
            return Ok(c);
        }
    }
    let c = match agent {
        Agent::Claude => claude_catalog()?,
        Agent::Codex => codex_catalog(codex_home)?,
    };
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(key(agent), (Instant::now(), c.clone()));
    Ok(c)
}

/// Cached model list of `agent`; `force` asks the CLI again.
pub fn models(agent: Agent, codex_home: &Path, force: bool) -> Result<Vec<ModelOption>, String> {
    catalog(agent, codex_home, force).map(|c| c.models)
}

/// Cached `/` menu entries of `agent`; `force` asks the CLI again.
pub fn commands(agent: Agent, codex_home: &Path, force: bool) -> Result<Vec<CommandOption>, String> {
    catalog(agent, codex_home, force).map(|c| c.commands)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_init_models() {
        let v = json!([
            {"value": "default", "resolvedModel": "claude-opus-5-5", "displayName": "Default (recommended)", "description": "Opus 5.5 · Best for everyday, complex tasks", "supportsEffort": true, "supportedEffortLevels": ["low", "medium", "high", "xhigh", "max"]},
            {"value": "sonnet", "displayName": "Sonnet 5.5", "description": "Most efficient"},
            {"value": "", "displayName": "skip"}
        ]);
        let m = parse_claude_models(&v);
        assert_eq!(m.len(), 2);
        assert!(m[0].is_default);
        assert_eq!(m[0].efforts, vec!["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(m[0].default_effort, None);
        assert_eq!((m[1].value.as_str(), m[1].display_name.as_str()), ("sonnet", "Sonnet 5.5"));
        assert!(m[1].efforts.is_empty());
    }

    #[test]
    fn parses_codex_list_and_cache_shapes() {
        let list = json!([
            {"id": "a", "model": "gpt-6.1-sol", "displayName": "GPT-6.1-Sol", "description": "Workhorse", "hidden": false, "isDefault": true,
             "supportedReasoningEfforts": [{"reasoningEffort": "low", "description": "x"}, {"reasoningEffort": "high", "description": "y"}],
             "defaultReasoningEffort": "low"},
            {"id": "b", "model": "secret", "displayName": "Hidden", "description": "", "hidden": true, "isDefault": false}
        ]);
        let m = parse_codex_models(&list);
        assert_eq!(m.len(), 1);
        assert!(m[0].is_default);
        assert_eq!(m[0].value, "gpt-6.1-sol");
        assert_eq!(m[0].efforts, vec!["low", "high"]);
        assert_eq!(m[0].default_effort.as_deref(), Some("low"));
        let cache = json!([
            {"slug": "gpt-6-astra", "display_name": "GPT-6-Astra", "description": "x", "visibility": "list",
             "default_reasoning_level": "medium", "supported_reasoning_levels": [{"effort": "medium", "description": "m"}, {"effort": "ultra", "description": "u"}]},
            {"slug": "gpt-reserve", "display_name": "R", "visibility": "hide"}
        ]);
        let m = parse_codex_models(&cache);
        assert_eq!(m.iter().map(|x| x.value.as_str()).collect::<Vec<_>>(), vec!["gpt-6-astra"]);
        assert_eq!(m[0].efforts, vec!["medium", "ultra"]);
        assert_eq!(m[0].default_effort.as_deref(), Some("medium"));
    }

    #[test]
    fn parses_claude_commands_with_builtin_flag() {
        let v = json!([
            {"name": "bug-bot", "description": "Bug bot (user)", "argumentHint": ""},
            {"name": "superpowers:brainstorming", "description": "Brainstorm", "argumentHint": "", "aliases": ["brainstorming"]},
            {"name": "compact", "description": "Summarize", "argumentHint": "<instructions>", "builtin": true},
            {"name": "__remote-workflow", "description": "internal", "builtin": true},
            {"name": "compact", "description": "dup", "builtin": true},
            {"name": "", "description": "no name"}
        ]);
        let c = parse_claude_commands(&v);
        let got: Vec<_> = c.iter().map(|x| (x.name.as_str(), x.kind)).collect();
        assert_eq!(
            got,
            vec![("bug-bot", CommandKind::Skill), ("superpowers:brainstorming", CommandKind::Skill), ("compact", CommandKind::Command)]
        );
        assert_eq!(c[2].argument_hint, "<instructions>");
        assert_eq!(serde_json::to_value(&c[0]).unwrap()["kind"], "skill");
        assert_eq!(serde_json::to_value(&c[2]).unwrap()["argument_hint"], "<instructions>");
    }

    #[test]
    fn parses_claude_commands_without_builtin_flag_by_heuristic() {
        let v = json!([
            {"name": "/review", "description": "Review a PR"},
            {"name": "mine", "description": "My skill (project)"},
            {"name": "pdf", "description": "PDF things (claude.ai sync)"},
            {"name": "fmt", "description": "Format (plugin:tools@market)"},
            {"name": "ns:cmd", "description": "Namespaced"}
        ]);
        let c = parse_claude_commands(&v);
        assert_eq!(c[0].name, "review");
        assert_eq!(c[0].kind, CommandKind::Command);
        assert!(c[1..].iter().all(|x| x.kind == CommandKind::Skill), "{c:?}");
    }

    #[test]
    fn parses_codex_skills() {
        let v = json!([
            {"cwd": "C:/a", "errors": [], "skills": [
                {"name": "pdf", "description": "Long description", "interface": {"shortDescription": "Short", "iconSmallUrl": null, "iconLargeUrl": null}, "path": "x", "scope": "user", "enabled": true, "pluginId": null},
                {"name": "off", "description": "Disabled", "path": "y", "scope": "user", "enabled": false, "pluginId": null},
                {"name": "plain", "description": "Plain one", "path": "z", "scope": "repo", "enabled": true, "pluginId": null}
            ]},
            {"cwd": "C:/b", "errors": [], "skills": [{"name": "pdf", "description": "dup", "path": "x", "scope": "user", "enabled": true, "pluginId": null}]}
        ]);
        let c = parse_codex_skills(&v);
        assert_eq!(
            c.iter().map(|x| (x.name.as_str(), x.description.as_str())).collect::<Vec<_>>(),
            vec![("pdf", "Short"), ("plain", "Plain one")]
        );
        assert!(c.iter().all(|x| x.kind == CommandKind::Skill && x.argument_hint.is_empty()));
    }

    /// `cargo test --lib live::models::tests::real -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real() {
        let home = dirs::home_dir().unwrap().join(".codex");
        for agent in [Agent::Claude, Agent::Codex] {
            let t = Instant::now();
            match catalog(agent, &home, true) {
                Ok(c) => {
                    println!("{agent:?} {:?}: {} models, {} commands", t.elapsed(), c.models.len(), c.commands.len());
                    for m in c.models.iter().take(4) {
                        println!("  model {} efforts={:?} default={:?}", m.value, m.efforts, m.default_effort);
                    }
                    for k in [CommandKind::Skill, CommandKind::Command] {
                        let names: Vec<_> = c.commands.iter().filter(|x| x.kind == k).map(|x| x.name.as_str()).take(12).collect();
                        println!("  {k:?} ({}): {names:?}", c.commands.iter().filter(|x| x.kind == k).count());
                    }
                    assert!(!c.models.is_empty());
                    assert!(!c.commands.is_empty(), "{agent:?} returned no commands");
                }
                Err(e) => panic!("{agent:?}: {e}"),
            }
            // Second call is served from the cache.
            let t = Instant::now();
            commands(agent, &home, false).unwrap();
            assert!(t.elapsed() < Duration::from_millis(50));
        }
    }
}
