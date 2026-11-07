//! Skill install manifest commands for the Skills page: this machine's
//! manifest, comparison with the synced `skills.json` (or an imported file),
//! export/import, and restoring plugin packages through their own CLI.

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::commands::{AppState, CommandError};
use crate::commands_sync::{cached_remote_manifest, manifest_context, ManifestContext};
use crate::skills::model::SkillScope;
use crate::sync::manifest::{compare, Manifest, ManifestScope, ManifestSkill};

type St<'a> = State<'a, Arc<AppState>>;

/// Import files above this size are rejected.
const MAX_IMPORT_BYTES: u64 = 4 * 1024 * 1024;

const EXPORT_DESCRIPTION: &str = "MemDock skill install manifest: records how each skill was \
installed (git source or plugin marketplace key, tool, scope). local_only entries have \
no known source and must be copied by hand. Import it in MemDock > Skills > Skill sync.";

/// A manifest entry missing on this machine, with where it would go here.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MissingSkillDto {
    #[serde(flatten)]
    pub skill: ManifestSkill,
    /// Local install scope; `None` when the project is not on this computer.
    pub local_scope: Option<SkillScope>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ManifestCompareDto {
    pub local: Manifest,
    /// `None` when there is nothing to compare with (not logged in / never synced).
    pub remote: Option<Manifest>,
    pub missing_here: Vec<MissingSkillDto>,
    pub only_here: Vec<ManifestSkill>,
    pub in_both: Vec<ManifestSkill>,
    /// Local entries without a known source (counted, never installable).
    pub local_only: Vec<ManifestSkill>,
    /// Last sync time (ms) when `remote` comes from the synced repository.
    pub remote_updated_at: Option<i64>,
    pub logged_in: bool,
}

fn err(code: &str, m: impl Into<String>) -> CommandError {
    CommandError::new(code, m)
}

fn local_of(ctx: &ManifestContext) -> Result<Vec<ManifestSkill>, CommandError> {
    ctx.local
        .clone()
        .ok_or_else(|| err("manifest", "skill registry or skill lock cannot be read"))
}

fn local_manifest(ctx: &ManifestContext) -> Result<Manifest, CommandError> {
    let mut skills = local_of(ctx)?;
    for s in &mut skills {
        s.installed_on.insert(ctx.machine.id.clone());
    }
    Ok(Manifest {
        format: Manifest::format_for(&skills),
        skills,
        ..Manifest::default()
    })
}

fn local_scope(m: &ManifestSkill, ctx: &ManifestContext) -> Option<SkillScope> {
    match &m.scope {
        ManifestScope::Global => Some(SkillScope::Global),
        ManifestScope::Project { key } => ctx
            .projects
            .iter()
            .find(|(_, k, dk)| k.as_deref() == Some(key.as_str()) || dk == key)
            .map(|(id, _, _)| SkillScope::Project {
                project_id: id.clone(),
            }),
    }
}

pub fn skill_manifest_local_impl(state: &AppState) -> Result<Manifest, CommandError> {
    local_manifest(&manifest_context(state)?)
}

pub fn skill_manifest_compare_impl(
    state: &AppState,
    remote: Option<Manifest>,
) -> Result<ManifestCompareDto, CommandError> {
    let ctx = manifest_context(state)?;
    let local = local_manifest(&ctx)?;
    let (remote, updated) = match remote {
        Some(m) => (Some(m), None),
        None if ctx.logged_in => match cached_remote_manifest(state)? {
            Some(m) => (Some(m), ctx.last_sync_at),
            None => (None, None),
        },
        None => (None, None),
    };
    let cmp = remote
        .as_ref()
        .map(|r| compare(&local.skills, r, &ctx.machine.id))
        .unwrap_or_default();
    let missing_here = cmp
        .missing_here
        .into_iter()
        .map(|skill| MissingSkillDto {
            local_scope: local_scope(&skill, &ctx),
            skill,
        })
        .collect();
    let local_only = local
        .skills
        .iter()
        .filter(|s| s.is_local_only())
        .cloned()
        .collect();
    let only_here = if remote.is_some() {
        cmp.only_here
    } else {
        local.skills.clone()
    };
    Ok(ManifestCompareDto {
        local,
        remote,
        missing_here,
        only_here,
        in_both: cmp.in_both,
        local_only,
        remote_updated_at: updated,
        logged_in: ctx.logged_in,
    })
}

