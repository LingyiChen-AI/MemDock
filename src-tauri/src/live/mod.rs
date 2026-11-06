//! Live chat: drive a real Claude Code / Codex session from the app.
//!
//! Each live session owns one long-running child process:
//! - Claude Code: `claude -p --input-format stream-json --output-format stream-json
//!   --verbose --include-partial-messages [--permission-prompt-tool stdio]`
//!   (see `claude.rs`), messages and permission answers go to stdin as JSON lines.
//! - Codex: `codex app-server` speaking JSON-RPC over stdio (see `codex.rs`):
//!   `initialize` → `thread/start|thread/resume` → `turn/start`, approvals are
//!   server→client requests answered with `{decision}`.
//!
//! Drivers turn the native streams into [`LiveEvent`]s. The [`LiveManager`]
//! keeps every session's event log (so the page can re-attach after navigating
//! away), tracks its state and forwards each event to the frontend as the
//! Tauri event `live://event` with payload [`LiveEnvelope`].
//!
//! Both CLIs write their normal session files, so a live session also shows up
//! in the history page and can be resumed in a terminal or converted.
pub mod claude;
pub mod codex;
pub mod models;
pub mod registry;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::convert::timeline::ToolKind;
pub use registry::{NativeRegistry, Owner, OwnerKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Agent {
    Claude,
    Codex,
}

/// How much the agent may do without asking.
///
/// | mode        | Claude Code                                         | Codex (approvalPolicy / sandbox)       |
/// |-------------|-----------------------------------------------------|----------------------------------------|
/// | `ask`       | `--permission-mode default` + prompt tool `stdio`   | `on-request` / `read-only`             |
/// | `auto_edit` | `--permission-mode acceptEdits` + prompt tool       | `on-request` / `workspace-write`       |
/// | `plan`      | `--permission-mode plan` + prompt tool              | `on-request` / `read-only`             |
/// | `bypass`    | `--dangerously-skip-permissions`                    | `never` / `danger-full-access`         |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    #[default]
    Ask,
    AutoEdit,
    Plan,
    Bypass,
}

/// Options picked in the "new chat" dialog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartOptions {
    pub agent: Agent,
    /// Project directory the agent works in.
    pub cwd: String,
    /// `None` = the CLI's default model.
    #[serde(default)]
    pub model: Option<String>,
    /// Thinking effort (`low`, `medium`, `high`...). Claude: `--effort`;
    /// Codex: `-c model_reasoning_effort=`. `None` = the CLI's default.
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub permission: PermissionMode,
    /// Native id to continue: Claude session id (`--resume`) or Codex thread id
    /// (`thread/resume`). `None` starts a new conversation.
    #[serde(default)]
    pub resume_id: Option<String>,
    /// Advanced, user-typed. Claude: extra CLI arguments appended as-is
    /// (already split into words). Codex: `key=value` config overrides, each
    /// passed as `codex app-server -c key=value`.
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// First message, sent right after start when present.
    #[serde(default)]
    pub prompt: Option<String>,
    /// Images of the first message, as `data:image/...;base64,` URIs.
    #[serde(default)]
    pub prompt_images: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveState {
    /// Process launched, not ready yet.
    Starting,
    /// Ready for the next message.
    Idle,
    /// A turn is in progress.
    Running,
    /// A turn is paused on an approval request.
    WaitingApproval,
    /// The process ended (stopped, crashed or failed to start).
    Exited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Allow this one call.
    Allow,
    /// Allow this and similar calls for the rest of the session.
    AllowSession,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TurnUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Claude only: this turn's share of `total_cost_usd` (the CLI reports the
    /// process's running total).
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
}

