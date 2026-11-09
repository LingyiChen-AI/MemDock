use super::*;
use crate::convert::timeline::ToolKind;
use crate::live::Agent;

// Trimmed lines from a real Claude Code 2.1.318 recording (Write tool + approval).
const INIT: &str = r#"{"type":"system","subtype":"init","cwd":"C:\\tmp\\p","session_id":"462f7148-74ee-4bd9-96da-1dfd5adddaaa","tools":["Write"],"model":"claude-haiku-5-5","permissionMode":"default","claude_code_version":"2.1.318","uuid":"u1"}"#;
const HOOK: &str = r#"{"type":"system","subtype":"hook_started","hook_id":"h","hook_name":"SessionStart:startup","session_id":"462f7148-74ee-4bd9-96da-1dfd5adddaaa"}"#;
const INIT_RESPONSE: &str = r#"{"type":"control_response","response":{"subtype":"success","request_id":"init1","response":{"commands":[]}}}"#;
const MSG1_START: &str = r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"claude-haiku-5-5","id":"msg_A","type":"message","role":"assistant","content":[]}},"session_id":"s","parent_tool_use_id":null}"#;
const THINK_DELTA: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":""}},"session_id":"s","parent_tool_use_id":null,"api_message_id":"msg_A"}"#;
const THINK_FULL: &str = r#"{"type":"assistant","message":{"model":"claude-haiku-5-5","id":"msg_A","type":"message","role":"assistant","content":[{"type":"thinking","thinking":"","signature":"CAQS"}]},"parent_tool_use_id":null,"session_id":"s"}"#;
const TOOL_INPUT_DELTA: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"file_path\": \"C:\\\\"}},"session_id":"s","parent_tool_use_id":null,"api_message_id":"msg_A"}"#;
const TOOL_FULL: &str = r#"{"type":"assistant","message":{"model":"claude-haiku-5-5","id":"msg_A","type":"message","role":"assistant","content":[{"type":"tool_use","id":"toolu_01","name":"Write","input":{"file_path":"C:\\tmp\\p\\hello.txt","content":"hi"},"caller":{"type":"direct"}}]},"parent_tool_use_id":null,"session_id":"s"}"#;
const CAN_USE: &str = r#"{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Write","display_name":"Write","input":{"file_path":"C:\\tmp\\p\\hello.txt","content":"hi"},"description":"hello.txt","permission_suggestions":[{"type":"setMode","mode":"acceptEdits","destination":"session"}],"tool_use_id":"toolu_01"}}"#;
const RATE: &str = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"},"session_id":"s"}"#;
const TOOL_RESULT: &str = r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_01","type":"tool_result","content":"File created successfully at: C:\\tmp\\p\\hello.txt"}]},"parent_tool_use_id":null,"session_id":"s","tool_use_result":{"type":"create"}}"#;
const MSG2_START: &str = r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"claude-haiku-5-5","id":"msg_B","type":"message","role":"assistant","content":[]}},"session_id":"s","parent_tool_use_id":null}"#;
const TEXT_DELTA1: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Created "}},"session_id":"s","parent_tool_use_id":null,"api_message_id":"msg_B"}"#;
const TEXT_DELTA2: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello.txt. Done."}},"session_id":"s","parent_tool_use_id":null,"api_message_id":"msg_B"}"#;
const TEXT_FULL: &str = r#"{"type":"assistant","message":{"model":"claude-haiku-5-5","id":"msg_B","type":"message","role":"assistant","content":[{"type":"text","text":"Created hello.txt. Done."}]},"parent_tool_use_id":null,"session_id":"s"}"#;
const RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":4187,"duration_api_ms":3821,"num_turns":2,"result":"Created hello.txt. Done.","stop_reason":"end_turn","session_id":"s","total_cost_usd":0.0066,"usage":{"input_tokens":4,"cache_creation_input_tokens":29281,"cache_read_input_tokens":63963,"output_tokens":235}}"#;

