use super::mapper::{codex_policy, Mapper, Step};
use crate::convert::timeline::ToolKind;
use crate::live::{Agent, Decision, LiveEvent, PermissionMode, StartOptions, TurnUsage};
use serde_json::{json, Value};

#[test]
fn app_server_args_pass_effort_first() {
    let mut o = opts(PermissionMode::Ask, None);
    assert_eq!(super::app_server_args(&o, false).unwrap(), vec!["app-server"]);
    o.effort = Some("xhigh".into());
    o.extra_args = vec!["model_verbosity=low".into()];
    assert_eq!(
        super::app_server_args(&o, true).unwrap().join(" "),
        "app-server -c model_reasoning_effort=xhigh -c model_verbosity=low"
    );
    o.extra_args = vec!["x=%PATH%".into()];
    assert!(super::app_server_args(&o, true).is_err());
}

fn opts(permission: PermissionMode, resume: Option<&str>) -> StartOptions {
    StartOptions {
        agent: Agent::Codex,
        cwd: "C:/proj".into(),
        model: None,
        effort: None,
        permission,
        resume_id: resume.map(str::to_string),
        extra_args: vec![],
        prompt: None,
        prompt_images: vec![],
    }
}

fn line(m: &mut Mapper, v: Value) -> Step {
    m.on_line(&v.to_string())
}

/// Mapper past `initialize` and `thread/start`, thread `T1`.
fn ready() -> Mapper {
    let mut m = Mapper::new(&opts(PermissionMode::Ask, None), "0.1.0");
    m.begin();
    line(&mut m, json!({"id": 1, "result": {"userAgent": "x"}}));
    let st = line(&mut m, json!({"id": 2, "result": {"thread": {"id": "T1", "model": "gpt-x"}, "model": "gpt-x"}}));
    assert_eq!(st.events, vec![LiveEvent::Started { native_id: "T1".into(), model: Some("gpt-x".into()) }]);
    m
}

#[test]
fn policy_table() {
    assert_eq!(codex_policy(PermissionMode::Ask), ("on-request", "read-only"));
    assert_eq!(codex_policy(PermissionMode::AutoEdit), ("on-request", "workspace-write"));
    assert_eq!(codex_policy(PermissionMode::Plan), ("on-request", "read-only"));
    assert_eq!(codex_policy(PermissionMode::Bypass), ("never", "danger-full-access"));
}

#[test]
fn handshake_and_queued_prompt() {
    let mut m = Mapper::new(&opts(PermissionMode::Plan, None), "0.1.0");
    let init = m.begin();
    assert_eq!(init[0]["method"], "initialize");
    assert_eq!(init[0]["params"]["clientInfo"]["name"], "memdock");
    // Prompt typed before the thread exists is queued.
    assert!(m.send("hello", &[]).is_empty());
    let st = line(&mut m, json!({"id": 1, "result": {}}));
    assert_eq!(st.writes[0], json!({"method": "initialized"}));
    assert_eq!(st.writes[1]["method"], "thread/start");
    assert_eq!(st.writes[1]["params"]["approvalPolicy"], "on-request");
    assert_eq!(st.writes[1]["params"]["sandbox"], "read-only");
    assert_eq!(st.writes[1]["params"]["cwd"], "C:/proj");
    // Unknown notifications are ignored.
    let st = line(&mut m, json!({"method": "mcpServer/startupStatus/updated", "params": {"name": "x"}}));
    assert_eq!(st, Step::default());
    let st = line(&mut m, json!({"id": 2, "result": {"thread": {"id": "T9", "model": "m1"}}}));
    assert_eq!(st.events, vec![LiveEvent::Started { native_id: "T9".into(), model: Some("m1".into()) }]);
    assert_eq!(st.writes.len(), 1);
    assert_eq!(st.writes[0]["method"], "turn/start");
    assert_eq!(st.writes[0]["params"], json!({"threadId": "T9", "input": [{"type": "text", "text": "hello", "text_elements": []}]}));
}

#[test]
fn resume_uses_thread_resume() {
    let mut o = opts(PermissionMode::Bypass, Some("OLD"));
    o.model = Some("gpt-5".into());
    let mut m = Mapper::new(&o, "0.1.0");
    m.begin();
    let st = line(&mut m, json!({"id": 1, "result": {}}));
    let p = &st.writes[1];
    assert_eq!(p["method"], "thread/resume");
    assert_eq!(p["params"]["threadId"], "OLD");
    assert_eq!(p["params"]["model"], "gpt-5");
    assert_eq!(p["params"]["approvalPolicy"], "never");
    assert_eq!(p["params"]["sandbox"], "danger-full-access");
}

