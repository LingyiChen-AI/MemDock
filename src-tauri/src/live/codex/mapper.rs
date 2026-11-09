//! Pure Codex app-server protocol state machine: JSON lines in, [`LiveEvent`]s
//! and JSON-RPC messages out. No IO, so it is unit-tested with fixture lines.
use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value};

use crate::convert::timeline::{classify, summarize_input, ToolKind};
use crate::live::{Decision, LiveEvent, PermissionMode, StartOptions, TurnUsage};

/// What a client request id was for.
#[derive(Debug, Clone, PartialEq)]
enum Pending {
    Initialize,
    Thread,
    Turn,
    Interrupt,
}

/// Which response shape an approval request expects.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ApprovalShape {
    /// v2 `item/*/requestApproval`: `accept` / `acceptForSession` / `decline`.
    V2,
    /// Legacy `execCommandApproval` / `applyPatchApproval`: `ReviewDecision`.
    Legacy,
}

#[derive(Debug, Clone, PartialEq)]
struct PendingApproval {
    rpc_id: Value,
    shape: ApprovalShape,
}

/// Token totals as reported by `thread/tokenUsage/updated`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Tokens {
    input: u64,
    output: u64,
}

/// Output of one step: events for the UI, messages for codex's stdin, and
/// whether the process must be killed (unrecoverable startup failure).
#[derive(Debug, Default, PartialEq)]
pub struct Step {
    pub events: Vec<LiveEvent>,
    pub writes: Vec<Value>,
    pub kill: bool,
}

pub struct Mapper {
    cwd: String,
    model: Option<String>,
    permission: PermissionMode,
    resume_id: Option<String>,
    fork_from: Option<String>,
    version: String,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    approvals: HashMap<String, PendingApproval>,
    thread_id: Option<String>,
    started: bool,
    /// Messages typed before the thread was ready.
    queued: Vec<(String, Vec<String>, PermissionMode)>,
    turn_id: Option<String>,
    /// `interrupt` arrived before the turn id was known.
    interrupt_wanted: bool,
    /// Tool items already announced with `ToolStarted`.
    tools_started: HashSet<String>,
    /// fileChange item id -> (summary, diff), for its approval request.
    file_changes: HashMap<String, (String, String)>,
    plan_seq: u64,
    tokens_total: Option<Tokens>,
    tokens_last: Option<Tokens>,
    tokens_baseline: Option<Tokens>,
    /// Last non-retry error message of the current turn (avoids repeating it).
    turn_error: Option<String>,
    /// A fatal error was already reported, or the user stopped the process.
    quiet_exit: bool,
}

/// `(approvalPolicy, sandbox)` for a permission mode (see the table in `live`).
pub fn codex_policy(mode: PermissionMode) -> (&'static str, &'static str) {
    match mode {
        // Codex no longer supports "untrusted": a read-only sandbox makes every write ask.
        PermissionMode::Ask => ("on-request", "read-only"),
        PermissionMode::AutoEdit => ("on-request", "workspace-write"),
        PermissionMode::Plan => ("on-request", "read-only"),
        PermissionMode::Bypass => ("never", "danger-full-access"),
    }
}

/// JSON-RPC ids may be numbers or strings; the UI gets a string.
pub fn id_key(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn one_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    crate::convert::truncate_chars(line, 200)
}