pub fn skill_manifest_export_impl(state: &AppState, path: &str) -> Result<usize, CommandError> {
    let p = Path::new(path);
    if path.trim().is_empty() || !p.is_absolute() {
        return Err(err("invalid", "export path must be absolute"));
    }
    let mut m = skill_manifest_local_impl(state)?;
    m.description = Some(EXPORT_DESCRIPTION.to_string());
    let json = m.to_json().map_err(|e| err("io", e.to_string()))?;
    std::fs::write(p, json.as_bytes()).map_err(|e| err("io", e.to_string()))?;
    Ok(m.skills.len())
}

pub fn skill_manifest_import_impl(path: &str) -> Result<Manifest, CommandError> {
    let p = Path::new(path);
    let meta = std::fs::metadata(p).map_err(|e| err("io", e.to_string()))?;
    if !meta.is_file() {
        return Err(err("invalid", "not a file"));
    }
    if meta.len() > MAX_IMPORT_BYTES {
        return Err(err("manifest_invalid", "manifest file is too large"));
    }
    let bytes = std::fs::read(p).map_err(|e| err("io", e.to_string()))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| err("manifest_invalid", "manifest file is not UTF-8 text"))?;
    let text = text.trim_start_matches('\u{feff}');
    Manifest::from_json(text).map_err(|e| err("manifest_invalid", e))
}

#[tauri::command(async)]
pub fn skill_manifest_local(state: St) -> Result<Manifest, CommandError> {
    skill_manifest_local_impl(&state)
}

#[tauri::command(async)]
pub fn skill_manifest_compare(
    state: St,
    remote: Option<Manifest>,
) -> Result<ManifestCompareDto, CommandError> {
    skill_manifest_compare_impl(&state, remote)
}

#[tauri::command(async)]
pub fn skill_manifest_export(state: St, path: String) -> Result<usize, CommandError> {
    skill_manifest_export_impl(&state, &path)
}

#[tauri::command(async)]
pub fn skill_manifest_import(path: String) -> Result<Manifest, CommandError> {
    skill_manifest_import_impl(&path)
}

/// Reinstall a package using the same marketplace selector as the source device.
#[tauri::command(async)]
pub fn skill_manifest_install_plugin(
    state: St,
    tool: crate::model::Tool,
    key: String,
    scope: SkillScope,
    local_scope: bool,
) -> Result<(), CommandError> {
    run_plugin_operation(&state, tool, &key, scope, local_scope, false)
}

#[tauri::command(async)]
pub fn update_skill_plugin(
    state: St,
    skill_id: String,
    scope: SkillScope,
) -> Result<(), CommandError> {
    let projects = state
        .snapshot_index()
        .projects
        .into_iter()
        .map(|p| p.project)
        .collect::<Vec<_>>();
    let entry = crate::skills::scan::scan_skills(&state.paths, &state.home, &projects)
        .into_iter()
        .find(|entry| entry.id == skill_id && entry.scope == scope && entry.plugin.is_some())
        .ok_or_else(|| err("plugin_update", "installed plugin was not found"))?;
    let key = entry.plugin.as_deref().expect("filtered plugin entry");
    run_plugin_operation(
        &state,
        entry.tool,
        key,
        scope,
        entry.plugin_local_scope,
        true,
    )
}