#[test]
fn thread_start_failure_is_fatal_and_quiet_on_exit() {
    let mut m = Mapper::new(&opts(PermissionMode::Ask, None), "0.1.0");
    m.begin();
    line(&mut m, json!({"id": 1, "result": {}}));
    let st = line(&mut m, json!({"id": 2, "error": {"code": -1, "message": "no auth"}}));
    assert!(st.kill);
    assert!(matches!(&st.events[0], LiveEvent::Error { fatal: true, message } if message.contains("no auth")));
    assert_eq!(m.on_exit(Some(1), ""), vec![LiveEvent::Exited { code: Some(1) }]);
}

#[test]
fn exit_before_start_reports_stderr() {
    let m = Mapper::new(&opts(PermissionMode::Ask, None), "0.1.0");
    let ev = m.on_exit(Some(2), "boom: not logged in");
    assert!(matches!(&ev[0], LiveEvent::Error { fatal: true, message } if message.contains("boom")));
    assert_eq!(ev[1], LiveEvent::Exited { code: Some(2) });
}

#[test]
fn turn_stream_messages_and_usage() {
    let mut m = ready();
    let w = m.send("hi", &[]);
    assert_eq!(w[0]["id"], 3);
    line(&mut m, json!({"id": 3, "result": {"turn": {"id": "U1", "status": "inProgress"}}}));
    line(&mut m, json!({"method": "turn/started", "params": {"threadId": "T1", "turn": {"id": "U1"}}}));
    // The user message item is not repeated (the manager emits it).
    let st = line(&mut m, json!({"method": "item/started", "params": {"item": {"type": "userMessage", "id": "u", "content": []}}}));
    assert!(st.events.is_empty());
    let st = line(&mut m, json!({"method": "item/agentMessage/delta", "params": {"itemId": "a1", "delta": "He"}}));
    assert_eq!(st.events, vec![LiveEvent::AssistantDelta { id: "a1".into(), text: "He".into() }]);
    let st = line(&mut m, json!({"method": "item/completed", "params": {"item": {"type": "agentMessage", "id": "a1", "text": "Hello"}}}));
    assert_eq!(st.events, vec![LiveEvent::AssistantMessage { id: "a1".into(), text: "Hello".into() }]);
    let st = line(&mut m, json!({"method": "item/completed", "params": {"item": {"type": "reasoning", "id": "r1", "summary": ["think"], "content": []}}}));
    assert_eq!(st.events, vec![LiveEvent::Thinking { id: "r1".into(), text: "think".into() }]);
    let usage = |i: u64, o: u64| json!({"totalTokens": i + o, "inputTokens": i, "cachedInputTokens": 0, "cacheWriteInputTokens": 0, "outputTokens": o, "reasoningOutputTokens": 0});
    line(&mut m, json!({"method": "thread/tokenUsage/updated", "params": {"threadId": "T1", "turnId": "U1", "tokenUsage": {"total": usage(100, 20), "last": usage(100, 20)}}}));
    let st = line(&mut m, json!({"method": "turn/completed", "params": {"threadId": "T1", "turn": {"id": "U1", "status": "completed", "error": null, "durationMs": 1500}}}));
    assert_eq!(
        st.events,
        vec![LiveEvent::TurnCompleted {
            usage: TurnUsage { input_tokens: Some(100), output_tokens: Some(20), cost_usd: None, duration_ms: Some(1500) },
            is_error: false
        }]
    );
    // Second turn reports the difference of the totals.
    m.send("again", &[]);
    line(&mut m, json!({"method": "turn/started", "params": {"threadId": "T1", "turn": {"id": "U2"}}}));
    line(&mut m, json!({"method": "thread/tokenUsage/updated", "params": {"tokenUsage": {"total": usage(250, 30), "last": usage(150, 10)}}}));
    let st = line(&mut m, json!({"method": "turn/completed", "params": {"turn": {"id": "U2", "status": "completed"}}}));
    assert!(matches!(&st.events[0], LiveEvent::TurnCompleted { usage, .. } if usage.input_tokens == Some(150) && usage.output_tokens == Some(10)));
}

