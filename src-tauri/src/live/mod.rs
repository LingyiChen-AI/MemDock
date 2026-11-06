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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::convert::timeline::ToolKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Claude only (`total_cost_usd` of the result line).
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

/// Forwards an envelope to the UI.
pub type Publish = Arc<dyn Fn(&LiveEnvelope) + Send + Sync>;

struct Entry {
    info: LiveSessionInfo,
    driver: Option<Box<dyn Driver>>,
    events: Vec<LiveEvent>,
    seq: u64,
}

/// Owns all live sessions of the app.
pub struct LiveManager {
    sessions: Arc<Mutex<HashMap<String, Entry>>>,
    spawner: Spawner,
    publish: Publish,
}

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

impl LiveManager {
    pub fn new(spawner: Spawner, publish: Publish) -> Self {
        Self { sessions: Arc::new(Mutex::new(HashMap::new())), spawner, publish }
    }

    /// Production manager: real CLIs, events emitted on `app`.
    pub fn production(app: tauri::AppHandle) -> Self {
        let spawner: Spawner = Arc::new(|opts: &StartOptions, emit: Emit| match opts.agent {
            Agent::Claude => claude::spawn(opts, emit),
            Agent::Codex => codex::spawn(opts, emit),
        });
        let publish: Publish = Arc::new(move |env: &LiveEnvelope| {
            use tauri::Emitter;
            let _ = app.emit("live://event", env);
        });
        Self::new(spawner, publish)
    }

    /// Records `ev` for `live_id`, updates the summary and publishes it.
    fn record(sessions: &Mutex<HashMap<String, Entry>>, publish: &Publish, live_id: &str, ev: LiveEvent) {
        let mut out = vec![];
        {
            let mut map = sessions.lock().unwrap();
            let Some(e) = map.get_mut(live_id) else { return };
            // `stop()` records the exit itself; the driver's reader reports it again.
            if matches!(ev, LiveEvent::Exited { .. }) && e.events.iter().any(|x| matches!(x, LiveEvent::Exited { .. })) {
                return;
            }
            let next_state = match &ev {
                LiveEvent::Started { native_id, model } => {
                    e.info.native_id = Some(native_id.clone());
                    if model.is_some() {
                        e.info.model = model.clone();
                    }
                    (e.info.state == LiveState::Starting).then_some(LiveState::Idle)
                }
                LiveEvent::UserMessage { .. } => Some(LiveState::Running),
                LiveEvent::ApprovalRequest { .. } => Some(LiveState::WaitingApproval),
                LiveEvent::ApprovalResolved { .. } => Some(LiveState::Running),
                LiveEvent::TurnCompleted { .. } => Some(LiveState::Idle),
                LiveEvent::Error { fatal: true, .. } | LiveEvent::Exited { .. } => Some(LiveState::Exited),
                _ => None,
            };
            if matches!(ev, LiveEvent::Exited { .. }) {
                e.driver = None;
            }
            e.info.updated_at = now_ms();
            for ev in std::iter::once(ev).chain(
                next_state
                    .filter(|s| *s != e.info.state && e.info.state != LiveState::Exited)
                    .map(|s| LiveEvent::Status { state: s }),
            ) {
                if let LiveEvent::Status { state } = ev {
                    e.info.state = state;
                }
                e.seq += 1;
                e.events.push(ev.clone());
                out.push(LiveEnvelope { live_id: live_id.to_string(), seq: e.seq, event: ev });
            }
        }
        for env in &out {
            publish(env);
        }
    }

    pub fn start(&self, opts: StartOptions) -> Result<LiveSessionInfo, String> {
        let cwd = std::path::Path::new(&opts.cwd);
        if !cwd.is_dir() {
            return Err(format!("目录不存在：{}", opts.cwd));
        }
        let live_id = uuid::Uuid::new_v4().to_string();
        let now = now_ms();
        let info = LiveSessionInfo {
            live_id: live_id.clone(),
            agent: opts.agent,
            cwd: opts.cwd.clone(),
            title: opts.prompt.as_deref().map(title_of).filter(|t| !t.is_empty()).unwrap_or_default(),
            state: LiveState::Starting,
            native_id: opts.resume_id.clone(),
            model: opts.model.clone(),
            permission: opts.permission,
            created_at: now,
            updated_at: now,
        };
        self.sessions
            .lock()
            .unwrap()
            .insert(live_id.clone(), Entry { info, driver: None, events: vec![], seq: 0 });
        let (sessions, publish, id) = (self.sessions.clone(), self.publish.clone(), live_id.clone());
        let emit: Emit = Arc::new(move |ev| Self::record(&sessions, &publish, &id, ev));
        let driver = match (self.spawner)(&opts, emit.clone()) {
            Ok(d) => d,
            Err(e) => {
                emit(LiveEvent::Error { message: e.clone(), fatal: true });
                return Err(e);
            }
        };
        if let Some(e) = self.sessions.lock().unwrap().get_mut(&live_id) {
            // The process may already have exited while spawning.
            if e.info.state != LiveState::Exited {
                e.driver = Some(driver);
            }
        }
        let prompt = opts.prompt.as_deref().unwrap_or("");
        if !prompt.trim().is_empty() || !opts.prompt_images.is_empty() {
            self.send(&live_id, prompt, &opts.prompt_images)?;
        }
        self.info(&live_id)
    }

