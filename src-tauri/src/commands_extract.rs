//! Tauri commands, events and shared state for memory extraction and the
//! suggestion review queue.
//!
//! Locking: `write_lock` (shared with the memory editor, injected into
//! `ApplyCtx` as its apply lock) is always taken before the extract store lock;
//! nothing takes `write_lock` while holding the store lock, and `authorize`
//! never touches the store. Store locks are short (no file IO under them
//! except the atomic JSON save).
//!
//! Events: `extract-progress` (Progress), `extract-finished` (RunRecord),
//! `suggestions-changed` (PendingSummary; after runs, applies and rejects) and
//! `memory-changed` (after a suggestion was written into a memory file).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use crate::commands::{lock, AppState, CommandError};
use crate::extract::apply::{self, ApplyCtx, ApplyError, ApplyOutcome};
use crate::extract::job::{
    choose_runner, start_run_with, ExtractCtx, ExtractError, Hooks, Progress, Scheduler,
};
use crate::extract::prompt::{redact, Evidence, Suggestion, SuggestionStatus};
use crate::extract::runner::{CliRunner, RunError};
use crate::extract::sessions::{context_around, Turn};
use crate::extract::store::{ExtractConfig, ProjectFilter, RunRecord, RunTrigger, Schedule, Store};
use crate::model::{MemoryIndex, Tool};
use crate::projects::normalize_path;

/// Key used in `PendingSummary::by_project` for files outside any project.
pub const GLOBAL_PROJECT: &str = "__global__";
/// Turns shown before / after each evidence turn.
const CONTEXT_TURNS: usize = 4;

/// Event sink: `(event name, JSON payload)`.
pub type Emit = Arc<dyn Fn(&str, Value) + Send + Sync>;

struct Inner {
    store: Arc<Mutex<Store>>,
    ctx: Arc<ExtractCtx>,
    apply: ApplyCtx,
    scheduler: Arc<Scheduler>,
}

