//! Orchestration of one extraction run, the single-run lock and the scheduler.
//!
//! A run: pick a CLI runner, read new session turns, group them per project
//! (sessions without a project form one extra "global" batch), and for every
//! project build a prompt from its writable memory files, call the CLI, store
//! the validated suggestions and advance that project's watermark. A failing
//! project is recorded as a warning and the run continues.
//!
//! Decisions:
//! - Targets of a project are its *existing* writable memory files
//!   (`ClaudeInstructions`, `ClaudeLocal`, `ClaudeAuto`, `CodexAgents`; never
//!   `CodexDb`). When the project has no existing Claude instruction file, an
//!   empty `<project>/CLAUDE.md` target is added so "add" suggestions have a
//!   place to go (only when the project directory exists). The global batch
//!   and (with `include_global`) every project batch also get the global
//!   `CLAUDE.md` / `AGENTS.md` under the same rule.
//! - Limits: at most `MAX_PROJECTS` (20) real projects per run, most recently
//!   active first; the "global" batch comes on top of that. The 60 000-char
//!   excerpt cap and the 180 s CLI timeout are applied below / here.
//! - Local time for the scheduler comes from `chrono::Local` (`clock` feature
//!   only); weekday numbering is 0 = Sunday .. 6 = Saturday (JS `getDay`).
//! - Schedules fire only while the app runs; a missed slot is never caught up.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use chrono::{Offset, TimeZone};
use serde::Serialize;

use super::prompt::{
    build_prompt_with, chunk_turns, output_schema, parse_output_full, MemoryTarget, MAX_EXCERPT_CHARS,
};
use super::runner::{CliRunner, RunError};
use super::sessions::{read_claude_turns, read_codex_turns, Turn};
use super::store::{RunErrorCode, RunRecord, RunStatus, RunTrigger, Schedule, Store};
use crate::memory::io::read_memory;
use crate::memory::scan::scan_all;
use crate::model::{MemoryFile, MemoryIndex, MemoryKind, Tool};
use crate::paths::AgentPaths;
use crate::projects::{discover_projects, normalize_path};

/// Watermark key (and display name) for sessions without a project.
pub const GLOBAL_KEY: &str = "__global__";
/// Projects processed per run (not counting the global batch).
pub const MAX_PROJECTS: usize = 20;
/// Excerpt chunks (CLI calls) per project per run.
pub const MAX_CHUNKS: usize = 3;
/// Timeout of one CLI call.
pub const CLI_TIMEOUT: Duration = Duration::from_secs(180);
/// Scheduler check interval.
pub const TICK_INTERVAL: Duration = Duration::from_secs(30);

const DAY_MS: i64 = 86_400_000;

pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;
pub type IndexProvider = Arc<dyn Fn() -> MemoryIndex + Send + Sync>;
/// `(tool, since_ms)` -> turns newer than `since_ms`.
pub type TurnSource = Arc<dyn Fn(Tool, i64) -> Vec<Turn> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExtractError {
    AlreadyRunning,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Progress {
    pub project: String,
    /// 1-based.
    pub index: u32,
    pub total: u32,
}

pub struct ExtractCtx {
    pub paths: AgentPaths,
    pub home: PathBuf,
    /// Loaded once at startup; never reloaded during a run.
    pub store: Arc<Mutex<Store>>,
    /// Swappable: the app re-detects installed CLIs on demand (`set_runners`).
    runners: Mutex<Vec<Arc<dyn CliRunner>>>,
    pub clock: Clock,
    pub index: IndexProvider,
    pub turns: TurnSource,
    /// Shared cancel flag for scheduled runs (and for UI-triggered cancel).
    pub cancel: Arc<AtomicBool>,
    hooks: Mutex<Hooks>,
    running: Mutex<Option<String>>,
}

/// Observers of runs started through `start_run` / `start_run_with` (manual and
/// scheduled alike); the app uses them to emit UI events.
#[derive(Clone, Default)]
pub struct Hooks {
    pub on_progress: Option<Arc<dyn Fn(&Progress) + Send + Sync>>,
    pub on_finished: Option<Arc<dyn Fn(&RunRecord) + Send + Sync>>,
}

fn system_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(0))
        .unwrap_or(0)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl ExtractCtx {
    /// Production context: system clock, projects discovered from session
    /// logs, turns read from the CLI session directories.
    pub fn new(
        paths: AgentPaths,
        home: PathBuf,
        store: Arc<Mutex<Store>>,
        runners: Vec<Arc<dyn CliRunner>>,
    ) -> Self {
        let p1 = paths.clone();
        let index: IndexProvider = Arc::new(move || {
            let projects = discover_projects(&p1, &[]);
            scan_all(&p1, &projects)
        });
        let p2 = paths.clone();
        let turns: TurnSource = Arc::new(move |tool, since| match tool {
            Tool::Claude => read_claude_turns(&p2, since),
            Tool::Codex => read_codex_turns(&p2, since),
        });
        ExtractCtx {
            paths,
            home,
            store,
            runners: Mutex::new(runners),
            clock: Arc::new(system_now_ms),
            index,
            turns,
            cancel: Arc::new(AtomicBool::new(false)),
            hooks: Mutex::new(Hooks::default()),
            running: Mutex::new(None),
        }
    }

    /// Replaces the CLI runners (a run already in progress keeps its runner).
    pub fn set_runners(&self, runners: Vec<Arc<dyn CliRunner>>) {
        *lock(&self.runners) = runners;
    }

    pub fn set_hooks(&self, hooks: Hooks) {
        *lock(&self.hooks) = hooks;
    }

    pub fn is_running(&self) -> bool {
        lock(&self.running).is_some()
    }

    /// Id of the run holding the lock, if any.
    pub fn current_run_id(&self) -> Option<String> {
        lock(&self.running).clone()
    }

    /// Takes the single-run lock. Released when the guard drops (also on panic).
    pub fn try_lock(&self, run_id: &str) -> Result<RunGuard<'_>, ExtractError> {
        let mut g = lock(&self.running);
        if g.is_some() {
            return Err(ExtractError::AlreadyRunning);
        }
        *g = Some(run_id.to_string());
        Ok(RunGuard { ctx: self })
    }
}

pub struct RunGuard<'a> {
    ctx: &'a ExtractCtx,
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        *lock(&self.ctx.running) = None;
    }
}

/// Takes the run lock and performs a run; `AlreadyRunning` when one is active.
pub fn start_run(
    ctx: &ExtractCtx,
    trigger: RunTrigger,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
) -> Result<RunRecord, ExtractError> {
    start_run_with(ctx, trigger, cancel, progress, &|_| {})
}

/// Like `start_run`; `started` is called with the Running record as soon as it
/// is stored (the lock is held). The ctx hooks see progress and the final record.
pub fn start_run_with(
    ctx: &ExtractCtx,
    trigger: RunTrigger,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
    started: &dyn Fn(&RunRecord),
) -> Result<RunRecord, ExtractError> {
    let id = uuid::Uuid::new_v4().to_string();
    let guard = ctx.try_lock(&id)?;
    let hooks = lock(&ctx.hooks).clone();
    let wrapped = |p: Progress| {
        if let Some(h) = &hooks.on_progress {
            h(&p);
        }
        progress(p);
    };
    let rec = execute(ctx, id, trigger, cancel, &wrapped, started);
    drop(guard);
    if let Some(h) = &hooks.on_finished {
        h(&rec);
    }
    Ok(rec)
}