/// Normalized live stream. `id`s are the driver's stable ids for one message /
/// tool call, so deltas and the final message can be matched.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveEvent {
    /// Native session / thread id is known (from Claude `system/init` or the
    /// Codex `thread/start` response) together with the resolved model.
    Started { native_id: String, model: Option<String> },
    /// Emitted by the manager whenever the state changes.
    Status { state: LiveState },
    /// The user's message as sent (emitted by the manager on `send`).
    UserMessage {
        id: String,
        text: String,
        /// Attached images as data URIs.
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        images: Vec<String>,
    },
    /// Streaming text appended to the assistant message `id`.
    AssistantDelta { id: String, text: String },
    /// The full assistant message `id`; replaces any deltas received for it.
    AssistantMessage { id: String, text: String },
    /// Reasoning / thinking summary text (optional, collapsed in the UI).
    Thinking { id: String, text: String },
    ToolStarted { id: String, name: String, kind: ToolKind, summary: String, input: String },
    /// Streaming tool output (shell stdout...), appended.
    ToolOutput { id: String, text: String },
    ToolFinished { id: String, output: Option<String>, is_error: bool },
    /// The agent asks before running a tool / command / edit.
    ApprovalRequest {
        request_id: String,
        tool_name: String,
        kind: ToolKind,
        /// One line: command, file path...
        summary: String,
        /// Pretty input or a unified diff.
        detail: String,
        /// Whether `allow_session` is offered.
        can_allow_session: bool,
    },
    ApprovalResolved { request_id: String, decision: Decision },
    TurnCompleted { usage: TurnUsage, is_error: bool },
    /// `fatal` errors end the session; others are shown inline (retries...).
    Error { message: String, fatal: bool },
    Exited { code: Option<i32> },
}

/// Tauri event `live://event` payload.
#[derive(Debug, Clone, Serialize)]
pub struct LiveEnvelope {
    pub live_id: String,
    pub seq: u64,
    pub event: LiveEvent,
}

/// Sink a driver reports into. Called from the driver's reader threads.
pub type Emit = Arc<dyn Fn(LiveEvent) + Send + Sync>;

/// One running CLI process. Methods must not block on the model; they only
/// write to the child's stdin (or kill it).
pub trait Driver: Send {
    /// `images` are `data:image/...;base64,` URIs.
    fn send(&mut self, text: &str, images: &[String]) -> Result<(), String>;
    fn respond(&mut self, request_id: &str, decision: Decision) -> Result<(), String>;
    /// Stop the current turn, keep the session.
    fn interrupt(&mut self) -> Result<(), String>;
    /// Kill the process (tree). Must make the reader emit `Exited`.
    fn stop(&mut self);
}

/// Summary shown in the live list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiveSessionInfo {
    pub live_id: String,
    pub agent: Agent,
    pub cwd: String,
    pub title: String,
    pub state: LiveState,
    pub native_id: Option<String>,
    pub model: Option<String>,
    pub permission: PermissionMode,
    /// Epoch ms.
    pub created_at: i64,
    pub updated_at: i64,
}

/// Starts a driver. Injectable so the manager can be tested without CLIs.
pub type Spawner = Arc<dyn Fn(&StartOptions, Emit) -> Result<Box<dyn Driver>, String> + Send + Sync>;

const TITLE_CHARS: usize = 40;

/// `(media_type, base64 payload)` of a `data:image/...;base64,...` URI.
pub fn decode_data_uri(uri: &str) -> Option<(&str, &str)> {
    let rest = uri.strip_prefix("data:")?;
    let (head, data) = rest.split_once(',')?;
    let media = head.strip_suffix(";base64")?;
    (media.starts_with("image/") && !data.is_empty()).then_some((media, data))
}

/// A usable effort level: trimmed, non-empty and only `[A-Za-z0-9_-]` (it is
/// passed on command lines, possibly through a `.cmd` shim). Others are ignored.
pub fn clean_effort(effort: &Option<String>) -> Option<&str> {
    effort
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn title_of(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    crate::convert::truncate_chars(line, TITLE_CHARS)
}

/// Forwards an envelope to the UI. Called under the session's lock, so the
/// envelopes of one session arrive in `seq` order.
pub type Publish = Arc<dyn Fn(&LiveEnvelope) + Send + Sync>;

/// Prefix of an image stored in the event log as a temp file path instead of a
/// data URI (`memdock-image:C:\...\x.png`); the UI loads it with `read_image_data_uri`.
pub const IMAGE_REF_PREFIX: &str = "memdock-image:";

/// Size limits and timeouts of the manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Most events kept per session (oldest dropped first).
    pub max_events: usize,
    /// Approximate most bytes (JSON size) of the kept events.
    pub max_bytes: usize,
    /// An idle session without an attached UI is stopped after this long.
    pub idle_timeout_ms: i64,
}

