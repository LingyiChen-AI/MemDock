//! File watcher: watches the memory locations and emits `memory-changed`, and
//! the skills roots (emits `skills-changed`), both debounced by 300ms, with the
//! normalized ids of the changed files.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::commands::AppState;
use crate::paths::AgentPaths;
use crate::commands_skills::SkillsChangedPayload;
use crate::projects::normalize_path;
use crate::skills::fsops::is_leftover_name;

const DEBOUNCE: Duration = Duration::from_millis(300);
/// Upper bound on how long a continuous stream of events can postpone a flush.
const MAX_WAIT: Duration = Duration::from_secs(1);

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

/// Skills root directories: global ones for both tools, the shared
/// `~/.agents/skills`, and each project's `.claude/skills` / `.agents/skills`.
pub fn skill_roots(paths: &AgentPaths, home: &Path, project_roots: &[String]) -> Vec<PathBuf> {
    let mut v = vec![
        paths.claude_home.join("skills"),
        paths.codex_home.join("skills"),
        home.join(".agents").join("skills"),
    ];
    for r in project_roots {
        let r = Path::new(r);
        v.push(r.join(".claude").join("skills"));
        v.push(r.join(".agents").join("skills"));
        v.push(r.join(".codex").join("skills"));
    }
    v
}

/// Whether a changed path adds / removes a directory that needs its own watch:
/// a skills root or a skill directory directly below one (depth <= 1).
/// Plain file edits (e.g. `SKILL.md`) never need a rebuild.
pub fn needs_rewatch(
    path: &Path,
    paths: &AgentPaths,
    home: &Path,
    project_roots: &[String],
) -> bool {
    let n = normalize_path(&path.to_string_lossy());
    skill_roots(paths, home, project_roots).iter().any(|root| {
        let r = normalize_path(&root.to_string_lossy());
        if n == r {
            return true;
        }
        n.strip_prefix(&format!("{r}/")).is_some_and(|rest| {
            !rest.contains('/') && !is_leftover_name(rest) && rest != ".git" && rest != "node_modules"
        }) && !path.is_file()
    })
}

/// Whether a changed `path` is a skills root or lies at most two levels below
/// one (the skill directory, or a file directly inside it). `.git`,
/// `node_modules` and the app's own leftover temp names are ignored.
pub fn is_skill_relevant(
    path: &Path,
    paths: &AgentPaths,
    home: &Path,
    project_roots: &[String],
) -> bool {
    let n = normalize_path(&path.to_string_lossy());
    skill_roots(paths, home, project_roots).iter().any(|root| {
        let r = normalize_path(&root.to_string_lossy());
        if n == r {
            return true;
        }
        let Some(rest) = n.strip_prefix(&format!("{r}/")) else {
            return false;
        };
        let parts: Vec<&str> = rest.split('/').collect();
        parts.len() <= 2
            && parts
                .iter()
                .all(|p| *p != ".git" && *p != "node_modules" && !is_leftover_name(p))
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
    let roots = project_roots(state);
    for root in &roots {
        let root = PathBuf::from(root);
        watch(&root, RecursiveMode::NonRecursive);
        watch(&root.join(".claude"), RecursiveMode::NonRecursive);
        watch(&root.join(".agents"), RecursiveMode::NonRecursive);
    }
    // Skills: each root and each skill directory directly below it, non-recursive.
    // The root below claude_home is already covered by its recursive watch.
    watch(&state.home.join(".agents"), RecursiveMode::NonRecursive);
    let claude_skills = normalize_path(&state.paths.claude_home.join("skills").to_string_lossy());
    for root in skill_roots(&state.paths, &state.home, &roots) {
        if normalize_path(&root.to_string_lossy()) == claude_skills {
            continue;
        }
        watch(&root, RecursiveMode::NonRecursive);
        if let Ok(rd) = std::fs::read_dir(&root) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    watch(&p, RecursiveMode::NonRecursive);
                }
            }
        }
    }
    Some(w)
}