/// Performs a run without taking the lock (use `start_run` unless the caller
/// already holds a `RunGuard`).
pub fn run_extraction(
    ctx: &ExtractCtx,
    trigger: RunTrigger,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
) -> RunRecord {
    execute(ctx, uuid::Uuid::new_v4().to_string(), trigger, cancel, progress, &|_| {})
}

// ----------------------------------------------------------------- helpers

fn describe(e: &RunError) -> String {
    match e {
        RunError::NotInstalled => "CLI not installed".into(),
        RunError::NotLoggedIn(m) => format!("CLI not logged in: {m}"),
        RunError::Timeout => "CLI timed out".into(),
        RunError::Cancelled => "cancelled".into(),
        RunError::Failed { code, stderr } => format!("CLI failed (code {code:?}): {stderr}"),
    }
}

fn code_of(e: &RunError) -> RunErrorCode {
    match e {
        RunError::NotInstalled => RunErrorCode::NotInstalled,
        RunError::NotLoggedIn(_) => RunErrorCode::NotLoggedIn,
        RunError::Timeout => RunErrorCode::Timeout,
        RunError::Cancelled => RunErrorCode::Cancelled,
        RunError::Failed { .. } => RunErrorCode::Other,
    }
}

/// Sets `error` + `error_code` unless an error is already recorded.
fn set_error(rec: &mut RunRecord, code: RunErrorCode, msg: String) {
    if rec.error.is_none() {
        rec.error = Some(msg);
        rec.error_code = Some(code);
    }
}

pub fn choose_runner(ctx: &ExtractCtx, want: Option<Tool>) -> Option<Arc<dyn CliRunner>> {
    let runners = lock(&ctx.runners).clone();
    let pick = |t: Tool| runners.iter().find(|r| r.tool() == t && r.available()).cloned();
    match want {
        Some(t) => pick(t),
        None => pick(Tool::Claude).or_else(|| pick(Tool::Codex)),
    }
}

fn is_writable(kind: MemoryKind) -> bool {
    !matches!(kind, MemoryKind::CodexDb)
}

fn to_target(f: &MemoryFile) -> Result<MemoryTarget, String> {
    let c = read_memory(Path::new(&f.path)).map_err(|e| format!("{}: {e}", f.path))?;
    Ok(MemoryTarget {
        file_id: f.id.clone(),
        path: f.path.clone(),
        tool: f.tool,
        kind: f.kind,
        content: c.text,
        hash: c.hash,
    })
}

/// Existing writable files, plus an empty CLAUDE.md placeholder (`fallback`)
/// when no Claude instruction file exists yet.
fn collect_targets(
    files: &[MemoryFile],
    fallback: Option<MemoryTarget>,
    only_instructions: bool,
    warnings: &mut Vec<String>,
) -> Vec<MemoryTarget> {
    let mut out = Vec::new();
    let mut has_claude_instr = false;
    for f in files.iter().filter(|f| f.exists && is_writable(f.kind)) {
        if only_instructions
            && !matches!(f.kind, MemoryKind::ClaudeInstructions | MemoryKind::CodexAgents)
        {
            continue;
        }
        match to_target(f) {
            Ok(t) => {
                has_claude_instr |= t.kind == MemoryKind::ClaudeInstructions;
                out.push(t);
            }
            Err(e) => warnings.push(format!("skipped unreadable memory file {e}")),
        }
    }
    if !has_claude_instr {
        out.extend(fallback);
    }
    out
}

fn placeholder(path: &Path) -> MemoryTarget {
    let p = path.to_string_lossy().into_owned();
    MemoryTarget {
        file_id: normalize_path(&p),
        path: p,
        tool: Tool::Claude,
        kind: MemoryKind::ClaudeInstructions,
        content: String::new(),
        hash: String::new(),
    }
}

fn global_targets(ctx: &ExtractCtx, index: &MemoryIndex, warnings: &mut Vec<String>) -> Vec<MemoryTarget> {
    // Prefer the index entry (stable id); fall back to the plain path.
    let fb = index
        .global
        .iter()
        .find(|f| f.kind == MemoryKind::ClaudeInstructions)
        .map(|f| MemoryTarget {
            file_id: f.id.clone(),
            path: f.path.clone(),
            tool: Tool::Claude,
            kind: MemoryKind::ClaudeInstructions,
            content: String::new(),
            hash: String::new(),
        })
        .or_else(|| Some(placeholder(&ctx.paths.claude_home.join("CLAUDE.md"))))
        // Only offer a missing global CLAUDE.md when the claude home exists.
        .filter(|_| ctx.paths.claude_home.is_dir());
    collect_targets(&index.global, fb, true, warnings)
}

struct Batch {
    /// Watermark key: project id or `GLOBAL_KEY`.
    key: String,
    name: String,
    turns: Vec<Turn>,
    last_ts: i64,
}

/// Writes the final record; a save failure is surfaced in the record itself.
fn finish(ctx: &ExtractCtx, rec: &mut RunRecord) {
    let mut st = lock(&ctx.store);
    st.update_run(rec.clone());
    if let Err(e) = st.save() {
        let m = format!("could not save run record: {e}");
        rec.warnings.push(m.clone());
        set_error(rec, RunErrorCode::Io, m);
        st.update_run(rec.clone());
    }
}

/// Marks the run Failed if `execute` unwinds (panic in runner / parsing).
struct PanicGuard<'a> {
    ctx: &'a ExtractCtx,
    id: String,
    armed: bool,
}

impl PanicGuard<'_> {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PanicGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut st = lock(&self.ctx.store);
        if let Some(mut r) = st.runs().iter().find(|r| r.id == self.id).cloned() {
            r.status = RunStatus::Failed;
            r.finished_at = Some((self.ctx.clock)());
            r.error = Some("panicked during extraction".into());
            r.error_code = Some(RunErrorCode::Other);
            st.update_run(r);
            let _ = st.save();
        }
    }
}

// --------------------------------------------------------------- the run