fn pretty(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

/// `message` plus `additionalDetails` of a `TurnError`.
fn error_text(err: &Value) -> String {
    let msg = s(err, "message");
    let details = s(err, "additionalDetails");
    match (msg.is_empty(), details.is_empty()) {
        (false, false) => format!("{msg}（{details}）"),
        (false, true) => msg.to_string(),
        (true, false) => details.to_string(),
        (true, true) => "Codex 报错".to_string(),
    }
}

fn tokens_of(v: &Value) -> Option<Tokens> {
    Some(Tokens {
        input: v.get("inputTokens")?.as_u64()?,
        output: v.get("outputTokens")?.as_u64()?,
    })
}

/// (summary, diff) of a v2 `FileUpdateChange[]`.
fn describe_changes(changes: &Value) -> (String, String, bool) {
    let mut paths = vec![];
    let mut diff = String::new();
    let mut all_add = true;
    for c in changes.as_array().into_iter().flatten() {
        let path = s(c, "path");
        paths.push(path.to_string());
        let kind = c.get("kind").map(|k| s(k, "type")).unwrap_or("");
        if kind != "add" {
            all_add = false;
        }
        if !diff.is_empty() {
            diff.push('\n');
        }
        diff.push_str(&format!("--- {path} ({kind})\n{}", s(c, "diff")));
    }
    if paths.is_empty() {
        all_add = false;
    }
    (paths.join(", "), diff, all_add)
}

/// (summary, diff) of a legacy `{path: FileChange}` map.
fn describe_legacy_changes(changes: &Value) -> (String, String) {
    let mut paths = vec![];
    let mut diff = String::new();
    if let Some(obj) = changes.as_object() {
        for (path, c) in obj {
            paths.push(path.clone());
            let kind = s(c, "type");
            let body = match kind {
                "update" => s(c, "unified_diff"),
                _ => s(c, "content"),
            };
            if !diff.is_empty() {
                diff.push('\n');
            }
            diff.push_str(&format!("--- {path} ({kind})\n{body}"));
        }
    }
    (paths.join(", "), diff)
}

/// Text of an MCP `CallToolResult` (`content[].text`).
fn mcp_output(item: &Value) -> Option<String> {
    if let Some(err) = item.get("error").filter(|e| !e.is_null()) {
        return Some(s(err, "message").to_string());
    }
    let result = item.get("result").filter(|r| !r.is_null())?;
    let texts: Vec<&str> = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.get("text").and_then(Value::as_str))
        .collect();
    if texts.is_empty() {
        Some(pretty(result))
    } else {
        Some(texts.join("\n"))
    }
}

impl Mapper {
    pub fn new(opts: &StartOptions, version: &str) -> Self {
        Self {
            cwd: opts.cwd.clone(),
            model: opts.model.clone().filter(|m| !m.trim().is_empty()),
            permission: opts.permission,
            resume_id: opts.resume_id.clone().filter(|r| !r.trim().is_empty()),
            fork_from: opts.fork_from.clone(),
            version: version.to_string(),
            next_id: 0,
            pending: HashMap::new(),
            approvals: HashMap::new(),
            thread_id: None,
            started: false,
            queued: vec![],
            turn_id: None,
            interrupt_wanted: false,
            tools_started: HashSet::new(),
            file_changes: HashMap::new(),
            plan_seq: 0,
            tokens_total: None,
            tokens_last: None,
            tokens_baseline: None,
            turn_error: None,
            quiet_exit: false,
        }
    }

    #[cfg(test)]
    pub fn started(&self) -> bool {
        self.started
    }

    /// The user stopped the process: its exit is not a startup failure.
    pub fn mark_stopped(&mut self) {
        self.quiet_exit = true;
    }

    fn request(&mut self, kind: Pending, method: &str, params: Value) -> Value {
        self.next_id += 1;
        self.pending.insert(self.next_id, kind);
        json!({"id": self.next_id, "method": method, "params": params})
    }

    /// First message: `initialize`.
    pub fn begin(&mut self) -> Vec<Value> {
        let params = json!({"clientInfo": {"name": "memdock", "title": "MemDock", "version": self.version}});
        vec![self.request(Pending::Initialize, "initialize", params)]
    }

    fn thread_request(&mut self) -> Value {
        let (approval, sandbox) = codex_policy(self.permission);
        let mut p = Map::new();
        p.insert("cwd".into(), json!(self.cwd));
        p.insert("approvalPolicy".into(), json!(approval));
        p.insert("sandbox".into(), json!(sandbox));
        if let Some(m) = &self.model {
            p.insert("model".into(), json!(m));
        }
        if let Some(id) = &self.fork_from {
            p.insert("threadId".into(), json!(id));
            p.insert("excludeTurns".into(), json!(true));
            return self.request(Pending::Thread, "thread/fork", Value::Object(p));
        }
        match self.resume_id.clone() {
            Some(id) => {
                p.insert("threadId".into(), json!(id));
                p.insert("excludeTurns".into(), json!(true));
                self.request(Pending::Thread, "thread/resume", Value::Object(p))
            }
            None => self.request(Pending::Thread, "thread/start", Value::Object(p)),
        }
    }