pub const MAX_EVENTS: usize = 5000;
pub const MAX_LOG_BYTES: usize = 8 * 1024 * 1024;
/// 30 minutes.
pub const IDLE_TIMEOUT_MS: i64 = 30 * 60 * 1000;
/// How often the idle reaper looks.
pub const REAP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

impl Default for Limits {
    fn default() -> Self {
        Self { max_events: MAX_EVENTS, max_bytes: MAX_LOG_BYTES, idle_timeout_ms: IDLE_TIMEOUT_MS }
    }
}

struct Stored {
    seq: u64,
    event: LiveEvent,
    size: usize,
}

fn size_of(ev: &LiveEvent) -> usize {
    serde_json::to_string(ev).map(|s| s.len()).unwrap_or(0)
}

/// State and log of one session; guarded by its own lock.
struct Entry {
    info: LiveSessionInfo,
    events: Vec<Stored>,
    bytes: usize,
    seq: u64,
    /// Index in `events` where the current turn starts (for compaction).
    turn_start: usize,
    exited: bool,
    /// Epoch ms of the last event or UI call.
    last_active: i64,
    /// A UI page shows this session (`live_attach`); never reaped then.
    attached: bool,
}

struct Session {
    entry: Mutex<Entry>,
    /// Separate lock: driver I/O never blocks recording or publishing.
    driver: Mutex<Option<Box<dyn Driver>>>,
}

struct Shared {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    publish: Publish,
    registry: Arc<NativeRegistry>,
    limits: Limits,
}

impl Shared {
    fn get(&self, live_id: &str) -> Option<Arc<Session>> {
        self.sessions.lock().unwrap().get(live_id).cloned()
    }
}

/// Owns all live sessions of the app.
pub struct LiveManager {
    shared: Arc<Shared>,
    spawner: Spawner,
}

impl LiveManager {
    pub fn new(spawner: Spawner, publish: Publish) -> Self {
        Self::with_registry(spawner, publish, Arc::new(NativeRegistry::new()))
    }

    pub fn with_registry(spawner: Spawner, publish: Publish, registry: Arc<NativeRegistry>) -> Self {
        let shared = Shared { sessions: Mutex::new(HashMap::new()), publish, registry, limits: Limits::default() };
        Self { shared: Arc::new(shared), spawner }
    }

    /// Replaces the limits (before any session starts).
    pub fn with_limits(mut self, limits: Limits) -> Self {
        if let Some(s) = Arc::get_mut(&mut self.shared) {
            s.limits = limits;
        }
        self
    }

    /// Production manager: real CLIs, events emitted on `app`.
    pub fn production(app: tauri::AppHandle, registry: Arc<NativeRegistry>) -> Self {
        let spawner: Spawner = Arc::new(|opts: &StartOptions, emit: Emit| match opts.agent {
            Agent::Claude => claude::spawn(opts, emit),
            Agent::Codex => codex::spawn(opts, emit),
        });
        let publish: Publish = Arc::new(move |env: &LiveEnvelope| {
            use tauri::Emitter;
            let _ = app.emit("live://event", env);
        });
        Self::with_registry(spawner, publish, registry)
    }

    pub fn registry(&self) -> &Arc<NativeRegistry> {
        &self.shared.registry
    }

    /// Appends `ev` (stored as `stored` when given) and publishes it. Caller holds the entry lock.
    fn push(shared: &Shared, e: &mut Entry, live_id: &str, ev: LiveEvent, stored: Option<LiveEvent>) {
        e.seq += 1;
        let stored = stored.unwrap_or_else(|| ev.clone());
        let size = size_of(&stored);
        e.bytes += size;
        e.events.push(Stored { seq: e.seq, event: stored, size });
        (shared.publish)(&LiveEnvelope { live_id: live_id.to_string(), seq: e.seq, event: ev });
    }

