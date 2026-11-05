//! Tauri commands. Logic lives in `*_impl(&AppState, ..)` functions so it can be
//! tested without a running app; the `#[tauri::command]` wrappers only delegate.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;
use tauri::State;

use crate::memory::analyze::{estimate_tokens, find_imports, ImportRef};
use crate::memory::codex_db::{read_codex_memories, CodexDbResult};
use crate::memory::io::{decode, read_memory, write_memory, FileContent, WriteError};
use crate::memory::scan::scan_all;
use crate::memory::search::{search, SearchHit};
use crate::memory::snapshot::{SnapshotInfo, SnapshotStore};
use crate::model::{MemoryFile, MemoryIndex, MemoryKind};
use crate::paths::AgentPaths;
use crate::projects::{discover_projects, normalize_path};
use crate::skills::git::{GitRunner, SystemGit};
use crate::settings::{load_settings, save_settings, Settings};
use crate::watcher::WatcherHandle;

const SEARCH_LIMIT: usize = 200;

/// Shared application state.
pub struct AppState {
    pub paths: AgentPaths,
    /// The user's real home directory (used to resolve `@~/` imports); not
    /// derived from `claude_home`, which `CLAUDE_CONFIG_DIR` can relocate.
    pub home: PathBuf,
    pub settings_path: PathBuf,
    pub snapshots: SnapshotStore,
    pub index: Mutex<MemoryIndex>,
    /// Serializes load-modify-save cycles on settings.json.
    settings_lock: Mutex<()>,
    /// Serializes writes so two concurrent writes cannot both pass the hash check.
    write_lock: Mutex<()>,
    /// App data directory (skill backups, git cache, registry live below it).
    pub data_dir: PathBuf,
    /// Git access for skill install / update; tests may replace it.
    pub git: Arc<dyn GitRunner>,
    /// Serializes every mutating skill operation.
    pub skills_lock: Mutex<()>,
    /// Serializes git-cache check / clone / cleanup (shared with the ops layer).
    pub cache_lock: Arc<Mutex<()>>,
}

impl AppState {
    /// Builds the state with settings at `data_dir/settings.json` and snapshots
    /// at `data_dir/snapshots`, then scans once.
    pub fn new(paths: AgentPaths, data_dir: PathBuf) -> Self {
        Self::with_home(paths, data_dir, dirs::home_dir().unwrap_or_default())
    }

    /// Like `new`, with an explicit home directory (tests inject a temp one).
    pub fn with_home(paths: AgentPaths, data_dir: PathBuf, home: PathBuf) -> Self {
        let state = Self {
            paths,
            home,
            settings_path: data_dir.join("settings.json"),
            data_dir: data_dir.clone(),
            git: Arc::new(SystemGit::new()),
            skills_lock: Mutex::new(()),
            cache_lock: Arc::new(Mutex::new(())),
            snapshots: SnapshotStore::new(data_dir.join("snapshots")),
            index: Mutex::new(MemoryIndex {
                global: vec![],
                projects: vec![],
            }),
            settings_lock: Mutex::new(()),
            write_lock: Mutex::new(()),
        };
        state.rescan();
        state
    }

    /// Re-discovers projects and rebuilds the index.
    pub fn rescan(&self) {
        let settings = load_settings(&self.settings_path);
        let projects = discover_projects(&self.paths, &settings.manual_projects);
        let fresh = scan_all(&self.paths, &projects);
        *lock(&self.index) = fresh;
    }

    pub(crate) fn snapshot_index(&self) -> MemoryIndex {
        lock(&self.index).clone()
    }

    fn home(&self) -> PathBuf {
        self.home.clone()
    }

    /// Path of the indexed, non-Codex-DB memory file matching `path`, if any.
    /// Returns the path and whether it is a project-scoped file.
    fn find_allowed(&self, path: &str) -> Option<(PathBuf, bool)> {
        let id = normalize_path(path);
        let idx = lock(&self.index);
        if let Some(p) = find_in(idx.global.iter(), &id) {
            return Some((p, false));
        }
        idx.projects
            .iter()
            .find_map(|pm| find_in(pm.files.iter(), &id).map(|p| (p, true)))
    }

    fn authorize(&self, path: &str, for_write: bool) -> Result<PathBuf, CommandError> {
        let mut found = self.find_allowed(path);
        if found.is_none() && for_write {
            self.rescan();
            found = self.find_allowed(path);
        }
        match found {
            None => Err(CommandError::new(
                "forbidden",
                "path is not a known memory file",
            )),
            // Never recreate a deleted project directory.
            Some((p, true)) if for_write && !p.parent().is_some_and(|d| d.exists()) => {
                Err(CommandError::new("invalid", "parent directory does not exist"))
            }
            Some((p, _)) => Ok(p),
        }
    }
}