fn execute(
    ctx: &ExtractCtx,
    id: String,
    trigger: RunTrigger,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
    started: &dyn Fn(&RunRecord),
) -> RunRecord {
    let now = (ctx.clock)();
    let cfg = lock(&ctx.store).config().clone();
    let runner = choose_runner(ctx, cfg.tool);
    let mut rec = RunRecord {
        id,
        started_at: now,
        finished_at: None,
        trigger,
        tool: runner.as_ref().map(|r| r.tool()).or(cfg.tool).unwrap_or(Tool::Claude),
        status: RunStatus::Running,
        sessions_read: 0,
        projects: 0,
        suggestions_new: 0,
        warnings: Vec::new(),
        error: None,
        error_code: None,
    };
    {
        let mut st = lock(&ctx.store);
        st.push_run(rec.clone());
        if let Err(e) = st.save() {
            rec.warnings.push(format!("could not save run record: {e}"));
        }
    }
    let mut guard = PanicGuard { ctx, id: rec.id.clone(), armed: true };
    started(&rec);

    let Some(runner) = runner else {
        rec.status = RunStatus::Failed;
        rec.error = Some("not_installed".into());
        rec.error_code = Some(RunErrorCode::NotInstalled);
        rec.finished_at = Some((ctx.clock)());
        finish(ctx, &mut rec);
        guard.disarm();
        return rec;
    };

    let index = (ctx.index)();
    let lang = cfg.lang.clone().unwrap_or_else(|| "zh".into());
    let lookback_since = now - i64::from(cfg.lookback_days) * DAY_MS;

    // ---- candidate keys and the coarse read window
    let allowed = |pid: &str| match &cfg.projects {
        super::store::ProjectFilter::All => true,
        super::store::ProjectFilter::Selected(ids) => ids.iter().any(|i| i == pid),
    };
    let wm = |key: &str, tool: Tool| lock(&ctx.store).watermark(key, tool);

    let mut all_turns: Vec<Turn> = Vec::new();
    for tool in [Tool::Claude, Tool::Codex] {
        let mut keys: Vec<String> = index
            .projects
            .iter()
            .filter(|p| allowed(&p.project.id))
            .map(|p| p.project.id.clone())
            .collect();
        if cfg.include_global {
            keys.push(GLOBAL_KEY.to_string());
        }
        let min_wm = keys.iter().map(|k| wm(k, tool)).min().unwrap_or(0);
        let since = lookback_since.max(min_wm);
        all_turns.extend((ctx.turns)(tool, since));
    }

    // ---- group
    let mut by_key: HashMap<String, Vec<Turn>> = HashMap::new();
    for t in all_turns {
        let key = match &t.project_id {
            Some(p) => {
                if !allowed(p) {
                    continue;
                }
                p.clone()
            }
            None => {
                if !cfg.include_global {
                    continue;
                }
                GLOBAL_KEY.to_string()
            }
        };
        if t.ts < lookback_since || t.ts <= wm(&key, t.tool) {
            continue;
        }
        by_key.entry(key).or_default().push(t);
    }
    let mut batches: Vec<Batch> = Vec::new();
    let mut seen_sessions: BTreeSet<String> = BTreeSet::new();
    for (key, turns) in by_key {
        for t in &turns {
            seen_sessions.insert(format!("{:?}/{}", t.tool, t.session_id));
        }
        let last_ts = turns.iter().map(|t| t.ts).max().unwrap_or(0);
        let name = if key == GLOBAL_KEY {
            GLOBAL_KEY.to_string()
        } else {
            index
                .projects
                .iter()
                .find(|p| p.project.id == key)
                .map(|p| p.project.name.clone())
                .unwrap_or_else(|| key.clone())
        };
        batches.push(Batch { key, name, turns, last_ts });
    }
    rec.sessions_read = seen_sessions.len() as u32;
    let (mut global, mut projects): (Vec<Batch>, Vec<Batch>) =
        batches.into_iter().partition(|b| b.key == GLOBAL_KEY);
    projects.sort_by(|a, b| b.last_ts.cmp(&a.last_ts).then_with(|| a.key.cmp(&b.key)));
    if projects.len() > MAX_PROJECTS {
        rec.warnings.push(format!(
            "{} projects have new sessions; only the {MAX_PROJECTS} most recent were processed",
            projects.len()
        ));
        projects.truncate(MAX_PROJECTS);
    }
    projects.append(&mut global);
    let total = projects.len() as u32;

    let mut cancelled = false;
    let mut fatal: Option<(RunErrorCode, String)> = None;
    let mut attempted = 0u32;
    let mut ok = 0u32;
    let mut first_failure: Option<(RunErrorCode, String)> = None;
    let mut save_error: Option<String> = None;

    for (i, b) in projects.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            cancelled = true;
            break;
        }
        progress(Progress { project: b.name.clone(), index: i as u32 + 1, total });

        // ---- targets
        let mut tw: Vec<String> = Vec::new();
        let mut targets: Vec<MemoryTarget> = if b.key == GLOBAL_KEY {
            global_targets(ctx, &index, &mut tw)
        } else if let Some(pm) = index.projects.iter().find(|p| p.project.id == b.key) {
            let root = Path::new(&pm.project.path);
            let fb = pm.project.exists.then(|| placeholder(&root.join("CLAUDE.md")));
            let mut t = collect_targets(&pm.files, fb, false, &mut tw);
            if cfg.include_global {
                t.extend(global_targets(ctx, &index, &mut tw));
            }
            t
        } else {
            Vec::new()
        };
        // Same file can appear twice (e.g. global placeholder); keep the first.
        let mut seen_ids = BTreeSet::new();
        targets.retain(|t| seen_ids.insert(t.file_id.clone()));
        rec.warnings.extend(tw.into_iter().map(|w| format!("{}: {w}", b.name)));
        if targets.is_empty() {
            rec.warnings.push(format!("{}: no memory file to propose changes for, skipped", b.name));
            continue;
        }

        // ---- chunks: oldest first, each fits the excerpt cap; at most
        // MAX_CHUNKS per project per run, the rest waits for the next run.
        attempted += 1;
        let all: Vec<&Turn> = b.turns.iter().collect();
        let chunks = chunk_turns(&all, MAX_EXCERPT_CHARS);
        let mut done_chunks = 0usize;
        let mut project_failed = false;
        for chunk in chunks.iter().take(MAX_CHUNKS) {
            if cancel.load(Ordering::SeqCst) {
                cancelled = true;
                break;
            }
            let prompt = build_prompt_with(&b.name, &targets, chunk, &lang, cfg.custom_prompt.as_deref());
            let raw = match runner.run(&prompt, &output_schema(), CLI_TIMEOUT, cancel) {
                Ok(r) => r,
                Err(RunError::Cancelled) => {
                    cancelled = true;
                    break;
                }
                Err(e @ (RunError::NotInstalled | RunError::NotLoggedIn(_))) => {
                    fatal = Some((code_of(&e), describe(&e)));
                    break;
                }
                Err(e) => {
                    let m = format!("{}: {}", b.name, describe(&e));
                    first_failure.get_or_insert_with(|| (code_of(&e), m.clone()));
                    rec.warnings.push(m);
                    project_failed = true;
                    break;
                }
            };
            let parsed = parse_output_full(&raw, &targets, runner.tool(), chunk);
            rec.warnings.extend(parsed.warnings.into_iter().map(|w| format!("{}: {w}", b.name)));
            if parsed.unusable {
                let m = format!("{}: model output unusable", b.name);
                first_failure.get_or_insert_with(|| (RunErrorCode::InvalidOutput, m.clone()));
                rec.warnings.push(m);
                project_failed = true;
                break;
            }

            // ---- store + watermark (only after this chunk succeeded)
            let mut st = lock(&ctx.store);
            rec.suggestions_new += st.add_suggestions(parsed.suggestions);
            for tool in [Tool::Claude, Tool::Codex] {
                if let Some(m) = chunk.iter().filter(|t| t.tool == tool).map(|t| t.ts).max() {
                    if m > st.watermark(&b.key, tool) {
                        st.set_watermark(&b.key, tool, m);
                    }
                }
            }
            if let Err(e) = st.save() {
                let m = format!("{}: could not save store: {e}", b.name);
                rec.warnings.push(m.clone());
                save_error.get_or_insert(m);
            }
            done_chunks += 1;
        }
        if done_chunks > 0 {
            ok += 1;
            rec.projects += 1;
        }
        if !cancelled && fatal.is_none() && !project_failed && done_chunks == MAX_CHUNKS && chunks.len() > MAX_CHUNKS {
            let n: usize = chunks[MAX_CHUNKS..].iter().map(Vec::len).sum();
            rec.warnings.push(format!("{}: {n} newer turns deferred to the next run", b.name));
        }
        if cancelled || fatal.is_some() {
            break;
        }
    }

    rec.status = if cancelled {
        RunStatus::Cancelled
    } else if let Some((code, m)) = fatal {
        set_error(&mut rec, code, m);
        RunStatus::Failed
    } else if attempted > 0 && ok == 0 {
        if let Some((code, m)) = first_failure {
            set_error(&mut rec, code, m);
        }
        RunStatus::Failed
    } else {
        RunStatus::Succeeded
    };
    rec.finished_at = Some((ctx.clock)());
    if let Some(m) = save_error {
        set_error(&mut rec, RunErrorCode::Io, m);
    }
    finish(ctx, &mut rec);
    guard.disarm();
    rec
}