    fn turn_request(&mut self, thread_id: &str, text: &str, images: &[String], permission: PermissionMode) -> Value {
        // `/compact` is a Codex command, not a message: ask the app-server to compact the thread.
        if images.is_empty() && text.trim() == "/compact" {
            self.turn_error = None;
            return self.request(Pending::Turn, "thread/compact/start", json!({"threadId": thread_id}));
        }
        self.turn_error = None;
        let mut input: Vec<Value> = images.iter().map(|p| json!({"type": "localImage", "path": p})).collect();
        if !text.trim().is_empty() || input.is_empty() {
            input.push(json!({"type": "text", "text": text, "text_elements": []}));
        }
        let (approval, _) = codex_policy(permission);
        let sandbox = match permission {
            PermissionMode::Ask | PermissionMode::Plan => "readOnly",
            PermissionMode::AutoEdit => "workspaceWrite",
            PermissionMode::Bypass => "dangerFullAccess",
        };
        self.request(Pending::Turn, "turn/start", json!({
            "threadId": thread_id, "input": input, "approvalPolicy": approval,
            "sandboxPolicy": {"type": sandbox},
        }))
    }

    /// A user message; `images` are local image file paths. Queued until the thread is ready.
    pub fn send(&mut self, text: &str, images: &[String], permission: Option<PermissionMode>) -> Vec<Value> {
        let permission = permission.unwrap_or(self.permission);
        match self.thread_id.clone() {
            Some(tid) if self.started => vec![self.turn_request(&tid, text, images, permission)],
            _ => {
                self.queued.push((text.to_string(), images.to_vec(), permission));
                vec![]
            }
        }
    }

    /// Interrupts the running turn (deferred until its id is known).
    pub fn interrupt(&mut self) -> Vec<Value> {
        match (self.thread_id.clone(), self.turn_id.clone()) {
            (Some(tid), Some(turn)) => {
                vec![self.request(Pending::Interrupt, "turn/interrupt", json!({"threadId": tid, "turnId": turn}))]
            }
            _ => {
                if self.pending.values().any(|p| *p == Pending::Turn) || !self.queued.is_empty() {
                    self.interrupt_wanted = true;
                }
                vec![]
            }
        }
    }

    /// Answer to an approval request.
    pub fn respond(&mut self, request_id: &str, decision: Decision) -> Result<Value, String> {
        let a = self.approvals.remove(request_id).ok_or("审批请求不存在或已处理")?;
        let d = match (a.shape, decision) {
            (ApprovalShape::V2, Decision::Allow) => json!("accept"),
            (ApprovalShape::V2, Decision::AllowSession) => json!("acceptForSession"),
            (ApprovalShape::V2, Decision::Deny) => json!("decline"),
            (ApprovalShape::Legacy, Decision::Allow) => json!("approved"),
            (ApprovalShape::Legacy, Decision::AllowSession) => json!("approved_for_session"),
            (ApprovalShape::Legacy, Decision::Deny) => json!({"denied": {"rejection": "用户拒绝"}}),
        };
        Ok(json!({"id": a.rpc_id, "result": {"decision": d}}))
    }

    /// The process ended: what to report.
    pub fn on_exit(&self, code: Option<i32>, stderr_tail: &str) -> Vec<LiveEvent> {
        let mut v = vec![];
        if !self.started && !self.quiet_exit {
            let tail = stderr_tail.trim();
            let mut msg = format!(
                "Codex 进程在会话就绪前退出（退出码 {}）。请确认已安装 Codex CLI 并已登录（codex login）。",
                code.map(|c| c.to_string()).unwrap_or_else(|| "未知".into())
            );
            if !tail.is_empty() {
                msg.push_str("\n");
                msg.push_str(tail);
            }
            v.push(LiveEvent::Error { message: msg, fatal: true });
        }
        v.push(LiveEvent::Exited { code });
        v
    }