fn events(p: &mut Parser, line: &str) -> Vec<LiveEvent> {
    p.line(line).expect("json").events
}

#[test]
fn recording_maps_to_events() {
    let mut p = Parser::default();
    for l in [HOOK, INIT_RESPONSE, RATE, MSG1_START, THINK_DELTA, THINK_FULL, TOOL_INPUT_DELTA] {
        assert!(events(&mut p, l).is_empty(), "{l}");
    }
    assert!(!p.started());
    assert_eq!(
        events(&mut p, INIT),
        vec![LiveEvent::Started {
            native_id: "462f7148-74ee-4bd9-96da-1dfd5adddaaa".into(),
            model: Some("claude-haiku-5-5".into())
        }]
    );
    assert!(p.started());
    // A repeated init for the same session is not reported again.
    assert!(events(&mut p, INIT).is_empty());

    let ev = events(&mut p, TOOL_FULL);
    let [LiveEvent::ToolStarted { id, name, kind, summary, input }] = ev.as_slice() else { panic!("{ev:?}") };
    assert_eq!((id.as_str(), name.as_str(), *kind), ("toolu_01", "Write", ToolKind::Write));
    assert_eq!(summary, "C:\\tmp\\p\\hello.txt");
    assert!(input.contains("\"content\": \"hi\""));

    let ev = events(&mut p, CAN_USE);
    let [LiveEvent::ApprovalRequest { request_id, tool_name, kind, summary, detail, can_allow_session, .. }] = ev.as_slice()
    else {
        panic!("{ev:?}")
    };
    assert_eq!((request_id.as_str(), tool_name.as_str(), *kind), ("req-1", "Write", ToolKind::Write));
    assert_eq!(summary, "C:\\tmp\\p\\hello.txt");
    assert!(detail.contains("hello.txt"));
    assert!(*can_allow_session);

    assert_eq!(
        events(&mut p, TOOL_RESULT),
        vec![LiveEvent::ToolFinished {
            id: "toolu_01".into(),
            output: Some("File created successfully at: C:\\tmp\\p\\hello.txt".into()),
            is_error: false
        }]
    );

    assert!(events(&mut p, MSG2_START).is_empty());
    assert_eq!(
        events(&mut p, TEXT_DELTA1),
        vec![LiveEvent::AssistantDelta { id: "msg_B:0".into(), text: "Created ".into() }]
    );
    assert_eq!(events(&mut p, TEXT_DELTA2).len(), 1);
    // The final message has the same id as its deltas.
    assert_eq!(
        events(&mut p, TEXT_FULL),
        vec![LiveEvent::AssistantMessage { id: "msg_B:0".into(), text: "Created hello.txt. Done.".into() }]
    );
    assert_eq!(
        events(&mut p, RESULT),
        vec![LiveEvent::TurnCompleted {
            usage: TurnUsage {
                input_tokens: Some(4 + 29281 + 63963),
                output_tokens: Some(235),
                cost_usd: Some(0.0066),
                duration_ms: Some(4187)
            },
            is_error: false
        }]
    );
}

#[test]
fn block_index_counts_across_assistant_lines() {
    let mut p = Parser::default();
    events(&mut p, THINK_FULL); // block 0 of msg_A (empty thinking, not shown)
    let text = r#"{"type":"assistant","message":{"id":"msg_A","content":[{"type":"text","text":"hi"}]},"parent_tool_use_id":null}"#;
    assert_eq!(events(&mut p, text), vec![LiveEvent::AssistantMessage { id: "msg_A:1".into(), text: "hi".into() }]);
    let think = r#"{"type":"assistant","message":{"id":"msg_C","content":[{"type":"thinking","thinking":"plan it"}]},"parent_tool_use_id":null}"#;
    assert_eq!(events(&mut p, think), vec![LiveEvent::Thinking { id: "msg_C:0".into(), text: "plan it".into() }]);
}