#[test]
fn command_execution_and_approval() {
    let mut m = ready();
    let st = line(&mut m, json!({"method": "item/started", "params": {"item": {
        "type": "commandExecution", "id": "c1", "command": "echo hi > hello.txt", "cwd": "C:/proj",
        "status": "inProgress", "commandActions": [], "aggregatedOutput": null, "exitCode": null}}}));
    assert_eq!(
        st.events,
        vec![LiveEvent::ToolStarted {
            id: "c1".into(),
            name: "shell".into(),
            kind: ToolKind::Shell,
            summary: "echo hi > hello.txt".into(),
            input: "echo hi > hello.txt".into()
        }]
    );
    let st = line(&mut m, json!({"id": 0, "method": "item/commandExecution/requestApproval", "params": {
        "kind": "command", "threadId": "T1", "turnId": "U1", "itemId": "c1", "startedAtMs": 1,
        "command": "echo hi > hello.txt", "cwd": "C:/proj", "reason": null}}));
    match &st.events[0] {
        LiveEvent::ApprovalRequest { request_id, kind, summary, can_allow_session, .. } => {
            assert_eq!(request_id, "0");
            assert_eq!(*kind, ToolKind::Shell);
            assert_eq!(summary, "echo hi > hello.txt");
            assert!(can_allow_session);
        }
        e => panic!("{e:?}"),
    }
    assert_eq!(m.respond("0", Decision::AllowSession).unwrap(), json!({"id": 0, "result": {"decision": "acceptForSession"}}));
    assert!(m.respond("0", Decision::Allow).is_err());
    let st = line(&mut m, json!({"method": "item/commandExecution/outputDelta", "params": {"itemId": "c1", "delta": "ok\n"}}));
    assert_eq!(st.events, vec![LiveEvent::ToolOutput { id: "c1".into(), text: "ok\n".into() }]);
    let st = line(&mut m, json!({"method": "item/completed", "params": {"item": {
        "type": "commandExecution", "id": "c1", "command": "x", "status": "failed", "aggregatedOutput": "denied", "exitCode": 1}}}));
    assert_eq!(st.events, vec![LiveEvent::ToolFinished { id: "c1".into(), output: Some("denied".into()), is_error: true }]);
}

#[test]
fn file_change_approval_uses_item_diff() {
    let mut m = ready();
    let st = line(&mut m, json!({"method": "item/started", "params": {"item": {
        "type": "fileChange", "id": "f1", "status": "inProgress",
        "changes": [{"path": "a.txt", "kind": {"type": "add"}, "diff": "+hi"}]}}}));
    assert!(matches!(&st.events[0], LiveEvent::ToolStarted { kind: ToolKind::Write, summary, .. } if summary == "a.txt"));
    let st = line(&mut m, json!({"id": "req-7", "method": "item/fileChange/requestApproval", "params": {
        "threadId": "T1", "turnId": "U1", "itemId": "f1", "startedAtMs": 1}}));
    match &st.events[0] {
        LiveEvent::ApprovalRequest { request_id, kind, summary, detail, .. } => {
            assert_eq!(request_id, "req-7");
            assert_eq!(*kind, ToolKind::Edit);
            assert_eq!(summary, "a.txt");
            assert!(detail.contains("+hi"));
        }
        e => panic!("{e:?}"),
    }
    assert_eq!(m.respond("req-7", Decision::Deny).unwrap(), json!({"id": "req-7", "result": {"decision": "decline"}}));
    let st = line(&mut m, json!({"method": "item/completed", "params": {"item": {
        "type": "fileChange", "id": "f1", "status": "declined", "changes": []}}}));
    assert_eq!(st.events, vec![LiveEvent::ToolFinished { id: "f1".into(), output: Some("已拒绝".into()), is_error: true }]);
}

#[test]
fn legacy_approvals_and_unsupported_requests() {
    let mut m = ready();
    let st = line(&mut m, json!({"id": 5, "method": "execCommandApproval", "params": {
        "conversationId": "T1", "callId": "c", "approvalId": null, "command": ["git", "status"], "cwd": "C:/proj", "reason": null, "parsedCmd": []}}));
    assert!(matches!(&st.events[0], LiveEvent::ApprovalRequest { summary, .. } if summary == "git status"));
    assert_eq!(m.respond("5", Decision::Allow).unwrap(), json!({"id": 5, "result": {"decision": "approved"}}));
    line(&mut m, json!({"id": 6, "method": "applyPatchApproval", "params": {
        "conversationId": "T1", "callId": "c", "fileChanges": {"b.rs": {"type": "update", "unified_diff": "@@", "move_path": null}}, "reason": null, "grantRoot": null}}));
    assert_eq!(
        m.respond("6", Decision::Deny).unwrap(),
        json!({"id": 6, "result": {"decision": {"denied": {"rejection": "用户拒绝"}}}})
    );
    let st = line(&mut m, json!({"id": 9, "method": "item/tool/requestUserInput", "params": {}}));
    assert!(st.events.is_empty());
    assert_eq!(st.writes[0]["id"], 9);
    assert!(st.writes[0]["error"]["message"].as_str().unwrap().contains("requestUserInput"));
}

