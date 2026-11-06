//! Models each CLI offers (what `/model` shows), without sending a message:
//! - Claude Code: the `initialize` control response of a stream-json process
//!   carries `models` ({value, displayName, description, resolvedModel}).
//! - Codex: `model/list` on `codex app-server`; when that fails, the CLI's own
//!   `~/.codex/models_cache.json` (entries with `visibility: "list"`).
//! Results are cached in memory for an hour.
use std::collections::HashMap;
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
}

const TTL: Duration = Duration::from_secs(3600);
const QUERY_TIMEOUT: Duration = Duration::from_secs(25);

static CACHE: Mutex<Option<HashMap<&'static str, (Instant, Vec<ModelOption>)>>> = Mutex::new(None);

fn key(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "claude",
        Agent::Codex => "codex",
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string()
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
            let value = [s(m, "model"), s(m, "slug"), s(m, "id")].into_iter().find(|x| !x.is_empty())?;
            let name = [s(m, "displayName"), s(m, "display_name")].into_iter().find(|x| !x.is_empty()).unwrap_or_else(|| value.clone());
            Some(ModelOption {
                is_default: m.get("isDefault").and_then(Value::as_bool).unwrap_or(false),
                display_name: name,
                description: s(m, "description"),
                value,
            })
        })
        .collect()
}

/// Runs `program args`, writes `input` lines, returns the first stdout JSON line `pick` accepts.
fn query(program: &Path, args: &[&str], input: &[Value], pick: impl Fn(&Value) -> Option<Vec<ModelOption>>) -> Option<Vec<ModelOption>> {
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
    let mut child = proc::spawn_hidden(&mut cmd).ok()?;
    let stdout = child.stdout.take()?;
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
    let mut found = None;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(v) => {
                if let Some(m) = pick(&v) {
                    found = Some(m);
                    break;
                }
            }
            Err(_) => break,
        }
    }
    proc::kill_tree(&mut child);
    found.filter(|m| !m.is_empty())
}

fn claude_models() -> Result<Vec<ModelOption>, String> {
    let program = find_program(OsStr::new("claude"), &path_dirs()).ok_or("未找到 Claude Code 命令行（claude）")?;
    let init = json!({"type": "control_request", "request_id": "memdock-models", "request": {"subtype": "initialize"}});
    query(
        &program,
        &["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"],
        &[init],
        |v| {
            (v.get("type").and_then(Value::as_str) == Some("control_response"))
                .then(|| parse_claude_models(&v["response"]["response"]["models"]))
        },
    )
    .ok_or_else(|| "没有从 Claude Code 读到模型列表".to_string())
}

fn codex_models(codex_home: &Path) -> Result<Vec<ModelOption>, String> {
    if let Some(program) = find_program(OsStr::new("codex"), &path_dirs()) {
        let input = [
            json!({"id": 1, "method": "initialize", "params": {"clientInfo": {"name": "memdock", "title": "MemDock", "version": env!("CARGO_PKG_VERSION")}}}),
            json!({"method": "initialized"}),
            json!({"id": 2, "method": "model/list", "params": {}}),
        ];
        let live = query(&program, &["app-server"], &input, |v| {
            (v.get("id").and_then(Value::as_i64) == Some(2)).then(|| parse_codex_models(&v["result"]["data"]))
        });
        if let Some(m) = live {
            return Ok(m);
        }
    }
    let text = std::fs::read_to_string(codex_home.join("models_cache.json")).map_err(|_| "没有从 Codex 读到模型列表".to_string())?;
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let m = parse_codex_models(&v["models"]);
    if m.is_empty() {
        Err("没有从 Codex 读到模型列表".into())
    } else {
        Ok(m)
    }
}

/// Cached model list of `agent`; `force` asks the CLI again.
pub fn models(agent: Agent, codex_home: &Path, force: bool) -> Result<Vec<ModelOption>, String> {
    if !force {
        if let Some((at, m)) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(key(agent))) {
            if at.elapsed() < TTL {
                return Ok(m.clone());
            }
        }
    }
    let m = match agent {
        Agent::Claude => claude_models()?,
        Agent::Codex => codex_models(codex_home)?,
    };
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(key(agent), (Instant::now(), m.clone()));
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_init_models() {
        let v = json!([
            {"value": "default", "resolvedModel": "claude-opus-5-5", "displayName": "Default (recommended)", "description": "Opus 5.5 · Best for everyday, complex tasks"},
            {"value": "sonnet", "displayName": "Sonnet 5.5", "description": "Most efficient"},
            {"value": "", "displayName": "skip"}
        ]);
        let m = parse_claude_models(&v);
        assert_eq!(m.len(), 2);
        assert!(m[0].is_default);
        assert_eq!((m[1].value.as_str(), m[1].display_name.as_str()), ("sonnet", "Sonnet 5.5"));
    }

    #[test]
    fn parses_codex_list_and_cache_shapes() {
        let list = json!([
            {"id": "a", "model": "gpt-6.1-sol", "displayName": "GPT-6.1-Sol", "description": "Workhorse", "hidden": false, "isDefault": true},
            {"id": "b", "model": "secret", "displayName": "Hidden", "description": "", "hidden": true, "isDefault": false}
        ]);
        let m = parse_codex_models(&list);
        assert_eq!(m.len(), 1);
        assert!(m[0].is_default);
        assert_eq!(m[0].value, "gpt-6.1-sol");
        let cache = json!([
            {"slug": "gpt-6-astra", "display_name": "GPT-6-Astra", "description": "x", "visibility": "list"},
            {"slug": "gpt-reserve", "display_name": "R", "visibility": "hide"}
        ]);
        let m = parse_codex_models(&cache);
        assert_eq!(m.iter().map(|x| x.value.as_str()).collect::<Vec<_>>(), vec!["gpt-6-astra"]);
    }

    /// `cargo test --lib live::models::tests::real -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real() {
        let home = dirs::home_dir().unwrap().join(".codex");
        for agent in [Agent::Claude, Agent::Codex] {
            let t = Instant::now();
            let m = models(agent, &home, true);
            println!("{agent:?} {:?}: {:?}", t.elapsed(), m.map(|v| v.into_iter().map(|x| x.value).collect::<Vec<_>>()));
        }
    }
}