fn find_in<'a>(mut files: impl Iterator<Item = &'a MemoryFile>, id: &str) -> Option<PathBuf> {
    files
        .find(|f| f.id == id && f.kind != MemoryKind::CodexDb)
        .map(|f| PathBuf::from(&f.path))
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Error returned to the frontend as `{ code, message, current_hash?, dir? }`.
#[derive(Serialize, Debug, Clone)]
pub struct CommandError {
    /// "disk_changed" | "io" | "forbidden" | "invalid"
    pub code: String,
    pub message: String,
    /// Present for `disk_changed`: the hash currently on disk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_hash: Option<String>,
    /// Present for skill `exists` errors: the directory that is in the way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    /// Present for skill `exists` errors: every directory that is in the way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirs: Option<Vec<String>>,
    /// Present for skill `local_changes`: no baseline exists, local edits cannot be ruled out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown: Option<bool>,
}

impl CommandError {
    pub(crate) fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            current_hash: None,
            dir: None,
            dirs: None,
            unknown: None,
        }
    }
}

impl From<io::Error> for CommandError {
    fn from(e: io::Error) -> Self {
        Self::new("io", e.to_string())
    }
}

impl From<WriteError> for CommandError {
    fn from(e: WriteError) -> Self {
        match e {
            WriteError::DiskChanged { current_hash } => Self {
                code: "disk_changed".into(),
                message: "file changed on disk".into(),
                current_hash: Some(current_hash),
                dir: None,
                dirs: None,
                unknown: None,
            },
            WriteError::Io(e) => e.into(),
        }
    }
}

/// File content plus derived data.
#[derive(Serialize, Debug, Clone)]
pub struct FileContentDto {
    pub text: String,
    pub hash: String,
    pub has_bom: bool,
    pub crlf: bool,
    pub tokens: u32,
    pub imports: Vec<ImportRef>,
}

fn to_dto(state: &AppState, path: &Path, c: FileContent) -> FileContentDto {
    let base = path.parent().unwrap_or(Path::new(""));
    FileContentDto {
        tokens: estimate_tokens(&c.text),
        imports: find_imports(&c.text, base, &state.home()),
        text: c.text,
        hash: c.hash,
        has_bom: c.has_bom,
        crlf: c.crlf,
    }
}

/// Returns the cached index (rescans happen at startup, on watcher events and
/// on manual-project changes).
pub fn list_memories_impl(state: &AppState) -> MemoryIndex {
    state.snapshot_index()
}

/// Forces a rescan, then returns the fresh index.
pub fn refresh_memories_impl(state: &AppState) -> MemoryIndex {
    state.rescan();
    state.snapshot_index()
}

pub fn read_memory_file_impl(state: &AppState, path: &str) -> Result<FileContentDto, CommandError> {
    let p = state.authorize(path, false)?;
    let c = read_memory(&p)?;
    Ok(to_dto(state, &p, c))
}

pub fn write_memory_file_impl(
    state: &AppState,
    path: &str,
    text: &str,
    expected_hash: &str,
) -> Result<FileContentDto, CommandError> {
    let _w = lock(&state.write_lock);
    let p = state.authorize(path, true)?;
    let c = write_memory(&p, text, expected_hash, &state.snapshots)?;
    Ok(to_dto(state, &p, c))
}

pub fn search_memories_impl(state: &AppState, query: &str) -> Vec<SearchHit> {
    search(&state.snapshot_index(), query, SEARCH_LIMIT)
}

pub fn list_snapshots_impl(
    state: &AppState,
    path: &str,
) -> Result<Vec<SnapshotInfo>, CommandError> {
    let p = state.authorize(path, false)?;
    Ok(state.snapshots.list(&p)?)
}

pub fn read_snapshot_impl(state: &AppState, path: &str, id: &str) -> Result<String, CommandError> {
    let p = state.authorize(path, false)?;
    let bytes = state.snapshots.read(&p, id)?;
    Ok(decode(&bytes)?.text)
}

pub fn read_codex_db_impl(state: &AppState) -> CodexDbResult {
    read_codex_memories(&state.paths.codex_home.join("memories_1.sqlite"))
}

fn modify_settings(
    state: &AppState,
    f: impl FnOnce(&mut Settings),
) -> Result<Settings, CommandError> {
    let _g = lock(&state.settings_lock);
    let mut s = load_settings(&state.settings_path);
    f(&mut s);
    save_settings(&state.settings_path, &s)?;
    Ok(s)
}