#[test]
fn subagent_lines_and_non_json_are_skipped() {
    let mut p = Parser::default();
    let nested = r#"{"type":"assistant","message":{"id":"msg_X","content":[{"type":"text","text":"inner"}]},"parent_tool_use_id":"toolu_task"}"#;
    assert!(events(&mut p, nested).is_empty());
    assert!(p.line("Error: something went wrong").is_none());
}

#[test]
fn permission_answers() {
    let mut p = Parser::default();
    events(&mut p, CAN_USE);
    let pend = p.take_pending("req-1").unwrap();
    assert!(p.take_pending("req-1").is_none());

    let allow = permission_response("req-1", &pend, Decision::Allow);
    assert_eq!(allow["type"], "control_response");
    assert_eq!(allow["response"]["subtype"], "success");
    assert_eq!(allow["response"]["request_id"], "req-1");
    assert_eq!(allow["response"]["response"]["behavior"], "allow");
    assert_eq!(allow["response"]["response"]["updatedInput"]["content"], "hi");
    assert!(allow["response"]["response"].get("updatedPermissions").is_none());

    let sess = permission_response("req-1", &pend, Decision::AllowSession);
    assert_eq!(sess["response"]["response"]["updatedPermissions"][0]["mode"], "acceptEdits");

    let deny = permission_response("req-1", &pend, Decision::Deny);
    assert_eq!(deny["response"]["response"]["behavior"], "deny");
    assert_eq!(deny["response"]["response"]["message"], DENY_MESSAGE);
}