fn run_plugin_operation(
    state: &AppState,
    tool: crate::model::Tool,
    key: &str,
    scope: SkillScope,
    local_scope: bool,
    update: bool,
) -> Result<(), CommandError> {
    use crate::model::Tool;

    let valid_part = |s: &str| {
        s.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
    };
    let Some((_, market)) = key
        .split_once('@')
        .filter(|(plugin, market)| valid_part(plugin) && valid_part(market))
    else {
        return Err(err("invalid", "invalid plugin marketplace selector"));
    };
    let _guard = crate::commands::lock(match tool {
        Tool::Claude => &state.claude_plugins_lock,
        Tool::Codex => &state.codex_plugins_lock,
    });
    let (dir, install_scope) = match scope {
        SkillScope::Global if !local_scope => (state.home.clone(), "user"),
        SkillScope::Project { project_id } if tool == Tool::Claude => {
            let project = state
                .snapshot_index()
                .projects
                .into_iter()
                .find(|p| p.project.id == project_id && p.project.exists)
                .ok_or_else(|| err("invalid", "project is not available on this device"))?;
            (
                std::path::PathBuf::from(project.project.path),
                if local_scope { "local" } else { "project" },
            )
        }
        _ => return Err(err("invalid", "invalid plugin install scope")),
    };
    let run = |args: &[&str]| plugin_command(state, tool, &dir, args, 180).map(|_| ());
    match (tool, update) {
        (Tool::Claude, false) => run(&["plugin", "install", "--scope", install_scope, key]),
        (Tool::Claude, true) => {
            run(&["plugin", "marketplace", "update", market])?;
            run(&["plugin", "update", "--scope", install_scope, key])
        }
        (Tool::Codex, false) => run(&["plugin", "add", key]),
        (Tool::Codex, true) => {
            let git_marketplace =
                std::fs::read_to_string(state.paths.codex_home.join("config.toml"))
                    .ok()
                    .and_then(|text| toml::from_str::<toml::Table>(&text).ok())
                    .is_some_and(|config| {
                        config
                            .get("marketplaces")
                            .and_then(|markets| markets.get(market))
                            .and_then(|market| market.get("source_type"))
                            .and_then(toml::Value::as_str)
                            == Some("git")
                    });
            if git_marketplace {
                run(&["plugin", "marketplace", "upgrade", market])?;
            }
            run(&["plugin", "add", key])
        }
    }
}