/// Extraction runtime managed by Tauri next to `AppState`. When the store files
/// could not be loaded (corrupt), every command fails with an `io` error that
/// carries the reason; nothing is silently reset.
pub struct ExtractState {
    app: Arc<AppState>,
    inner: Result<Inner, String>,
    emitter: Mutex<Option<Emit>>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct PendingSummary {
    pub total: u32,
    pub by_project: BTreeMap<String, u32>,
}

#[derive(Serialize, Debug, Clone)]
pub struct RunnerInfo {
    pub tool: Tool,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
}

#[derive(Serialize, Debug, Clone)]
pub struct ContextItem {
    pub evidence: Evidence,
    pub turns: Vec<Turn>,
}

#[derive(Serialize, Debug, Clone)]
pub struct AcceptResult {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ApplyOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CommandError>,
}

#[derive(Serialize, Clone)]
struct MemoryChanged {
    paths: Vec<String>,
}

impl From<ApplyError> for CommandError {
    fn from(e: ApplyError) -> Self {
        match e {
            ApplyError::NotFound => CommandError::new("not_found", "suggestion not found"),
            ApplyError::NotPending => CommandError::new("not_pending", "suggestion is not pending"),
            ApplyError::Invalid { message } => CommandError::new("invalid", message),
            ApplyError::Io { message } => CommandError::new("io", message),
        }
    }
}

impl From<RunError> for CommandError {
    fn from(e: RunError) -> Self {
        match e {
            RunError::NotInstalled => CommandError::new("not_installed", "CLI is not installed"),
            RunError::NotLoggedIn(m) => CommandError::new("not_logged_in", m),
            RunError::Timeout => CommandError::new("io", "CLI timed out"),
            RunError::Cancelled => CommandError::new("io", "cancelled"),
            RunError::Failed { stderr, .. } => CommandError::new("io", stderr),
        }
    }
}

impl From<ExtractError> for CommandError {
    fn from(e: ExtractError) -> Self {
        match e {
            ExtractError::AlreadyRunning => {
                CommandError::new("already_running", "an extraction is already running")
            }
        }
    }
}

/// Project id (plan 1) of a memory file path; `__global__` for global files
/// and anything outside the known projects.
fn project_of(idx: &MemoryIndex, path: &str) -> String {
    let id = normalize_path(path);
    if idx.global.iter().any(|f| f.id == id) {
        return GLOBAL_PROJECT.into();
    }
    if let Some(pm) = idx.projects.iter().find(|pm| pm.files.iter().any(|f| f.id == id)) {
        return pm.project.id.clone();
    }
    // Not indexed yet (e.g. a CLAUDE.md that does not exist): longest project root prefix.
    idx.projects
        .iter()
        .filter_map(|pm| {
            let root = normalize_path(&pm.project.path);
            let root = root.trim_end_matches('/');
            id.starts_with(&format!("{root}/")).then(|| (root.len(), pm.project.id.clone()))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, pid)| pid)
        .unwrap_or_else(|| GLOBAL_PROJECT.into())
}

fn compute_summary(app: &AppState, store: &Mutex<Store>) -> PendingSummary {
    // Index first, then the store lock: never nest them.
    let idx = app.snapshot_index();
    let st = lock(store);
    let by_project = st
        .pending_by_project(|s| Some(project_of(&idx, &s.path)))
        .into_iter()
        .collect();
    PendingSummary { total: st.pending_count(), by_project }
}

fn emit_to(emit: &Emit, name: &str, payload: &impl Serialize) {
    if let Ok(v) = serde_json::to_value(payload) {
        emit(name, v);
    }
}

impl ExtractState {
    /// Loads the store from the app data directory. `customize` may adjust the
    /// context (tests inject turn sources / clocks); the index provider is the
    /// app's cached `MemoryIndex` (includes manual projects).
    pub fn build(
        app: Arc<AppState>,
        runners: Vec<Arc<dyn CliRunner>>,
        customize: impl FnOnce(&mut ExtractCtx),
    ) -> ExtractState {
        let inner = Store::load(&app.data_dir).map(|store| {
            let store = Arc::new(Mutex::new(store));
            let mut ctx =
                ExtractCtx::new(app.paths.clone(), app.home.clone(), store.clone(), runners);
            let a = app.clone();
            ctx.index = Arc::new(move || a.snapshot_index());
            customize(&mut ctx);
            let ctx = Arc::new(ctx);
            let apply = ApplyCtx::new(store.clone(), app.snapshots.clone(), app.write_lock.clone());
            let scheduler = Scheduler::new(ctx.clone());
            Inner { store, ctx, apply, scheduler }
        });
        ExtractState { app, inner, emitter: Mutex::new(None) }
    }

    /// Production state: CLI runners found on PATH.
    pub fn new(app: Arc<AppState>) -> ExtractState {
        Self::build(app, crate::extract::runner::detect_runners_from_env(), |_| {})
    }

    fn inner(&self) -> Result<&Inner, CommandError> {
        self.inner
            .as_ref()
            .map_err(|e| CommandError::new("io", format!("extraction data unavailable: {e}")))
    }

    pub fn store(&self) -> Result<Arc<Mutex<Store>>, CommandError> {
        Ok(self.inner()?.store.clone())
    }

    /// Connects the event sink and makes manual and scheduled runs report
    /// progress / completion through it.
    pub fn set_emitter(&self, emit: Emit) {
        *lock(&self.emitter) = Some(emit.clone());
        let Ok(inner) = self.inner() else { return };
        let (e1, e2) = (emit.clone(), emit);
        let app = self.app.clone();
        let store = inner.store.clone();
        inner.ctx.set_hooks(Hooks {
            on_progress: Some(Arc::new(move |p: &Progress| emit_to(&e1, "extract-progress", p))),
            on_finished: Some(Arc::new(move |r: &RunRecord| {
                emit_to(&e2, "extract-finished", r);
                emit_to(&e2, "suggestions-changed", &compute_summary(&app, &store));
            })),
        });
    }