    fn set_state(shared: &Shared, e: &mut Entry, live_id: &str, state: LiveState) {
        if state == e.info.state || e.info.state == LiveState::Exited {
            return;
        }
        e.info.state = state;
        Self::push(shared, e, live_id, LiveEvent::Status { state }, None);
        if state == LiveState::Exited {
            shared.registry.release_owner(&Owner::live(live_id));
        }
    }

    /// Records `ev` for `live_id`, updates the summary and publishes it, all
    /// under the session's lock. `stored` replaces the event in the log.
    fn record_with(shared: &Shared, live_id: &str, ev: LiveEvent, stored: Option<LiveEvent>) {
        let Some(s) = shared.get(live_id) else { return };
        let mut guard = s.entry.lock().unwrap();
        let e = &mut *guard;
        // `stop()` records the exit itself; the driver's reader reports it again.
        if matches!(ev, LiveEvent::Exited { .. }) {
            if e.exited {
                return;
            }
            e.exited = true;
        }
        let next_state = match &ev {
            LiveEvent::Started { native_id, model } => {
                e.info.native_id = Some(native_id.clone());
                if model.is_some() {
                    e.info.model = model.clone();
                }
                if e.info.state != LiveState::Exited {
                    let me = Owner::live(live_id);
                    shared.registry.release_owner(&me);
                    let _ = shared.registry.claim(e.info.agent, native_id, me);
                }
                (e.info.state == LiveState::Starting).then_some(LiveState::Idle)
            }
            LiveEvent::UserMessage { .. } => Some(LiveState::Running),
            LiveEvent::ApprovalRequest { .. } => Some(LiveState::WaitingApproval),
            LiveEvent::ApprovalResolved { .. } => {
                (e.info.state == LiveState::WaitingApproval).then_some(LiveState::Running)
            }
            LiveEvent::TurnCompleted { .. } => Some(LiveState::Idle),
            LiveEvent::Error { fatal: true, .. } | LiveEvent::Exited { .. } => Some(LiveState::Exited),
            _ => None,
        };
        let completed = matches!(ev, LiveEvent::TurnCompleted { .. });
        let now = now_ms();
        e.info.updated_at = now;
        e.last_active = now;
        Self::push(shared, e, live_id, ev, stored);
        if let Some(st) = next_state {
            Self::set_state(shared, e, live_id, st);
        }
        if completed {
            compact_turn(e);
        }
        enforce_cap(e, &shared.limits);
    }

    fn record(shared: &Shared, live_id: &str, ev: LiveEvent) {
        Self::record_with(shared, live_id, ev, None)
    }

    pub fn start(&self, opts: StartOptions) -> Result<LiveSessionInfo, String> {
        let cwd = std::path::Path::new(&opts.cwd);
        if !cwd.is_dir() {
            return Err(format!("目录不存在：{}", opts.cwd));
        }
        let live_id = uuid::Uuid::new_v4().to_string();
        let resume = opts.resume_id.as_deref().map(str::trim).filter(|r| !r.is_empty()).map(String::from);
        if let Some(rid) = &resume {
            let reg = &self.shared.registry;
            if let Err(cur) = reg.claim(opts.agent, rid, Owner::live(&live_id)) {
                match cur.kind {
                    OwnerKind::Pty => return Err(format!("busy_pty:{}", cur.id)),
                    OwnerKind::Live => match self.info(&cur.id) {
                        // Already driven here: hand back that session.
                        Ok(info) if info.state != LiveState::Exited => {
                            let prompt = opts.prompt.as_deref().unwrap_or("");
                            if !prompt.trim().is_empty() || !opts.prompt_images.is_empty() {
                                self.send(&cur.id, prompt, &opts.prompt_images)?;
                            }
                            return self.info(&cur.id);
                        }
                        _ => reg.replace(opts.agent, rid, Owner::live(&live_id)),
                    },
                }
            }
        }
        let now = now_ms();
        let info = LiveSessionInfo {
            live_id: live_id.clone(),
            agent: opts.agent,
            cwd: opts.cwd.clone(),
            title: opts.prompt.as_deref().map(title_of).filter(|t| !t.is_empty()).unwrap_or_default(),
            state: LiveState::Starting,
            native_id: resume.clone(),
            model: opts.model.clone(),
            permission: opts.permission,
            created_at: now,
            updated_at: now,
        };
        let entry = Entry {
            info,
            events: vec![],
            bytes: 0,
            seq: 0,
            turn_start: 0,
            exited: false,
            last_active: now,
            attached: false,
        };
        let session = Arc::new(Session { entry: Mutex::new(entry), driver: Mutex::new(None) });
        self.shared.sessions.lock().unwrap().insert(live_id.clone(), session.clone());
        let (shared, id) = (self.shared.clone(), live_id.clone());
        let emit: Emit = Arc::new(move |ev| Self::record(&shared, &id, ev));
        let driver = match (self.spawner)(&opts, emit.clone()) {
            Ok(d) => d,
            Err(e) => {
                emit(LiveEvent::Error { message: e.clone(), fatal: true });
                self.shared.registry.release_owner(&Owner::live(&live_id));
                return Err(e);
            }
        };
        // The process may already have exited while spawning.
        if session.entry.lock().unwrap().info.state != LiveState::Exited {
            *session.driver.lock().unwrap() = Some(driver);
        }
        let prompt = opts.prompt.as_deref().unwrap_or("");
        if !prompt.trim().is_empty() || !opts.prompt_images.is_empty() {
            self.send(&live_id, prompt, &opts.prompt_images)?;
        }
        self.info(&live_id)
    }

