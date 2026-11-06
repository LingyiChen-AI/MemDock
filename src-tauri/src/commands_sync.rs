//! Tauri commands for GitHub sync: login (device flow), repository binding,
//! sync start / finish, history and restore. Logic lives in `*_impl`
//! functions; the `#[tauri::command]` wrappers only delegate.
//!
//! Events: `sync://login` (`LoginEvent`) and `sync://status-changed`
//! (`SyncStatusDto`). The access token never appears in an event, error or log.
//!
//! Locks: `sync.lock` serializes every command that syncs or changes the
//! login / repository state (`try_lock` -> `sync_busy`). Order:
//! `sync.lock -> write_lock -> index` (the engine takes `write_lock` itself).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError, Weak};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use crate::sync::engine::SecretPolicy;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use crate::commands::{lock, AppState, CommandError};
use crate::memory::io::{read_memory, write_memory, WriteError};
use crate::projects::normalize_path;
use crate::sync::engine::{
    self, finish, prepare, AbsentFile, ConflictFile, FileSecretHit, Resolution, SkippedFile,
    SyncCtx, SyncError, SyncPlan, SyncSummary,
};
use crate::sync::github::{
    device_login, ensure_repo_with, ApiError, DeviceCode, GhRepo, GhUser, GitHubApi, HttpGitHub,
    LoginError, PollResult, RepoOutcome, CLIENT_ID,
};
use crate::sync::layout::{
    dir_key, expected_local_path, load_or_create_machine, map_files, project_key, safe_key,
    Machine, SyncFile,
};
use crate::sync::manifest::{local_entries, lock_entries_for_sync, ManifestSkill};
use crate::sync::repo::{Auth, CommitInfo, SyncGit, SyncGitError};
use crate::sync::state::{load_state, save_state, SyncState};
use crate::sync::token::{KeyringStore, TokenStore};

/// The only URL `open_verification_url` opens.
pub const VERIFICATION_URL: &str = "https://github.com/login/device";
const LOGIN_EVENT: &str = "sync://login";
const STATUS_EVENT: &str = "sync://status-changed";
const MEMORY_EVENT: &str = "memory-changed";
const SLEEP_SLICE: Duration = Duration::from_millis(250);
const HISTORY_MAX: usize = 200;

pub type Emit = Arc<dyn Fn(&str, Value) + Send + Sync>;
type CloneUrl = Box<dyn Fn(&str, &str) -> String + Send + Sync>;
type Sleep = Arc<dyn Fn(Duration) + Send + Sync>;

// ------------------------------------------------------------------ runtime

/// `HttpGitHub` built on first use: its proxy discovery can block for seconds,
/// so it must never run on the UI thread at startup.
#[derive(Default)]
struct LazyHttpGitHub(OnceLock<HttpGitHub>);

impl LazyHttpGitHub {
    fn get(&self) -> &HttpGitHub {
        self.0.get_or_init(HttpGitHub::new)
    }
}

impl GitHubApi for LazyHttpGitHub {
    fn device_code(&self, client_id: &str) -> Result<DeviceCode, ApiError> {
        self.get().device_code(client_id)
    }
    fn poll_token(&self, client_id: &str, device_code: &str) -> Result<PollResult, ApiError> {
        self.get().poll_token(client_id, device_code)
    }
    fn user(&self, token: &str) -> Result<GhUser, ApiError> {
        self.get().user(token)
    }
    fn get_repo(&self, token: &str, owner: &str, name: &str) -> Result<Option<GhRepo>, ApiError> {
        self.get().get_repo(token, owner, name)
    }
    fn create_repo(&self, token: &str, name: &str) -> Result<GhRepo, ApiError> {
        self.get().create_repo(token, name)
    }
    fn get_file(
        &self,
        token: &str,
        owner: &str,
        name: &str,
        path: &str,
    ) -> Result<Option<Vec<u8>>, ApiError> {
        self.get().get_file(token, owner, name, path)
    }
    fn put_file(
        &self,
        token: &str,
        owner: &str,
        name: &str,
        path: &str,
        bytes: &[u8],
        message: &str,
    ) -> Result<(), ApiError> {
        self.get().put_file(token, owner, name, path, bytes, message)
    }
    fn list_root(&self, token: &str, owner: &str, name: &str) -> Result<Vec<String>, ApiError> {
        self.get().list_root(token, owner, name)
    }
}

/// Sync part of `AppState`.
pub struct SyncRuntime {
    /// Serializes sync and login-state-changing commands.
    pub lock: Mutex<()>,
    /// Plans waiting for the user's resolutions, by plan id (never sent whole
    /// to the frontend).
    pub plans: Mutex<HashMap<String, SyncPlan>>,
    /// Secret handling chosen for the sync in progress (reset by every `sync_start`).
    pub secret_policy: Mutex<SecretPolicy>,
    pub login_cancel: AtomicBool,
    /// A device-flow login thread is running.
    login_active: AtomicBool,
    pub api: Arc<dyn GitHubApi>,
    pub tokens: Arc<dyn TokenStore>,
    pub git: SyncGit,
    /// `(owner, name) -> clone URL` (tests point it at a `file://` bare repo).
    pub clone_url: CloneUrl,
    /// Sleep used by device-flow polling and repo bootstrap retries.
    pub sleep: Sleep,
    /// Quiet period before a memory change re-emits the sync status.
    pub status_debounce: Duration,
    emitter: Mutex<Option<Emit>>,
    status_tx: Mutex<Option<Sender<()>>>,
    /// Last `pending` sent with `sync://status-changed` (-1: none yet).
    last_pending: AtomicI64,
    /// Whether the token store holds a token, as last seen. Status reads use
    /// this so they never touch the keychain; `None` until first known.
    has_token: Mutex<Option<bool>>,
}

fn github_clone_url(owner: &str, name: &str) -> String {
    format!("https://github.com/{owner}/{name}.git")
}

impl SyncRuntime {
    /// Real GitHub (built lazily), system keychain, system git.
    pub fn production() -> Self {
        Self::new(
            Arc::new(LazyHttpGitHub::default()),
            Arc::new(KeyringStore),
            SyncGit::new(),
        )
    }

    pub fn new(api: Arc<dyn GitHubApi>, tokens: Arc<dyn TokenStore>, git: SyncGit) -> Self {
        SyncRuntime {
            lock: Mutex::new(()),
            plans: Mutex::new(HashMap::new()),
            secret_policy: Mutex::new(SecretPolicy::default()),
            login_cancel: AtomicBool::new(false),
            login_active: AtomicBool::new(false),
            api,
            tokens,
            git,
            clone_url: Box::new(github_clone_url),
            sleep: Arc::new(std::thread::sleep),
            status_debounce: Duration::from_secs(1),
            emitter: Mutex::new(None),
            status_tx: Mutex::new(None),
            last_pending: AtomicI64::new(-1),
            has_token: Mutex::new(None),
        }
    }