// ------------------------------------------------------------- scheduler

/// Offset of the local time zone from UTC in seconds, at `now_ms`.
pub type OffsetFn = Arc<dyn Fn(i64) -> i32 + Send + Sync>;

fn local_offset_secs(now_ms: i64) -> i32 {
    chrono::Local
        .timestamp_millis_opt(now_ms)
        .single()
        .map(|d| d.offset().fix().local_minus_utc())
        .unwrap_or(0)
}

pub struct Scheduler {
    ctx: Arc<ExtractCtx>,
    offset: OffsetFn,
    /// UTC minute of the last fired slot.
    last_slot: Mutex<Option<i64>>,
    stop: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl Scheduler {
    pub fn new(ctx: Arc<ExtractCtx>) -> Arc<Self> {
        Self::with_offset(ctx, Arc::new(local_offset_secs))
    }

    pub fn with_offset(ctx: Arc<ExtractCtx>, offset: OffsetFn) -> Arc<Self> {
        Arc::new(Scheduler {
            ctx,
            offset,
            last_slot: Mutex::new(None),
            stop: Arc::new(AtomicBool::new(false)),
            handle: Mutex::new(None),
        })
    }

    /// True when `now_ms` falls in a configured slot not fired yet; marks it fired.
    pub fn due(&self, now_ms: i64) -> bool {
        let schedule = lock(&self.ctx.store).config().schedule;
        let local_ms = now_ms + i64::from((self.offset)(now_ms)) * 1000;
        let minute = local_ms.div_euclid(60_000);
        let mins_of_day = minute.rem_euclid(1440);
        let (hh, mm) = ((mins_of_day / 60) as u8, (mins_of_day % 60) as u8);
        // 1970-01-01 was a Thursday (4 with Sunday = 0).
        let weekday = ((minute.div_euclid(1440) + 4).rem_euclid(7)) as u8;
        let hit = match schedule {
            Schedule::Off => false,
            Schedule::Daily { hh: h, mm: m } => (h, m) == (hh, mm),
            Schedule::Weekly { weekday: w, hh: h, mm: m } => (w, h, m) == (weekday, hh, mm),
        };
        if !hit {
            return false;
        }
        // Dedupe on the UTC minute, with a 120-minute window, so a DST fall-back
        // (the same local time twice) cannot fire a slot twice.
        let utc_minute = now_ms.div_euclid(60_000);
        let mut last = lock(&self.last_slot);
        if let Some(prev) = *last {
            if utc_minute >= prev && utc_minute - prev < 120 {
                return false;
            }
        }
        *last = Some(utc_minute);
        true
    }

    /// One scheduler check at `now_ms`. Runs (blocking) when a slot is due
    /// and no run is in progress; returns the run record in that case.
    pub fn tick(&self, now_ms: i64) -> Option<RunRecord> {
        if self.stop.load(Ordering::SeqCst) || !self.due(now_ms) || self.ctx.is_running() {
            return None;
        }
        // The shared cancel flag is reset only once the run lock is held (like the
        // manual path), so a cancel aimed at a just-started manual run survives.
        // A racing stop() must not be undone by the reset.
        let reset = |_: &RunRecord| {
            self.ctx.cancel.store(false, Ordering::SeqCst);
            if self.stop.load(Ordering::SeqCst) {
                self.ctx.cancel.store(true, Ordering::SeqCst);
            }
        };
        start_run_with(&self.ctx, RunTrigger::Scheduled, &self.ctx.cancel, &|_| {}, &reset).ok()
    }

    /// Starts the background thread (every `TICK_INTERVAL`, real clock from ctx).
    pub fn start(self: &Arc<Self>) {
        let mut h = lock(&self.handle);
        if h.is_some() {
            return;
        }
        let me = Arc::clone(self);
        *h = Some(thread::spawn(move || {
            while !me.stop.load(Ordering::SeqCst) {
                me.tick((me.ctx.clock)());
                let mut waited = Duration::ZERO;
                while waited < TICK_INTERVAL && !me.stop.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(250));
                    waited += Duration::from_millis(250);
                }
            }
        }));
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.ctx.cancel.store(true, Ordering::SeqCst);
        if let Some(h) = lock(&self.handle).take() {
            let _ = h.join();
        }
    }

    /// Like `stop`, but waits at most `limit` for the thread; returns whether it ended.
    pub fn stop_within(&self, limit: Duration) -> bool {
        self.stop.store(true, Ordering::SeqCst);
        self.ctx.cancel.store(true, Ordering::SeqCst);
        let Some(h) = lock(&self.handle).take() else { return true };
        let t0 = std::time::Instant::now();
        while !h.is_finished() {
            if t0.elapsed() >= limit {
                return false;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = h.join();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::sessions::Role;
    use crate::extract::store::{ExtractConfig, ProjectFilter};
    use crate::model::Project;
    use regex::Regex;
    use serde_json::json;
    use std::fs;

    const NOW: i64 = 20 * DAY_MS;

    type Handler = Box<dyn Fn(&str) -> Result<String, RunError> + Send + Sync>;

    struct Fake {
        tool: Tool,
        avail: bool,
        calls: Mutex<Vec<String>>,
        handler: Handler,
        cancel_on_call: bool,
        /// Blocks (up to 10 s) until cancelled, then reports Cancelled.
        slow: bool,
    }

    impl Fake {
        fn ok() -> Self {
            Fake {
                tool: Tool::Claude,
                avail: true,
                calls: Mutex::new(vec![]),
                handler: Box::new(|p| Ok(reply(p))),
                cancel_on_call: false,
                slow: false,
            }
        }
        fn with(handler: Handler) -> Self {
            Fake { handler, ..Fake::ok() }
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl CliRunner for Fake {
        fn tool(&self) -> Tool {
            self.tool
        }
        fn available(&self) -> bool {
            self.avail
        }
        fn run(&self, prompt: &str, _s: &serde_json::Value, _t: Duration, cancel: &AtomicBool) -> Result<String, RunError> {
            self.calls.lock().unwrap().push(prompt.to_string());
            if self.cancel_on_call {
                cancel.store(true, Ordering::SeqCst);
            }
            if self.slow {
                let t0 = std::time::Instant::now();
                while !cancel.load(Ordering::SeqCst) && t0.elapsed() < Duration::from_secs(10) {
                    thread::sleep(Duration::from_millis(10));
                }
                return Err(RunError::Cancelled);
            }
            (self.handler)(prompt)
        }
    }

    fn cap(re: &str, s: &str) -> String {
        Regex::new(re).unwrap().captures(s).unwrap()[1].to_string()
    }

    /// One valid "add" suggestion against the first target / first turn of the prompt.
    fn reply(prompt: &str) -> String {
        let fid = cap(r#"file_id="([^"]+)""#, prompt);
        let sid = cap(r#"session_id="([^"]+)""#, prompt);
        let ts: i64 = cap(r#" ts="(\d+)""#, prompt).parse().unwrap();
        json!({"suggestions": [{
            "kind": "add", "file_id": fid, "new_text": format!("rule for {fid}"),
            "reason": "user said so",
            "evidence": [{"session_id": sid, "ts": ts, "quote": "always run tests"}]
        }]})
        .to_string()
    }

    struct Env {
        _tmp: tempfile::TempDir,
        ctx: Arc<ExtractCtx>,
        fake: Arc<Fake>,
    }

    fn proj(home: &Path, name: &str, with_claude_md: bool) -> Project {
        let dir = home.join(name);
        fs::create_dir_all(&dir).unwrap();
        if with_claude_md {
            fs::write(dir.join("CLAUDE.md"), format!("# {name}\n")).unwrap();
        }
        let path = dir.to_string_lossy().into_owned();
        Project { id: normalize_path(&path), path, name: name.into(), exists: true, sources: vec![] }
    }

    fn turn(p: &Project, sid: &str, ts: i64) -> Turn {
        Turn {
            tool: Tool::Claude,
            session_id: sid.into(),
            project_id: Some(p.id.clone()),
            ts,
            role: Role::User,
            text: "always run tests".into(),
        }
    }

    fn env_with(
        tmp: tempfile::TempDir,
        fake: Fake,
        projects: Vec<Project>,
        turns: Vec<Turn>,
        cfg: Option<ExtractConfig>,
    ) -> Env {
        let home = tmp.path().join("home");
        let paths = AgentPaths::from_home(&home);
        fs::create_dir_all(&paths.claude_home).unwrap();
        fs::create_dir_all(&paths.codex_home).unwrap();
        let data = tmp.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let mut store = Store::load(&data).unwrap();
        if let Some(c) = cfg {
            store.set_config(c);
        }
        let fake = Arc::new(fake);
        let mut ctx = ExtractCtx::new(paths.clone(), home, Arc::new(Mutex::new(store)), vec![fake.clone()]);
        ctx.clock = Arc::new(|| NOW);
        ctx.index = Arc::new(move || scan_all(&paths, &projects));
        ctx.turns = Arc::new(move |tool, since| {
            turns.iter().filter(|t| t.tool == tool && t.ts >= since).cloned().collect()
        });
        Env { _tmp: tmp, ctx: Arc::new(ctx), fake }
    }

    /// Two projects (alpha, beta) with one new turn each; alpha is more recent.
    fn two() -> Env {
        two_with(Fake::ok())
    }

    fn two_with(fake: Fake) -> Env {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().to_path_buf();
        let a = proj(&base, "alpha", true);
        let b = proj(&base, "beta", true);
        let turns = vec![turn(&a, "sa", NOW - 1000), turn(&b, "sb", NOW - 5000)];
        env_with(t, fake, vec![a, b], turns, None)
    }

    fn go(e: &Env) -> RunRecord {
        let c = AtomicBool::new(false);
        start_run(&e.ctx, RunTrigger::Manual, &c, &|_| {}).unwrap()
    }

    fn wm(e: &Env, key: &str) -> i64 {
        lock(&e.ctx.store).watermark(key, Tool::Claude)
    }

    #[test]
    fn runs_per_project_and_stores_suggestions() {
        let e = two();
        let seen = Mutex::new(Vec::new());
        let c = AtomicBool::new(false);
        let rec = start_run(&e.ctx, RunTrigger::Manual, &c, &|p| seen.lock().unwrap().push(p)).unwrap();
        assert_eq!(rec.status, RunStatus::Succeeded, "{rec:?}");
        assert_eq!(rec.projects, 2);
        assert_eq!(rec.suggestions_new, 2);
        assert_eq!(rec.sessions_read, 2);
        assert_eq!(e.fake.calls().len(), 2);
        let prog = seen.into_inner().unwrap();
        assert_eq!(prog[0], Progress { project: "alpha".into(), index: 1, total: 2 });
        assert_eq!(prog[1], Progress { project: "beta".into(), index: 2, total: 2 });
        let st = lock(&e.ctx.store);
        assert_eq!(st.suggestions().len(), 2);
        assert_eq!(st.runs().last().unwrap().id, rec.id);
        assert_eq!(st.runs().last().unwrap().status, RunStatus::Succeeded);
        // base_hash comes from the file as read
        assert!(st.suggestions().iter().any(|s| !s.base_hash.is_empty()));
    }

    #[test]
    fn skips_projects_without_new_turns() {
        let e = two();
        let a = pid(&e, "alpha");
        lock(&e.ctx.store).set_watermark(&a, Tool::Claude, NOW - 1000);
        let rec = go(&e);
        assert_eq!(rec.projects, 1);
        assert_eq!(e.fake.calls().len(), 1);
        assert!(e.fake.calls()[0].contains("beta"));
    }

    fn pid(e: &Env, name: &str) -> String {
        (e.ctx.index)().projects.iter().find(|p| p.project.name == name).unwrap().project.id.clone()
    }

    #[test]
    fn failed_project_recorded_as_warning() {
        let e = two_with(Fake::with(Box::new(|p| {
            if p.contains("alpha") {
                Err(RunError::Failed { code: Some(1), stderr: "boom".into() })
            } else {
                Ok(reply(p))
            }
        })));
        let rec = go(&e);
        assert_eq!(rec.status, RunStatus::Succeeded);
        assert_eq!(rec.projects, 1);
        assert!(rec.warnings.iter().any(|w| w.starts_with("alpha") && w.contains("boom")), "{:?}", rec.warnings);
        assert_eq!(e.fake.calls().len(), 2, "run continued after the failure");
    }

    #[test]
    fn watermark_advanced_only_on_success() {
        let e = two_with(Fake::with(Box::new(|p| {
            if p.contains("alpha") {
                Err(RunError::Timeout)
            } else {
                Ok(reply(p))
            }
        })));
        go(&e);
        assert_eq!(wm(&e, &pid(&e, "alpha")), 0);
        assert_eq!(wm(&e, &pid(&e, "beta")), NOW - 5000);
        // unusable output counts as failure too
        let e2 = two_with(Fake::with(Box::new(|_| Ok("not json".into()))));
        let rec = go(&e2);
        assert_eq!(rec.status, RunStatus::Failed);
        assert_eq!(wm(&e2, &pid(&e2, "alpha")), 0);
        assert_eq!(wm(&e2, &pid(&e2, "beta")), 0);
    }

    #[test]
    fn failed_runs_carry_an_error_code() {
        let code = |f: Fake| go(&two_with(f)).error_code;
        let mut missing = Fake::ok();
        missing.avail = false;
        assert_eq!(code(missing), Some(RunErrorCode::NotInstalled));
        assert_eq!(code(Fake::with(Box::new(|_| Err(RunError::NotLoggedIn("x".into()))))), Some(RunErrorCode::NotLoggedIn));
        assert_eq!(code(Fake::with(Box::new(|_| Err(RunError::NotInstalled)))), Some(RunErrorCode::NotInstalled));
        assert_eq!(code(Fake::with(Box::new(|_| Err(RunError::Timeout)))), Some(RunErrorCode::Timeout));
        assert_eq!(
            code(Fake::with(Box::new(|_| Err(RunError::Failed { code: Some(1), stderr: "boom".into() })))),
            Some(RunErrorCode::Other)
        );
        assert_eq!(code(Fake::with(Box::new(|_| Ok("not json".into())))), Some(RunErrorCode::InvalidOutput));
        assert_eq!(code(Fake::ok()), None);
        // the raw detail is kept next to the code
        let rec = go(&two_with(Fake::with(Box::new(|_| Err(RunError::Timeout)))));
        assert!(rec.error.as_deref().is_some_and(|e| e.contains("timed out")), "{rec:?}");
        // old records without the field still load
        let r: RunRecord = serde_json::from_value(serde_json::json!({
            "id": "x", "started_at": 1, "finished_at": null, "trigger": "manual", "tool": "claude",
            "status": "failed", "sessions_read": 0, "projects": 0, "suggestions_new": 0, "error": "boom"
        }))
        .unwrap();
        assert_eq!(r.error_code, None);
    }

    #[test]
    fn save_failure_has_io_code() {
        let e = two();
        let data = e._tmp.path().join("data");
        fs::remove_dir_all(&data).unwrap();
        fs::write(&data, "x").unwrap();
        assert_eq!(go(&e).error_code, Some(RunErrorCode::Io));
    }

    #[test]
    fn cancel_marks_cancelled() {
        let mut f = Fake::ok();
        f.cancel_on_call = true;
        let e = two_with(f);
        let rec = go(&e);
        assert_eq!(rec.status, RunStatus::Cancelled);
        assert_eq!(e.fake.calls().len(), 1, "stopped before the second project");
        assert_eq!(lock(&e.ctx.store).runs().last().unwrap().status, RunStatus::Cancelled);
        // runner reporting Cancelled also ends as Cancelled
        let e2 = two_with(Fake::with(Box::new(|_| Err(RunError::Cancelled))));
        assert_eq!(go(&e2).status, RunStatus::Cancelled);
    }

    #[test]
    fn second_run_rejected_while_running() {
        let e = two();
        let g = e.ctx.try_lock("held").unwrap();
        assert!(e.ctx.is_running());
        let c = AtomicBool::new(false);
        assert_eq!(
            start_run(&e.ctx, RunTrigger::Manual, &c, &|_| {}).err(),
            Some(ExtractError::AlreadyRunning)
        );
        assert!(e.fake.calls().is_empty());
        drop(g);
        assert!(!e.ctx.is_running());
        assert!(start_run(&e.ctx, RunTrigger::Manual, &c, &|_| {}).is_ok());
        assert!(!e.ctx.is_running(), "released after the run");
        // released on panic as well
        let ctx = e.ctx.clone();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _g = ctx.try_lock("p").unwrap();
            panic!("boom");
        }));
        assert!(r.is_err());
        assert!(!e.ctx.is_running());
    }

    #[test]
    fn orphan_running_marked_interrupted_on_start() {
        let tmp = tempfile::tempdir().unwrap();
        let rec = RunRecord {
            id: "old".into(),
            started_at: 1,
            finished_at: None,
            trigger: RunTrigger::Manual,
            tool: Tool::Claude,
            status: RunStatus::Running,
            sessions_read: 0,
            projects: 0,
            suggestions_new: 0,
            warnings: vec![],
            error: None,
            error_code: None,
        };
        let mut s = Store::load(tmp.path()).unwrap();
        s.push_run(rec);
        s.save().unwrap();
        let store = Arc::new(Mutex::new(Store::load(tmp.path()).unwrap()));
        let ctx = ExtractCtx::new(AgentPaths::from_home(tmp.path()), tmp.path().into(), store, vec![]);
        assert_eq!(lock(&ctx.store).runs()[0].status, RunStatus::Interrupted);
        assert!(!ctx.is_running());
    }

    #[test]
    fn runner_missing_fails_run() {
        let mut f = Fake::ok();
        f.avail = false;
        let e = two_with(f);
        let rec = go(&e);
        assert_eq!(rec.status, RunStatus::Failed);
        assert_eq!(rec.error.as_deref(), Some("not_installed"));
        assert!(e.fake.calls().is_empty());
        assert_eq!(lock(&e.ctx.store).runs().last().unwrap().status, RunStatus::Failed);
    }

    #[test]
    fn configured_tool_without_runner_fails() {
        let mut cfg = ExtractConfig::default();
        cfg.tool = Some(Tool::Codex);
        let t = tempfile::tempdir().unwrap();
        let tp = t.path().to_path_buf();
        let a = proj(&tp, "alpha", true);
        let turns = vec![turn(&a, "sa", NOW - 10)];
        let e = env_with(t, Fake::ok(), vec![a], turns, Some(cfg));
        let rec = go(&e);
        assert_eq!(rec.status, RunStatus::Failed);
        assert_eq!(rec.tool, Tool::Codex);
    }

    #[test]
    fn global_batch_uses_global_key_and_only_global_targets() {
        let t = tempfile::tempdir().unwrap();
        let tp = t.path().to_path_buf();
        let a = proj(&tp, "alpha", true);
        let mut g = turn(&a, "sg", NOW - 50);
        g.project_id = None;
        let e = env_with(t, Fake::ok(), vec![a], vec![g], None);
        fs::write(e.ctx.paths.claude_home.join("CLAUDE.md"), "# global\n").unwrap();
        let rec = go(&e);
        assert_eq!(rec.status, RunStatus::Succeeded, "{rec:?}");
        let calls = e.fake.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains("__global__"));
        assert!(!calls[0].contains("alpha"), "no project file among the targets");
        assert_eq!(wm(&e, GLOBAL_KEY), NOW - 50);
        // include_global = false ignores such sessions
        let mut cfg = ExtractConfig::default();
        cfg.include_global = false;
        let t2 = tempfile::tempdir().unwrap();
        let tp2 = t2.path().to_path_buf();
        let a2 = proj(&tp2, "alpha", true);
        let mut g2 = turn(&a2, "sg", NOW - 50);
        g2.project_id = None;
        let e2 = env_with(t2, Fake::ok(), vec![a2], vec![g2], Some(cfg));
        assert_eq!(go(&e2).projects, 0);
        assert!(e2.fake.calls().is_empty());
    }

    #[test]
    fn project_without_claude_md_gets_placeholder_target() {
        let t = tempfile::tempdir().unwrap();
        let tp = t.path().to_path_buf();
        let a = proj(&tp, "gamma", false);
        let turns = vec![turn(&a, "sa", NOW - 10)];
        let e = env_with(t, Fake::with(Box::new(|p| {
            // propose into the placeholder (first target that is gamma's CLAUDE.md)
            let fid = cap(r#"(?i)file_id="([^"]*gamma[^"]*claude\.md)""#, p);
            Ok(json!({"suggestions": [{"kind": "add", "file_id": fid, "new_text": "x",
                "reason": "r", "evidence": [{"session_id": cap(r#"session_id="([^"]+)""#, p),
                "ts": 1, "quote": "always run tests"}]}]})
            .to_string())
        })), vec![a], turns, None);
        let rec = go(&e);
        assert_eq!(rec.suggestions_new, 1, "{rec:?}");
        let st = lock(&e.ctx.store);
        assert!(st.suggestions()[0].path.ends_with("CLAUDE.md"));
        assert_eq!(st.suggestions()[0].base_hash, "");
    }

    #[test]
    fn at_most_20_projects_most_recent_first() {
        let t = tempfile::tempdir().unwrap();
        let tp = t.path().to_path_buf();
        let mut projects = vec![];
        let mut turns = vec![];
        for i in 0..22 {
            let p = proj(&tp, &format!("p{i:02}"), true);
            turns.push(turn(&p, &format!("s{i}"), NOW - 1000 - i as i64));
            projects.push(p);
        }
        let e = env_with(t, Fake::ok(), projects, turns, None);
        let rec = go(&e);
        assert_eq!(rec.projects, 20);
        assert_eq!(e.fake.calls().len(), 20);
        assert!(rec.warnings.iter().any(|w| w.contains("20")));
        // the two oldest were left out
        let all = e.fake.calls().join("\n");
        assert!(!all.contains("p21") && !all.contains("p20"));
    }

    #[test]
    fn selected_filter_limits_projects() {
        let t = tempfile::tempdir().unwrap();
        let tp = t.path().to_path_buf();
        let a = proj(&tp, "alpha", true);
        let b = proj(&tp, "beta", true);
        let turns = vec![turn(&a, "sa", NOW - 10), turn(&b, "sb", NOW - 20)];
        let mut cfg = ExtractConfig::default();
        cfg.projects = ProjectFilter::Selected(vec![b.id.clone()]);
        let e = env_with(t, Fake::ok(), vec![a, b], turns, Some(cfg));
        assert_eq!(go(&e).projects, 1);
        assert!(e.fake.calls()[0].contains("beta"));
    }

    // ---------------------------------------------------------- scheduler

    fn sched(e: &Env, s: Schedule) -> Arc<Scheduler> {
        let mut c = ExtractConfig::default();
        c.schedule = s;
        lock(&e.ctx.store).set_config(c);
        Scheduler::with_offset(e.ctx.clone(), Arc::new(|_| 0))
    }

    const MIN: i64 = 60_000;

    #[test]
    fn scheduler_fires_once_per_slot() {
        let e = two();
        let s = sched(&e, Schedule::Daily { hh: 9, mm: 30 });
        let slot = 100 * DAY_MS + (9 * 60 + 30) * MIN;
        assert!(s.tick(slot - MIN).is_none(), "before the slot");
        let r = s.tick(slot).expect("fires at the slot");
        assert_eq!(r.trigger, RunTrigger::Scheduled);
        assert!(s.tick(slot + 20_000).is_none(), "same minute, already fired");
        assert!(s.tick(slot + MIN).is_none());
        assert_eq!(lock(&e.ctx.store).runs().len(), 1);
        assert!(s.tick(slot + DAY_MS).is_some(), "next day fires again");
        assert_eq!(lock(&e.ctx.store).runs().len(), 2);
    }

    #[test]
    fn scheduler_weekly_and_off_and_busy_and_offset() {
        let e = two();
        // Day 3 since the epoch (1970-01-04) is a Sunday = 0.
        let s = sched(&e, Schedule::Weekly { weekday: 0, hh: 8, mm: 0 });
        assert!(s.tick(4 * DAY_MS + 8 * 60 * MIN).is_none(), "Monday");
        assert!(s.tick(3 * DAY_MS + 8 * 60 * MIN).is_some(), "Sunday 08:00");
        assert!(s.tick(10 * DAY_MS + 8 * 60 * MIN).is_some(), "next Sunday");

        let e2 = two();
        let off = sched(&e2, Schedule::Off);
        assert!(off.tick(100 * DAY_MS).is_none());

        // a run in progress: slot is skipped and not caught up later in the minute
        let e3 = two();
        let s3 = sched(&e3, Schedule::Daily { hh: 0, mm: 0 });
        let g = e3.ctx.try_lock("busy").unwrap();
        assert!(s3.tick(50 * DAY_MS).is_none());
        drop(g);
        assert!(s3.tick(50 * DAY_MS + 5_000).is_none());
        assert!(e3.fake.calls().is_empty());

        // local time: UTC+8 shifts the slot
        let e4 = two();
        let mut c = ExtractConfig::default();
        c.schedule = Schedule::Daily { hh: 9, mm: 0 };
        lock(&e4.ctx.store).set_config(c);
        let s4 = Scheduler::with_offset(e4.ctx.clone(), Arc::new(|_| 8 * 3600));
        assert!(s4.tick(100 * DAY_MS + 9 * 60 * MIN).is_none(), "09:00 UTC is 17:00 local");
        assert!(s4.tick(100 * DAY_MS + 60 * MIN).is_some(), "01:00 UTC is 09:00 local");
    }

    #[test]
    fn scheduler_thread_starts_and_stops() {
        let e = two();
        let s = sched(&e, Schedule::Off);
        s.start();
        s.stop();
    }

    // ------------------------------------------------- chunking / fixes

    /// One project with `n` large turns (ts ascending, 6000 chars each).
    fn big(n: usize, fake: Fake) -> (Env, Vec<i64>) {
        let t = tempfile::tempdir().unwrap();
        let tp = t.path().to_path_buf();
        let a = proj(&tp, "alpha", true);
        let mut turns = vec![];
        for i in 0..n {
            let mut x = turn(&a, "sa", NOW - 1_000_000 + i as i64);
            x.text = format!("always run tests {}", "y".repeat(6000));
            turns.push(x);
        }
        let ts = turns.iter().map(|x| x.ts).collect();
        (env_with(t, fake, vec![a], turns, None), ts)
    }

    fn prompt_ts(p: &str) -> Vec<i64> {
        Regex::new(r#" ts="(\d+)""#).unwrap().captures_iter(p).map(|c| c[1].parse().unwrap()).collect()
    }

    #[test]
    fn oversized_batch_is_chunked_in_order() {
        let (e, ts) = big(14, Fake::ok());
        let rec = go(&e);
        assert_eq!(rec.status, RunStatus::Succeeded, "{rec:?}");
        let calls = e.fake.calls();
        assert!(calls.len() >= 2 && calls.len() <= MAX_CHUNKS, "{}", calls.len());
        let seen: Vec<i64> = calls.iter().flat_map(|c| prompt_ts(c)).collect();
        assert_eq!(seen, ts, "every turn once, oldest first");
        assert_eq!(wm(&e, &pid(&e, "alpha")), *ts.last().unwrap());
        assert!(!rec.warnings.iter().any(|w| w.contains("deferred")));
    }

    #[test]
    fn more_than_three_chunks_defer_and_nothing_is_skipped_across_runs() {
        let (e, ts) = big(40, Fake::ok());
        let rec = go(&e);
        assert_eq!(e.fake.calls().len(), MAX_CHUNKS);
        let first: Vec<i64> = e.fake.calls().iter().flat_map(|c| prompt_ts(c)).collect();
        assert_eq!(wm(&e, &pid(&e, "alpha")), *first.last().unwrap(), "watermark = last processed chunk");
        let left = ts.len() - first.len();
        assert!(left > 0);
        assert!(
            rec.warnings.iter().any(|w| w.contains(&format!("{left} newer turns deferred"))),
            "{:?}",
            rec.warnings
        );
        // keep running until nothing is left
        for _ in 0..10 {
            go(&e);
        }
        let all: Vec<i64> = e.fake.calls().iter().flat_map(|c| prompt_ts(c)).collect();
        assert_eq!(all, ts, "each turn processed exactly once, in order");
    }

    #[test]
    fn failed_chunk_stops_project_without_advancing_past_it() {
        let n = Mutex::new(0);
        let (e, ts) = big(14, Fake::with(Box::new(move |p| {
            let mut c = n.lock().unwrap();
            *c += 1;
            if *c == 2 { Err(RunError::Timeout) } else { Ok(reply(p)) }
        })));
        let rec = go(&e);
        assert_eq!(rec.status, RunStatus::Succeeded);
        assert_eq!(e.fake.calls().len(), 2, "stopped after the failing chunk");
        let first = prompt_ts(&e.fake.calls()[0]);
        assert_eq!(wm(&e, &pid(&e, "alpha")), *first.last().unwrap());
        assert!(*first.last().unwrap() < *ts.last().unwrap());
        assert!(rec.warnings.iter().any(|w| w.contains("timed out")));
    }

    #[test]
    fn panic_in_runner_marks_run_failed() {
        let e = two_with(Fake::with(Box::new(|_| panic!("runner exploded"))));
        let c = AtomicBool::new(false);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            start_run(&e.ctx, RunTrigger::Manual, &c, &|_| {})
        }));
        assert!(r.is_err());
        assert!(!e.ctx.is_running());
        let st = lock(&e.ctx.store);
        let last = st.runs().last().unwrap();
        assert_eq!(last.status, RunStatus::Failed);
        assert!(last.finished_at.is_some());
    }

    #[test]
    fn save_failure_is_surfaced() {
        let e = two();
        // Replace the data dir with a file so every save fails.
        let data = e._tmp.path().join("data");
        fs::remove_dir_all(&data).unwrap();
        fs::write(&data, "x").unwrap();
        let rec = go(&e);
        assert!(rec.warnings.iter().any(|w| w.contains("could not save")), "{:?}", rec.warnings);
        assert!(rec.error.is_some());
    }

    #[test]
    fn global_placeholder_needs_claude_home() {
        let t = tempfile::tempdir().unwrap();
        let tp = t.path().to_path_buf();
        let a = proj(&tp, "alpha", true);
        let mut g = turn(&a, "sg", NOW - 50);
        g.project_id = None;
        let e = env_with(t, Fake::ok(), vec![a], vec![g], None);
        fs::remove_dir_all(&e.ctx.paths.claude_home).unwrap();
        let rec = go(&e);
        assert_eq!(e.fake.calls().len(), 0, "no target at all, skipped: {rec:?}");
        assert!(rec.warnings.iter().any(|w| w.contains("no memory file")));
    }

    #[test]
    fn scheduler_stop_cancels_long_run_quickly() {
        let mut f = Fake::ok();
        f.slow = true;
        let e = two_with(f);
        // The ctx clock is NOW = day 20, 00:00 UTC, which is the slot.
        let s = sched(&e, Schedule::Daily { hh: 0, mm: 0 });
        s.start();
        let t0 = std::time::Instant::now();
        while e.fake.calls().is_empty() && t0.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!e.fake.calls().is_empty(), "scheduled run started");
        let t1 = std::time::Instant::now();
        s.stop();
        assert!(t1.elapsed() < Duration::from_secs(3), "stop returned quickly");
        assert_eq!(lock(&e.ctx.store).runs().last().unwrap().status, RunStatus::Cancelled);
        assert!(!e.ctx.is_running());
    }

    #[test]
    fn scheduler_does_not_fire_twice_on_dst_fall_back() {
        let e = two();
        let mut c = ExtractConfig::default();
        c.schedule = Schedule::Daily { hh: 1, mm: 30 };
        lock(&e.ctx.store).set_config(c);
        let t0 = 100 * DAY_MS;
        // UTC+1 until 01:00 UTC, then UTC+0: local 01:30 happens at 00:30 and again at 01:30 UTC.
        let off: OffsetFn = Arc::new(move |now| if now < t0 + 60 * MIN { 3600 } else { 0 });
        let s = Scheduler::with_offset(e.ctx.clone(), off);
        assert!(s.tick(t0 + 30 * MIN).is_some());
        assert!(s.tick(t0 + 90 * MIN).is_none(), "repeated local hour");
        assert_eq!(lock(&e.ctx.store).runs().len(), 1);
    }

    #[test]
    fn tick_after_stop_does_not_start_a_run() {
        let e = two();
        let s = sched(&e, Schedule::Daily { hh: 0, mm: 0 });
        s.stop.store(true, Ordering::SeqCst);
        e.ctx.cancel.store(true, Ordering::SeqCst);
        assert!(s.tick(100 * DAY_MS).is_none());
        assert!(lock(&e.ctx.store).runs().is_empty());
        assert!(e.ctx.cancel.load(Ordering::SeqCst), "cancel not reset");
    }
}
