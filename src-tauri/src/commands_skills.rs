//! Tauri commands for skill management. Logic lives in `*_impl(&AppState, ..)`
//! functions (testable without an app); the `#[tauri::command]` wrappers only
//! delegate and emit `skills-changed` after successful writes.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::commands::{lock, AppState, CommandError};
use crate::memory::analyze::{estimate_tokens, find_imports, ImportRef};
use crate::memory::io::{read_memory, write_memory};
use crate::model::{Project, Tool};
use crate::skills::fsops::{list_backups, BackupInfo};
use crate::skills::git::GitError;
use crate::skills::git_cache::{list_repo_skills, parse_github_input, RepoSkill};
use crate::skills::model::{SkillEntry, SkillScope};
use crate::skills::ops::{
    self, InstallFrom, InstallTarget, OpsCtx, OpsError, UpdateStatus,
};
use crate::skills::scan::scan_skills;

/// Files above this size are shown as "too large" and never edited.
const MAX_TEXT_BYTES: u64 = 1024 * 1024;

type St<'a> = State<'a, Arc<AppState>>;

/// Payload of the `skills-changed` event (empty `paths` for app-made changes).
#[derive(Serialize, Clone, Debug)]
pub struct SkillsChangedPayload {
    pub paths: Vec<String>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

fn git_message(e: &GitError) -> String {
    match e {
        GitError::NotInstalled => "git is not installed".to_string(),
        GitError::Timeout => "git timed out".to_string(),
        GitError::Failed { stderr } => stderr.clone(),
    }
}

impl From<OpsError> for CommandError {
    fn from(e: OpsError) -> Self {
        match e {
            OpsError::Exists { dir } => {
                let mut c = CommandError::new("exists", format!("already exists: {dir}"));
                c.dir = Some(dir);
                c
            }
            OpsError::ReadOnly => CommandError::new("read_only", "skill is read-only"),
            OpsError::GitUnavailable => {
                CommandError::new("git_unavailable", "git is not installed")
            }
            OpsError::Git(g) => CommandError::new("git_failed", git_message(&g)),
            OpsError::LocalChanges => {
                CommandError::new("local_changes", "skill has local changes")
            }
            OpsError::Invalid { message } => CommandError::new("invalid", message),
            OpsError::Io { message } => CommandError::new("io", message),
        }
    }
}

fn invalid(m: impl Into<String>) -> CommandError {
    CommandError::new("invalid", m)
}

// ---------------------------------------------------------------------------
// Input DTOs
// ---------------------------------------------------------------------------

/// Where to install from. Field names are snake_case, `type` is the tag.
#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InstallFromDto {
    Git {
        url: String,
        clone_url: String,
        #[serde(default)]
        git_ref: Option<String>,
        skill_path: String,
    },
    Folder {
        path: String,
    },
    Zip {
        path: String,
    },
    Template {
        name: String,
        description: String,
    },
}