/// Pure debounce state: only relevant events count; the flush is due 300ms
/// after the last relevant event, but never later than `max_wait` after the first.
#[derive(Debug, Clone)]
pub struct Debouncer {
    debounce: Duration,
    max_wait: Duration,
    first: Option<Instant>,
    last: Option<Instant>,
}

impl Debouncer {
    pub fn new(debounce: Duration, max_wait: Duration) -> Self {
        Self {
            debounce,
            max_wait,
            first: None,
            last: None,
        }
    }

    /// Records an event. Irrelevant events are ignored and never move the deadline.
    pub fn offer(&mut self, relevant: bool, now: Instant) {
        if !relevant {
            return;
        }
        self.first.get_or_insert(now);
        self.last = Some(now);
    }

    /// When the pending batch must be flushed; `None` if nothing is pending.
    pub fn deadline(&self) -> Option<Instant> {
        let (first, last) = (self.first?, self.last?);
        Some((last + self.debounce).min(first + self.max_wait))
    }

    pub fn reset(&mut self) {
        self.first = None;
        self.last = None;
    }
}

/// Starts the watcher thread and returns the handle to manage in Tauri state.
pub fn start(app: AppHandle, state: Arc<AppState>) -> WatcherHandle {
    let (tx, rx) = mpsc::channel::<Msg>();
    let thread_tx = tx.clone();
    std::thread::spawn(move || {
        let mut _watcher = build_watcher(&thread_tx, &state);
        let mut roots = project_roots(&state);
        let mut pending: BTreeSet<PathBuf> = BTreeSet::new();
        let mut pending_skills: BTreeSet<PathBuf> = BTreeSet::new();
        let mut rewatch = false;
        let mut deb = Debouncer::new(DEBOUNCE, MAX_WAIT);
        loop {
            let msg = match deb.deadline() {
                Some(dl) => match rx.recv_timeout(dl.saturating_duration_since(Instant::now())) {
                    Ok(m) => Some(m),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                },
                None => match rx.recv() {
                    Ok(m) => Some(m),
                    Err(_) => return,
                },
            };
            match msg {
                Some(Msg::Fs(paths)) => {
                    let now = Instant::now();
                    for p in paths {
                        let memory = is_relevant(&p, &state.paths, &roots);
                        let skill = is_skill_relevant(&p, &state.paths, &state.home, &roots);
                        deb.offer(memory || skill, now);
                        if skill {
                            rewatch |= needs_rewatch(&p, &state.paths, &state.home, &roots);
                            pending_skills.insert(p.clone());
                        }
                        if memory {
                            pending.insert(p);
                        }
                    }
                }
                Some(Msg::Reload) => {
                    _watcher = build_watcher(&thread_tx, &state);
                    roots = project_roots(&state);
                }
                None => {
                    deb.reset();
                    let changed: BTreeSet<String> = std::mem::take(&mut pending)
                        .iter()
                        .map(|p| normalize_path(&p.to_string_lossy()))
                        .collect();
                    let skills_changed: BTreeSet<String> = std::mem::take(&mut pending_skills)
                        .iter()
                        .map(|p| normalize_path(&p.to_string_lossy()))
                        .collect();
                    if !changed.is_empty() {
                        state.rescan();
                        roots = project_roots(&state);
                        let _ = app.emit(
                            "memory-changed",
                            ChangedPayload {
                                paths: changed.into_iter().collect(),
                            },
                        );
                    }
                    if !skills_changed.is_empty() {
                        // New or removed skill directories need their own watches;
                        // plain file edits do not.
                        if std::mem::take(&mut rewatch) {
                            _watcher = build_watcher(&thread_tx, &state);
                        }
                        let _ = app.emit(
                            "skills-changed",
                            SkillsChangedPayload {
                                paths: skills_changed.into_iter().collect(),
                            },
                        );
                    }
                }
            }
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

    #[test]
    fn is_skill_relevant_filters_paths() {
        let home = Path::new("/h");
        let paths = AgentPaths::from_home(home);
        let roots = vec!["/work/app".to_string()];
        let yes = |p: &str| is_skill_relevant(Path::new(p), &paths, home, &roots);
        assert!(yes("/h/.claude/skills"));
        assert!(yes("/h/.claude/skills/demo"));
        assert!(yes("/h/.claude/skills/demo/SKILL.md"));
        assert!(yes("/h/.codex/skills/.system/x"));
        assert!(yes("/h/.agents/skills/demo/SKILL.md"));
        assert!(yes("/work/app/.claude/skills/demo/SKILL.md"));
        assert!(yes("/work/app/.agents/skills/demo"));
        assert!(!yes("/h/.claude/skills/demo/ref/a.md"));
        assert!(!yes("/h/.claude/skills/demo/.git"));
        assert!(!yes("/h/.claude/skills/demo/node_modules"));
        assert!(!yes("/h/.claude/skills/.demo.memdock-old-123"));
        assert!(!yes("/h/.claude/CLAUDE.md"));
        assert!(!yes("/h/.claude/settings.json"));
        assert!(!yes("/other/.claude/skills/demo"));
        // Skills paths never count as memory changes.
        assert!(!is_relevant(Path::new("/h/.claude/skills/demo/SKILL.md"), &paths, &roots));
    }

    #[test]
    fn needs_rewatch_only_for_skill_dirs() {
        let home = Path::new("/h");
        let paths = AgentPaths::from_home(home);
        let roots = vec!["/work/app".to_string()];
        let yes = |p: &str| needs_rewatch(Path::new(p), &paths, home, &roots);
        assert!(yes("/h/.claude/skills"));
        assert!(yes("/h/.codex/skills/demo"));
        assert!(yes("/work/app/.agents/skills/demo"));
        assert!(!yes("/h/.codex/skills/demo/SKILL.md"));
        assert!(!yes("/h/.codex/skills/.demo.memdock-new-1"));
        assert!(!yes("/h/.codex/skills/demo/.git"));
        assert!(!yes("/h/.claude/CLAUDE.md"));
    }

    #[test]
    fn irrelevant_events_do_not_reset_timer() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut d = Debouncer::new(ms(300), ms(1000));
        assert_eq!(d.deadline(), None);
        d.offer(false, t0);
        assert_eq!(d.deadline(), None);
        d.offer(true, t0 + ms(100));
        assert_eq!(d.deadline(), Some(t0 + ms(400)));
        d.offer(false, t0 + ms(350));
        assert_eq!(d.deadline(), Some(t0 + ms(400)));
        d.reset();
        assert_eq!(d.deadline(), None);
    }

    #[test]
    fn max_wait_cap_fires() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut d = Debouncer::new(ms(300), ms(1000));
        for i in 0..20 {
            d.offer(true, t0 + ms(i * 200));
        }
        // Events keep coming every 200ms, yet the deadline is capped at first + 1s.
        assert_eq!(d.deadline(), Some(t0 + ms(1000)));
    }

    #[test]
    fn skill_roots_include_project_codex_skills() {
        let paths = AgentPaths::from_home(Path::new("/h"));
        let roots = skill_roots(&paths, Path::new("/h"), &["/work/app".to_string()]);
        let has = |rel: &str| {
            roots
                .iter()
                .any(|r| normalize_path(&r.to_string_lossy()) == normalize_path(rel))
        };
        assert!(has("/work/app/.claude/skills"));
        assert!(has("/work/app/.agents/skills"));
        assert!(has("/work/app/.codex/skills"));
        assert!(is_skill_relevant(
            Path::new("/work/app/.codex/skills/x/SKILL.md"),
            &paths,
            Path::new("/h"),
            &["/work/app".to_string()]
        ));
    }
}
