//! Claude Code driver for live chat. See the module docs of `live` for the protocol.
//!
//! One `claude -p --input-format stream-json --output-format stream-json` process
//! per session. stdin takes JSON lines (user messages, control requests and the
//! answers to permission prompts), stdout yields JSON lines that [`Parser`] maps
//! to [`LiveEvent`]s.
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{Decision, Driver, Emit, LiveEvent, PermissionMode, StartOptions, TurnUsage};
use crate::convert::timeline::{classify, summarize_input};
use crate::convert::truncate_chars;
use crate::extract::runner::{find_program, path_dirs};
use crate::proc;

/// Max chars of a tool input / output / approval detail sent to the UI.
const IO_MAX: usize = 4000;
/// Max chars of stderr quoted in an error message.
const STDERR_TAIL: usize = 1200;
const DENY_MESSAGE: &str = "User denied this action in MemDock";

// ---- command line -----------------------------------------------------------

/// CLI arguments (without the program) for `opts`.
/// Session id known before the CLI reports it: the resumed id, or a fresh
/// UUID passed as `--session-id` (unless the user's extra args pick one).
pub(crate) fn known_session_id(opts: &StartOptions) -> Option<(String, bool)> {
    if let Some(r) = opts.resume_id.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        // `--fork-session` gets a new id, only known after `system/init`.
        let forks = opts.extra_args.iter().any(|a| a == "--fork-session");
        return (!forks).then(|| (r.to_string(), false));
    }
    let picks = opts
        .extra_args
        .iter()
        .any(|a| matches!(a.as_str(), "--session-id" | "-c" | "--continue" | "-r" | "--resume"));
    (!picks).then(|| (uuid::Uuid::new_v4().to_string(), true))
}

/// CLI arguments (without the program) for `opts`; `new_session_id` is
/// passed as `--session-id`.
pub(crate) fn build_args(opts: &StartOptions, new_session_id: Option<&str>) -> Vec<String> {
    let mut v: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some(m) = opts.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        v.extend(["--model".into(), m.to_string()]);
    }
    if let Some(e) = super::clean_effort(&opts.effort) {
        v.extend(["--effort".into(), e.to_string()]);
    }
    if let Some(r) = opts.resume_id.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        v.extend(["--resume".into(), r.to_string()]);
    } else if let Some(sid) = new_session_id {
        v.extend(["--session-id".into(), sid.to_string()]);
    }
    let mode = match opts.permission {
        PermissionMode::Ask => Some("default"),
        PermissionMode::AutoEdit => Some("acceptEdits"),
        PermissionMode::Plan => Some("plan"),
        PermissionMode::Bypass => None,
    };
    match mode {
        Some(m) => v.extend(
            ["--permission-mode", m, "--permission-prompt-tool", "stdio"].iter().map(|s| s.to_string()),
        ),
        None => v.push("--dangerously-skip-permissions".into()),
    }
    v.extend(opts.extra_args.iter().cloned());
    v
}

// ---- stdin messages ---------------------------------------------------------

pub(crate) fn user_message(text: &str, images: &[String]) -> Value {
    if images.is_empty() {
        return json!({"type": "user", "message": {"role": "user", "content": text}});
    }
    let mut content: Vec<Value> = images
        .iter()
        .filter_map(|uri| super::decode_data_uri(uri))
        .map(|(media, data)| json!({"type": "image", "source": {"type": "base64", "media_type": media, "data": data}}))
        .collect();
    if !text.trim().is_empty() {
        content.push(json!({"type": "text", "text": text}));
    }
    json!({"type": "user", "message": {"role": "user", "content": content}})
}

pub(crate) fn control_request(request_id: &str, subtype: &str) -> Value {
    json!({"type": "control_request", "request_id": request_id, "request": {"subtype": subtype}})
}

/// A permission prompt still waiting for the user's answer.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pending {
    input: Value,
    suggestions: Vec<Value>,
}

pub(crate) fn permission_response(request_id: &str, p: &Pending, decision: Decision) -> Value {
    let inner = match decision {
        Decision::Deny => json!({"behavior": "deny", "message": DENY_MESSAGE}),
        Decision::Allow => json!({"behavior": "allow", "updatedInput": p.input}),
        Decision::AllowSession if p.suggestions.is_empty() => {
            json!({"behavior": "allow", "updatedInput": p.input})
        }
        Decision::AllowSession => {
            json!({"behavior": "allow", "updatedInput": p.input, "updatedPermissions": p.suggestions})
        }
    };
    json!({"type": "control_response", "response": {"subtype": "success", "request_id": request_id, "response": inner}})
}