impl From<InstallFromDto> for InstallFrom {
    fn from(d: InstallFromDto) -> Self {
        match d {
            InstallFromDto::Git {
                url,
                clone_url,
                git_ref,
                skill_path,
            } => InstallFrom::Git {
                url,
                clone_url,
                git_ref,
                skill_path,
            },
            InstallFromDto::Folder { path } => InstallFrom::Folder { path },
            InstallFromDto::Zip { path } => InstallFrom::Zip { path },
            InstallFromDto::Template { name, description } => {
                InstallFrom::Template { name, description }
            }
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct InstallTargetDto {
    pub tool: Tool,
    pub scope: SkillScope,
}

impl From<InstallTargetDto> for InstallTarget {
    fn from(d: InstallTargetDto) -> Self {
        InstallTarget {
            tool: d.tool,
            scope: d.scope,
        }
    }
}

// ---------------------------------------------------------------------------
// Output DTOs
// ---------------------------------------------------------------------------

/// Result of `inspect_git_source`.
#[derive(Serialize, Debug, Clone)]
pub struct InspectResult {
    pub url: String,
    pub clone_url: String,
    pub git_ref: Option<String>,
    pub commit: String,
    pub skills: Vec<RepoSkill>,
    /// Skill path pinned by the input (a `tree/<ref>/<path>` URL), if any.
    pub preselected: Option<String>,
}

/// At most this many files are listed for one skill.
pub const MAX_LISTED_FILES: usize = 5000;

#[derive(Serialize, Debug, Clone)]
pub struct SkillFileList {
    pub files: Vec<SkillFileInfo>,
    /// True when the listing stopped at `MAX_LISTED_FILES`.
    pub truncated: bool,
}

#[derive(Serialize, Debug, Clone)]
pub struct SkillFileInfo {
    /// `/`-separated path relative to the skill directory.
    pub rel_path: String,
    pub size_bytes: u64,
}

/// A skill file. Binary / oversized files carry no text and are read-only.
#[derive(Serialize, Debug, Clone)]
pub struct SkillFileDto {
    pub text: String,
    pub hash: String,
    pub has_bom: bool,
    pub crlf: bool,
    pub tokens: u32,
    pub imports: Vec<ImportRef>,
    pub binary: bool,
    pub too_large: bool,
}

impl SkillFileDto {
    fn unreadable(too_large: bool) -> Self {
        Self {
            text: String::new(),
            hash: String::new(),
            has_bom: false,
            crlf: false,
            tokens: 0,
            imports: vec![],
            binary: true,
            too_large,
        }
    }
}

// ---------------------------------------------------------------------------
// Context helpers
// ---------------------------------------------------------------------------

fn projects_of(state: &AppState) -> Vec<Project> {
    state
        .snapshot_index()
        .projects
        .into_iter()
        .map(|p| p.project)
        .collect()
}

fn ops_ctx(state: &AppState) -> OpsCtx {
    OpsCtx {
        paths: state.paths.clone(),
        home: state.home.clone(),
        projects: projects_of(state),
        appdata: state.data_dir.clone(),
        git: state.git.clone(),
        registry_path: state.data_dir.join("skills-registry.json"),
        cache_lock: state.cache_lock.clone(),
    }
}

fn find_entry(state: &AppState, skill_id: &str) -> Result<SkillEntry, CommandError> {
    scan_skills(&state.paths, &state.home, &projects_of(state))
        .into_iter()
        .find(|e| e.id == skill_id)
        .ok_or_else(|| invalid(format!("skill not found: {skill_id}")))
}

// ---------------------------------------------------------------------------
// Skill operations
// ---------------------------------------------------------------------------

pub fn list_skills_impl(state: &AppState) -> Result<Vec<SkillEntry>, CommandError> {
    Ok(ops::scan_with_sources(&ops_ctx(state))?.0)
}

pub fn git_available_impl(state: &AppState) -> bool {
    state.git.available()
}

pub fn inspect_git_source_impl(
    state: &AppState,
    input: &str,
) -> Result<InspectResult, CommandError> {
    let parsed = parse_github_input(input).map_err(invalid)?;
    if !state.git.available() {
        return Err(CommandError::new("git_unavailable", "git is not installed"));
    }
    // No skills_lock here: only the cache lock inside `ensure_cache`.
    let (cache, commit) = ops::ensure_cache(
        &ops_ctx(state),
        &parsed.url,
        &parsed.clone_url,
        parsed.git_ref.as_deref(),
    )?;
    Ok(InspectResult {
        skills: list_repo_skills(&cache),
        url: parsed.url,
        clone_url: parsed.clone_url,
        git_ref: parsed.git_ref,
        commit,
        preselected: parsed.skill_path,
    })
}

pub fn install_skill_impl(
    state: &AppState,
    from: InstallFromDto,
    targets: Vec<InstallTargetDto>,
    overwrite: bool,
) -> Result<Vec<SkillEntry>, CommandError> {
    let _g = lock(&state.skills_lock);
    let targets: Vec<InstallTarget> = targets.into_iter().map(Into::into).collect();
    Ok(ops::install(&ops_ctx(state), from.into(), &targets, overwrite)?)
}

pub fn check_skill_updates_impl(state: &AppState) -> Result<Vec<UpdateStatus>, CommandError> {
    let ctx = ops_ctx(state);
    let (entries, _) = ops::scan_with_sources(&ctx)?;
    Ok(ops::check_updates(&ctx, &entries))
}

pub fn update_skill_impl(
    state: &AppState,
    skill_id: &str,
    accept_local_changes: bool,
) -> Result<SkillEntry, CommandError> {
    let _g = lock(&state.skills_lock);
    Ok(ops::update(&ops_ctx(state), skill_id, accept_local_changes)?)
}

pub fn rollback_skill_impl(
    state: &AppState,
    skill_id: &str,
    backup_id: &str,
) -> Result<SkillEntry, CommandError> {
    let _g = lock(&state.skills_lock);
    Ok(ops::rollback(&ops_ctx(state), skill_id, backup_id)?)
}

pub fn list_skill_backups_impl(
    state: &AppState,
    skill_id: &str,
) -> Result<Vec<BackupInfo>, CommandError> {
    let entry = find_entry(state, skill_id)?;
    let key = ops::skill_key(&entry)?;
    Ok(list_backups(&ops_ctx(state).backups_root(), &key))
}

/// An empty `id` in the result means no backup was made.
pub fn delete_skill_impl(state: &AppState, skill_id: &str) -> Result<BackupInfo, CommandError> {
    let _g = lock(&state.skills_lock);
    Ok(ops::delete(&ops_ctx(state), skill_id)?)
}

pub fn sync_skill_impl(
    state: &AppState,
    skill_id: &str,
    target: InstallTargetDto,
    overwrite: bool,
) -> Result<SkillEntry, CommandError> {
    let _g = lock(&state.skills_lock);
    Ok(ops::sync_to(
        &ops_ctx(state),
        skill_id,
        target.into(),
        overwrite,
    )?)
}

pub fn register_skill_source_impl(
    state: &AppState,
    skill_id: &str,
    url: &str,
    skill_path: &str,
) -> Result<SkillEntry, CommandError> {
    let _g = lock(&state.skills_lock);
    Ok(ops::register_source(
        &ops_ctx(state),
        skill_id,
        url,
        skill_path,
    )?)
}

// ---------------------------------------------------------------------------
// Skill files
// ---------------------------------------------------------------------------

/// Directory that actually holds the files (the link target for symlinks).
fn files_dir(entry: &SkillEntry) -> PathBuf {
    entry
        .symlink_target
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&entry.dir))
}

/// CON, PRN, AUX, NUL, COM1-9, LPT1-9 (case-insensitive, with or without extension).
fn is_windows_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    ["con", "prn", "aux", "nul"].contains(&stem.as_str())
        || ["com", "lpt"].iter().any(|p| {
            stem.strip_prefix(p)
                .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
        })
}