    fn info(&self, live_id: &str) -> Result<LiveSessionInfo, String> {
        let s = self.shared.get(live_id).ok_or("会话不存在")?;
        let info = s.entry.lock().unwrap().info.clone();
        Ok(info)
    }

    /// The session, when it is still running.
    fn alive(&self, live_id: &str) -> Result<Arc<Session>, String> {
        let s = self.shared.get(live_id).ok_or("会话不存在")?;
        let mut e = s.entry.lock().unwrap();
        if e.exited || e.info.state == LiveState::Exited {
            return Err("会话已结束".into());
        }
        e.last_active = now_ms();
        drop(e);
        Ok(s)
    }

    /// Runs `f` on the driver under the driver lock only.
    fn with_driver<T>(s: &Session, f: impl FnOnce(&mut dyn Driver) -> Result<T, String>) -> Result<T, String> {
        let mut d = s.driver.lock().unwrap();
        let d = d.as_mut().ok_or("会话已结束")?;
        f(d.as_mut())
    }

    /// Puts the state back to `to` when it is still `from` (after a failed write).
    fn roll_back(&self, s: &Session, live_id: &str, err: &str, from: LiveState, to: LiveState) {
        let mut e = s.entry.lock().unwrap();
        Self::push(&self.shared, &mut e, live_id, LiveEvent::Error { message: err.to_string(), fatal: false }, None);
        if e.info.state == from {
            Self::set_state(&self.shared, &mut e, live_id, to);
        }
    }

    pub fn send(&self, live_id: &str, text: &str, images: &[String]) -> Result<(), String> {
        if let Some(bad) = images.iter().find(|i| decode_data_uri(i).is_none()) {
            return Err(format!("无法识别的图片数据：{}", crate::convert::truncate_chars(bad, 40)));
        }
        let s = self.alive(live_id)?;
        // The log keeps file references, not the (large) data URIs. No lock held.
        let refs = images
            .iter()
            .map(|uri| codex::save_image(uri).map(|p| format!("{IMAGE_REF_PREFIX}{p}")))
            .collect::<Result<Vec<_>, _>>()?;
        let prev = {
            let mut e = s.entry.lock().unwrap();
            if e.info.title.is_empty() {
                e.info.title = title_of(text);
            }
            e.info.state
        };
        let id = format!("user-{}", uuid::Uuid::new_v4());
        // Recorded before the write, so the turn's own events can't overtake it.
        Self::record_with(
            &self.shared,
            live_id,
            LiveEvent::UserMessage { id: id.clone(), text: text.to_string(), images: images.to_vec() },
            Some(LiveEvent::UserMessage { id, text: text.to_string(), images: refs }),
        );
        let r = Self::with_driver(&s, |d| d.send(text, images));
        if let Err(err) = &r {
            let to = if prev == LiveState::Starting { LiveState::Starting } else { LiveState::Idle };
            self.roll_back(&s, live_id, &format!("发送失败：{err}"), LiveState::Running, to);
        }
        r
    }