fn plugin_command(
    state: &AppState,
    tool: crate::model::Tool,
    dir: &Path,
    args: &[&str],
    timeout: u64,
) -> Result<String, CommandError> {
    use crate::extract::runner::{find_program, path_dirs};
    use crate::model::Tool;
    use std::process::{Command, Stdio};
    let name = match tool {
        Tool::Claude => "claude",
        Tool::Codex => "codex",
    };
    let dirs = path_dirs();
    let program = find_program(name.as_ref(), &dirs)
        .ok_or_else(|| err("plugin_install", format!("{name} CLI is not installed")))?;
    let mut command = Command::new(&program);
    command
        .args(args)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Ok(path) = std::env::join_paths(&dirs) {
        command.env("PATH", path);
    }
    match tool {
        Tool::Claude => command.env("CLAUDE_CONFIG_DIR", &state.paths.claude_home),
        Tool::Codex => command.env("CODEX_HOME", &state.paths.codex_home),
    };
    let child = crate::proc::spawn_hidden(&mut command)
        .map_err(|e| err("plugin_operation", e.to_string()))?;
    let output =
        crate::proc::wait_with_timeout(child, std::time::Duration::from_secs(timeout), None)
            .map_err(|e| {
                err(
                    "plugin_operation",
                    format!("{name} plugin command failed: {e:?}"),
                )
            })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        return Err(err("plugin_operation", format!("{name}: {detail}")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[derive(Serialize)]
pub struct PluginUpdateStatus {
    skill_id: String,
    tool: crate::model::Tool,
    scope: SkillScope,
    latest: Option<String>,
    has_update: Option<bool>,
}

#[tauri::command(async)]
pub fn check_skill_plugin_updates(state: St) -> Vec<PluginUpdateStatus> {
    use crate::model::Tool;
    use serde_json::Value;

    fn read(path: &Path) -> Option<Value> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    let projects = state
        .snapshot_index()
        .projects
        .into_iter()
        .map(|p| p.project)
        .collect::<Vec<_>>();
    let entries = crate::skills::scan::scan_skills(&state.paths, &state.home, &projects);
    let mut seen = std::collections::HashSet::new();
    let claude_dir = state.paths.claude_home.join("plugins");
    let (known, codex) = std::thread::scope(|scope| {
        let claude = scope.spawn(|| {
            entries
                .iter()
                .any(|e| e.tool == Tool::Claude && e.plugin.is_some())
                .then(|| {
                    let _guard = crate::commands::lock(&state.claude_plugins_lock);
                    plugin_command(
                        &state,
                        Tool::Claude,
                        &state.home,
                        &["plugin", "marketplace", "update"],
                        45,
                    )
                })
                .filter(Result::is_ok)
                .and_then(|_| read(&claude_dir.join("known_marketplaces.json")))
        });
        let codex = entries
            .iter()
            .any(|e| e.tool == Tool::Codex && e.plugin.is_some())
            .then(|| {
                plugin_command(
                    &state,
                    Tool::Codex,
                    &state.home,
                    &["plugin", "list", "--available", "--json"],
                    45,
                )
            })
            .and_then(Result::ok)
            .and_then(|json| serde_json::from_str::<Value>(&json).ok());
        (claude.join().expect("Claude marketplace check"), codex)
    });
    let installed = read(&claude_dir.join("installed_plugins.json"));
    entries
        .into_iter()
        .filter_map(|entry| {
            let key = entry.plugin.as_deref()?;
            if !seen.insert((
                entry.tool,
                key.to_string(),
                entry.plugin_install_path.clone(),
                format!("{:?}", entry.scope),
            )) {
                return None;
            }
            let (name, market) = key.split_once('@').unwrap_or((key, ""));
            let current = entry.plugin_version.as_deref();
            let mut latest = None;
            let mut has_update = None;
            match entry.tool {
                Tool::Claude => {
                    if let Some(root) = known
                        .as_ref()
                        .and_then(|v| v[market]["installLocation"].as_str())
                    {
                        let root = Path::new(root);
                        let catalog = read(&root.join(".claude-plugin/marketplace.json"));
                        let plugin = catalog
                            .as_ref()
                            .and_then(|v| v["plugins"].as_array())
                            .and_then(|items| {
                                items
                                    .iter()
                                    .find(|item| item["name"].as_str() == Some(name))
                            });
                        if let Some(plugin) = plugin {
                            latest = plugin["version"].as_str().map(str::to_owned);
                            if let Some(source) =
                                plugin["source"].as_str().filter(|p| p.starts_with("./"))
                            {
                                let version = root
                                    .join(source)
                                    .canonicalize()
                                    .ok()
                                    .filter(|p| {
                                        root.canonicalize().is_ok_and(|root| p.starts_with(root))
                                    })
                                    .and_then(|p| read(&p.join(".claude-plugin/plugin.json")))
                                    .and_then(|v| v["version"].as_str().map(str::to_owned));
                                latest = version.or(latest);
                            }
                            if let Some(sha) = plugin["source"]["sha"].as_str() {
                                let installed_sha = installed
                                    .as_ref()
                                    .and_then(|v| v["plugins"][key].as_array())
                                    .and_then(|items| {
                                        items.iter().find(|item| {
                                            item["installPath"].as_str()
                                                == entry.plugin_install_path.as_deref()
                                        })
                                    })
                                    .and_then(|item| item["gitCommitSha"].as_str());
                                has_update = installed_sha.map(|old| old != sha);
                                if has_update == Some(true) && latest.is_none() {
                                    latest = Some(sha.chars().take(8).collect());
                                }
                            }
                        }
                    }
                }
                Tool::Codex => {
                    if let Some(plugin) = codex
                        .as_ref()
                        .into_iter()
                        .flat_map(|v| {
                            ["installed", "available"]
                                .iter()
                                .filter_map(|bucket| v[*bucket].as_array())
                                .flatten()
                        })
                        .find(|plugin| plugin["pluginId"].as_str() == Some(key))
                    {
                        if let Some(path) = plugin["source"]["path"]
                            .as_str()
                            .filter(|_| plugin["source"]["source"].as_str() == Some("local"))
                        {
                            let path = Path::new(path);
                            latest = read(&path.join("plugin.json"))
                                .or_else(|| read(&path.join(".codex-plugin/plugin.json")))
                                .and_then(|v| v["version"].as_str().map(str::to_owned));
                        } else {
                            latest = plugin["version"]
                                .as_str()
                                .filter(|version| Some(*version) != current)
                                .map(str::to_owned);
                        }
                    }
                }
            }
            if has_update.is_none() {
                has_update = current
                    .zip(latest.as_deref())
                    .and_then(|(current, latest)| {
                        let current = semver::Version::parse(current).ok()?;
                        let latest = semver::Version::parse(latest).ok()?;
                        Some(latest > current)
                    });
            }
            Some(PluginUpdateStatus {
                skill_id: entry.id,
                tool: entry.tool,
                scope: entry.scope,
                latest,
                has_update,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AgentPaths;
    use std::fs;

    fn state(tmp: &Path) -> AppState {
        let paths = AgentPaths::from_home(&tmp.join("home"));
        fs::create_dir_all(&paths.claude_home).unwrap();
        fs::create_dir_all(&paths.codex_home).unwrap();
        AppState::with_home(paths, tmp.join("data"), tmp.join("home"))
    }

    fn add_skill(st: &AppState, name: &str) {
        let dir = st.paths.claude_home.join("skills").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), format!("---\nname: {name}\n---\n")).unwrap();
    }

    fn remote_skill(name: &str, url: &str, on: &str) -> ManifestSkill {
        ManifestSkill {
            plugin: None,
            name: name.into(),
            url: url.into(),
            git_ref: None,
            commit: None,
            skill_path: String::new(),
            tool: crate::model::Tool::Claude,
            scope: ManifestScope::Global,
            installed_on: [on.to_string()].into(),
            removed_on: Default::default(),
            local_only: url.is_empty(),
        }
    }

    #[test]
    fn local_manifest_includes_plain_skill_as_local_only() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        add_skill(&st, "plain");
        let m = skill_manifest_local_impl(&st).unwrap();
        assert_eq!(m.skills.len(), 1);
        assert!(m.skills[0].local_only);
        assert_eq!(m.skills[0].installed_on.len(), 1);
    }

    #[test]
    fn compare_without_remote_lists_everything_as_only_here() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        add_skill(&st, "plain");
        let c = skill_manifest_compare_impl(&st, None).unwrap();
        assert!(c.remote.is_none());
        assert!(!c.logged_in);
        assert_eq!(c.only_here.len(), 1);
        assert_eq!(c.local_only.len(), 1);
        assert!(c.missing_here.is_empty());
    }

    #[test]
    fn compare_with_imported_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        add_skill(&st, "plain");
        let remote = Manifest {
            skills: vec![
                remote_skill("plain", "", "other"),
                remote_skill("gh", "https://github.com/o/gh", "other"),
                remote_skill("copyme", "", "other"),
            ],
            ..Manifest::default()
        };
        let c = skill_manifest_compare_impl(&st, Some(remote)).unwrap();
        assert_eq!(c.in_both.len(), 1);
        assert!(c.only_here.is_empty());
        assert_eq!(c.missing_here.len(), 2);
        let gh = c
            .missing_here
            .iter()
            .find(|m| m.skill.name == "gh")
            .unwrap();
        assert_eq!(gh.local_scope, Some(SkillScope::Global));
        assert!(c.missing_here.iter().any(|m| m.skill.local_only));
        assert!(c.remote_updated_at.is_none());
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["missing_here"][0]["local_scope"]["type"], "global");
        assert!(v["missing_here"][0]["url"].is_string());
    }

    #[test]
    fn export_then_import_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        add_skill(&st, "plain");
        let file = tmp.path().join("out").join("skills.json");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let n = skill_manifest_export_impl(&st, &file.to_string_lossy()).unwrap();
        assert_eq!(n, 1);
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"description\""));
        assert!(text.contains("\n  \"format\": 1"));
        let m = skill_manifest_import_impl(&file.to_string_lossy()).unwrap();
        assert_eq!(m.skills.len(), 1);
        assert_eq!(m.skills[0].name, "plain");
        assert!(skill_manifest_export_impl(&st, "relative.json").is_err());
    }

    #[test]
    fn import_rejects_bad_files() {
        let tmp = tempfile::tempdir().unwrap();
        let bad = tmp.path().join("bad.json");
        fs::write(&bad, "{nope").unwrap();
        assert_eq!(
            skill_manifest_import_impl(&bad.to_string_lossy())
                .unwrap_err()
                .code,
            "manifest_invalid"
        );
        fs::write(&bad, r#"{"format":9,"skills":[]}"#).unwrap();
        assert_eq!(
            skill_manifest_import_impl(&bad.to_string_lossy())
                .unwrap_err()
                .code,
            "manifest_invalid"
        );
        assert!(
            skill_manifest_import_impl(&tmp.path().join("none.json").to_string_lossy()).is_err()
        );
    }
}
