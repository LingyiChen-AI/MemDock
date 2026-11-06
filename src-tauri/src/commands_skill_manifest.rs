//! Skill install manifest commands for the Skills page: this machine's
//! manifest, comparison with the synced `skills.json` (or an imported file),
//! and export / import of manifest files. Nothing here installs a skill or
//! touches the network; installs go through `install_skill`.

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
installed (git url, path, commit, tool, scope), not the skill files. local_only entries have \
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
        let gh = c.missing_here.iter().find(|m| m.skill.name == "gh").unwrap();
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
            skill_manifest_import_impl(&bad.to_string_lossy()).unwrap_err().code,
            "manifest_invalid"
        );
        fs::write(&bad, r#"{"format":9,"skills":[]}"#).unwrap();
        assert_eq!(
            skill_manifest_import_impl(&bad.to_string_lossy()).unwrap_err().code,
            "manifest_invalid"
        );
        assert!(skill_manifest_import_impl(&tmp.path().join("none.json").to_string_lossy()).is_err());
    }
}
