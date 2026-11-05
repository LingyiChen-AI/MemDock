//! File watcher: watches the memory locations and emits `memory-changed`
//! (debounced by 300ms) with the normalized ids of the changed files.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::commands::AppState;
use crate::paths::AgentPaths;
use crate::projects::normalize_path;

const DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Serialize, Clone, Debug)]
struct ChangedPayload {
    paths: Vec<String>,
}

enum Msg {
    Fs(Vec<PathBuf>),
    Reload,
}

/// Handle used to ask the watcher thread to re-register its watches.
pub struct WatcherHandle {
    tx: Sender<Msg>,
}

impl WatcherHandle {
    /// Re-registers watches (call after the project list changed).
    pub fn reload(&self) {
        let _ = self.tx.send(Msg::Reload);
    }
}

/// Whether a changed `path` is one of the memory files/directories we care about.
/// `project_roots` are the project directories (raw paths).
pub fn is_relevant(path: &Path, paths: &AgentPaths, project_roots: &[String]) -> bool {
    let n = normalize_path(&path.to_string_lossy());
    let claude = normalize_path(&paths.claude_home.to_string_lossy());
    let codex = normalize_path(&paths.codex_home.to_string_lossy());

    if n == normalize_path(&format!("{claude}/CLAUDE.md"))
        || n == normalize_path(&format!("{codex}/AGENTS.md"))
    {
        return true;
    }
    if let Some(rest) = n.strip_prefix(&format!("{claude}/projects/")) {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() >= 2 && parts[1] == "memory" {
            return true;
        }
    }
    project_roots.iter().any(|root| {
        let r = normalize_path(root);
        [
            "CLAUDE.md",
            "CLAUDE.local.md",
            "AGENTS.md",
            ".claude/CLAUDE.md",
            ".claude/CLAUDE.local.md",
        ]
        .iter()
        .any(|f| n == normalize_path(&format!("{r}/{f}")))
    })
}

fn project_roots(state: &AppState) -> Vec<String> {
    let idx = state.index.lock().unwrap_or_else(|e| e.into_inner());
    idx.projects
        .iter()
        .filter(|p| p.project.exists)
        .map(|p| p.project.path.clone())
        .collect()
}

fn build_watcher(tx: &Sender<Msg>, state: &AppState) -> Option<RecommendedWatcher> {
    let tx = tx.clone();
    let mut w = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(ev) = res {
            if !matches!(ev.kind, EventKind::Access(_)) {
                let _ = tx.send(Msg::Fs(ev.paths));
            }
        }
    })
    .ok()?;
    let mut watch = |p: &Path, mode: RecursiveMode| {
        if p.is_dir() {
            let _ = w.watch(p, mode);
        }
    };
    watch(&state.paths.claude_home, RecursiveMode::Recursive);
    watch(&state.paths.codex_home, RecursiveMode::NonRecursive);
    for root in project_roots(state) {
        let root = PathBuf::from(root);
        watch(&root, RecursiveMode::NonRecursive);
        watch(&root.join(".claude"), RecursiveMode::NonRecursive);
    }
    Some(w)
}

/// Starts the watcher thread and returns the handle to manage in Tauri state.
pub fn start(app: AppHandle, state: Arc<AppState>) -> WatcherHandle {
    let (tx, rx) = mpsc::channel::<Msg>();
    let thread_tx = tx.clone();
    std::thread::spawn(move || {
        let mut _watcher = build_watcher(&thread_tx, &state);
        let mut pending: BTreeSet<PathBuf> = BTreeSet::new();
        loop {
            match rx.recv() {
                Err(_) => return,
                Ok(Msg::Reload) => _watcher = build_watcher(&thread_tx, &state),
                Ok(Msg::Fs(p)) => pending.extend(p),
            }
            // Debounce: keep collecting until 300ms pass without events.
            loop {
                match rx.recv_timeout(DEBOUNCE) {
                    Ok(Msg::Fs(p)) => pending.extend(p),
                    Ok(Msg::Reload) => _watcher = build_watcher(&thread_tx, &state),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            let roots = project_roots(&state);
            let changed: BTreeSet<String> = std::mem::take(&mut pending)
                .iter()
                .filter(|p| is_relevant(p, &state.paths, &roots))
                .map(|p| normalize_path(&p.to_string_lossy()))
                .collect();
            if changed.is_empty() {
                continue;
            }
            state.rescan();
            let _ = app.emit(
                "memory-changed",
                ChangedPayload {
                    paths: changed.into_iter().collect(),
                },
            );
        }
    });
    
    WatcherHandle { tx }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_relevant_filters_paths() {
        let paths = AgentPaths::from_home(Path::new("/h"));
        let roots = vec!["/work/app".to_string()];
        let yes = |p: &str| is_relevant(Path::new(p), &paths, &roots);
        assert!(yes("/h/.claude/CLAUDE.md"));
        assert!(yes("/h/.claude/projects/-work-app/memory/MEMORY.md"));
        assert!(yes("/h/.claude/projects/-work-app/memory"));
        assert!(yes("/h/.codex/AGENTS.md"));
        assert!(yes("/work/app/CLAUDE.md"));
        assert!(yes("/work/app/.claude/CLAUDE.local.md"));
        assert!(!yes("/h/.claude/projects/-work-app/session.jsonl"));
        assert!(!yes("/h/.claude/settings.json"));
        assert!(!yes("/work/app/src/main.rs"));
        assert!(!yes("/other/CLAUDE.md"));
    }
}