/// Normalizes a user-supplied relative path: no absolute paths, drive
/// letters, `..` or empty result.
fn clean_rel(rel: &str) -> Result<PathBuf, CommandError> {
    if rel.contains('\0') {
        return Err(invalid("invalid path"));
    }
    let norm = rel.replace('\\', "/");
    if norm.starts_with('/') {
        return Err(invalid("path must be relative"));
    }
    let mut out = PathBuf::new();
    for part in norm.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(invalid("path must not contain '..'")),
            p if p.contains(':') => return Err(invalid("invalid path")),
            p if p.eq_ignore_ascii_case(".git") || p.eq_ignore_ascii_case("node_modules") => {
                return Err(invalid(format!("access to {p} is not allowed")));
            }
            p if p.ends_with('.') || p.ends_with(' ') || is_windows_reserved(p) => {
                return Err(invalid(format!("invalid file name: {p:?}")));
            }
            p => out.push(p),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(invalid("empty path"));
    }
    Ok(out)
}

/// Resolves `rel` inside the skill and checks that, after resolving symlinks,
/// the deepest existing part stays inside the skill directory.
fn resolve_in_skill(entry: &SkillEntry, rel: &str) -> Result<PathBuf, CommandError> {
    let rel = clean_rel(rel)?;
    let base = fs::canonicalize(files_dir(entry))?;
    let target = base.join(&rel);
    let mut probe = target.clone();
    loop {
        match fs::symlink_metadata(&probe) {
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if !probe.pop() {
                    return Err(invalid("invalid path"));
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    // A dangling link fails to canonicalize and is rejected as well.
    let real = fs::canonicalize(&probe).map_err(|_| invalid("path escapes the skill"))?;
    if !real.starts_with(&base) {
        return Err(invalid("path escapes the skill directory"));
    }
    Ok(target)
}

fn to_file_dto(state: &AppState, path: &Path, c: crate::memory::io::FileContent) -> SkillFileDto {
    let base = path.parent().unwrap_or(Path::new(""));
    SkillFileDto {
        tokens: estimate_tokens(&c.text),
        imports: find_imports(&c.text, base, &state.home),
        text: c.text,
        hash: c.hash,
        has_bom: c.has_bom,
        crlf: c.crlf,
        binary: false,
        too_large: false,
    }
}

pub fn list_skill_files_impl(
    state: &AppState,
    skill_id: &str,
) -> Result<SkillFileList, CommandError> {
    let entry = find_entry(state, skill_id)?;
    let base = fs::canonicalize(files_dir(&entry))?;
    let walker = walkdir::WalkDir::new(&base)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0 || !matches!(e.file_name().to_str(), Some(".git" | "node_modules"))
        });
    let mut out = Vec::new();
    let mut truncated = false;
    for e in walker.flatten() {
        if !e.file_type().is_file() {
            continue;
        }
        if out.len() >= MAX_LISTED_FILES {
            truncated = true;
            break;
        }
        let Ok(rel) = e.path().strip_prefix(&base) else {
            continue;
        };
        out.push(SkillFileInfo {
            rel_path: rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
            size_bytes: e.metadata().map(|m| m.len()).unwrap_or(0),
        });
    }
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(SkillFileList {
        files: out,
        truncated,
    })
}

pub fn read_skill_file_impl(
    state: &AppState,
    skill_id: &str,
    rel_path: &str,
) -> Result<SkillFileDto, CommandError> {
    let entry = find_entry(state, skill_id)?;
    let p = resolve_in_skill(&entry, rel_path)?;
    let meta = fs::metadata(&p)?;
    if !meta.is_file() {
        return Err(invalid("not a file"));
    }
    if meta.len() > MAX_TEXT_BYTES {
        return Ok(SkillFileDto::unreadable(true));
    }
    match read_memory(&p) {
        Ok(c) => Ok(to_file_dto(state, &p, c)),
        Err(e) if e.kind() == io::ErrorKind::InvalidData => Ok(SkillFileDto::unreadable(false)),
        Err(e) => Err(e.into()),
    }
}

pub fn write_skill_file_impl(
    state: &AppState,
    skill_id: &str,
    rel_path: &str,
    text: &str,
    expected_hash: &str,
) -> Result<SkillFileDto, CommandError> {
    let _g = lock(&state.skills_lock);
    let entry = find_entry(state, skill_id)?;
    if entry.readonly {
        return Err(CommandError::from(OpsError::ReadOnly));
    }
    let p = resolve_in_skill(&entry, rel_path)?;
    if text.len() as u64 > MAX_TEXT_BYTES {
        return Err(invalid("file too large to edit"));
    }
    match fs::metadata(&p) {
        Ok(m) if !m.is_file() => return Err(invalid("not a file")),
        Ok(m) if m.len() > MAX_TEXT_BYTES => return Err(invalid("binary or too large file")),
        Ok(_) => {
            if let Err(e) = read_memory(&p) {
                if e.kind() == io::ErrorKind::InvalidData {
                    return Err(invalid("binary or too large file"));
                }
                return Err(e.into());
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let c = write_memory(&p, text, expected_hash, &state.snapshots)?;
    Ok(to_file_dto(state, &p, c))
}

// ---------------------------------------------------------------------------
// Tauri wrappers
// ---------------------------------------------------------------------------

fn emit_changed(app: &AppHandle) {
    let _ = app.emit("skills-changed", SkillsChangedPayload { paths: vec![] });
}

fn emitting<T>(app: &AppHandle, r: Result<T, CommandError>) -> Result<T, CommandError> {
    if r.is_ok() {
        emit_changed(app);
    }
    r
}

#[tauri::command(async)]
pub fn list_skills(state: St) -> Result<Vec<SkillEntry>, CommandError> {
    list_skills_impl(&state)
}

#[tauri::command(async)]
pub fn git_available(state: St) -> bool {
    git_available_impl(&state)
}

#[tauri::command(async)]
pub fn inspect_git_source(state: St, input: String) -> Result<InspectResult, CommandError> {
    inspect_git_source_impl(&state, &input)
}

#[tauri::command(async)]
pub fn install_skill(
    state: St,
    app: AppHandle,
    from: InstallFromDto,
    targets: Vec<InstallTargetDto>,
    overwrite: bool,
) -> Result<Vec<SkillEntry>, CommandError> {
    emitting(&app, install_skill_impl(&state, from, targets, overwrite))
}

#[tauri::command(async)]
pub fn check_skill_updates(state: St) -> Result<Vec<UpdateStatus>, CommandError> {
    check_skill_updates_impl(&state)
}

#[tauri::command(async)]
pub fn update_skill(
    state: St,
    app: AppHandle,
    skill_id: String,
    accept_local_changes: bool,
) -> Result<SkillEntry, CommandError> {
    emitting(
        &app,
        update_skill_impl(&state, &skill_id, accept_local_changes),
    )
}

#[tauri::command(async)]
pub fn rollback_skill(
    state: St,
    app: AppHandle,
    skill_id: String,
    backup_id: String,
) -> Result<SkillEntry, CommandError> {
    emitting(&app, rollback_skill_impl(&state, &skill_id, &backup_id))
}

#[tauri::command(async)]
pub fn list_skill_backups(state: St, skill_id: String) -> Result<Vec<BackupInfo>, CommandError> {
    list_skill_backups_impl(&state, &skill_id)
}

#[tauri::command(async)]
pub fn delete_skill(
    state: St,
    app: AppHandle,
    skill_id: String,
) -> Result<BackupInfo, CommandError> {
    emitting(&app, delete_skill_impl(&state, &skill_id))
}

#[tauri::command(async)]
pub fn sync_skill(
    state: St,
    app: AppHandle,
    skill_id: String,
    target: InstallTargetDto,
    overwrite: bool,
) -> Result<SkillEntry, CommandError> {
    emitting(
        &app,
        sync_skill_impl(&state, &skill_id, target, overwrite),
    )
}

#[tauri::command(async)]
pub fn register_skill_source(
    state: St,
    app: AppHandle,
    skill_id: String,
    url: String,
    skill_path: String,
) -> Result<SkillEntry, CommandError> {
    emitting(
        &app,
        register_skill_source_impl(&state, &skill_id, &url, &skill_path),
    )
}

#[tauri::command(async)]
pub fn list_skill_files(state: St, skill_id: String) -> Result<SkillFileList, CommandError> {
    list_skill_files_impl(&state, &skill_id)
}

#[tauri::command(async)]
pub fn read_skill_file(
    state: St,
    skill_id: String,
    rel_path: String,
) -> Result<SkillFileDto, CommandError> {
    read_skill_file_impl(&state, &skill_id, &rel_path)
}

#[tauri::command(async)]
pub fn write_skill_file(
    state: St,
    app: AppHandle,
    skill_id: String,
    rel_path: String,
    text: String,
    expected_hash: String,
) -> Result<SkillFileDto, CommandError> {
    emitting(
        &app,
        write_skill_file_impl(&state, &skill_id, &rel_path, &text, &expected_hash),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AgentPaths;
    use crate::skills::ops::OpsError;
    use std::fs;

    fn state(tmp: &std::path::Path) -> AppState {
        let paths = AgentPaths::from_home(&tmp.join("home"));
        fs::create_dir_all(&paths.claude_home).unwrap();
        fs::create_dir_all(&paths.codex_home).unwrap();
        AppState::with_home(paths, tmp.join("data"), tmp.join("home"))
    }

    fn template() -> InstallFromDto {
        InstallFromDto::Template {
            name: "demo".into(),
            description: "a demo".into(),
        }
    }

    fn claude_global() -> InstallTargetDto {
        serde_json::from_str(r#"{"tool":"claude","scope":{"type":"global"}}"#).unwrap()
    }

    fn install_demo(st: &AppState) -> SkillEntry {
        install_skill_impl(st, template(), vec![claude_global()], false)
            .unwrap()
            .remove(0)
    }

    #[test]
    fn install_then_list_includes_new_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        assert!(list_skills_impl(&st).unwrap().is_empty());
        let e = install_demo(&st);
        assert_eq!(e.name, "demo");
        let all = list_skills_impl(&st).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, e.id);
        // Installing again without overwrite reports the directory in the way.
        let err = install_skill_impl(&st, template(), vec![claude_global()], false).unwrap_err();
        assert_eq!(err.code, "exists");
        assert!(err.dir.is_some());
    }

    #[test]
    fn rel_path_traversal_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let e = install_demo(&st);
        fs::write(tmp.path().join("secret.txt"), "s").unwrap();
        for bad in [
            "../secret.txt",
            "ref/../../x",
            "..\\x",
            "/etc/passwd",
            "\\windows\\x",
            "C:/x",
            "C:\\x",
            "",
        ] {
            let err = read_skill_file_impl(&st, &e.id, bad).unwrap_err();
            assert_eq!(err.code, "invalid", "read {bad:?}");
            let err = write_skill_file_impl(&st, &e.id, bad, "x", "").unwrap_err();
            assert_eq!(err.code, "invalid", "write {bad:?}");
        }
        let abs = tmp.path().join("secret.txt");
        let err = read_skill_file_impl(&st, &e.id, abs.to_str().unwrap()).unwrap_err();
        assert_eq!(err.code, "invalid");
        assert_eq!(fs::read_to_string(&abs).unwrap(), "s");
        // A normal nested path works and normalizes "./".
        let c = read_skill_file_impl(&st, &e.id, "./SKILL.md").unwrap();
        assert!(c.text.contains("name: demo"));
        assert!(!c.binary);
    }

    #[test]
    fn git_dir_node_modules_and_reserved_names_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let e = install_demo(&st);
        for bad in [
            ".git/config",
            "ref/.GIT/hooks/pre-commit",
            "node_modules/x.js",
            "CON",
            "nul.txt",
            "ref/com1.md",
            "LPT9.log",
            "trailing.",
            "space ",
        ] {
            assert_eq!(
                read_skill_file_impl(&st, &e.id, bad).unwrap_err().code,
                "invalid",
                "read {bad:?}"
            );
            assert_eq!(
                write_skill_file_impl(&st, &e.id, bad, "x", "").unwrap_err().code,
                "invalid",
                "write {bad:?}"
            );
        }
        assert!(!std::path::Path::new(&e.dir).join(".git").exists());
        // Similar but legal names pass.
        write_skill_file_impl(&st, &e.id, "ref/console.md", "ok", "").unwrap();
        write_skill_file_impl(&st, &e.id, ".gitignore", "ok", "").unwrap();
    }

    #[test]
    fn list_skill_files_truncates() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let e = install_demo(&st);
        let d = std::path::Path::new(&e.dir).join("many");
        fs::create_dir_all(&d).unwrap();
        for i in 0..(MAX_LISTED_FILES + 5) {
            fs::write(d.join(format!("f{i}.txt")), "x").unwrap();
        }
        let l = list_skill_files_impl(&st, &e.id).unwrap();
        assert!(l.truncated);
        assert_eq!(l.files.len(), MAX_LISTED_FILES);
    }

    #[cfg(windows)]
    #[test]
    fn junction_escape_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let e = install_demo(&st);
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("f.txt"), "o").unwrap();
        let link = std::path::Path::new(&e.dir).join("jlink");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(status.status.success(), "{status:?}");
        assert_eq!(
            read_skill_file_impl(&st, &e.id, "jlink/f.txt").unwrap_err().code,
            "invalid"
        );
        assert_eq!(
            write_skill_file_impl(&st, &e.id, "jlink/new.txt", "x", "")
                .unwrap_err()
                .code,
            "invalid"
        );
        assert!(!outside.join("new.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let e = install_demo(&st);
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("f.txt"), "o").unwrap();
        std::os::unix::fs::symlink(&outside, std::path::Path::new(&e.dir).join("link")).unwrap();
        assert_eq!(
            read_skill_file_impl(&st, &e.id, "link/f.txt").unwrap_err().code,
            "invalid"
        );
        assert_eq!(
            write_skill_file_impl(&st, &e.id, "link/new.txt", "x", "")
                .unwrap_err()
                .code,
            "invalid"
        );
        assert!(!outside.join("new.txt").exists());
    }

    #[test]
    fn write_readonly_skill_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let d = st.paths.codex_home.join("skills").join(".system").join("sys");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("SKILL.md"), "---\nname: sys\ndescription: s\n---\nb\n").unwrap();
        let all = list_skills_impl(&st).unwrap();
        let e = all.iter().find(|e| e.name == "sys").unwrap();
        assert!(e.readonly);
        let r = read_skill_file_impl(&st, &e.id, "SKILL.md").unwrap();
        let err = write_skill_file_impl(&st, &e.id, "SKILL.md", "x", &r.hash).unwrap_err();
        assert_eq!(err.code, "read_only");
        assert!(fs::read_to_string(d.join("SKILL.md")).unwrap().contains("name: sys"));
    }

    #[test]
    fn write_skill_file_roundtrip_conflict_and_listing() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let e = install_demo(&st);
        let dir = std::path::Path::new(&e.dir);
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join(".git/HEAD"), "x").unwrap();
        fs::create_dir_all(dir.join("node_modules/m")).unwrap();
        fs::write(dir.join("node_modules/m/i.js"), "x").unwrap();
        fs::write(dir.join("big.bin"), vec![0u8; 2 * 1024 * 1024]).unwrap();
        fs::write(dir.join("bin.dat"), [0xff, 0xfe, 0x00, 0x80]).unwrap();
        let listing = list_skill_files_impl(&st, &e.id).unwrap();
        assert!(!listing.truncated);
        let files = listing.files;
        let names: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();
        assert!(names.contains(&"SKILL.md") && names.contains(&"big.bin"));
        assert!(!names.iter().any(|n| n.contains(".git") || n.contains("node_modules")));
        let big = read_skill_file_impl(&st, &e.id, "big.bin").unwrap();
        assert!(big.binary && big.too_large);
        let bin = read_skill_file_impl(&st, &e.id, "bin.dat").unwrap();
        assert!(bin.binary && !bin.too_large);
        assert_eq!(
            write_skill_file_impl(&st, &e.id, "big.bin", "x", "").unwrap_err().code,
            "invalid"
        );

        let c = read_skill_file_impl(&st, &e.id, "SKILL.md").unwrap();
        let w = write_skill_file_impl(&st, &e.id, "SKILL.md", "new body", &c.hash).unwrap();
        assert_eq!(fs::read_to_string(dir.join("SKILL.md")).unwrap(), "new body");
        let err = write_skill_file_impl(&st, &e.id, "SKILL.md", "again", &c.hash).unwrap_err();
        assert_eq!(err.code, "disk_changed");
        assert_eq!(err.current_hash.as_deref(), Some(w.hash.as_str()));
        // New files can be created inside the skill.
        write_skill_file_impl(&st, &e.id, "ref/new.md", "n", "").unwrap();
        assert!(dir.join("ref/new.md").is_file());
    }

    #[test]
    fn error_codes_serialize() {
        let cases: Vec<(OpsError, &str)> = vec![
            (OpsError::Exists { dir: "/d".into() }, "exists"),
            (OpsError::ReadOnly, "read_only"),
            (OpsError::GitUnavailable, "git_unavailable"),
            (
                OpsError::Git(crate::skills::git::GitError::Failed {
                    stderr: "boom".into(),
                }),
                "git_failed",
            ),
            (OpsError::LocalChanges, "local_changes"),
            (OpsError::Invalid { message: "m".into() }, "invalid"),
            (OpsError::Io { message: "m".into() }, "io"),
        ];
        for (e, code) in cases {
            let ce = CommandError::from(e);
            let v = serde_json::to_value(&ce).unwrap();
            assert_eq!(v["code"], code);
            assert!(v["message"].is_string());
            assert_eq!(v.get("dir").is_some(), code == "exists");
        }
        let ce = CommandError::from(OpsError::Git(crate::skills::git::GitError::Failed {
            stderr: "boom".into(),
        }));
        assert!(ce.message.contains("boom"));
        let ce = CommandError::from(OpsError::Exists { dir: "/d".into() });
        assert_eq!(ce.dir.as_deref(), Some("/d"));
    }

    #[test]
    fn delete_and_backups_flow() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let e = install_demo(&st);
        let b = delete_skill_impl(&st, &e.id).unwrap();
        assert!(!b.id.is_empty());
        assert!(list_skills_impl(&st).unwrap().is_empty());
        // Backups are keyed by the directory name; reinstall, then roll back.
        let e = install_demo(&st);
        assert_eq!(list_skill_backups_impl(&st, &e.id).unwrap().len(), 1);
        let back = rollback_skill_impl(&st, &e.id, &b.id).unwrap();
        assert_eq!(back.id, e.id);
        let _ = git_available_impl(&st);
    }

    struct FakeGit;
    impl crate::skills::git::GitRunner for FakeGit {
        fn available(&self) -> bool {
            true
        }
        fn ls_remote_head(&self, _: &str, _: Option<&str>) -> Result<String, GitError> {
            Ok("abcdef0123456789abcdef0123456789abcdef01".into())
        }
        fn shallow_clone(&self, _: &str, _: Option<&str>, dest: &Path) -> Result<String, GitError> {
            let d = dest.join("skills").join("one");
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("SKILL.md"), "---
name: one
description: first
---
b
").unwrap();
            fs::write(dest.join(crate::skills::git::COMPLETE_MARKER), "x").unwrap();
            Ok("abcdef0123456789abcdef0123456789abcdef01".into())
        }
        fn remote_url(&self, _: &Path) -> Option<String> {
            None
        }
        fn head_commit(&self, _: &Path) -> Option<String> {
            None
        }
        fn repo_root(&self, _: &Path) -> Option<PathBuf> {
            None
        }
        fn pull_ff_only(&self, _: &Path) -> Result<String, GitError> {
            Err(GitError::NotInstalled)
        }
    }

    #[test]
    fn inspect_lists_repo_skills_and_marks_preselected() {
        let tmp = tempfile::tempdir().unwrap();
        let mut st = state(tmp.path());
        st.git = Arc::new(FakeGit);
        let r = inspect_git_source_impl(&st, "https://github.com/o/r/tree/main/skills/one").unwrap();
        assert_eq!(r.url, "https://github.com/o/r");
        assert_eq!(r.commit, "abcdef0123456789abcdef0123456789abcdef01");
        assert_eq!(r.skills.len(), 1);
        assert_eq!(r.preselected.as_deref(), Some("skills/one"));
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("clone_url").is_some() && v.get("git_ref").is_some());
        // Second call reuses the complete cache.
        assert_eq!(inspect_git_source_impl(&st, "o/r").unwrap().preselected, None);
        let err = inspect_git_source_impl(&st, "not a repo").unwrap_err();
        assert_eq!(err.code, "invalid");
    }
}