    pub fn respond(&self, live_id: &str, request_id: &str, decision: Decision) -> Result<(), String> {
        let s = self.alive(live_id)?;
        Self::record(
            &self.shared,
            live_id,
            LiveEvent::ApprovalResolved { request_id: request_id.to_string(), decision },
        );
        let r = Self::with_driver(&s, |d| d.respond(request_id, decision));
        if let Err(err) = &r {
            self.roll_back(&s, live_id, &format!("回复审批失败：{err}"), LiveState::Running, LiveState::WaitingApproval);
        }
        r
    }

    pub fn interrupt(&self, live_id: &str) -> Result<(), String> {
        let s = self.alive(live_id)?;
        Self::with_driver(&s, |d| d.interrupt())
    }

    pub fn stop(&self, live_id: &str) -> Result<(), String> {
        let s = self.shared.get(live_id).ok_or("会话不存在")?;
        let driver = s.driver.lock().unwrap().take();
        if let Some(mut d) = driver {
            d.stop();
        }
        Self::record(&self.shared, live_id, LiveEvent::Exited { code: None });
        Ok(())
    }

    /// Stops the session and forgets it.
    pub fn close(&self, live_id: &str) -> Result<(), String> {
        let _ = self.stop(live_id);
        self.shared.sessions.lock().unwrap().remove(live_id);
        self.shared.registry.release_owner(&Owner::live(live_id));
        Ok(())
    }

    /// Marks whether a UI page shows the session (attached sessions are never reaped).
    pub fn attach(&self, live_id: &str, attached: bool) -> Result<(), String> {
        let s = self.shared.get(live_id).ok_or("会话不存在")?;
        let mut e = s.entry.lock().unwrap();
        e.attached = attached;
        e.last_active = now_ms();
        Ok(())
    }

    fn all(&self) -> Vec<Arc<Session>> {
        self.shared.sessions.lock().unwrap().values().cloned().collect()
    }

    /// Newest first.
    pub fn list(&self) -> Vec<LiveSessionInfo> {
        let mut v: Vec<_> = self.all().iter().map(|s| s.entry.lock().unwrap().info.clone()).collect();
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        v
    }

    /// The kept events with their original `seq` (compaction leaves gaps), to
    /// rebuild the page after re-attaching. Images are `memdock-image:` paths.
    pub fn transcript(&self, live_id: &str) -> Result<Vec<LiveEnvelope>, String> {
        let s = self.shared.get(live_id).ok_or("会话不存在")?;
        let mut e = s.entry.lock().unwrap();
        e.last_active = now_ms();
        Ok(e.events
            .iter()
            .map(|x| LiveEnvelope { live_id: live_id.to_string(), seq: x.seq, event: x.event.clone() })
            .collect())
    }

    /// Stops sessions idle (state `Idle`, no attached UI) since `idle_timeout_ms`
    /// before `now` (epoch ms). Returns the stopped live ids; they stay listed
    /// and resumable.
    pub fn reap_idle(&self, now: i64) -> Vec<String> {
        let timeout = self.shared.limits.idle_timeout_ms;
        let ids: Vec<String> = self
            .shared
            .sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(id, s)| (id.clone(), s.clone()))
            .collect::<Vec<_>>()
            .into_iter()
            .filter(|(_, s)| {
                let e = s.entry.lock().unwrap();
                e.info.state == LiveState::Idle && !e.attached && now - e.last_active >= timeout
            })
            .map(|(id, _)| id)
            .collect();
        for id in &ids {
            let _ = self.stop(id);
        }
        ids
    }

    /// Background thread running [`Self::reap_idle`] every [`REAP_INTERVAL`]
    /// while the manager lives.
    pub fn spawn_reaper(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(REAP_INTERVAL);
                let Some(mgr) = weak.upgrade() else { break };
                mgr.reap_idle(now_ms());
            }
        });
    }

    /// Kills every process (app exit).
    pub fn stop_all(&self) {
        for s in self.all() {
            let d = s.driver.lock().unwrap().take();
            if let Some(mut d) = d {
                d.stop();
            }
        }
    }
}