pub fn add_manual_project_impl(state: &AppState, path: &str) -> Result<MemoryIndex, CommandError> {
    let id = normalize_path(path);
    modify_settings(state, |s| {
        if !s.manual_projects.iter().any(|p| normalize_path(p) == id) {
            s.manual_projects.push(path.to_string());
        }
    })?;
    state.rescan();
    Ok(state.snapshot_index())
}

pub fn remove_manual_project_impl(
    state: &AppState,
    path: &str,
) -> Result<MemoryIndex, CommandError> {
    let id = normalize_path(path);
    modify_settings(state, |s| {
        s.manual_projects.retain(|p| normalize_path(p) != id);
    })?;
    state.rescan();
    Ok(state.snapshot_index())
}

pub fn get_settings_impl(state: &AppState) -> Settings {
    load_settings(&state.settings_path)
}

pub fn update_ui_prefs_impl(
    state: &AppState,
    language: Option<String>,
    theme: Option<String>,
    accent: Option<String>,
) -> Result<Settings, CommandError> {
    if let Some(a) = &accent {
        if !matches!(a.as_str(), "green" | "blue" | "violet" | "orange" | "teal") {
            return Err(CommandError::new("invalid", format!("invalid accent: {a}")));
        }
    }
    if let Some(t) = &theme {
        if !matches!(t.as_str(), "dark" | "light" | "system") {
            return Err(CommandError::new("invalid", format!("invalid theme: {t}")));
        }
    }
    modify_settings(state, |s| {
        match language.as_deref() {
            None => {}
            // "system" clears the stored language so the UI follows the system again.
            Some("system") => s.language = None,
            Some(_) => s.language = language,
        }
        if theme.is_some() {
            s.theme = theme;
        }
        if accent.is_some() {
            s.accent = accent;
        }
    })
}

type St<'a> = State<'a, Arc<AppState>>;

#[tauri::command(async)]
pub fn list_memories(state: St) -> MemoryIndex {
    list_memories_impl(&state)
}

#[tauri::command(async)]
pub fn refresh_memories(state: St) -> MemoryIndex {
    refresh_memories_impl(&state)
}

#[tauri::command(async)]
pub fn read_memory_file(state: St, path: String) -> Result<FileContentDto, CommandError> {
    read_memory_file_impl(&state, &path)
}

#[tauri::command(async)]
pub fn write_memory_file(
    state: St,
    path: String,
    text: String,
    expected_hash: String,
) -> Result<FileContentDto, CommandError> {
    write_memory_file_impl(&state, &path, &text, &expected_hash)
}

#[tauri::command(async)]
pub fn search_memories(state: St, query: String) -> Vec<SearchHit> {
    search_memories_impl(&state, &query)
}

#[tauri::command(async)]
pub fn list_snapshots(state: St, path: String) -> Result<Vec<SnapshotInfo>, CommandError> {
    list_snapshots_impl(&state, &path)
}

#[tauri::command(async)]
pub fn read_snapshot(state: St, path: String, id: String) -> Result<String, CommandError> {
    read_snapshot_impl(&state, &path, &id)
}

#[tauri::command(async)]
pub fn read_codex_db(state: St) -> CodexDbResult {
    read_codex_db_impl(&state)
}

#[tauri::command(async)]
pub fn add_manual_project(
    state: St,
    watcher: State<WatcherHandle>,
    path: String,
) -> Result<MemoryIndex, CommandError> {
    let idx = add_manual_project_impl(&state, &path)?;
    watcher.reload();
    Ok(idx)
}

#[tauri::command(async)]
pub fn remove_manual_project(
    state: St,
    watcher: State<WatcherHandle>,
    path: String,
) -> Result<MemoryIndex, CommandError> {
    let idx = remove_manual_project_impl(&state, &path)?;
    watcher.reload();
    Ok(idx)
}

#[tauri::command(async)]
pub fn get_settings(state: St) -> Settings {
    get_settings_impl(&state)
}