    fn set_has_token(&self, v: bool) {
        *lock(&self.has_token) = Some(v);
    }

    /// Reads the token store (keychain) and refreshes the cached flag.
    fn read_token(&self) -> Option<String> {
        let t = self.tokens.get().ok().flatten().filter(|t| !t.is_empty());
        self.set_has_token(t.is_some());
        t
    }

    /// The stored GitHub token, if any (reads the keychain). Used by the
    /// GitHub import to raise rate limits and reach private repositories.
    pub(crate) fn import_token(&self) -> Option<String> {
        self.read_token()
    }

    fn store_token(&self, t: &str) -> Result<(), String> {
        self.tokens.set(t)?;
        self.set_has_token(true);
        Ok(())
    }

    fn clear_token(&self) -> Result<(), String> {
        let r = self.tokens.clear();
        if r.is_ok() {
            self.set_has_token(false);
        }
        r
    }

    pub fn set_emitter(&self, emit: Emit) {
        *lock(&self.emitter) = Some(emit);
    }

    /// Wires events to a Tauri app handle.
    pub fn attach(&self, handle: AppHandle) {
        self.set_emitter(Arc::new(move |name, payload| {
            let _ = handle.emit(name, payload);
        }));
    }

    fn emit(&self, name: &str, payload: &impl Serialize) {
        let emit = lock(&self.emitter).clone();
        if let (Some(e), Ok(v)) = (emit, serde_json::to_value(payload)) {
            e(name, v);
        }
    }

    /// Called by the memory watcher after `memory-changed`: re-checks
    /// `pending` after a quiet period and emits `sync://status-changed` when
    /// it changed.
    pub fn memory_changed(&self) {
        if let Some(tx) = lock(&self.status_tx).as_ref() {
            let _ = tx.send(());
        }
    }

    /// Sleeps `d` in short slices, stopping early once login is cancelled.
    fn login_sleep(&self, d: Duration) {
        let mut left = d;
        while !left.is_zero() && !self.login_cancel.load(Ordering::SeqCst) {
            let s = left.min(SLEEP_SLICE);
            (self.sleep)(s);
            left -= s;
        }
    }
}