fn error_response(request_id: &str, message: &str) -> Value {
    json!({"type": "control_response", "response": {"subtype": "error", "request_id": request_id, "error": message}})
}

// ---- stdout parsing ---------------------------------------------------------

/// What one stdout line produced.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Parsed {
    pub events: Vec<LiveEvent>,
    /// Lines to write back to stdin (answers to unsupported control requests).
    pub replies: Vec<Value>,
}

/// Line → events mapping with the little state it needs.
#[derive(Debug, Default)]
pub(crate) struct Parser {
    /// Session id of the last `system/init`.
    session_id: Option<String>,
    /// Last `Started` reported (id, model), to skip repeats.
    announced: Option<(String, Option<String>)>,
    /// Current streamed message id (from `message_start`).
    current_msg: Option<String>,
    /// message id → content blocks already seen in `assistant` lines; the k-th
    /// block seen there is stream block index k.
    blocks_seen: HashMap<String, usize>,
    /// request_id → permission prompt awaiting an answer.
    pending: HashMap<String, Pending>,
    /// `total_cost_usd` of the last `result`: the CLI reports the process's
    /// running total, turns get the difference.
    cost_total: f64,
    /// Compactions seen (ids of the compacting / compacted notices).
    compactions: u32,
    /// A compaction is under way (its notice is shown; repeats of the status are ignored).
    compacting: bool,
    /// Local slash command outputs seen (ids of their notices).
    notices: u32,
    /// The user interrupted the current turn: its error result is the
    /// interruption itself, not an error to show.
    interrupted: bool,
}

fn pretty(v: &Value, max: usize) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => serde_json::to_string_pretty(v).unwrap_or_default(),
    };
    truncate_chars(&s, max)
}

/// Text of a `tool_result` content (string or `[{type:text,text}]`).
fn result_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

fn str_of_ptr<'a>(v: &'a Value, ptr: &str) -> Option<&'a str> {
    v.pointer(ptr).and_then(Value::as_str)
}

