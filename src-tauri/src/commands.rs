//! Tauri commands. Logic lives in `*_impl(&AppState, ..)` functions so it can be
//! tested without a running app; the `#[tauri::command]` wrappers only delegate.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;
use tauri::State;

use crate::memory::analyze::{estimate_tokens, find_imports, ImportRef};
use crate::memory::codex_db::{read_codex_memories, CodexDbResult};
use crate::memory::io::{read_memory, write_memory, FileContent, WriteError};
use crate::memory::scan::scan_all;
use crate::memory::search::{search, SearchHit};
use crate::memory::snapshot::{SnapshotInfo, SnapshotStore};
use crate::model::{MemoryFile, MemoryIndex, MemoryKind};
use crate::paths::AgentPaths;
use crate::projects::{discover_projects, normalize_path};
use crate::settings::{load_settings, save_settings, Settings};
use crate::watcher::WatcherHandle;

const SEARCH_LIMIT: usize = 200;

/// Shared application state.
pub struct AppState {
    pub paths: AgentPaths,
    pub settings_path: PathBuf,
    pub snapshots: SnapshotStore,
    pub index: Mutex<MemoryIndex>,
    /// Serializes load-modify-save cycles on settings.json.
    settings_lock: Mutex<()>,
}

impl AppState {
    /// Builds the state with settings at `data_dir/settings.json` and snapshots
    /// at `data_dir/snapshots`, then scans once.
    pub fn new(paths: AgentPaths, data_dir: PathBuf) -> Self {
        let state = Self {
            paths,
            settings_path: data_dir.join("settings.json"),
            snapshots: SnapshotStore::new(data_dir.join("snapshots")),
            index: Mutex::new(MemoryIndex {
                global: vec![],
                projects: vec![],
            }),
            settings_lock: Mutex::new(()),
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

    fn snapshot_index(&self) -> MemoryIndex {
        lock(&self.index).clone()
    }

    fn home(&self) -> PathBuf {
        self.paths
            .claude_home
            .parent()
            .map(Path::to_path_buf)
            .or_else(dirs::home_dir)
            .unwrap_or_default()
    }

    /// Path of the indexed, non-Codex-DB memory file matching `path`, if any.
    fn find_allowed(&self, path: &str) -> Option<PathBuf> {
        let id = normalize_path(path);
        let idx = lock(&self.index);
        let all = idx
            .global
            .iter()
            .chain(idx.projects.iter().flat_map(|p| p.files.iter()));
        find_in(all, &id)
    }

    fn authorize(&self, path: &str, rescan_if_missing: bool) -> Result<PathBuf, CommandError> {
        if let Some(p) = self.find_allowed(path) {
            return Ok(p);
        }
        if rescan_if_missing {
            self.rescan();
            if let Some(p) = self.find_allowed(path) {
                return Ok(p);
            }
        }
        Err(CommandError::new(
            "forbidden",
            "path is not a known memory file",
        ))
    }
}

fn find_in<'a>(mut files: impl Iterator<Item = &'a MemoryFile>, id: &str) -> Option<PathBuf> {
    files
        .find(|f| f.id == id && f.kind != MemoryKind::CodexDb)
        .map(|f| PathBuf::from(&f.path))
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Error returned to the frontend as `{ code, message, current_hash? }`.
#[derive(Serialize, Debug, Clone)]
pub struct CommandError {
    /// "disk_changed" | "io" | "forbidden" | "invalid"
    pub code: String,
    pub message: String,
    /// Present for `disk_changed`: the hash currently on disk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_hash: Option<String>,
}

impl CommandError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            current_hash: None,
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

pub fn list_memories_impl(state: &AppState) -> MemoryIndex {
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
    Ok(String::from_utf8_lossy(&bytes).into_owned())
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
) -> Result<Settings, CommandError> {
    if let Some(t) = &theme {
        if !matches!(t.as_str(), "dark" | "light" | "system") {
            return Err(CommandError::new("invalid", format!("invalid theme: {t}")));
        }
    }
    modify_settings(state, |s| {
        if language.is_some() {
            s.language = language;
        }
        if theme.is_some() {
            s.theme = theme;
        }
    })
}

type St<'a> = State<'a, Arc<AppState>>;

#[tauri::command]
pub fn list_memories(state: St) -> MemoryIndex {
    list_memories_impl(&state)
}

#[tauri::command]
pub fn read_memory_file(state: St, path: String) -> Result<FileContentDto, CommandError> {
    read_memory_file_impl(&state, &path)
}

#[tauri::command]
pub fn write_memory_file(
    state: St,
    path: String,
    text: String,
    expected_hash: String,
) -> Result<FileContentDto, CommandError> {
    write_memory_file_impl(&state, &path, &text, &expected_hash)
}

#[tauri::command]
pub fn search_memories(state: St, query: String) -> Vec<SearchHit> {
    search_memories_impl(&state, &query)
}

#[tauri::command]
pub fn list_snapshots(state: St, path: String) -> Result<Vec<SnapshotInfo>, CommandError> {
    list_snapshots_impl(&state, &path)
}

#[tauri::command]
pub fn read_snapshot(state: St, path: String, id: String) -> Result<String, CommandError> {
    read_snapshot_impl(&state, &path, &id)
}

#[tauri::command]
pub fn read_codex_db(state: St) -> CodexDbResult {
    read_codex_db_impl(&state)
}

#[tauri::command]
pub fn add_manual_project(
    state: St,
    watcher: State<WatcherHandle>,
    path: String,
) -> Result<MemoryIndex, CommandError> {
    let idx = add_manual_project_impl(&state, &path)?;
    watcher.reload();
    Ok(idx)
}

#[tauri::command]
pub fn remove_manual_project(
    state: St,
    watcher: State<WatcherHandle>,
    path: String,
) -> Result<MemoryIndex, CommandError> {
    let idx = remove_manual_project_impl(&state, &path)?;
    watcher.reload();
    Ok(idx)
}

#[tauri::command]
pub fn get_settings(state: St) -> Settings {
    get_settings_impl(&state)
}

#[tauri::command]
pub fn update_ui_prefs(
    state: St,
    language: Option<String>,
    theme: Option<String>,
) -> Result<Settings, CommandError> {
    update_ui_prefs_impl(&state, language, theme)
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
        AppState::new(paths, tmp.join("data"))
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
    fn ui_prefs_validate_and_persist() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let s = update_ui_prefs_impl(&st, Some("zh-CN".into()), Some("light".into())).unwrap();
        assert_eq!(s.theme.as_deref(), Some("light"));
        let s = update_ui_prefs_impl(&st, None, Some("system".into())).unwrap();
        assert_eq!(s.language.as_deref(), Some("zh-CN"));
        let err = update_ui_prefs_impl(&st, None, Some("neon".into())).unwrap_err();
        assert_eq!(err.code, "invalid");
        assert_eq!(get_settings_impl(&st).theme.as_deref(), Some("system"));
    }
}