/// Starts the debounce thread behind `SyncRuntime::memory_changed`.
pub fn start_status_watch(state: &Arc<AppState>) {
    let (tx, rx) = mpsc::channel::<()>();
    *lock(&state.sync.status_tx) = Some(tx);
    let weak: Weak<AppState> = Arc::downgrade(state);
    let quiet = state.sync.status_debounce;
    std::thread::spawn(move || loop {
        if rx.recv().is_err() {
            return;
        }
        loop {
            match rx.recv_timeout(quiet) {
                Ok(()) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        let Some(state) = weak.upgrade() else { return };
        let st = sync_status_impl(&state);
        let prev = state.sync.last_pending.swap(st.pending as i64, Ordering::SeqCst);
        if prev != st.pending as i64 {
            state.sync.emit(STATUS_EVENT, &st);
        }
    });
}

// ---------------------------------------------------------------------- DTOs

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct RepoDto {
    pub owner: String,
    pub name: String,
    pub url: String,
    /// `None` when GitHub's answer is not known (e.g. `not_ours`).
    pub private: Option<bool>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct SyncStatusDto {
    pub logged_in: bool,
    pub login: Option<String>,
    pub repo: Option<RepoDto>,
    pub last_sync_at: Option<i64>,
    pub pending: usize,
    /// `public_ack` | `not_ours` | `relogin`
    pub needs: Option<String>,
    /// Never synced on this machine while the cloned repository already holds
    /// memory files (a new computer): offer a download.
    pub remote_has_memory: bool,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct DeviceCodeDto {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
}

/// Payload of `sync://login`.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct LoginEvent {
    /// `ok` | `expired` | `denied` | `cancelled` | `error` | `relogin`
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl LoginEvent {
    fn of(status: &str) -> Self {
        LoginEvent {
            status: status.into(),
            needs: None,
            message: None,
        }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct PlanCounts {
    pub pull: usize,
    pub push: usize,
    pub merged: usize,
    pub conflicts: usize,
    pub remote_deleted: usize,
    pub local_all_deleted: usize,
    pub skipped: usize,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct DeletedFileDto {
    pub repo_path: String,
    pub local_path: Option<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct HeldProjectDto {
    pub project_key: String,
    pub files: Vec<DeletedFileDto>,
}

/// What the resolution UI needs from a `SyncPlan`.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct SyncPlanDto {
    pub id: String,
    pub counts: PlanCounts,
    pub conflicts: Vec<ConflictFile>,
    pub remote_deleted: Vec<DeletedFileDto>,
    pub local_all_deleted: Vec<HeldProjectDto>,
    pub skipped: Vec<SkippedFile>,
    pub secret_hits: Vec<FileSecretHit>,
    pub missing_skills: Vec<ManifestSkill>,
    pub removed_elsewhere: Vec<ManifestSkill>,
    /// A download plan: resolving writes local files only.
    pub download: bool,
    /// Download: local changes left as they are.
    pub kept_local: Vec<String>,
    /// Download: repository files with no place on this machine.
    pub absent: Vec<AbsentFile>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct SyncSummaryDto {
    pub commit: Option<String>,
    pub committed: bool,
    pub pulled: usize,
    pub pushed: usize,
    pub merged: usize,
}

impl From<&SyncSummary> for SyncSummaryDto {
    fn from(s: &SyncSummary) -> Self {
        SyncSummaryDto {
            commit: s.commit.clone(),
            committed: s.committed,
            pulled: s.pulled,
            pushed: s.pushed,
            merged: s.merged,
        }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncStartDto {
    /// `skipped` / skill differences are reported even when nothing needed
    /// a decision.
    Done {
        summary: SyncSummaryDto,
        skipped: Vec<SkippedFile>,
        missing_skills: Vec<ManifestSkill>,
        removed_elsewhere: Vec<ManifestSkill>,
        download: bool,
        kept_local: Vec<String>,
        absent: Vec<AbsentFile>,
    },
    NeedsResolution { plan: SyncPlanDto },
    Blocked { secret_hits: Vec<FileSecretHit> },
}

/// A resolution sent by the frontend, keyed by repo path.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolutionDto {
    Text { text: String },
    DeleteLocal,
    KeepLocal,
    DeleteRemote,
    KeepRemote,
}

impl From<ResolutionDto> for Resolution {
    fn from(r: ResolutionDto) -> Self {
        match r {
            ResolutionDto::Text { text } => Resolution::Text(text),
            ResolutionDto::DeleteLocal => Resolution::DeleteLocal,
            ResolutionDto::KeepLocal => Resolution::KeepLocal,
            ResolutionDto::DeleteRemote => Resolution::DeleteRemote,
            ResolutionDto::KeepRemote => Resolution::KeepRemote,
        }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct SyncProjectDto {
    pub project_id: String,
    pub name: String,
    /// Key from the project's git remote; `None` for projects without one.
    pub key: Option<String>,
    pub dir_key: String,
    pub included: bool,
}

// -------------------------------------------------------------------- errors

fn err(code: &str, msg: impl Into<String>) -> CommandError {
    CommandError::new(code, msg)
}

fn api_err(e: ApiError) -> CommandError {
    match e {
        ApiError::Unauthorized => err("unauthorized", e.to_string()),
        ApiError::Network(_) => err("network", e.to_string()),
        ApiError::Http { .. } => err("github_error", e.to_string()),
    }
}

fn git_err(e: SyncGitError) -> CommandError {
    match e {
        SyncGitError::NotInstalled => err("git_missing", "git is not installed"),
        SyncGitError::Timeout => err("network", "git timed out"),
        SyncGitError::Rejected => err("remote_moved", "the remote changed during sync"),
        SyncGitError::Unauthorized => err("unauthorized", "GitHub rejected the credentials"),
        SyncGitError::Failed { stderr } => err("git_error", stderr),
    }
}

fn sync_err(e: SyncError) -> CommandError {
    match e {
        SyncError::Git(g) => git_err(g),
        SyncError::Io(m) => err("io", m),
        SyncError::Manifest(m) => err("manifest", format!("skills.json: {m}")),
        SyncError::Unresolved { paths } => {
            let mut c = err("unresolved", "some files still need a decision");
            c.paths = Some(paths);
            c
        }
        SyncError::Secrets { hits, .. } => {
            let mut c = err("secrets", format!("{} secret(s) found", hits.len()));
            c.secret_hits = Some(hits);
            c
        }
        SyncError::LocalChanged { paths } => {
            let mut c = err("local_changed", "files changed during sync");
            c.paths = Some(paths);
            c
        }
        SyncError::RemoteMoved { .. } => err("remote_moved", "the remote changed during sync"),
        SyncError::PushFailed { message, cause, .. } => match cause {
            Some(SyncGitError::Failed { .. }) | None => err("push_failed", message),
            Some(g) => git_err(g),
        },
    }
}

fn busy() -> CommandError {
    err("sync_busy", "a sync or login operation is already running")
}

fn try_sync_lock(state: &AppState) -> Result<MutexGuard<'_, ()>, CommandError> {
    match state.sync.lock.try_lock() {
        Ok(g) => Ok(g),
        Err(TryLockError::Poisoned(p)) => Ok(p.into_inner()),
        Err(TryLockError::WouldBlock) => Err(busy()),
    }
}

/// `unauthorized`: forget the token and the login, ask the UI to log in again.
fn handle_unauthorized(state: &AppState) {
    let _ = state.sync.clear_token();
    if let Ok(mut s) = load(state) {
        s.login = None;
        s.needs = Some("relogin".into());
        let _ = store(state, &s);
    }
    state.sync.emit(LOGIN_EVENT, &LoginEvent::of("relogin"));
    emit_status(state);
}

/// Applies the side effects every error code needs before returning it.
fn on_error(state: &AppState, e: CommandError) -> CommandError {
    if e.code == "unauthorized" {
        handle_unauthorized(state);
    }
    e
}

// -------------------------------------------------------------------- helpers

fn sync_dir(state: &AppState) -> PathBuf {
    state.data_dir.join("sync")
}

fn repo_dir(state: &AppState) -> PathBuf {
    sync_dir(state).join("repo")
}

fn load(state: &AppState) -> Result<SyncState, CommandError> {
    load_state(&sync_dir(state).join("state.json")).map_err(|e| err("io", e))
}

fn store(state: &AppState, s: &SyncState) -> Result<(), CommandError> {
    Ok(save_state(&sync_dir(state).join("state.json"), s)?)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Reads the real token (keychain). Only sync / login operations call this.
fn token(state: &AppState) -> Option<String> {
    state.sync.read_token()
}

/// Cached "a token is stored" flag: no keychain access.
fn has_token_cached(state: &AppState) -> bool {
    lock(&state.sync.has_token).unwrap_or(false)
}

/// Reads the keychain once in the background at startup, then re-emits the
/// status so the UI learns whether the user is logged in.
pub fn warm_token_cache(state: &Arc<AppState>) {
    let st = state.clone();
    std::thread::spawn(move || {
        st.sync.read_token();
        emit_status(&st);
    });
}

fn require_token(state: &AppState, s: &SyncState) -> Result<String, CommandError> {
    match (token(state), &s.login) {
        (Some(t), Some(_)) => Ok(t),
        _ => Err(err("not_logged_in", "log in to GitHub to sync")),
    }
}

fn reset_base(s: &mut SyncState) {
    s.last_sync_commit = None;
    s.last_sync_at = None;
    s.synced_projects.clear();
    s.unsynced_paths.clear();
    s.held_projects.clear();
}

fn split_full(full: &str, owner: &str, name: &str) -> (String, String) {
    match full.split_once('/') {
        Some((o, n)) if !o.is_empty() && !n.is_empty() => (o.to_string(), n.to_string()),
        _ => (owner.to_string(), name.to_string()),
    }
}

/// Binds (creates / checks) the sync repository and clones it when needed.
/// Returns what still blocks sync (`not_ours` / `public_ack`), if anything.
fn setup_repo(
    state: &AppState,
    token: &str,
    login: &str,
    s: &mut SyncState,
) -> Result<Option<String>, CommandError> {
    let rt = &state.sync;
    let sleep = |d: Duration| (rt.sleep)(d);
    let name = s.repo_name.clone();
    // An acknowledgement only counts for the exact repository it was given for.
    let acked = s
        .public_ack_for
        .as_deref()
        .is_some_and(|a| a.eq_ignore_ascii_case(&format!("{login}/{name}")));
    let outcome =
        ensure_repo_with(&*rt.api, token, login, &name, acked, &sleep).map_err(api_err)?;
    match outcome {
        RepoOutcome::Ready(repo) | RepoOutcome::Created(repo) => {
            let (owner, name) = split_full(&repo.full_name, login, &name);
            let full = format!("{owner}/{name}");
            if s
                .public_ack_for
                .as_deref()
                .is_some_and(|a| !a.eq_ignore_ascii_case(&full))
            {
                s.public_ack_for = None;
            }
            let dir = repo_dir(state);
            let other_repo = s.clone_of.as_deref() != Some(full.as_str());
            if other_repo || !dir.join(".git").exists() {
                let url = (rt.clone_url)(&owner, &name);
                rt.git
                    .clone(&url, &dir, &Some(token.to_string()))
                    .map_err(git_err)?;
                if other_repo {
                    // A different repository: the old base means nothing here.
                    reset_base(s);
                }
                s.clone_of = Some(full);
            }
            s.repo_owner = Some(owner);
            s.repo_name = name;
            s.branch = Some(repo.default_branch).filter(|b| !b.is_empty());
            s.repo_private = Some(repo.private);
            s.needs = None;
            store(state, s)?;
            Ok(None)
        }
        RepoOutcome::NotOurs => {
            s.repo_owner = Some(login.to_string());
            s.repo_private = None;
            s.needs = Some("not_ours".into());
            store(state, s)?;
            Ok(s.needs.clone())
        }
        RepoOutcome::PublicNeedsAck(repo) => {
            let (owner, _) = split_full(&repo.full_name, login, &name);
            s.repo_owner = Some(owner);
            s.repo_private = Some(false);
            s.needs = Some("public_ack".into());
            store(state, s)?;
            Ok(s.needs.clone())
        }
    }
}

/// Makes sure the repository is bound and cloned; errors with the reason
/// otherwise. Always asks GitHub (sync is online anyway), so a repository
/// made public, deleted or replaced since the last sync is noticed.
fn ensure_ready(state: &AppState, token: &str, s: &mut SyncState) -> Result<(), CommandError> {
    let login = s
        .login
        .clone()
        .ok_or_else(|| err("not_logged_in", "log in to GitHub to sync"))?;
    match setup_repo(state, token, &login, s)?.as_deref() {
        Some("not_ours") => Err(err(
            "repo_not_ours",
            "a repository with this name exists and was not created by MemDock",
        )),
        Some(_) => Err(err(
            "public_not_acked",
            "the sync repository is public; confirm before syncing",
        )),
        None => Ok(()),
    }
}

/// Everything a `SyncCtx` borrows.
struct Parts {
    st: SyncState,
    machine: Machine,
    files: Vec<SyncFile>,
    projects: Vec<(String, PathBuf)>,
    local_skills: Option<Vec<ManifestSkill>>,
    repo_dir: PathBuf,
    branch: String,
}

fn gather(state: &AppState, st: SyncState, with_skills: bool) -> Result<Parts, CommandError> {
    let index = state.snapshot_index();
    let machine = load_or_create_machine(&sync_dir(state).join("machine.json"))?;
    let files = map_files(&index, &st.included_local);
    let projects = engine::sync_projects(&index, &st.included_local);
    let local_skills = if with_skills {
        local_skills(state, &index, &projects)
    } else {
        None
    };
    let dir = repo_dir(state);
    let branch = match &st.branch {
        Some(b) => b.clone(),
        None if with_skills && dir.join(".git").exists() => state
            .sync
            .git
            .default_branch(&dir)
            .unwrap_or_else(|_| "main".into()),
        None => "main".into(),
    };
    Ok(Parts {
        st,
        machine,
        files,
        projects,
        local_skills,
        repo_dir: dir,
        branch,
    })
}

/// This machine's skill installs for `skills.json`; `None` (manifest left
/// unchanged) when the registry or the skill lock cannot be read.
fn local_skills(
    state: &AppState,
    index: &crate::model::MemoryIndex,
    projects: &[(String, PathBuf)],
) -> Option<Vec<ManifestSkill>> {
    let registry =
        crate::skills::registry::load_registry(&state.data_dir.join("skills-registry.json")).ok()?;
    // An unreadable lock would make its skills look local-only: skip the merge.
    lock_entries_for_sync(&state.home).ok()?;
    let lock = crate::skills::source::read_skill_lock(&state.home);
    let all: Vec<crate::model::Project> =
        index.projects.iter().map(|pm| pm.project.clone()).collect();
    let mut entries = crate::skills::scan::scan_skills(&state.paths, &state.home, &all);
    for e in entries.iter_mut().filter(|e| !e.readonly) {
        e.source = crate::skills::source::infer_source(e, &registry, &lock, state.git.as_ref());
    }
    let keyed: Vec<(String, Option<String>)> = index
        .projects
        .iter()
        .map(|pm| {
            let key = projects
                .iter()
                .find(|(_, d)| normalize_path(&d.to_string_lossy()) == pm.project.id)
                .map(|(k, _)| k.clone());
            (pm.project.id.clone(), key)
        })
        .collect();
    Some(local_entries(&entries, &keyed))
}

/// What the skill-manifest commands need from the sync side (local only: no
/// network, no keychain).
pub(crate) struct ManifestContext {
    /// `None` when the registry or skill lock cannot be read.
    pub local: Option<Vec<ManifestSkill>>,
    pub machine: Machine,
    /// `(project_id, sync key, dir key)` of every known project.
    pub projects: Vec<(String, Option<String>, String)>,
    pub logged_in: bool,
    pub last_sync_at: Option<i64>,
}

pub(crate) fn manifest_context(state: &AppState) -> Result<ManifestContext, CommandError> {
    let st = load(state)?;
    let index = state.snapshot_index();
    let machine = load_or_create_machine(&sync_dir(state).join("machine.json"))?;
    let projects = engine::sync_projects(&index, &st.included_local);
    let local = local_skills(state, &index, &projects);
    let keyed = index
        .projects
        .iter()
        .map(|pm| {
            let dir = Path::new(&pm.project.path);
            let key = projects
                .iter()
                .find(|(_, d)| normalize_path(&d.to_string_lossy()) == pm.project.id)
                .map(|(k, _)| k.clone());
            (pm.project.id.clone(), key, dir_key(dir))
        })
        .collect();
    Ok(ManifestContext {
        local,
        machine,
        projects: keyed,
        logged_in: st.login.is_some() && has_token_cached(state),
        last_sync_at: st.last_sync_at,
    })
}

/// `skills.json` in the local clone of the sync repository: the fetched remote
/// branch, else the last synced commit. `Ok(None)` when there is no clone or
/// the file is absent.
pub(crate) fn cached_remote_manifest(
    state: &AppState,
) -> Result<Option<crate::sync::manifest::Manifest>, CommandError> {
    let st = load(state)?;
    let dir = repo_dir(state);
    if !dir.join(".git").exists() {
        return Ok(None);
    }
    let branch = st.branch.clone().unwrap_or_else(|| "main".into());
    let mut refs = vec![format!("origin/{branch}")];
    refs.extend(st.last_sync_commit.clone());
    for r in refs {
        match state.sync.git.read_at(&dir, &r, "skills.json") {
            Ok(Some(bytes)) => {
                let text = String::from_utf8(bytes)
                    .map_err(|_| err("manifest", "skills.json: not UTF-8 text"))?;
                return crate::sync::manifest::Manifest::from_json(&text)
                    .map(Some)
                    .map_err(|m| err("manifest", format!("skills.json: {m}")));
            }
            Ok(None) => return Ok(None),
            Err(_) => continue,
        }
    }
    Ok(None)
}

fn make_ctx<'a>(state: &'a AppState, p: &'a Parts, auth: Auth) -> SyncCtx<'a> {
    SyncCtx {
        git: &state.sync.git,
        repo_dir: &p.repo_dir,
        auth,
        branch: p.branch.clone(),
        base_commit: p.st.last_sync_commit.clone(),
        files: p.files.clone(),
        projects: p.projects.clone(),
        synced_projects: p.st.synced_projects.clone(),
        unsynced_paths: p.st.unsynced_paths.clone(),
        held_projects: p.st.held_projects.clone(),
        paths: &state.paths,
        machine: &p.machine,
        local_skills: p.local_skills.clone(),
        snapshots: &state.snapshots,
        write_lock: &state.write_lock,
        secrets: lock(&state.sync.secret_policy).clone(),
        download: false,
    }
}

/// Local file a repo path maps to on this machine, if any.
fn local_of(state: &AppState, p: &Parts, repo_path: &str) -> Option<PathBuf> {
    p.files
        .iter()
        .find(|f| f.repo_path == repo_path)
        .map(|f| f.local_path.clone())
        .or_else(|| expected_local_path(repo_path, &p.projects, &state.paths))
}

/// Project key of a `projects/<key>/...` path (longest key present here).
fn key_of(repo_path: &str, projects: &[(String, PathBuf)]) -> String {
    let rest = repo_path.strip_prefix("projects/").unwrap_or(repo_path);
    projects
        .iter()
        .filter(|(k, _)| rest.strip_prefix(k.as_str()).is_some_and(|t| t.starts_with('/')))
        .max_by_key(|(k, _)| k.len())
        .map(|(k, _)| k.clone())
        .unwrap_or_else(|| rest.rsplit_once('/').map(|(k, _)| k).unwrap_or(rest).to_string())
}

fn plan_dto(state: &AppState, plan: &SyncPlan, p: &Parts) -> SyncPlanDto {
    let file = |rp: &String| DeletedFileDto {
        repo_path: rp.clone(),
        local_path: local_of(state, p, rp).map(|l| l.to_string_lossy().into_owned()),
    };
    let mut held: Vec<HeldProjectDto> = Vec::new();
    for rp in &plan.local_all_deleted {
        let key = key_of(rp, &p.projects);
        match held.iter_mut().find(|h| h.project_key == key) {
            Some(h) => h.files.push(file(rp)),
            None => held.push(HeldProjectDto {
                project_key: key,
                files: vec![file(rp)],
            }),
        }
    }
    SyncPlanDto {
        id: plan.id.clone(),
        counts: PlanCounts {
            pull: plan.take_remote.len(),
            push: plan.push_local.len(),
            merged: plan.merged.len(),
            conflicts: plan.conflicts.len(),
            remote_deleted: plan.remote_deleted.len(),
            local_all_deleted: plan.local_all_deleted.len(),
            skipped: plan.skipped.len(),
        },
        conflicts: plan.conflicts.clone(),
        remote_deleted: plan.remote_deleted.iter().map(file).collect(),
        local_all_deleted: held,
        skipped: plan.skipped.clone(),
        secret_hits: plan.secret_hits.clone(),
        missing_skills: plan.missing_skills.clone(),
        removed_elsewhere: plan.removed_elsewhere.clone(),
        download: plan.download,
        kept_local: plan.kept_local.clone(),
        absent: plan.absent.clone(),
    }
}

/// Normalized ids of the local files a plan may have written.
fn touched(state: &AppState, plan: &SyncPlan, p: &Parts) -> Vec<String> {
    let mut out: Vec<String> = plan
        .take_remote
        .iter()
        .chain(&plan.merged)
        .chain(&plan.remote_deleted)
        .chain(&plan.local_all_deleted)
        .chain(plan.conflicts.iter().map(|c| &c.repo_path))
        .filter_map(|rp| local_of(state, p, rp))
        .map(|l| normalize_path(&l.to_string_lossy()))
        .collect();
    out.sort();
    out.dedup();
    out
}

#[derive(Serialize)]
struct MemoryChanged {
    paths: Vec<String>,
}

/// Rescans and tells the UI which memory files may have changed.
fn after_local_writes(state: &AppState, paths: Vec<String>) {
    state.rescan();
    if !paths.is_empty() {
        state.sync.emit(MEMORY_EVENT, &MemoryChanged { paths });
    }
}

fn emit_status(state: &AppState) {
    let st = sync_status_impl(state);
    state.sync.last_pending.store(st.pending as i64, Ordering::SeqCst);
    state.sync.emit(STATUS_EVENT, &st);
}

/// Persists the outcome of `finish` (the new base, also after errors that
/// carry one), refreshes the index and emits events.
fn settle(
    state: &AppState,
    plan: &SyncPlan,
    p: &Parts,
    res: Result<SyncSummary, SyncError>,
) -> Result<SyncSummary, CommandError> {
    let base = match &res {
        Ok(sum) => Some((
            sum.commit.clone(),
            sum.synced_projects.clone(),
            sum.unsynced_paths.clone(),
            sum.held_projects.clone(),
        )),
        Err(e) => e.base_to_persist().map(|b| {
            (
                b.commit.clone(),
                b.synced_projects.clone(),
                b.unsynced_paths.clone(),
                b.held_projects.clone(),
            )
        }),
    };
    if let Some((commit, synced, unsynced, held)) = base {
        let mut s = load(state)?;
        s.last_sync_commit = commit;
        s.synced_projects = synced;
        s.unsynced_paths = unsynced;
        s.held_projects = held;
        s.skipped_files = engine::update_skip_records(plan, &s.skipped_files, now_ms());
        if res.is_ok() {
            s.last_sync_at = Some(now_ms());
        }
        store(state, &s)?;
        after_local_writes(state, touched(state, plan, p));
        emit_status(state);
    }
    res.map_err(sync_err)
}

// ---------------------------------------------------------------------- impls

pub fn sync_status_impl(state: &AppState) -> SyncStatusDto {
    let s = load(state).unwrap_or_default();
    let logged_in = s.login.is_some() && has_token_cached(state);
    let repo = s.repo_owner.as_ref().map(|owner| RepoDto {
        owner: owner.clone(),
        name: s.repo_name.clone(),
        url: format!("https://github.com/{owner}/{}", s.repo_name),
        private: s.repo_private,
    });
    SyncStatusDto {
        logged_in,
        login: if logged_in { s.login.clone() } else { None },
        repo,
        last_sync_at: s.last_sync_at,
        needs: s.needs.clone(),
        remote_has_memory: logged_in && remote_has_memory(state, &s),
        pending: pending(state, s),
    }
}

/// Never synced here, and the clone's remote branch has memory files.
/// Local only (reads the clone, no network).
fn remote_has_memory(state: &AppState, s: &SyncState) -> bool {
    if s.last_sync_commit.is_some() {
        return false;
    }
    let dir = repo_dir(state);
    if !dir.join(".git").exists() {
        return false;
    }
    let branch = s.branch.clone().unwrap_or_else(|| "main".into());
    state
        .sync
        .git
        .list_at(&dir, &format!("origin/{branch}"))
        .map(|files| {
            files
                .iter()
                .any(|f| f.starts_with("global/") || f.starts_with("projects/"))
        })
        .unwrap_or(false)
}

/// Files that differ from the last synced commit; local only (no network,
/// no `write_lock`).
fn pending(state: &AppState, s: SyncState) -> usize {
    let Ok(p) = gather(state, s, false) else { return 0 };
    if p.st.last_sync_commit.is_some() && !p.repo_dir.join(".git").exists() {
        return 0;
    }
    engine::pending_local(&make_ctx(state, &p, None)).unwrap_or(0)
}

pub fn login_start_impl(
    state: &Arc<AppState>,
) -> Result<(DeviceCodeDto, JoinHandle<()>), CommandError> {
    let rt = &state.sync;
    // A just-cancelled login may still be finishing its current sleep slice.
    if rt.login_cancel.load(Ordering::SeqCst) {
        for _ in 0..40 {
            if !rt.login_active.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    if rt.login_active.swap(true, Ordering::SeqCst) {
        return Err(busy());
    }
    let code = match rt.api.device_code(CLIENT_ID) {
        Ok(c) => c,
        Err(e) => {
            rt.login_active.store(false, Ordering::SeqCst);
            return Err(api_err(e));
        }
    };
    rt.login_cancel.store(false, Ordering::SeqCst);
    let dto = DeviceCodeDto {
        user_code: code.user_code.clone(),
        verification_uri: code.verification_uri.clone(),
        expires_in: code.expires_in,
    };
    let st = state.clone();
    let handle = std::thread::spawn(move || {
        let event = match run_login(&st, &code) {
            Ok(needs) => LoginEvent {
                status: "ok".into(),
                needs,
                message: None,
            },
            Err(LoginFail::Login(LoginError::Expired)) => LoginEvent::of("expired"),
            Err(LoginFail::Login(LoginError::Denied)) => LoginEvent::of("denied"),
            Err(LoginFail::Login(LoginError::Cancelled)) => LoginEvent::of("cancelled"),
            Err(LoginFail::Login(LoginError::Api(ApiError::Unauthorized))) => {
                LoginEvent::of("denied")
            }
            Err(LoginFail::Login(e)) => LoginEvent {
                message: Some(e.to_string()),
                ..LoginEvent::of("error")
            },
            Err(LoginFail::Command(e)) => LoginEvent {
                message: Some(e.message.clone()),
                ..LoginEvent::of(if e.code == "unauthorized" {
                    "relogin"
                } else {
                    "error"
                })
            },
        };
        // TODO(exit): app exit does not cancel this thread or a running sync;
        // the process ends with them. Hook `login_cancel` into RunEvent::Exit
        // if a clean shutdown is ever needed.
        st.sync.login_active.store(false, Ordering::SeqCst);
        st.sync.emit(LOGIN_EVENT, &event);
        emit_status(&st);
    });
    Ok((dto, handle))
}

enum LoginFail {
    Login(LoginError),
    Command(CommandError),
}

/// Polls for the token, then stores it, records the login and binds the
/// repository. Returns what still blocks sync.
fn run_login(state: &AppState, code: &DeviceCode) -> Result<Option<String>, LoginFail> {
    let rt = &state.sync;
    let sleep = |d: Duration| rt.login_sleep(d);
    let token = device_login(&*rt.api, CLIENT_ID, code, &rt.login_cancel, &sleep)
        .map_err(LoginFail::Login)?;
    // Wait for a running sync instead of failing a login the user just approved.
    let _g = lock(&rt.lock);
    if rt.login_cancel.load(Ordering::SeqCst) {
        return Err(LoginFail::Login(LoginError::Cancelled));
    }
    let fail = |e: CommandError| LoginFail::Command(e);
    let user = rt.api.user(&token).map_err(|e| fail(api_err(e)))?;
    rt.store_token(&token)
        .map_err(|e| fail(err("keychain", e)))?;
    let mut s = load(state).map_err(fail)?;
    // A public-repository acknowledgement belongs to one account's repository.
    let prev_account = s.login.clone().or_else(|| s.repo_owner.clone());
    let ack_owner_differs = s.public_ack_for.as_deref().is_some_and(|a| {
        !a.split('/').next().unwrap_or("").eq_ignore_ascii_case(&user.login)
    });
    if ack_owner_differs
        || prev_account.is_some_and(|p| !p.eq_ignore_ascii_case(&user.login))
    {
        s.public_ack_for = None;
    }
    s.login = Some(user.login.clone());
    s.needs = None;
    store(state, &s).map_err(fail)?;
    match setup_repo(state, &token, &user.login, &mut s) {
        Ok(needs) => Ok(needs),
        Err(e) => {
            if e.code == "unauthorized" {
                let _ = rt.clear_token();
                if let Ok(mut s) = load(state) {
                    s.login = None;
                    s.needs = Some("relogin".into());
                    let _ = store(state, &s);
                }
            }
            // Other failures keep the login: the next sync retries the setup.
            Err(fail(e))
        }
    }
}

pub fn login_cancel_impl(state: &AppState) {
    state.sync.login_cancel.store(true, Ordering::SeqCst);
}

/// Forgets the token and the login; keeps the clone and the sync base.
pub fn logout_impl(state: &AppState) -> Result<SyncStatusDto, CommandError> {
    let _g = try_sync_lock(state)?;
    state.sync.login_cancel.store(true, Ordering::SeqCst);
    state
        .sync
        .clear_token()
        .map_err(|e| err("keychain", e))?;
    let mut s = load(state)?;
    s.login = None;
    s.needs = None;
    s.public_ack_for = None;
    store(state, &s)?;
    lock(&state.sync.plans).clear();
    emit_status(state);
    Ok(sync_status_impl(state))
}

fn valid_repo_name(name: &str) -> bool {
    (1..=100).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && name != "."
        && name != ".."
}

/// Binds another repository name (after `not_ours`), then re-checks it when
/// logged in.
pub fn set_repo_name_impl(state: &AppState, name: &str) -> Result<SyncStatusDto, CommandError> {
    if !valid_repo_name(name) {
        return Err(err("invalid", "repository names use A-Z a-z 0-9 . _ - (max 100)"));
    }
    let _g = try_sync_lock(state)?;
    let r = (|| {
        let mut s = load(state)?;
        if s.repo_name != name {
            s.repo_name = name.to_string();
            s.public_ack_for = None;
        }
        if matches!(s.needs.as_deref(), Some("not_ours" | "public_ack")) {
            s.needs = None;
        }
        store(state, &s)?;
        if let (Some(t), Some(login)) = (token(state), s.login.clone()) {
            setup_repo(state, &t, &login, &mut s)?;
        }
        Ok(())
    })();
    r.map_err(|e| on_error(state, e))?;
    emit_status(state);
    Ok(sync_status_impl(state))
}

/// The user accepted syncing into a public repository.
pub fn ack_public_repo_impl(state: &AppState) -> Result<SyncStatusDto, CommandError> {
    let _g = try_sync_lock(state)?;
    let r = (|| {
        let mut s = load(state)?;
        let owner = s
            .repo_owner
            .clone()
            .or_else(|| s.login.clone())
            .ok_or_else(|| err("not_logged_in", "log in to GitHub to sync"))?;
        s.public_ack_for = Some(format!("{owner}/{}", s.repo_name));
        if s.needs.as_deref() == Some("public_ack") {
            s.needs = None;
        }
        store(state, &s)?;
        if let (Some(t), Some(login)) = (token(state), s.login.clone()) {
            setup_repo(state, &t, &login, &mut s)?;
        }
        Ok(())
    })();
    r.map_err(|e| on_error(state, e))?;
    emit_status(state);
    Ok(sync_status_impl(state))
}

pub fn sync_start_impl(state: &AppState) -> Result<SyncStartDto, CommandError> {
    sync_start_with(state, "block")
}

/// An upload (`direction` `up`) with the given secret policy.
pub fn sync_start_with(state: &AppState, secrets: &str) -> Result<SyncStartDto, CommandError> {
    sync_start_dir(state, secrets, "up")
}

/// `direction`: `up` (merge the remote, then push this machine's changes) or
/// `down` (bring the remote to this machine; push nothing, ignore secrets).
/// `secrets`: `block` (stop and report), `allow` (sync them anyway) or
/// `skip` (leave the files that look like secrets out of this sync).
pub fn sync_start_dir(
    state: &AppState,
    secrets: &str,
    direction: &str,
) -> Result<SyncStartDto, CommandError> {
    let download = match direction {
        "up" => false,
        "down" => true,
        _ => return Err(err("invalid", "direction must be up or down")),
    };
    let _g = try_sync_lock(state)?;
    *lock(&state.sync.secret_policy) = SecretPolicy {
        allow: secrets == "allow",
        exclude: BTreeSet::new(),
    };
    let r = (|| {
        let mut s = load(state)?;
        let token = require_token(state, &s)?;
        ensure_ready(state, &token, &mut s)?;
        state.rescan();
        let p = gather(state, s, true)?;
        let mut ctx = make_ctx(state, &p, Some(token));
        ctx.download = download;
        let mut plan = prepare(&ctx).map_err(sync_err)?;
        if !plan.secret_hits.is_empty() && secrets == "skip" {
            let exclude: BTreeSet<String> = plan.secret_hits.iter().map(|h| h.repo_path.clone()).collect();
            lock(&state.sync.secret_policy).exclude = exclude.clone();
            ctx.secrets.exclude = exclude;
            plan = prepare(&ctx).map_err(sync_err)?;
        }
        if !plan.secret_hits.is_empty() {
            return Ok(SyncStartDto::Blocked {
                secret_hits: plan.secret_hits,
            });
        }
        if plan.conflicts.is_empty()
            && plan.remote_deleted.is_empty()
            && plan.local_all_deleted.is_empty()
        {
            let res = finish(&ctx, &plan, &BTreeMap::new());
            let sum = settle(state, &plan, &p, res)?;
            return Ok(SyncStartDto::Done {
                summary: (&sum).into(),
                skipped: plan.skipped,
                missing_skills: plan.missing_skills,
                removed_elsewhere: plan.removed_elsewhere,
                download: plan.download,
                kept_local: plan.kept_local,
                absent: plan.absent,
            });
        }
        let dto = plan_dto(state, &plan, &p);
        let mut plans = lock(&state.sync.plans);
        plans.clear();
        plans.insert(plan.id.clone(), plan);
        Ok(SyncStartDto::NeedsResolution { plan: dto })
    })();
    r.map_err(|e| on_error(state, e))
}

pub fn sync_finish_impl(
    state: &AppState,
    plan_id: &str,
    resolutions: HashMap<String, ResolutionDto>,
) -> Result<SyncSummaryDto, CommandError> {
    let _g = try_sync_lock(state)?;
    let r = (|| {
        let not_found = || err("plan_not_found", "this sync plan is no longer available");
        if !lock(&state.sync.plans).contains_key(plan_id) {
            return Err(not_found());
        }
        // Everything that can fail before the engine runs is checked while
        // the plan stays stored, so the user can retry it.
        let s = load(state)?;
        let token = require_token(state, &s)?;
        state.rescan();
        let p = gather(state, s, true)?;
        let plan = lock(&state.sync.plans).remove(plan_id).ok_or_else(not_found)?;
        let ctx = make_ctx(state, &p, Some(token));
        let res: BTreeMap<String, Resolution> = resolutions
            .into_iter()
            .map(|(k, v)| (k, v.into()))
            .collect();
        let out = finish(&ctx, &plan, &res);
        // Nothing was written: the user can fix the choices and try again.
        if matches!(
            &out,
            Err(SyncError::Unresolved { .. }) | Err(SyncError::Secrets { base: None, .. })
        ) {
            lock(&state.sync.plans).insert(plan.id.clone(), plan.clone());
        }
        settle(state, &plan, &p, out).map(|s| SyncSummaryDto::from(&s))
    })();
    r.map_err(|e| on_error(state, e))
}

pub fn sync_discard_impl(state: &AppState, plan_id: &str) {
    lock(&state.sync.plans).remove(plan_id);
}

pub fn set_included_local_impl(
    state: &AppState,
    keys: Vec<String>,
) -> Result<Vec<SyncProjectDto>, CommandError> {
    let _g = try_sync_lock(state)?;
    let mut keys = keys;
    keys.sort();
    keys.dedup();
    let mut s = load(state)?;
    s.included_local = keys;
    store(state, &s)?;
    drop(_g);
    emit_status(state);
    sync_projects_impl(state)
}

pub fn sync_projects_impl(state: &AppState) -> Result<Vec<SyncProjectDto>, CommandError> {
    let s = load(state)?;
    let index = state.snapshot_index();
    Ok(index
        .projects
        .iter()
        .map(|pm| {
            let dir = Path::new(&pm.project.path);
            let key = project_key(dir).and_then(|k| safe_key(&k));
            let dk = dir_key(dir);
            SyncProjectDto {
                project_id: pm.project.id.clone(),
                name: pm.project.name.clone(),
                included: key.is_some() || s.included_local.contains(&dk),
                key,
                dir_key: dk,
            }
        })
        .collect())
}

/// A file left out of a sync and not synced since.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct SkippedRecordDto {
    pub repo_path: String,
    pub reason: String,
    /// Milliseconds since the epoch.
    pub at: i64,
    /// The local file the repo path maps to on this machine, if any.
    pub local_path: Option<String>,
    /// Index id of that file when it is a known memory file.
    pub file_id: Option<String>,
}

/// Persistent skip records with their local files (local only, no network).
pub fn sync_skipped_files_impl(state: &AppState) -> Result<Vec<SkippedRecordDto>, CommandError> {
    let s = load(state)?;
    if s.skipped_files.is_empty() {
        return Ok(Vec::new());
    }
    let records = s.skipped_files.clone();
    let p = gather(state, s, false)?;
    Ok(records
        .into_iter()
        .map(|(repo_path, r)| {
            let file = p.files.iter().find(|f| f.repo_path == repo_path);
            SkippedRecordDto {
                local_path: local_of(state, &p, &repo_path).map(|l| l.to_string_lossy().into_owned()),
                file_id: file.map(|f| f.file_id.clone()),
                repo_path,
                reason: r.reason,
                at: r.at,
            }
        })
        .collect())
}

fn read_parts(state: &AppState) -> Result<Option<Parts>, CommandError> {
    let s = load(state)?;
    if !repo_dir(state).join(".git").exists() {
        return Ok(None);
    }
    Ok(Some(gather(state, s, false)?))
}

pub fn sync_history_impl(state: &AppState, limit: usize) -> Result<Vec<CommitInfo>, CommandError> {
    let Some(p) = read_parts(state)? else {
        return Ok(Vec::new());
    };
    engine::history(&make_ctx(state, &p, None), limit.clamp(1, HISTORY_MAX)).map_err(sync_err)
}

fn check_commit(commit: &str) -> Result<(), CommandError> {
    let ok = (commit.len() == 40 || commit.len() == 64)
        && commit.bytes().all(|b| b.is_ascii_hexdigit());
    if ok {
        Ok(())
    } else {
        Err(err("invalid", "invalid commit id"))
    }
}

pub fn sync_file_at_impl(
    state: &AppState,
    commit: &str,
    repo_path: &str,
) -> Result<Option<String>, CommandError> {
    check_commit(commit)?;
    let Some(p) = read_parts(state)? else {
        return Ok(None);
    };
    engine::file_at(&make_ctx(state, &p, None), commit, repo_path).map_err(sync_err)
}

/// Writes the version of `repo_path` at `commit` to the matching local file
/// (snapshot first). It becomes an ordinary local change.
pub fn restore_from_history_impl(
    state: &AppState,
    commit: &str,
    repo_path: &str,
) -> Result<(), CommandError> {
    check_commit(commit)?;
    let _g = try_sync_lock(state)?;
    state.rescan();
    let Some(p) = read_parts(state)? else {
        return Err(err("not_found", "the sync repository is not cloned"));
    };
    let text = engine::file_at(&make_ctx(state, &p, None), commit, repo_path)
        .map_err(sync_err)?
        .ok_or_else(|| err("not_found", "the file does not exist in that version"))?;
    let local = local_of(state, &p, repo_path)
        .ok_or_else(|| err("invalid", "this file has no place on this machine"))?;
    {
        let _w = lock(&state.write_lock);
        let exists = local.exists();
        let current = if exists {
            read_memory(&local)?.hash
        } else {
            String::new()
        };
        let text = if !exists && cfg!(windows) {
            text.replace('\n', "\r\n")
        } else {
            text
        };
        write_memory(&local, &text, &current, &state.snapshots).map_err(|e| match e {
            WriteError::DiskChanged { .. } => {
                let mut c = err("local_changed", "the file changed while restoring");
                c.paths = Some(vec![local.to_string_lossy().into_owned()]);
                c
            }
            other => other.into(),
        })?;
    }
    after_local_writes(state, vec![normalize_path(&local.to_string_lossy())]);
    emit_status(state);
    Ok(())
}

// ------------------------------------------------------------------ commands

type St<'a> = State<'a, Arc<AppState>>;

#[tauri::command(async)]
pub fn sync_status(state: St) -> SyncStatusDto {
    sync_status_impl(&state)
}

#[tauri::command(async)]
pub fn login_start(state: St) -> Result<DeviceCodeDto, CommandError> {
    login_start_impl(state.inner()).map(|(dto, _)| dto)
}

#[tauri::command(async)]
pub fn login_cancel(state: St) {
    login_cancel_impl(&state)
}

#[tauri::command(async)]
pub fn logout(state: St) -> Result<SyncStatusDto, CommandError> {
    logout_impl(&state)
}

#[tauri::command(async)]
pub fn set_repo_name(state: St, name: String) -> Result<SyncStatusDto, CommandError> {
    set_repo_name_impl(&state, &name)
}

#[tauri::command(async)]
pub fn ack_public_repo(state: St) -> Result<SyncStatusDto, CommandError> {
    ack_public_repo_impl(&state)
}

#[tauri::command(async)]
pub fn open_verification_url(app: AppHandle) -> Result<(), CommandError> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(VERIFICATION_URL, None::<&str>)
        .map_err(|e| {
            eprintln!("open_verification_url failed: {e}");
            err("io", e.to_string())
        })
}

#[tauri::command(async)]
pub fn sync_start(
    state: St,
    secrets: Option<String>,
    direction: Option<String>,
) -> Result<SyncStartDto, CommandError> {
    sync_start_dir(
        &state,
        secrets.as_deref().unwrap_or("block"),
        direction.as_deref().unwrap_or("up"),
    )
}

#[tauri::command(async)]
pub fn sync_skipped_files(state: St) -> Result<Vec<SkippedRecordDto>, CommandError> {
    sync_skipped_files_impl(&state)
}

#[tauri::command(async)]
pub fn sync_finish(
    state: St,
    plan_id: String,
    resolutions: HashMap<String, ResolutionDto>,
) -> Result<SyncSummaryDto, CommandError> {
    sync_finish_impl(&state, &plan_id, resolutions)
}

#[tauri::command(async)]
pub fn sync_discard(state: St, plan_id: String) {
    sync_discard_impl(&state, &plan_id)
}

#[tauri::command(async)]
pub fn set_included_local(
    state: St,
    keys: Vec<String>,
) -> Result<Vec<SyncProjectDto>, CommandError> {
    set_included_local_impl(&state, keys)
}

#[tauri::command(async)]
pub fn sync_projects(state: St) -> Result<Vec<SyncProjectDto>, CommandError> {
    sync_projects_impl(&state)
}

#[tauri::command(async)]
pub fn sync_history(state: St, limit: usize) -> Result<Vec<CommitInfo>, CommandError> {
    sync_history_impl(&state, limit)
}

#[tauri::command(async)]
pub fn sync_file_at(
    state: St,
    commit: String,
    repo_path: String,
) -> Result<Option<String>, CommandError> {
    sync_file_at_impl(&state, &commit, &repo_path)
}

#[tauri::command(async)]
pub fn restore_from_history(
    state: St,
    commit: String,
    repo_path: String,
) -> Result<(), CommandError> {
    restore_from_history_impl(&state, &commit, &repo_path)
}

#[cfg(test)]
#[path = "commands_sync_tests.rs"]
mod tests;