/// The text inside `<local-command-stdout>` / `<local-command-stderr>`, trimmed; `None` when absent or empty.
/// Only a message that *is* command output counts (it starts with the tag): a compaction summary
/// that merely mentions the tags is not output. `Compacted` (from `/compact`) is left to the
/// compaction notice.
pub(crate) fn local_command_output(text: &str) -> Option<String> {
    if !text.trim_start().starts_with("<local-command-") {
        return None;
    }
    let mut parts = vec![];
    for tag in ["local-command-stdout", "local-command-stderr"] {
        let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
        let mut rest = text;
        while let Some(i) = rest.find(&open) {
            let after = &rest[i + open.len()..];
            let end = after.find(&close).unwrap_or(after.len());
            let t = after[..end].trim();
            if !t.is_empty() {
                parts.push(t.to_string());
            }
            rest = &after[end..];
        }
    }
    parts.retain(|p| !p.eq_ignore_ascii_case("compacted"));
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

impl Parser {
    /// True once the CLI itself sent `system/init`.
    pub fn started(&self) -> bool {
        self.session_id.is_some()
    }

    /// The CLI sends `system/init` only with the first turn, so the driver
    /// announces the id it already knows (pre-assigned or resumed) at spawn.
    /// Returns the `Started` event to emit.
    pub fn announce(&mut self, native_id: &str, model: Option<String>) -> LiveEvent {
        self.announced = Some((native_id.to_string(), model.clone()));
        LiveEvent::Started { native_id: native_id.to_string(), model }
    }

    /// A new user message starts a turn (clears a stale interrupt).
    pub fn on_send(&mut self) {
        self.interrupted = false;
    }

    /// The user asked to interrupt the running turn.
    pub fn on_interrupt(&mut self) {
        self.interrupted = true;
    }

    /// Removes and returns the pending prompt `request_id`.
    pub fn take_pending(&mut self, request_id: &str) -> Option<Pending> {
        self.pending.remove(request_id)
    }

    /// `None` when the line is not JSON.
    pub fn line(&mut self, line: &str) -> Option<Parsed> {
        let v: Value = serde_json::from_str(line.trim()).ok()?;
        let mut out = Parsed::default();
        // Lines of a subagent (Task tool) are shown only through its tool call.
        let nested = v.get("parent_tool_use_id").is_some_and(|p| !p.is_null());
        match str_of(&v, "type").unwrap_or("") {
            "system" if str_of(&v, "subtype") == Some("init") => {
                if let Some(sid) = str_of(&v, "session_id") {
                    self.session_id = Some(sid.to_string());
                    let now = (sid.to_string(), str_of(&v, "model").map(str::to_string));
                    if self.announced.as_ref() != Some(&now) {
                        self.announced = Some(now.clone());
                        out.events.push(LiveEvent::Started { native_id: now.0, model: now.1 });
                    }
                }
            }
            "stream_event" if !nested => self.stream_event(&v, &mut out),
            "assistant" if !nested => self.assistant(&v, &mut out),
            "system" if str_of(&v, "subtype") == Some("status") => {
                if str_of(&v, "status") == Some("compacting") && !self.compacting {
                    self.compacting = true;
                    self.compactions += 1;
                    out.events.push(LiveEvent::Notice {
                        id: format!("compact-{}", self.compactions),
                        kind: "compacting".into(),
                        text: String::new(),
                        before: None,
                        after: None,
                    });
                }
            }
            "system" if str_of(&v, "subtype") == Some("compact_boundary") => {
                self.compacting = false;
                if self.compactions == 0 {
                    self.compactions = 1; // automatic compaction without a status line
                }
                let meta = v.get("compact_metadata");
                let tok = |k: &str| meta.and_then(|m| m.get(k)).and_then(Value::as_u64);
                out.events.push(LiveEvent::Notice {
                    id: format!("compact-{}", self.compactions),
                    kind: "compacted".into(),
                    text: String::new(),
                    before: tok("pre_tokens"),
                    after: tok("post_tokens"),
                });
            }
            "user" if !nested && v.pointer("/message/content").is_some_and(Value::is_string) => {
                // Output of a local slash command (/compact, /cost...): <local-command-stdout>…</…>.
                let text = str_of_ptr(&v, "/message/content").unwrap_or("");
                if let Some(out_text) = local_command_output(text) {
                    self.notices += 1;
                    out.events.push(LiveEvent::Notice {
                        id: format!("cmd-{}", self.notices),
                        kind: "command_output".into(),
                        text: out_text,
                        before: None,
                        after: None,
                    });
                }
            }
            "user" if !nested => {
                let content = v.pointer("/message/content").and_then(Value::as_array);
                for b in content.into_iter().flatten() {
                    if str_of(b, "type") != Some("tool_result") {
                        continue;
                    }
                    let Some(id) = str_of(b, "tool_use_id") else { continue };
                    let text = result_text(b.get("content").unwrap_or(&Value::Null));
                    out.events.push(LiveEvent::ToolFinished {
                        id: id.to_string(),
                        output: Some(truncate_chars(&text, IO_MAX)),
                        is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                    });
                }
            }
            "control_request" => self.control_request(&v, &mut out),
            "control_cancel_request" => {
                if let Some(id) = str_of(&v, "request_id") {
                    self.pending.remove(id);
                }
            }
            "result" => {
                // An interrupted turn ends as `error_during_execution`; like Codex's
                // `interrupted` status it is not an error.
                let interrupted = std::mem::take(&mut self.interrupted)
                    && str_of(&v, "subtype") == Some("error_during_execution");
                let is_error = !interrupted && v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                let cost = v.get("total_cost_usd").and_then(Value::as_f64).map(|total| {
                    let turn = (total - self.cost_total).max(0.0);
                    self.cost_total = total.max(self.cost_total);
                    turn
                });
                let u = v.get("usage").cloned().unwrap_or(Value::Null);
                let n = |k: &str| u.get(k).and_then(Value::as_u64);
                // Cached prompt tokens are most of the input; count them too.
                let input = match (n("input_tokens"), n("cache_creation_input_tokens"), n("cache_read_input_tokens")) {
                    (None, None, None) => None,
                    (a, b, c) => Some(a.unwrap_or(0) + b.unwrap_or(0) + c.unwrap_or(0)),
                };
                if is_error {
                    let msg = str_of(&v, "result")
                        .filter(|s| !s.trim().is_empty())
                        .map(str::to_string)
                        .or_else(|| v.get("errors").and_then(Value::as_array).map(|a| result_text(&Value::Array(a.clone()))).filter(|s| !s.is_empty()))
                        .unwrap_or_else(|| format!("本轮出错（{}）", str_of(&v, "subtype").unwrap_or("error")));
                    out.events.push(LiveEvent::Error { message: msg, fatal: false });
                }
                self.pending.clear();
                out.events.push(LiveEvent::TurnCompleted {
                    usage: TurnUsage {
                        input_tokens: input,
                        output_tokens: n("output_tokens"),
                        cost_usd: cost,
                        duration_ms: v.get("duration_ms").and_then(Value::as_u64),
                    },
                    is_error,
                });
            }
            // system/hook_*, system/status, control_response, rate_limit_event...
            _ => {}
        }
        Some(out)
    }

    fn stream_event(&mut self, v: &Value, out: &mut Parsed) {
        let Some(ev) = v.get("event") else { return };
        match str_of(ev, "type").unwrap_or("") {
            "message_start" => {
                self.current_msg = ev.pointer("/message/id").and_then(Value::as_str).map(str::to_string);
            }
            "content_block_delta" => {
                let Some(d) = ev.get("delta") else { return };
                if str_of(d, "type") != Some("text_delta") {
                    return;
                }
                let text = str_of(d, "text").unwrap_or("");
                let msg = str_of(v, "api_message_id").map(str::to_string).or_else(|| self.current_msg.clone());
                let (Some(msg), Some(idx)) = (msg, ev.get("index").and_then(Value::as_u64)) else { return };
                if !text.is_empty() {
                    out.events.push(LiveEvent::AssistantDelta { id: format!("{msg}:{idx}"), text: text.to_string() });
                }
            }
            _ => {}
        }
    }

    fn assistant(&mut self, v: &Value, out: &mut Parsed) {
        let Some(msg) = v.get("message") else { return };
        let msg_id = str_of(msg, "id").unwrap_or("msg").to_string();
        let blocks = msg.get("content").and_then(Value::as_array).cloned().unwrap_or_default();
        for b in &blocks {
            let seen = self.blocks_seen.entry(msg_id.clone()).or_insert(0);
            let idx = *seen;
            *seen += 1;
            let id = format!("{msg_id}:{idx}");
            match str_of(b, "type").unwrap_or("") {
                "text" => {
                    let text = str_of(b, "text").unwrap_or("");
                    if !text.is_empty() {
                        out.events.push(LiveEvent::AssistantMessage { id, text: text.to_string() });
                    }
                }
                "thinking" => {
                    let text = str_of(b, "thinking").unwrap_or("");
                    if !text.trim().is_empty() {
                        out.events.push(LiveEvent::Thinking { id, text: text.to_string() });
                    }
                }
                "tool_use" => {
                    let name = str_of(b, "name").unwrap_or("tool").to_string();
                    let input = b.get("input").cloned().unwrap_or(Value::Null);
                    out.events.push(LiveEvent::ToolStarted {
                        id: str_of(b, "id").map(str::to_string).unwrap_or(id),
                        kind: classify(&name),
                        summary: summarize_input(&input, None),
                        input: pretty(&input, IO_MAX),
                        name,
                    });
                }
                _ => {}
            }
        }
    }

    fn control_request(&mut self, v: &Value, out: &mut Parsed) {
        let Some(request_id) = str_of(v, "request_id") else { return };
        let req = v.get("request").cloned().unwrap_or(Value::Null);
        if str_of(&req, "subtype") != Some("can_use_tool") {
            out.replies.push(error_response(request_id, "Not supported by MemDock"));
            return;
        }
        let tool_name = str_of(&req, "tool_name").unwrap_or("tool").to_string();
        let input = req.get("input").cloned().unwrap_or(json!({}));
        let suggestions = req.get("permission_suggestions").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut summary = summarize_input(&input, None);
        if summary.is_empty() {
            summary = str_of(&req, "description").unwrap_or("").to_string();
        }
        out.events.push(LiveEvent::ApprovalRequest {
            request_id: request_id.to_string(),
            kind: classify(&tool_name),
            summary,
            detail: pretty(&input, IO_MAX),
            can_allow_session: !suggestions.is_empty(),
            tool_name,
        });
        self.pending.insert(request_id.to_string(), Pending { input, suggestions });
    }
}

// ---- process ----------------------------------------------------------------

/// Chinese message for a process that died before `system/init`.
pub(crate) fn startup_error(stderr: &str, code: Option<i32>) -> String {
    let s = stderr.trim();
    let low = s.to_ascii_lowercase();
    if low.contains("not logged in") || low.contains("/login") || low.contains("invalid api key") || low.contains("authentication") {
        return format!("Claude Code 未登录或凭据无效，请先在终端运行 claude 完成登录。\n{}", tail(s));
    }
    if low.contains("is not recognized") || low.contains("command not found") || (low.contains("cannot find") && low.contains("node")) {
        return format!("Claude Code 无法启动（找不到命令或 Node.js），请确认已安装并在 PATH 中。\n{}", tail(s));
    }
    if low.contains("no conversation found") {
        return format!("找不到要继续的 Claude Code 会话。\n{}", tail(s));
    }
    let code = code.map(|c| c.to_string()).unwrap_or_else(|| "未知".into());
    if s.is_empty() {
        format!("Claude Code 启动后立即退出（退出码 {code}），没有输出。")
    } else {
        format!("Claude Code 启动失败（退出码 {code}）：\n{}", tail(s))
    }
}

fn tail(s: &str) -> String {
    let n = s.chars().count();
    if n <= STDERR_TAIL {
        s.to_string()
    } else {
        format!("…{}", s.chars().skip(n - STDERR_TAIL).collect::<String>())
    }
}

type SharedStdin = Arc<Mutex<Option<ChildStdin>>>;

fn write_line(stdin: &SharedStdin, v: &Value) -> Result<(), String> {
    let mut g = stdin.lock().unwrap();
    let w = g.as_mut().ok_or("Claude Code 进程已退出")?;
    let mut line = serde_json::to_string(v).map_err(|e| e.to_string())?;
    line.push('\n');
    w.write_all(line.as_bytes())
        .and_then(|_| w.flush())
        .map_err(|e| format!("Claude Code 进程已退出：{e}"))
}

struct ClaudeDriver {
    child: Arc<Mutex<Child>>,
    stdin: SharedStdin,
    parser: Arc<Mutex<Parser>>,
    stopped: Arc<AtomicBool>,
}

impl Driver for ClaudeDriver {
    fn send(&mut self, text: &str, images: &[String]) -> Result<(), String> {
        self.parser.lock().unwrap().on_send();
        write_line(&self.stdin, &user_message(text, images))
    }

    fn respond(&mut self, request_id: &str, decision: Decision) -> Result<(), String> {
        let p = self.parser.lock().unwrap().take_pending(request_id).ok_or("该授权请求已失效")?;
        write_line(&self.stdin, &permission_response(request_id, &p, decision))
    }

    fn interrupt(&mut self) -> Result<(), String> {
        let id = format!("int-{}", uuid::Uuid::new_v4());
        self.parser.lock().unwrap().on_interrupt();
        write_line(&self.stdin, &control_request(&id, "interrupt"))
    }

    fn stop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.stdin.lock().unwrap().take();
        proc::kill_tree(&mut self.child.lock().unwrap());
    }
}