#[test]
fn unsupported_control_request_gets_error_reply() {
    let mut p = Parser::default();
    let r = p.line(r#"{"type":"control_request","request_id":"x1","request":{"subtype":"hook_callback"}}"#).unwrap();
    assert!(r.events.is_empty());
    assert_eq!(r.replies.len(), 1);
    assert_eq!(r.replies[0]["response"]["subtype"], "error");
    assert_eq!(r.replies[0]["response"]["request_id"], "x1");
}

#[test]
fn error_result_reports_message() {
    let mut p = Parser::default();
    let ev = events(&mut p, r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login","usage":{}}"#);
    assert_eq!(ev[0], LiveEvent::Error { message: "Not logged in · Please run /login".into(), fatal: false });
    assert!(matches!(ev[1], LiveEvent::TurnCompleted { is_error: true, .. }));
}

fn opts(permission: PermissionMode) -> StartOptions {
    StartOptions {
        agent: Agent::Claude,
        cwd: ".".into(),
        model: Some("haiku".into()),
        effort: None,
        permission,
        resume_id: Some("sid".into()),
        fork_from: None,
        extra_args: vec!["--add-dir".into(), "x".into()],
        prompt: None,
        prompt_images: vec![],
    }
}

#[test]
fn args_per_permission_mode() {
    let a = build_args(&opts(PermissionMode::AutoEdit), None).join(" ");
    assert!(a.starts_with("-p --input-format stream-json --output-format stream-json --verbose --include-partial-messages"));
    assert!(a.contains("--model haiku --resume sid --permission-mode acceptEdits --permission-prompt-tool stdio --add-dir x"));
    assert!(build_args(&opts(PermissionMode::Ask), None).join(" ").contains("--permission-mode default --permission-prompt-tool stdio"));
    assert!(build_args(&opts(PermissionMode::Plan), None).join(" ").contains("--permission-mode plan"));
    let b = build_args(&opts(PermissionMode::Bypass), None).join(" ");
    assert!(b.contains("--dangerously-skip-permissions") && !b.contains("--permission-prompt-tool"));
}

#[test]
fn args_pass_effort_and_ignore_bad_values() {
    let mut o = opts(PermissionMode::Ask);
    o.effort = Some(" high ".into());
    let a = build_args(&o, None).join(" ");
    assert!(a.contains("--model haiku --effort high --resume sid"), "{a}");
    for bad in ["", "  ", "high; rm", "a&b"] {
        o.effort = Some(bad.into());
        assert!(!build_args(&o, None).contains(&"--effort".to_string()), "{bad}");
    }
}

#[test]
fn startup_errors_are_chinese() {
    assert!(startup_error("Invalid API key · Please run /login", Some(1)).contains("未登录"));
    assert!(startup_error("", Some(1)).contains("退出码 1"));
    assert!(startup_error("boom", None).contains("boom"));
}

/// Real session: `cargo test --lib live::claude -- --ignored --nocapture`.
#[test]
#[ignore]
fn real_smoke_haiku() {
    use std::sync::mpsc;
    let dir = std::env::temp_dir().join(format!("memdock-live-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let (tx, rx) = mpsc::channel::<LiveEvent>();
    let tx = Mutex::new(tx);
    let emit: Emit = Arc::new(move |ev| {
        let _ = tx.lock().unwrap().send(ev);
    });
    let mut o = opts(PermissionMode::Ask);
    o.cwd = dir.to_string_lossy().into_owned();
    o.resume_id = None;
    o.extra_args.clear();
    let mut d = spawn(&o, emit).expect("spawn");
    // The pre-assigned session id is announced before any turn.
    let early = rx.recv_timeout(Duration::from_secs(5));
    eprintln!("before first message: {early:?}");
    let Ok(LiveEvent::Started { native_id, .. }) = early else { panic!("no early Started") };
    d.send("Reply with exactly: pong", &[]).unwrap();
    let mut got_text = false;
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        let Ok(ev) = rx.recv_timeout(Duration::from_secs(5)) else { continue };
        eprintln!("{ev:?}");
        match ev {
            LiveEvent::AssistantMessage { text, .. } if text.to_lowercase().contains("pong") => got_text = true,
            LiveEvent::Started { native_id: n, .. } => assert_eq!(n, native_id),
            LiveEvent::TurnCompleted { .. } => break,
            LiveEvent::Exited { .. } => panic!("exited early"),
            _ => {}
        }
    }
    d.stop();
    let exited = (0..50).any(|_| matches!(rx.recv_timeout(Duration::from_millis(200)), Ok(LiveEvent::Exited { .. })));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(got_text, "no pong");
    assert!(exited, "no Exited after stop");
}

#[test]
fn session_id_known_up_front() {
    let mut o = opts(PermissionMode::Ask);
    assert_eq!(known_session_id(&o), Some(("sid".into(), false)));
    o.extra_args = vec!["--fork-session".into()];
    assert_eq!(known_session_id(&o), None);
    o.resume_id = None;
    o.extra_args.clear();
    let (id, fresh) = known_session_id(&o).unwrap();
    assert!(fresh && uuid::Uuid::parse_str(&id).is_ok());
    assert!(build_args(&o, Some(&id)).join(" ").contains(&format!("--session-id {id}")));
    o.extra_args = vec!["--continue".into()];
    assert_eq!(known_session_id(&o), None);
}

#[test]
fn early_announce_is_not_repeated_but_model_update_is() {
    let mut p = Parser::default();
    p.announce("462f7148-74ee-4bd9-96da-1dfd5adddaaa", Some("claude-haiku-5-5".into()));
    assert!(events(&mut p, INIT).is_empty());
    let mut p = Parser::default();
    p.announce("462f7148-74ee-4bd9-96da-1dfd5adddaaa", Some("haiku".into()));
    assert_eq!(events(&mut p, INIT).len(), 1);
}

#[test]
fn user_message_with_images_uses_content_blocks() {
    let v = user_message("look", &["data:image/jpeg;base64,QUJD".to_string()]);
    let content = &v["message"]["content"];
    assert_eq!(content[0]["type"], "image");
    assert_eq!(content[0]["source"]["media_type"], "image/jpeg");
    assert_eq!(content[0]["source"]["data"], "QUJD");
    assert_eq!(content[1], serde_json::json!({"type": "text", "text": "look"}));
}

fn result_line(subtype: &str, is_error: bool, total: f64) -> String {
    serde_json::json!({"type": "result", "subtype": subtype, "is_error": is_error, "total_cost_usd": total, "usage": {"output_tokens": 3}}).to_string()
}

fn turn_cost(ev: &[LiveEvent]) -> f64 {
    match ev.last() {
        Some(LiveEvent::TurnCompleted { usage, .. }) => usage.cost_usd.unwrap(),
        other => panic!("no TurnCompleted: {other:?}"),
    }
}

#[test]
fn turn_cost_is_the_difference_of_running_totals() {
    let mut p = Parser::default();
    let a = turn_cost(&events(&mut p, &result_line("success", false, 0.004)));
    let b = turn_cost(&events(&mut p, &result_line("success", false, 0.007)));
    let c = turn_cost(&events(&mut p, &result_line("success", false, 0.007)));
    assert!((a - 0.004).abs() < 1e-9 && (b - 0.003).abs() < 1e-9 && c.abs() < 1e-9, "{a} {b} {c}");
}

#[test]
fn interrupted_turn_is_not_an_error() {
    let mut p = Parser::default();
    p.on_send();
    p.on_interrupt();
    let ev = events(&mut p, &result_line("error_during_execution", true, 0.0));
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert!(matches!(ev[0], LiveEvent::TurnCompleted { is_error: false, .. }));
    // The flag is per turn: a later real error is reported again.
    let ev = events(&mut p, &result_line("error_during_execution", true, 0.0));
    assert!(matches!(ev[0], LiveEvent::Error { .. }));
    assert!(matches!(ev[1], LiveEvent::TurnCompleted { is_error: true, .. }));
    // A stale interrupt (sent while idle) is cleared by the next message.
    p.on_interrupt();
    p.on_send();
    let ev = events(&mut p, &result_line("error_during_execution", true, 0.0));
    assert!(matches!(ev[1], LiveEvent::TurnCompleted { is_error: true, .. }));
}


#[test]
fn compaction_and_local_command_output_become_notices() {
    let mut p = Parser::default();
    let evs = |p: &mut Parser, l: &str| p.line(l).map(|o| o.events).unwrap_or_default();
    let a = evs(&mut p, r#"{"type":"system","subtype":"status","status":"compacting","session_id":"s"}"#);
    assert!(matches!(&a[..], [LiveEvent::Notice { id, kind, .. }] if id == "compact-1" && kind == "compacting"));
    let b = evs(&mut p, r#"{"type":"system","subtype":"compact_boundary","compact_metadata":{"trigger":"manual","pre_tokens":45993,"post_tokens":9575}}"#);
    assert!(matches!(&b[..], [LiveEvent::Notice { id, kind, before: Some(45993), after: Some(9575), .. }] if id == "compact-1" && kind == "compacted"));
    // The summary that continues the conversation is not shown.
    assert!(evs(&mut p, r#"{"type":"user","message":{"role":"user","content":"This session is being continued from a previous conversation."}}"#).is_empty());
    // `Compacted` duplicates the compacted notice; other command output is shown.
    assert!(evs(&mut p, r#"{"type":"user","message":{"role":"user","content":"<local-command-stdout>Compacted </local-command-stdout>"}}"#).is_empty());
    let c = evs(&mut p, r#"{"type":"user","message":{"role":"user","content":"<local-command-stdout>Total cost: $0.12</local-command-stdout>"}}"#);
    assert!(matches!(&c[..], [LiveEvent::Notice { kind, text, .. }] if kind == "command_output" && text == "Total cost: $0.12"));
    // A summary that mentions the tags is not command output.
    assert!(evs(&mut p, r#"{"type":"user","message":{"role":"user","content":"Summary: we parsed <local-command-stdout>x</local-command-stdout> lines"}}"#).is_empty());
    // A second "compacting" status of the same compaction adds no notice.
    let d = evs(&mut p, r#"{"type":"system","subtype":"status","status":"compacting"}"#);
    let e = evs(&mut p, r#"{"type":"system","subtype":"status","status":"compacting"}"#);
    assert_eq!(d.len(), 1);
    assert!(e.is_empty());
    assert_eq!(local_command_output("<local-command-stdout>  </local-command-stdout>"), None);
}