#[tauri::command(async)]
pub fn update_ui_prefs(
    state: St,
    language: Option<String>,
    theme: Option<String>,
    accent: Option<String>,
) -> Result<Settings, CommandError> {
    update_ui_prefs_impl(&state, language, theme, accent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MemoryKind;
    use crate::paths::AgentPaths;
    use std::fs;

    fn state(tmp: &std::path::Path) -> AppState {
        let paths = AgentPaths::from_home(&tmp.join("home"));
        fs::create_dir_all(&paths.claude_home).unwrap();
        fs::create_dir_all(&paths.codex_home).unwrap();
        AppState::with_home(paths, tmp.join("data"), tmp.join("home"))
    }

    #[test]
    fn write_outside_index_is_forbidden() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let target = tmp.path().join("evil.txt");
        let err = write_memory_file_impl(&st, target.to_str().unwrap(), "x", "").unwrap_err();
        assert_eq!(err.code, "forbidden");
        assert!(!target.exists());
        let err = read_memory_file_impl(&st, target.to_str().unwrap()).unwrap_err();
        assert_eq!(err.code, "forbidden");
    }

    #[test]
    fn write_codex_db_is_forbidden() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let db = st.paths.codex_home.join("memories_1.sqlite");
        fs::write(&db, b"x").unwrap();
        st.rescan();
        let idx = st.index.lock().unwrap();
        assert!(idx.global.iter().any(|f| f.kind == MemoryKind::CodexDb));
        drop(idx);
        let err = write_memory_file_impl(&st, db.to_str().unwrap(), "y", "").unwrap_err();
        assert_eq!(err.code, "forbidden");
        assert!(read_memory_file_impl(&st, db.to_str().unwrap()).is_err());
        assert_eq!(fs::read(&db).unwrap(), b"x");
    }

    #[test]
    fn write_in_index_roundtrip_and_disk_changed() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let md = st.paths.claude_home.join("CLAUDE.md");
        fs::write(&md, "hello").unwrap();
        st.rescan();
        let c = read_memory_file_impl(&st, md.to_str().unwrap()).unwrap();
        assert_eq!(c.tokens, 2);
        let w = write_memory_file_impl(&st, md.to_str().unwrap(), "new", &c.hash).unwrap();
        assert_eq!(fs::read_to_string(&md).unwrap(), "new");
        let err = write_memory_file_impl(&st, md.to_str().unwrap(), "again", &c.hash).unwrap_err();
        assert_eq!(err.code, "disk_changed");
        assert_eq!(err.current_hash.as_deref(), Some(w.hash.as_str()));
        assert_eq!(list_snapshots_impl(&st, md.to_str().unwrap()).unwrap().len(), 1);
    }

    #[test]
    fn manual_project_add_remove_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let proj = tmp.path().join("myproj");
        fs::create_dir_all(&proj).unwrap();
        let idx = add_manual_project_impl(&st, proj.to_str().unwrap()).unwrap();
        let pid = crate::projects::normalize_path(proj.to_str().unwrap());
        assert!(idx.projects.iter().any(|p| p.project.id == pid));
        let saved = crate::settings::load_settings(&st.settings_path);
        assert_eq!(saved.manual_projects.len(), 1);
        let idx = remove_manual_project_impl(&st, proj.to_str().unwrap()).unwrap();
        assert!(!idx.projects.iter().any(|p| p.project.id == pid));
        assert!(crate::settings::load_settings(&st.settings_path).manual_projects.is_empty());
    }

    #[test]
    fn list_is_cached_and_refresh_rescans() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let md = st.paths.claude_home.join("CLAUDE.md");
        fs::write(&md, "hi").unwrap();
        let cached = list_memories_impl(&st);
        assert!(cached.global.iter().any(|f| f.path.ends_with("CLAUDE.md") && !f.exists));
        let fresh = refresh_memories_impl(&st);
        assert!(fresh.global.iter().any(|f| f.path.ends_with("CLAUDE.md") && f.exists));
    }

    #[test]
    fn read_snapshot_strips_bom_and_crlf() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let md = st.paths.claude_home.join("CLAUDE.md");
        fs::write(&md, b"\xEF\xBB\xBFa\r\nb\r\n").unwrap();
        st.rescan();
        let c = read_memory_file_impl(&st, md.to_str().unwrap()).unwrap();
        write_memory_file_impl(&st, md.to_str().unwrap(), "c\n", &c.hash).unwrap();
        let snaps = list_snapshots_impl(&st, md.to_str().unwrap()).unwrap();
        let text = read_snapshot_impl(&st, md.to_str().unwrap(), &snaps[0].id).unwrap();
        assert_eq!(text, c.text);
        assert_eq!(text, "a\nb\n");
    }

    fn missing_project_with_auto_memory(st: &AppState, base: &std::path::Path) -> (PathBuf, PathBuf) {
        let proj = base.join("gone");
        let enc = st.paths.claude_home.join("projects").join("enc");
        fs::create_dir_all(enc.join("memory")).unwrap();
        let cwd = proj.to_string_lossy().replace('\\', "/");
        fs::write(
            enc.join("s.jsonl"),
            format!("{{\"type\":\"user\",\"cwd\":\"{cwd}\"}}\n"),
        )
        .unwrap();
        let target = enc.join("memory").join("MEMORY.md");
        fs::write(&target, "m").unwrap();
        (proj, target)
    }

    #[test]
    fn auto_memory_of_missing_project_is_writable() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let (_proj, target) = missing_project_with_auto_memory(&st, tmp.path());
        let idx = refresh_memories_impl(&st);
        assert!(idx.projects.iter().any(|p| !p.project.exists && !p.files.is_empty()));
        let c = read_memory_file_impl(&st, target.to_str().unwrap()).unwrap();
        write_memory_file_impl(&st, target.to_str().unwrap(), "new", &c.hash).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
    }

    #[test]
    fn project_file_with_missing_parent_is_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let (proj, _target) = missing_project_with_auto_memory(&st, tmp.path());
        refresh_memories_impl(&st);
        // Inject an entry for <missing project>/CLAUDE.md (scan does not list it).
        let file = proj.join("CLAUDE.md");
        {
            let mut idx = st.index.lock().unwrap();
            let mut f = idx.projects[0].files[0].clone();
            f.id = crate::projects::normalize_path(file.to_str().unwrap());
            f.path = file.to_string_lossy().into_owned();
            f.kind = MemoryKind::ClaudeInstructions;
            idx.projects[0].files.push(f);
        }
        let err = write_memory_file_impl(&st, file.to_str().unwrap(), "x", "").unwrap_err();
        assert_eq!(err.code, "invalid");
        assert!(!proj.exists());
    }

    #[test]
    fn language_system_clears_but_theme_system_is_stored() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        update_ui_prefs_impl(&st, Some("en".into()), Some("system".into()), None).unwrap();
        let s = update_ui_prefs_impl(&st, Some("system".into()), None, None).unwrap();
        assert_eq!(s.language, None);
        assert_eq!(s.theme.as_deref(), Some("system"));
    }

    #[test]
    fn update_ui_prefs_accent_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        for a in ["green", "blue", "violet", "orange", "teal"] {
            let s = update_ui_prefs_impl(&st, None, None, Some(a.into())).unwrap();
            assert_eq!(s.accent.as_deref(), Some(a));
            assert_eq!(get_settings_impl(&st).accent.as_deref(), Some(a));
        }
        // Other fields are untouched by an accent-only update.
        update_ui_prefs_impl(&st, Some("en".into()), Some("light".into()), None).unwrap();
        let s = get_settings_impl(&st);
        assert_eq!(s.accent.as_deref(), Some("teal"));
    }

    #[test]
    fn update_ui_prefs_rejects_unknown_accent() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        update_ui_prefs_impl(&st, None, None, Some("blue".into())).unwrap();
        let err = update_ui_prefs_impl(&st, None, None, Some("pink".into())).unwrap_err();
        assert_eq!(err.code, "invalid");
        assert_eq!(get_settings_impl(&st).accent.as_deref(), Some("blue"));
    }

    #[test]
    fn ui_prefs_validate_and_persist() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let s = update_ui_prefs_impl(&st, Some("zh-CN".into()), Some("light".into()), None).unwrap();
        assert_eq!(s.theme.as_deref(), Some("light"));
        let s = update_ui_prefs_impl(&st, None, Some("system".into()), None).unwrap();
        assert_eq!(s.language.as_deref(), Some("zh-CN"));
        let err = update_ui_prefs_impl(&st, None, Some("neon".into()), None).unwrap_err();
        assert_eq!(err.code, "invalid");
        assert_eq!(get_settings_impl(&st).theme.as_deref(), Some("system"));
    }

    #[test]
    fn imports_resolve_against_real_home_not_claude_home_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("realhome");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("shared.md"), "x").unwrap();
        // claude_home lives elsewhere (as with CLAUDE_CONFIG_DIR).
        let paths = AgentPaths {
            claude_home: tmp.path().join("elsewhere").join("cfg"),
            codex_home: tmp.path().join("codex"),
        };
        fs::create_dir_all(&paths.claude_home).unwrap();
        fs::create_dir_all(&paths.codex_home).unwrap();
        let md = paths.claude_home.join("CLAUDE.md");
        fs::write(&md, "@~/shared.md
").unwrap();
        let st = AppState::with_home(paths, tmp.path().join("data"), home.clone());
        let dto = read_memory_file_impl(&st, &md.to_string_lossy()).unwrap();
        assert_eq!(dto.imports.len(), 1);
        assert!(dto.imports[0].exists, "{:?}", dto.imports[0]);
        assert!(dto.imports[0].resolved.contains("realhome"));
    }
}