    /// One stdout line.
    pub fn on_line(&mut self, line: &str) -> Step {
        let mut st = Step::default();
        let line = line.trim();
        if line.is_empty() {
            return st;
        }
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            return st;
        };
        let method = msg.get("method").and_then(Value::as_str);
        let id = msg.get("id").filter(|i| !i.is_null());
        match (method, id) {
            (Some(m), Some(id)) => self.on_server_request(m, id.clone(), msg.get("params").unwrap_or(&Value::Null), &mut st),
            (Some(m), None) => self.on_notification(m, msg.get("params").unwrap_or(&Value::Null), &mut st),
            (None, Some(id)) => self.on_response(id, &msg, &mut st),
            (None, None) => {}
        }
        st
    }

    fn on_response(&mut self, id: &Value, msg: &Value, st: &mut Step) {
        let Some(kind) = id.as_u64().and_then(|n| self.pending.remove(&n)) else {
            return;
        };
        let error = msg.get("error").filter(|e| !e.is_null());
        let result = msg.get("result").unwrap_or(&Value::Null);
        match kind {
            Pending::Initialize => {
                if let Some(e) = error {
                    st.events.push(LiveEvent::Error {
                        message: format!("Codex 初始化失败：{}", s(e, "message")),
                        fatal: true,
                    });
                    st.kill = true;
                    self.quiet_exit = true;
                    return;
                }
                st.writes.push(json!({"method": "initialized"}));
                st.writes.push(self.thread_request());
            }
            Pending::Thread => {
                let tid = result.get("thread").map(|t| s(t, "id")).unwrap_or("");
                if let Some(e) = error.or_else(|| tid.is_empty().then_some(result)) {
                    let what = if self.fork_from.is_some() { "分叉" } else if self.resume_id.is_some() { "恢复" } else { "创建" };
                    let detail = if e.is_null() { "未返回会话 id".to_string() } else { s(e, "message").to_string() };
                    st.events.push(LiveEvent::Error { message: format!("Codex {what}会话失败：{detail}"), fatal: true });
                    st.kill = true;
                    self.quiet_exit = true;
                    return;
                }
                let model = result
                    .get("model")
                    .and_then(Value::as_str)
                    .or_else(|| result.get("thread").and_then(|t| t.get("model")).and_then(Value::as_str))
                    .map(str::to_string)
                    .or_else(|| self.model.clone());
                self.thread_id = Some(tid.to_string());
                self.started = true;
                st.events.push(LiveEvent::Started { native_id: tid.to_string(), model });
                for (text, images, permission) in std::mem::take(&mut self.queued) {
                    let tid = tid.to_string();
                    st.writes.push(self.turn_request(&tid, &text, &images, permission));
                }
            }
            Pending::Turn => {
                if let Some(e) = error {
                    self.interrupt_wanted = false;
                    st.events.push(LiveEvent::Error {
                        message: format!("发送消息失败：{}", s(e, "message")),
                        fatal: false,
                    });
                    st.events.push(LiveEvent::TurnCompleted { usage: TurnUsage::default(), is_error: true });
                    return;
                }
                let turn = result.get("turn").map(|t| s(t, "id")).unwrap_or("");
                if !turn.is_empty() && self.turn_id.is_none() {
                    self.turn_id = Some(turn.to_string());
                }
                self.flush_interrupt(st);
            }
            Pending::Interrupt => {
                if let Some(e) = error {
                    st.events.push(LiveEvent::Error { message: format!("中断失败：{}", s(e, "message")), fatal: false });
                }
            }
        }
    }

    fn flush_interrupt(&mut self, st: &mut Step) {
        if self.interrupt_wanted && self.turn_id.is_some() {
            self.interrupt_wanted = false;
            st.writes.extend(self.interrupt());
        }
    }

    fn on_server_request(&mut self, method: &str, id: Value, p: &Value, st: &mut Step) {
        let key = id_key(&id);
        let ev = match method {
            "item/commandExecution/requestApproval" => {
                let command = s(p, "command");
                let reason = s(p, "reason");
                let stdin = s(p, "kind") == "writeStdin";
                let mut detail = command.to_string();
                if !s(p, "cwd").is_empty() {
                    detail.push_str(&format!("\n\n目录：{}", s(p, "cwd")));
                }
                if !reason.is_empty() {
                    detail.push_str(&format!("\n原因：{reason}"));
                }
                LiveEvent::ApprovalRequest {
                    request_id: key.clone(),
                    tool_id: p.get("itemId").and_then(Value::as_str).map(str::to_owned),
                    tool_name: if stdin { "write_stdin".into() } else { "shell".into() },
                    kind: ToolKind::Shell,
                    summary: if command.is_empty() { one_line(reason) } else { one_line(command) },
                    detail: detail.trim().to_string(),
                    can_allow_session: true,
                }
            }
            "item/fileChange/requestApproval" => {
                let (summary, diff) = self.file_changes.get(s(p, "itemId")).cloned().unwrap_or_default();
                let reason = s(p, "reason");
                let mut detail = diff;
                if !reason.is_empty() {
                    detail = format!("原因：{reason}\n\n{detail}");
                }
                if let Some(root) = p.get("grantRoot").and_then(Value::as_str) {
                    detail = format!("申请写入目录：{root}\n\n{detail}");
                }
                LiveEvent::ApprovalRequest {
                    request_id: key.clone(),
                    tool_id: p.get("itemId").and_then(Value::as_str).map(str::to_owned),
                    tool_name: "apply_patch".into(),
                    kind: ToolKind::Edit,
                    summary: if summary.is_empty() { one_line(reason) } else { summary },
                    detail: detail.trim().to_string(),
                    can_allow_session: true,
                }
            }
            "execCommandApproval" => {
                let command: Vec<&str> = p
                    .get("command")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                let command = command.join(" ");
                let mut detail = format!("{command}\n\n目录：{}", s(p, "cwd"));
                if !s(p, "reason").is_empty() {
                    detail.push_str(&format!("\n原因：{}", s(p, "reason")));
                }
                LiveEvent::ApprovalRequest {
                    request_id: key.clone(),
                    tool_id: None,
                    tool_name: "shell".into(),
                    kind: ToolKind::Shell,
                    summary: one_line(&command),
                    detail,
                    can_allow_session: true,
                }
            }
            "applyPatchApproval" => {
                let (summary, diff) = describe_legacy_changes(p.get("fileChanges").unwrap_or(&Value::Null));
                LiveEvent::ApprovalRequest {
                    request_id: key.clone(),
                    tool_id: None,
                    tool_name: "apply_patch".into(),
                    kind: ToolKind::Edit,
                    summary,
                    detail: diff,
                    can_allow_session: true,
                }
            }
            _ => {
                // Unsupported (user input, MCP elicitation, permissions, auth
                // refresh...): fail it so codex does not wait forever.
                st.writes.push(json!({
                    "id": id,
                    "error": {"code": -32601, "message": format!("MemDock does not support {method}")}
                }));
                return;
            }
        };
        let shape = if method.starts_with("item/") { ApprovalShape::V2 } else { ApprovalShape::Legacy };
        self.approvals.insert(key, PendingApproval { rpc_id: id, shape });
        st.events.push(ev);
    }

    fn tool_start(&mut self, id: &str, name: &str, kind: ToolKind, summary: String, input: String, st: &mut Step) {
        if self.tools_started.insert(id.to_string()) {
            st.events.push(LiveEvent::ToolStarted {
                id: id.to_string(),
                name: name.to_string(),
                kind,
                summary,
                input,
            });
        }
    }

    /// `ToolStarted` fields of a tool-like item, `None` for other items.
    fn tool_head(&mut self, item: &Value) -> Option<(String, ToolKind, String, String)> {
        let id = s(item, "id");
        Some(match s(item, "type") {
            "commandExecution" => {
                let cmd = s(item, "command");
                ("shell".into(), ToolKind::Shell, one_line(cmd), cmd.to_string())
            }
            "fileChange" => {
                let (summary, diff, all_add) = describe_changes(item.get("changes").unwrap_or(&Value::Null));
                self.file_changes.insert(id.to_string(), (summary.clone(), diff.clone()));
                let kind = if all_add { ToolKind::Write } else { ToolKind::Edit };
                ("apply_patch".into(), kind, summary, diff)
            }
            "mcpToolCall" => {
                let args = item.get("arguments").unwrap_or(&Value::Null);
                let name = format!("mcp__{}__{}", s(item, "server"), s(item, "tool"));
                (name, ToolKind::Other, summarize_input(args, None), pretty(args))
            }
            "dynamicToolCall" => {
                let args = item.get("arguments").unwrap_or(&Value::Null);
                let tool = s(item, "tool").to_string();
                let kind = classify(&tool);
                (tool, kind, summarize_input(args, None), pretty(args))
            }
            "webSearch" => {
                let q = s(item, "query");
                ("web_search".into(), ToolKind::Web, one_line(q), q.to_string())
            }
            "collabAgentToolCall" => {
                let prompt = s(item, "prompt");
                let tool = item.get("tool").map(pretty).unwrap_or_default();
                (format!("agent:{tool}"), ToolKind::Agent, one_line(prompt), prompt.to_string())
            }
            "imageView" => {
                let path = s(item, "path");
                ("view_image".into(), ToolKind::Read, path.to_string(), path.to_string())
            }
            _ => return None,
        })
    }

    fn on_item(&mut self, item: &Value, completed: bool, st: &mut Step) {
        let id = s(item, "id").to_string();
        let ty = s(item, "type");
        match ty {
            // `/compact` runs as a turn with one contextCompaction item.
            "contextCompaction" => st.events.push(LiveEvent::Notice {
                id: format!("compact-{id}"),
                kind: if completed { "compacted" } else { "compacting" }.into(),
                text: String::new(),
                before: None,
                after: None,
            }),
            "agentMessage" if completed => {
                st.events.push(LiveEvent::AssistantMessage { id, text: s(item, "text").to_string() })
            }
            "plan" if completed => st.events.push(LiveEvent::AssistantMessage { id, text: s(item, "text").to_string() }),
            "reasoning" if completed => {
                let pick = |k: &str| -> Vec<String> {
                    item.get(k)
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .filter(|t| !t.trim().is_empty())
                        .collect()
                };
                let mut parts = pick("summary");
                if parts.is_empty() {
                    parts = pick("content");
                }
                if !parts.is_empty() {
                    st.events.push(LiveEvent::Thinking { id, text: parts.join("\n\n") });
                }
            }
            _ => {
                let Some((name, kind, summary, input)) = self.tool_head(item) else { return };
                self.tool_start(&id, &name, kind, summary, input, st);
                if completed {
                    let status = s(item, "status");
                    let (output, is_error) = match ty {
                        "commandExecution" => {
                            let out = item.get("aggregatedOutput").and_then(Value::as_str).map(str::to_string);
                            let code = item.get("exitCode").and_then(Value::as_i64);
                            let bad = matches!(status, "failed" | "declined") || code.is_some_and(|c| c != 0);
                            let out = if status == "declined" && out.as_deref().unwrap_or("").is_empty() {
                                Some("已拒绝".to_string())
                            } else {
                                out
                            };
                            (out, bad)
                        }
                        "fileChange" => {
                            let out = match status {
                                "declined" => Some("已拒绝".to_string()),
                                "failed" => Some("应用修改失败".to_string()),
                                _ => None,
                            };
                            (out, matches!(status, "failed" | "declined"))
                        }
                        "mcpToolCall" => (mcp_output(item), status == "failed"),
                        "dynamicToolCall" => {
                            let out = item.get("contentItems").filter(|c| !c.is_null()).map(pretty);
                            (out, status == "failed" || item.get("success") == Some(&Value::Bool(false)))
                        }
                        _ => (None, status == "failed"),
                    };
                    self.file_changes.remove(&id);
                    st.events.push(LiveEvent::ToolFinished { id, output, is_error });
                }
            }
        }
    }

    fn on_notification(&mut self, method: &str, p: &Value, st: &mut Step) {
        match method {
            "item/started" | "item/completed" => {
                if let Some(item) = p.get("item") {
                    self.on_item(item, method == "item/completed", st);
                }
            }
            "item/agentMessage/delta" => st.events.push(LiveEvent::AssistantDelta {
                id: s(p, "itemId").to_string(),
                text: s(p, "delta").to_string(),
            }),
            "item/commandExecution/outputDelta" | "item/fileChange/outputDelta" => {
                st.events.push(LiveEvent::ToolOutput { id: s(p, "itemId").to_string(), text: s(p, "delta").to_string() })
            }
            "turn/started" => {
                if let Some(t) = p.get("turn") {
                    let id = s(t, "id");
                    if !id.is_empty() {
                        self.turn_id = Some(id.to_string());
                    }
                }
                self.tokens_baseline = self.tokens_total;
                self.flush_interrupt(st);
            }
            "turn/plan/updated" => {
                let steps: Vec<String> = p
                    .get("plan")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|step| {
                        let mark = match s(step, "status") {
                            "completed" => "[x]",
                            "inProgress" => "[~]",
                            _ => "[ ]",
                        };
                        format!("{mark} {}", s(step, "step"))
                    })
                    .collect();
                self.plan_seq += 1;
                let id = format!("plan-{}-{}", s(p, "turnId"), self.plan_seq);
                let explanation = s(p, "explanation");
                let summary = if explanation.is_empty() { format!("{} 项", steps.len()) } else { one_line(explanation) };
                self.tool_start(&id, "update_plan", ToolKind::Todo, summary, steps.join("\n"), st);
                st.events.push(LiveEvent::ToolFinished { id, output: None, is_error: false });
            }
            "thread/tokenUsage/updated" => {
                if let Some(u) = p.get("tokenUsage") {
                    self.tokens_total = u.get("total").and_then(tokens_of);
                    self.tokens_last = u.get("last").and_then(tokens_of);
                }
            }
            "error" => {
                let text = error_text(p.get("error").unwrap_or(&Value::Null));
                let retry = p.get("willRetry").and_then(Value::as_bool).unwrap_or(false);
                if !retry {
                    self.turn_error = Some(text.clone());
                }
                st.events.push(LiveEvent::Error { message: text, fatal: false });
            }
            "turn/completed" => {
                let turn = p.get("turn").unwrap_or(&Value::Null);
                let status = s(turn, "status");
                if status == "failed" {
                    if let Some(e) = turn.get("error").filter(|e| !e.is_null()) {
                        let text = error_text(e);
                        if self.turn_error.as_deref() != Some(text.as_str()) {
                            st.events.push(LiveEvent::Error { message: text, fatal: false });
                        }
                    }
                }
                let usage = self.turn_usage(turn.get("durationMs").and_then(Value::as_u64));
                st.events.push(LiveEvent::TurnCompleted {
                    usage,
                    is_error: status == "failed" || (status != "interrupted" && self.turn_error.is_some()),
                });
                self.turn_id = None;
                self.turn_error = None;
                self.interrupt_wanted = false;
            }
            "serverRequest/resolved" => {
                // Codex resolved a request itself (turn ended / interrupted).
                if let Some(id) = p.get("requestId") {
                    self.approvals.remove(&id_key(id));
                }
            }
            _ => {}
        }
    }

    fn turn_usage(&mut self, duration_ms: Option<u64>) -> TurnUsage {
        let tokens = match (self.tokens_total, self.tokens_baseline) {
            (Some(total), Some(base)) => Some(Tokens {
                input: total.input.saturating_sub(base.input),
                output: total.output.saturating_sub(base.output),
            }),
            // First turn of a resumed thread: no baseline, use the last call.
            (Some(total), None) if self.resume_id.is_none() => Some(total),
            _ => self.tokens_last,
        };
        self.tokens_baseline = self.tokens_total;
        TurnUsage {
            input_tokens: tokens.map(|t| t.input),
            output_tokens: tokens.map(|t| t.output),
            cost_usd: None,
            duration_ms,
        }
    }
}