/// On `TurnCompleted`: drops the turn's `AssistantDelta`s whose message has a
/// final `AssistantMessage` (the final takes the place of the first delta, so
/// the replay order stays the same) and the `ToolOutput`s of tools whose
/// `ToolFinished` carries the output. Remaining events keep their `seq`.
fn compact_turn(e: &mut Entry) {
    use std::collections::{HashMap as Map, HashSet};
    let from = e.turn_start.min(e.events.len());
    let turn = &e.events[from..];
    let mut finals: Map<&str, usize> = Map::new();
    let mut tools_done: HashSet<&str> = HashSet::new();
    for (i, x) in turn.iter().enumerate() {
        match &x.event {
            LiveEvent::AssistantMessage { id, .. } => {
                finals.insert(id.as_str(), from + i);
            }
            LiveEvent::ToolFinished { id, output: Some(_), .. } => {
                tools_done.insert(id.as_str());
            }
            _ => {}
        }
    }
    let mut gone = vec![false; e.events.len()];
    let mut moves: Vec<(usize, usize)> = vec![]; // (first delta, final)
    let mut seen: HashSet<&str> = HashSet::new();
    for (i, x) in turn.iter().enumerate() {
        let i = from + i;
        match &x.event {
            LiveEvent::AssistantDelta { id, .. } => {
                if let Some(&fi) = finals.get(id.as_str()) {
                    if fi > i && seen.insert(id.as_str()) {
                        moves.push((i, fi));
                    } else {
                        gone[i] = true;
                    }
                }
            }
            LiveEvent::ToolOutput { id, .. } if tools_done.contains(id.as_str()) => gone[i] = true,
            _ => {}
        }
    }
    for (di, fi) in moves {
        let ev = e.events[fi].event.clone();
        let size = e.events[fi].size;
        e.bytes = e.bytes - e.events[di].size + size;
        e.events[di].event = ev;
        e.events[di].size = size;
        gone[fi] = true;
    }
    let mut i = 0;
    let mut removed = 0;
    e.events.retain(|x| {
        let d = gone[i];
        i += 1;
        if d {
            removed += x.size;
        }
        !d
    });
    e.bytes -= removed;
    e.turn_start = e.events.len();
}

/// Drops the oldest events beyond the caps, keeping the latest dropped
/// `Started` and `Status` as a leading snapshot.
fn enforce_cap(e: &mut Entry, limits: &Limits) {
    let n = e.events.len();
    if n <= limits.max_events && e.bytes <= limits.max_bytes {
        return;
    }
    // Room for the two snapshot events.
    let keep_events = limits.max_events.saturating_sub(2).max(1);
    let (mut k, mut bytes) = (0, e.bytes);
    while k < n - 1 && (n - k > keep_events || bytes > limits.max_bytes) {
        bytes -= e.events[k].size;
        k += 1;
    }
    if k == 0 {
        return;
    }
    let dropped: Vec<Stored> = e.events.drain(..k).collect();
    let started = dropped.iter().rposition(|x| matches!(x.event, LiveEvent::Started { .. }));
    let status = dropped.iter().rposition(|x| matches!(x.event, LiveEvent::Status { .. }));
    let mut keep: Vec<usize> = started.into_iter().chain(status).collect();
    keep.sort_unstable();
    let mut head: Vec<Stored> = vec![];
    for (i, x) in dropped.into_iter().enumerate() {
        if keep.contains(&i) {
            head.push(x);
        }
    }
    let kept = head.len();
    e.bytes = bytes + head.iter().map(|x| x.size).sum::<usize>();
    e.events.splice(0..0, head);
    e.turn_start = if e.turn_start >= k { e.turn_start - k + kept } else { kept };
}

// ---- Tauri commands -------------------------------------------------------