    fn info(&self, live_id: &str) -> Result<LiveSessionInfo, String> {
        self.sessions.lock().unwrap().get(live_id).map(|e| e.info.clone()).ok_or_else(|| "会话不存在".to_string())
    }

    fn with_driver<T>(&self, live_id: &str, f: impl FnOnce(&mut dyn Driver) -> Result<T, String>) -> Result<T, String> {
        let mut map = self.sessions.lock().unwrap();
        let e = map.get_mut(live_id).ok_or("会话不存在")?;
        let d = e.driver.as_mut().ok_or("会话已结束")?;
        f(d.as_mut())
    }

    pub fn send(&self, live_id: &str, text: &str, images: &[String]) -> Result<(), String> {
        if let Some(bad) = images.iter().find(|i| decode_data_uri(i).is_none()) {
            return Err(format!("无法识别的图片数据：{}", crate::convert::truncate_chars(bad, 40)));
        }
        self.with_driver(live_id, |d| d.send(text, images))?;
        {
            let mut map = self.sessions.lock().unwrap();
            if let Some(e) = map.get_mut(live_id) {
                if e.info.title.is_empty() {
                    e.info.title = title_of(text);
                }
            }
        }
        let id = format!("user-{}", uuid::Uuid::new_v4());
        Self::record(
            &self.sessions,
            &self.publish,
            live_id,
            LiveEvent::UserMessage { id, text: text.to_string(), images: images.to_vec() },
        );
        Ok(())
    }

    pub fn respond(&self, live_id: &str, request_id: &str, decision: Decision) -> Result<(), String> {
        self.with_driver(live_id, |d| d.respond(request_id, decision))?;
        Self::record(
            &self.sessions,
            &self.publish,
            live_id,
            LiveEvent::ApprovalResolved { request_id: request_id.to_string(), decision },
        );
        Ok(())
    }

    pub fn interrupt(&self, live_id: &str) -> Result<(), String> {
        self.with_driver(live_id, |d| d.interrupt())
    }

    pub fn stop(&self, live_id: &str) -> Result<(), String> {
        let driver = {
            let mut map = self.sessions.lock().unwrap();
            map.get_mut(live_id).ok_or("会话不存在")?.driver.take()
        };
        if let Some(mut d) = driver {
            d.stop();
        }
        Self::record(&self.sessions, &self.publish, live_id, LiveEvent::Exited { code: None });
        Ok(())
    }

    /// Stops the session and forgets it.
    pub fn close(&self, live_id: &str) -> Result<(), String> {
        let _ = self.stop(live_id);
        self.sessions.lock().unwrap().remove(live_id);
        Ok(())
    }

    /// Newest first.
    pub fn list(&self) -> Vec<LiveSessionInfo> {
        let mut v: Vec<_> = self.sessions.lock().unwrap().values().map(|e| e.info.clone()).collect();
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        v
    }

    /// Every event so far, to rebuild the page after re-attaching.
    pub fn transcript(&self, live_id: &str) -> Result<Vec<LiveEnvelope>, String> {
        let map = self.sessions.lock().unwrap();
        let e = map.get(live_id).ok_or("会话不存在")?;
        Ok(e.events
            .iter()
            .enumerate()
            .map(|(i, ev)| LiveEnvelope { live_id: live_id.to_string(), seq: i as u64 + 1, event: ev.clone() })
            .collect())
    }

    /// Kills every process (app exit).
    pub fn stop_all(&self) {
        let drivers: Vec<_> = self.sessions.lock().unwrap().values_mut().filter_map(|e| e.driver.take()).collect();
        for mut d in drivers {
            d.stop();
        }
    }
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