/// Resolved `claude` program (exe or npm `.cmd` wrapper).
fn claude_program() -> Result<PathBuf, String> {
    find_program(OsStr::new("claude"), &path_dirs())
        .ok_or_else(|| "未找到 Claude Code 命令行（claude），请先安装并确认它在 PATH 中。".to_string())
}

/// Launches the CLI and returns its driver; the reader threads report through `emit`.
pub fn spawn(opts: &StartOptions, emit: Emit) -> Result<Box<dyn Driver>, String> {
    let program = claude_program()?;
    spawn_with(program, opts, emit)
}

pub(crate) fn spawn_with(program: PathBuf, opts: &StartOptions, emit: Emit) -> Result<Box<dyn Driver>, String> {
    let known = known_session_id(opts);
    let new_id = known.as_ref().filter(|(_, fresh)| *fresh).map(|(id, _)| id.as_str());
    let mut cmd = Command::new(&program);
    cmd.args(build_args(opts, new_id))
        .current_dir(&opts.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Started from inside a Claude Code session (dev), don't look nested.
        .env_remove("CLAUDECODE");
    if cfg!(target_os = "macos") {
        if let Ok(p) = std::env::join_paths(path_dirs()) {
            cmd.env("PATH", p);
        }
    }
    let mut child = proc::spawn_hidden(&mut cmd).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "未找到 Claude Code 命令行（claude），请先安装并确认它在 PATH 中。".to_string(),
        std::io::ErrorKind::InvalidInput => format!("附加参数无法安全传给 {}：{e}", program.display()),
        _ => format!("启动 Claude Code 失败：{e}"),
    })?;
    let stdout = child.stdout.take().ok_or("无法读取 Claude Code 输出")?;
    let stderr = child.stderr.take().ok_or("无法读取 Claude Code 输出")?;
    let stdin: SharedStdin = Arc::new(Mutex::new(child.stdin.take()));
    let child = Arc::new(Mutex::new(child));
    let mut p = Parser::default();
    // Announced before the reader starts so a fast `system/init` can't race it.
    let early = known.map(|(id, _)| p.announce(&id, opts.model.clone().filter(|m| !m.trim().is_empty())));
    let parser = Arc::new(Mutex::new(p));
    if let Some(ev) = early {
        emit(ev);
    }
    let stopped = Arc::new(AtomicBool::new(false));

    // stderr: keep everything (bounded) for startup errors.
    let err_buf = Arc::new(Mutex::new(String::new()));
    let (err_done_tx, err_done) = std::sync::mpsc::channel::<()>();
    {
        let err_buf = err_buf.clone();
        thread::spawn(move || {
            let mut r = BufReader::new(stderr);
            let mut chunk = [0u8; 4096];
            while let Ok(n) = r.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let mut b = err_buf.lock().unwrap();
                b.push_str(&String::from_utf8_lossy(&chunk[..n]));
                if b.len() > 64 * 1024 {
                    let cut = b.len() - 32 * 1024;
                    let cut = (cut..b.len()).find(|i| b.is_char_boundary(*i)).unwrap_or(b.len());
                    b.drain(..cut);
                }
            }
            let _ = err_done_tx.send(());
        });
    }

    // stdout: parse lines, emit events, answer unsupported control requests.
    {
        let (stdin, parser, child, stopped) = (stdin.clone(), parser.clone(), child.clone(), stopped.clone());
        thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match r.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let line = String::from_utf8_lossy(&buf);
                if line.trim().is_empty() {
                    continue;
                }
                let parsed = parser.lock().unwrap().line(&line);
                match parsed {
                    Some(p) => {
                        for reply in &p.replies {
                            let _ = write_line(&stdin, reply);
                        }
                        for ev in p.events {
                            emit(ev);
                        }
                    }
                    None => {
                        let mut b = err_buf.lock().unwrap();
                        b.push_str(&line);
                    }
                }
            }
            stdin.lock().unwrap().take();
            // Exit code: poll without holding the lock so `stop` can still kill.
            let deadline = Instant::now() + Duration::from_secs(3);
            let code = loop {
                match child.lock().unwrap().try_wait() {
                    Ok(Some(s)) => break s.code(),
                    Ok(None) if Instant::now() < deadline => {}
                    _ => break None,
                }
                thread::sleep(Duration::from_millis(20));
            };
            let _ = err_done.recv_timeout(Duration::from_secs(2));
            if !stopped.load(Ordering::SeqCst) {
                let started = parser.lock().unwrap().started();
                let err = err_buf.lock().unwrap().clone();
                if !started {
                    emit(LiveEvent::Error { message: startup_error(&err, code), fatal: true });
                } else if code.is_some_and(|c| c != 0) && !err.trim().is_empty() {
                    emit(LiveEvent::Error { message: format!("Claude Code 异常退出：\n{}", tail(err.trim())), fatal: false });
                }
            }
            emit(LiveEvent::Exited { code });
        });
    }

    let id = format!("init-{}", uuid::Uuid::new_v4());
    let _ = write_line(&stdin, &control_request(&id, "initialize"));
    Ok(Box::new(ClaudeDriver { child, stdin, parser, stopped }))
}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod tests;