#[test]
fn network_errors_are_non_fatal_and_end_the_turn() {
    let mut m = ready();
    m.send("hi", &[]);
    line(&mut m, json!({"id": 3, "result": {"turn": {"id": "U1"}}}));
    let retry = json!({"method": "error", "params": {"error": {"message": "Reconnecting... 2/5",
        "additionalDetails": "stream disconnected before completion: (os error 10061)"}, "willRetry": true, "threadId": "T1", "turnId": "U1"}});
    let st = line(&mut m, retry);
    assert!(matches!(&st.events[0], LiveEvent::Error { fatal: false, message } if message.contains("10061")));
    let st = line(&mut m, json!({"method": "error", "params": {"error": {"message": "stream failed", "additionalDetails": null}, "willRetry": false}}));
    assert_eq!(st.events, vec![LiveEvent::Error { message: "stream failed".into(), fatal: false }]);
    let st = line(&mut m, json!({"method": "turn/completed", "params": {"turn": {"id": "U1", "status": "failed",
        "error": {"message": "stream failed", "additionalDetails": null}}}}));
    // The same error is not repeated; the turn ends as failed.
    assert_eq!(st.events.len(), 1);
    assert!(matches!(&st.events[0], LiveEvent::TurnCompleted { is_error: true, .. }));
}

#[test]
fn turn_start_rpc_error_ends_turn() {
    let mut m = ready();
    m.send("hi", &[]);
    let st = line(&mut m, json!({"id": 3, "error": {"code": -32600, "message": "bad"}}));
    assert!(matches!(&st.events[0], LiveEvent::Error { fatal: false, .. }));
    assert!(matches!(&st.events[1], LiveEvent::TurnCompleted { is_error: true, .. }));
}

#[test]
fn interrupt_waits_for_turn_id() {
    let mut m = ready();
    m.send("hi", &[]);
    assert!(m.interrupt().is_empty());
    let st = line(&mut m, json!({"id": 3, "result": {"turn": {"id": "U1"}}}));
    assert_eq!(st.writes[0]["method"], "turn/interrupt");
    assert_eq!(st.writes[0]["params"], json!({"threadId": "T1", "turnId": "U1"}));
    let st = line(&mut m, json!({"method": "turn/completed", "params": {"turn": {"id": "U1", "status": "interrupted"}}}));
    assert!(matches!(&st.events[0], LiveEvent::TurnCompleted { is_error: false, .. }));
    // No turn running: nothing to send.
    assert!(m.interrupt().is_empty());
}

#[test]
fn mcp_web_search_and_plan_updates() {
    let mut m = ready();
    let st = line(&mut m, json!({"method": "item/completed", "params": {"item": {
        "type": "mcpToolCall", "id": "m1", "server": "fs", "tool": "read", "status": "completed",
        "arguments": {"path": "x.md"}, "result": {"content": [{"type": "text", "text": "body"}], "structuredContent": null}, "error": null}}}));
    assert!(matches!(&st.events[0], LiveEvent::ToolStarted { name, summary, .. } if name == "mcp__fs__read" && summary == "x.md"));
    assert_eq!(st.events[1], LiveEvent::ToolFinished { id: "m1".into(), output: Some("body".into()), is_error: false });
    let st = line(&mut m, json!({"method": "item/started", "params": {"item": {"type": "webSearch", "id": "w1", "query": "rust"}}}));
    assert!(matches!(&st.events[0], LiveEvent::ToolStarted { kind: ToolKind::Web, .. }));
    let st = line(&mut m, json!({"method": "turn/plan/updated", "params": {"turnId": "U1", "explanation": null,
        "plan": [{"step": "a", "status": "completed"}, {"step": "b", "status": "pending"}]}}));
    match &st.events[0] {
        LiveEvent::ToolStarted { kind, input, .. } => {
            assert_eq!(*kind, ToolKind::Todo);
            assert_eq!(input, "[x] a\n[ ] b");
        }
        e => panic!("{e:?}"),
    }
    assert!(matches!(&st.events[1], LiveEvent::ToolFinished { .. }));
}