    /// Wires events to a Tauri app handle.
    pub fn attach(&self, handle: AppHandle) {
        self.set_emitter(Arc::new(move |name, payload| {
            let _ = handle.emit(name, payload);
        }));
    }

    fn emit(&self, name: &str, payload: &impl Serialize) {
        let emit = lock(&self.emitter).clone();
        if let Some(e) = emit {
            emit_to(&e, name, payload);
        }
    }

    fn emit_summary(&self) {
        if let Ok(inner) = self.inner() {
            self.emit("suggestions-changed", &compute_summary(&self.app, &inner.store));
        }
    }

    pub fn start_scheduler(&self) {
        if let Ok(inner) = self.inner() {
            inner.scheduler.start();
        }
    }

    /// Stops the scheduler and cancels a running extraction (app exit).
    pub fn shutdown(&self) {
        if let Ok(inner) = self.inner() {
            inner.scheduler.stop();
        }
    }
}

// ------------------------------------------------------------------ config

fn validate_config(c: &ExtractConfig) -> Result<(), CommandError> {
    let bad = |m: &str| Err(CommandError::new("invalid", m));
    if !(1..=90).contains(&c.lookback_days) {
        return bad("lookback_days must be 1..=90");
    }
    let time_ok = |hh: u8, mm: u8| hh <= 23 && mm <= 59;
    match c.schedule {
        Schedule::Off => {}
        Schedule::Daily { hh, mm } if !time_ok(hh, mm) => return bad("invalid schedule time"),
        Schedule::Weekly { weekday, hh, mm } if weekday > 6 || !time_ok(hh, mm) => {
            return bad("invalid weekly schedule");
        }
        _ => {}
    }
    if matches!(&c.projects, ProjectFilter::Selected(ids) if ids.is_empty()) {
        return bad("select at least one project");
    }
    Ok(())
}

pub fn get_extract_config_impl(es: &ExtractState) -> Result<ExtractConfig, CommandError> {
    Ok(lock(&es.inner()?.store).config().clone())
}

pub fn set_extract_config_impl(
    es: &ExtractState,
    config: ExtractConfig,
) -> Result<ExtractConfig, CommandError> {
    validate_config(&config)?;
    let mut st = lock(&es.inner()?.store);
    let old = st.config().clone();
    st.set_config(config);
    if let Err(e) = st.save() {
        st.set_config(old);
        return Err(e.into());
    }
    Ok(st.config().clone())
}

pub fn extract_runners_impl(es: &ExtractState) -> Vec<RunnerInfo> {
    let Ok(inner) = es.inner() else {
        return [Tool::Claude, Tool::Codex]
            .into_iter()
            .map(|tool| RunnerInfo { tool, available: false, program: None })
            .collect();
    };
    [Tool::Claude, Tool::Codex]
        .into_iter()
        .map(|tool| match inner.ctx.runners.iter().find(|r| r.tool() == tool) {
            Some(r) => RunnerInfo { tool, available: r.available(), program: r.program() },
            None => RunnerInfo { tool, available: false, program: None },
        })
        .collect()
}

// -------------------------------------------------------------------- runs

/// Starts a manual run in a background thread and returns its Running record.
pub fn start_extraction_impl(es: &ExtractState) -> Result<RunRecord, CommandError> {
    let inner = es.inner()?;
    let ctx = inner.ctx.clone();
    if ctx.is_running() {
        return Err(ExtractError::AlreadyRunning.into());
    }
    let tool = lock(&inner.store).config().tool;
    if choose_runner(&ctx, tool).is_none() {
        return Err(RunError::NotInstalled.into());
    }
    let (tx, rx) = std::sync::mpsc::channel::<Result<RunRecord, ExtractError>>();
    let spawned = std::thread::Builder::new().name("extract-run".into()).spawn(move || {
        let started_tx = tx.clone();
        let c2 = ctx.clone();
        let res = start_run_with(
            &ctx,
            RunTrigger::Manual,
            &ctx.cancel,
            &|_| {},
            &move |r| {
                // The lock is held and the record stored: from here on cancel applies to this run.
                c2.cancel.store(false, std::sync::atomic::Ordering::SeqCst);
                let _ = started_tx.send(Ok(r.clone()));
            },
        );
        if let Err(e) = res {
            let _ = tx.send(Err(e));
        }
    });
    spawned.map_err(CommandError::from)?;
    match rx.recv() {
        Ok(r) => r.map_err(CommandError::from),
        Err(_) => Err(CommandError::new("io", "extraction thread ended before starting")),
    }
}

/// Cancels the running extraction (manual or scheduled).
pub fn cancel_extraction_impl(es: &ExtractState) -> Result<(), CommandError> {
    es.inner()?.ctx.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

/// Run records, newest first.
pub fn list_extract_runs_impl(es: &ExtractState) -> Result<Vec<RunRecord>, CommandError> {
    let mut runs = lock(&es.inner()?.store).runs().to_vec();
    runs.reverse();
    Ok(runs)
}

// ------------------------------------------------------------- suggestions

/// Suggestions (optionally of one status), newest first.
pub fn list_suggestions_impl(
    es: &ExtractState,
    status: Option<SuggestionStatus>,
) -> Result<Vec<Suggestion>, CommandError> {
    let mut v: Vec<Suggestion> = lock(&es.inner()?.store)
        .suggestions()
        .iter()
        .filter(|s| status.is_none_or(|st| s.status == st))
        .cloned()
        .collect();
    v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(v)
}

pub fn pending_summary_impl(es: &ExtractState) -> Result<PendingSummary, CommandError> {
    Ok(compute_summary(&es.app, &es.inner()?.store))
}

fn suggestion_path(es: &ExtractState, id: &str) -> Option<String> {
    let inner = es.inner().ok()?;
    let st = lock(&inner.store);
    st.suggestions().iter().find(|s| s.id == id).map(|s| s.path.clone())
}

/// After a write: refresh the cached index and tell the UI which file changed.
fn announce_write(es: &ExtractState, path: &str) {
    es.app.rescan();
    es.emit("memory-changed", &MemoryChanged { paths: vec![normalize_path(path)] });
}

pub fn apply_suggestion_impl(
    es: &ExtractState,
    id: &str,
    edited_new_text: Option<String>,
    confirm_changed: bool,
) -> Result<ApplyOutcome, CommandError> {
    let inner = es.inner()?;
    let path = suggestion_path(es, id);
    let out = apply::apply(&inner.apply, id, edited_new_text, confirm_changed)?;
    match &out {
        ApplyOutcome::Applied { .. } => {
            if let Some(p) = path {
                announce_write(es, &p);
            }
            es.emit_summary();
        }
        ApplyOutcome::Stale | ApplyOutcome::AlreadyApplied => es.emit_summary(),
        ApplyOutcome::NeedsConfirm { .. } => {}
    }
    Ok(out)
}

pub fn reject_suggestion_impl(es: &ExtractState, id: &str) -> Result<(), CommandError> {
    apply::reject(&es.inner()?.apply, id)?;
    es.emit_summary();
    Ok(())
}

pub fn accept_suggestions_impl(
    es: &ExtractState,
    ids: &[String],
) -> Result<Vec<AcceptResult>, CommandError> {
    let inner = es.inner()?;
    let paths: Vec<Option<String>> = ids.iter().map(|i| suggestion_path(es, i)).collect();
    let results = apply::accept_many(&inner.apply, ids);
    let mut out = Vec::with_capacity(results.len());
    let mut changed = false;
    for ((id, res), path) in results.into_iter().zip(paths) {
        match res {
            Ok(o) => {
                if matches!(o, ApplyOutcome::Applied { .. }) {
                    if let Some(p) = &path {
                        announce_write(es, p);
                    }
                }
                changed |= !matches!(o, ApplyOutcome::NeedsConfirm { .. });
                out.push(AcceptResult { id, outcome: Some(o), error: None });
            }
            Err(e) => out.push(AcceptResult { id, outcome: None, error: Some(e.into()) }),
        }
    }
    if changed {
        es.emit_summary();
    }
    Ok(out)
}

/// For each evidence of the suggestion: the turns around it (4 before, 4 after),
/// redacted.
pub fn suggestion_context_impl(es: &ExtractState, id: &str) -> Result<Vec<ContextItem>, CommandError> {
    let inner = es.inner()?;
    let s = lock(&inner.store)
        .suggestions()
        .iter()
        .find(|s| s.id == id)
        .cloned()
        .ok_or(ApplyError::NotFound)?;
    Ok(s.evidence
        .into_iter()
        .map(|mut ev| {
            let mut turns = context_around(
                &inner.ctx.paths,
                ev.tool,
                &ev.session_id,
                ev.ts,
                CONTEXT_TURNS,
                CONTEXT_TURNS,
            );
            for t in &mut turns {
                t.text = redact(&t.text);
            }
            ev.quote = redact(&ev.quote);
            ContextItem { evidence: ev, turns }
        })
        .collect())
}

// ---------------------------------------------------------------- commands

type Es<'a> = State<'a, Arc<ExtractState>>;

#[tauri::command(async)]
pub fn get_extract_config(es: Es) -> Result<ExtractConfig, CommandError> {
    get_extract_config_impl(&es)
}

#[tauri::command(async)]
pub fn set_extract_config(es: Es, config: ExtractConfig) -> Result<ExtractConfig, CommandError> {
    set_extract_config_impl(&es, config)
}

#[tauri::command(async)]
pub fn extract_runners(es: Es) -> Vec<RunnerInfo> {
    extract_runners_impl(&es)
}

#[tauri::command(async)]
pub fn start_extraction(es: Es) -> Result<RunRecord, CommandError> {
    start_extraction_impl(&es)
}

#[tauri::command(async)]
pub fn cancel_extraction(es: Es) -> Result<(), CommandError> {
    cancel_extraction_impl(&es)
}

#[tauri::command(async)]
pub fn list_extract_runs(es: Es) -> Result<Vec<RunRecord>, CommandError> {
    list_extract_runs_impl(&es)
}

#[tauri::command(async)]
pub fn list_suggestions(
    es: Es,
    status: Option<SuggestionStatus>,
) -> Result<Vec<Suggestion>, CommandError> {
    list_suggestions_impl(&es, status)
}

#[tauri::command(async)]
pub fn pending_summary(es: Es) -> Result<PendingSummary, CommandError> {
    pending_summary_impl(&es)
}

#[tauri::command(async)]
pub fn apply_suggestion(
    es: Es,
    id: String,
    edited_new_text: Option<String>,
    confirm_changed: bool,
) -> Result<ApplyOutcome, CommandError> {
    apply_suggestion_impl(&es, &id, edited_new_text, confirm_changed)
}

#[tauri::command(async)]
pub fn reject_suggestion(es: Es, id: String) -> Result<(), CommandError> {
    reject_suggestion_impl(&es, &id)
}

#[tauri::command(async)]
pub fn accept_suggestions(es: Es, ids: Vec<String>) -> Result<Vec<AcceptResult>, CommandError> {
    accept_suggestions_impl(&es, &ids)
}

#[tauri::command(async)]
pub fn suggestion_context(es: Es, id: String) -> Result<Vec<ContextItem>, CommandError> {
    suggestion_context_impl(&es, &id)
}

#[cfg(test)]
#[path = "commands_extract_tests.rs"]
mod tests;