#[tauri::command]
pub async fn live_start(mgr: tauri::State<'_, Arc<LiveManager>>, opts: StartOptions) -> Result<LiveSessionInfo, String> {
    let mgr = mgr.inner().clone();
    tauri::async_runtime::spawn_blocking(move || mgr.start(opts)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn live_send(
    mgr: tauri::State<'_, Arc<LiveManager>>,
    live_id: String,
    text: String,
    images: Option<Vec<String>>,
) -> Result<(), String> {
    mgr.send(&live_id, &text, &images.unwrap_or_default())
}

#[tauri::command]
pub fn live_respond(
    mgr: tauri::State<'_, Arc<LiveManager>>,
    live_id: String,
    request_id: String,
    decision: Decision,
) -> Result<(), String> {
    mgr.respond(&live_id, &request_id, decision)
}

#[tauri::command]
pub fn live_interrupt(mgr: tauri::State<'_, Arc<LiveManager>>, live_id: String) -> Result<(), String> {
    mgr.interrupt(&live_id)
}

#[tauri::command]
pub fn live_stop(mgr: tauri::State<'_, Arc<LiveManager>>, live_id: String) -> Result<(), String> {
    mgr.stop(&live_id)
}

#[tauri::command]
pub fn live_close(mgr: tauri::State<'_, Arc<LiveManager>>, live_id: String) -> Result<(), String> {
    mgr.close(&live_id)
}

#[tauri::command]
pub fn live_list(mgr: tauri::State<'_, Arc<LiveManager>>) -> Vec<LiveSessionInfo> {
    mgr.list()
}

#[tauri::command]
pub fn live_transcript(mgr: tauri::State<'_, Arc<LiveManager>>, live_id: String) -> Result<Vec<LiveEnvelope>, String> {
    mgr.transcript(&live_id)
}

/// Optional: the UI shows (`true`) / left (`false`) the session; attached
/// sessions are never stopped by the idle reaper.
#[tauri::command]
pub fn live_attach(mgr: tauri::State<'_, Arc<LiveManager>>, live_id: String, attached: bool) -> Result<(), String> {
    mgr.attach(&live_id, attached)
}

/// Who in this app drives the native session / thread, if anyone.
#[tauri::command]
pub fn native_owner(registry: tauri::State<'_, Arc<NativeRegistry>>, agent: Agent, native_id: String) -> Option<Owner> {
    registry.owner(agent, native_id.trim())
}

/// Largest image file `read_image_data_uri` accepts.
pub const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;

/// Media type of an image file by extension (`None` = not an image we send).
pub fn image_media_type(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Reads a dropped / picked image file as a `data:image/...;base64,` URI (max 5 MB).
pub fn image_data_uri(path: &std::path::Path) -> Result<String, String> {
    use base64::Engine;
    let media = image_media_type(path).ok_or("不支持的图片格式（支持 png / jpg / gif / webp）")?;
    let meta = std::fs::metadata(path).map_err(|e| format!("读取图片失败：{e}"))?;
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(format!("图片超过 5 MB：{}", path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("读取图片失败：{e}"))?;
    Ok(format!("data:{media};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

#[tauri::command]
pub async fn read_image_data_uri(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || image_data_uri(std::path::Path::new(&path)))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests;

#[tauri::command]
pub async fn live_models(
    state: tauri::State<'_, Arc<crate::commands::AppState>>,
    agent: Agent,
    force: Option<bool>,
) -> Result<Vec<models::ModelOption>, String> {
    let codex_home = state.paths.codex_home.clone();
    tauri::async_runtime::spawn_blocking(move || models::models(agent, &codex_home, force.unwrap_or(false)))
        .await
        .map_err(|e| e.to_string())?
}

/// Slash commands and skills for the `/` menu (same probe and cache as `live_models`).
#[tauri::command]
pub async fn live_commands(
    state: tauri::State<'_, Arc<crate::commands::AppState>>,
    agent: Agent,
    force: Option<bool>,
) -> Result<Vec<models::CommandOption>, String> {
    let codex_home = state.paths.codex_home.clone();
    tauri::async_runtime::spawn_blocking(move || models::commands(agent, &codex_home, force.unwrap_or(false)))
        .await
        .map_err(|e| e.to_string())?
}

/// Writes a pasted image (`data:image/...;base64,` URI) to `%TEMP%/memdock-images/`
/// and returns its absolute path (for typing into a terminal).
#[tauri::command]
pub fn save_temp_image(data_uri: String) -> Result<String, String> {
    codex::save_image(&data_uri)
}