#[test]
fn probe_log_replays_without_fatal_errors() {
    // Lines from a real `codex app-server` run (0.166.0) whose network was down.
    let lines = [
        r#"{"id":1,"result":{"userAgent":"memdock/0.166.0","codexHome":"C:\\Users\\x\\.codex","platformFamily":"windows","platformOs":"windows"}}"#,
        r#"{"method":"remoteControl/status/changed","params":{"status":"disabled"},"emittedAtMs":1}"#,
        r#"{"method":"account/updated","params":{"authMode":"chatgpt","planType":"pro"},"emittedAtMs":1}"#,
        r#"{"id":2,"result":{"thread":{"id":"01a1b035","model":"gpt-6.1-sol"},"model":"gpt-6.1-sol","approvalPolicy":"untrusted"}}"#,
        r#"{"method":"thread/started","params":{"thread":{"id":"01a1b035"}}}"#,
        r#"{"id":3,"result":{"turn":{"id":"01a1b036","items":[],"status":"inProgress","error":null}}}"#,
        r#"{"method":"turn/started","params":{"threadId":"01a1b035","turn":{"id":"01a1b036","status":"inProgress"}}}"#,
        r#"{"method":"error","params":{"error":{"message":"Reconnecting... 2/5","codexErrorInfo":{"responseStreamDisconnected":{"httpStatusCode":null}},"additionalDetails":"stream disconnected before completion: 由于目标计算机积极拒绝，无法连接。 (os error 10061)","misalignment":null},"willRetry":true,"threadId":"01a1b035","turnId":"01a1b036"},"emittedAtMs":1}"#,
        r#"{"method":"warning","params":{"threadId":"01a1b035","message":"Falling back from WebSockets to HTTPS transport."},"emittedAtMs":1}"#,
    ];
    let mut m = Mapper::new(&opts(PermissionMode::Ask, None), "0.1.0");
    m.begin();
    let mut events = vec![];
    for (i, l) in lines.iter().enumerate() {
        if i == 5 {
            m.send("Run echo", &[]);
        }
        events.extend(m.on_line(l).events);
    }
    assert_eq!(events[0], LiveEvent::Started { native_id: "01a1b035".into(), model: Some("gpt-6.1-sol".into()) });
    assert!(events.iter().all(|e| !matches!(e, LiveEvent::Error { fatal: true, .. })));
    assert_eq!(events.iter().filter(|e| matches!(e, LiveEvent::Error { .. })).count(), 1);
    assert!(m.started());
}

/// Real `codex app-server` up to `Started`, then one turn. Run manually:
/// `cargo test --lib live::codex::tests::real_codex -- --ignored --nocapture`
#[test]
#[ignore]
fn real_codex() {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    let dir = std::env::temp_dir().join(format!("memdock-codex-live-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let events: Arc<Mutex<Vec<LiveEvent>>> = Arc::default();
    let sink = events.clone();
    let emit: crate::live::Emit = Arc::new(move |e| {
        println!("{e:?}");
        sink.lock().unwrap().push(e)
    });
    let mut o = opts(PermissionMode::Ask, None);
    o.cwd = dir.to_string_lossy().into_owned();
    let mut d = super::spawn(&o, emit).expect("spawn");
    d.send("Reply with the single word: pong", &[]).unwrap();
    let wait = |pred: &dyn Fn(&LiveEvent) -> bool, secs: u64| {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(secs) {
            if events.lock().unwrap().iter().any(|e| pred(e)) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        false
    };
    assert!(wait(&|e| matches!(e, LiveEvent::Started { .. }), 60), "no Started");
    let done = wait(&|e| matches!(e, LiveEvent::TurnCompleted { .. }), 45);
    if !done {
        d.interrupt().unwrap();
        println!("interrupt sent: {}", wait(&|e| matches!(e, LiveEvent::TurnCompleted { .. }), 20));
    }
    d.stop();
    assert!(wait(&|e| matches!(e, LiveEvent::Exited { .. }), 15), "no Exited");
    assert!(events.lock().unwrap().iter().all(|e| !matches!(e, LiveEvent::Error { fatal: true, .. })));
}
