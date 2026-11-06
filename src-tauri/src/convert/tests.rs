use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use super::*;

pub(crate) fn write_lines(path: &Path, lines: &[Value]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let body: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    fs::write(path, body.join("\n") + "\n").unwrap();
}

fn cl(kind: &str, uuid: &str, parent: Option<&str>, ts: &str, content: Value) -> Value {
    let mut msg = json!({"role": kind, "content": content});
    if kind == "assistant" {
        msg["model"] = json!("claude-opus-4");
    }
    json!({
        "type": kind, "uuid": uuid, "parentUuid": parent, "sessionId": "src-claude",
        "cwd": "/work/demo", "timestamp": ts, "isSidechain": false, "message": msg
    })
}

/// A Claude session: prompt, thinking + text + Bash call, result, Read call,
/// result, a sidechain line, a meta line and an abandoned branch.
pub(crate) fn claude_sample() -> Vec<Value> {
    vec![
        json!({"type": "summary", "summary": "Fix the build", "leafUuid": "a3"}),
        cl("user", "u1", None, "2026-11-01T10:00:00.000Z", json!("Please fix the build")),
        {
            let mut m = cl("user", "meta", Some("u1"), "2026-11-01T10:00:00.500Z", json!("<system-reminder>x</system-reminder>"));
            m["isMeta"] = json!(true);
            m
        },
        cl("assistant", "a1", Some("u1"), "2026-11-01T10:00:01.000Z", json!([
            {"type": "thinking", "thinking": "secret", "signature": "sig"},
            {"type": "text", "text": "Let me look."},
            {"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "cargo build", "description": "build"}}
        ])),
        cl("user", "r1", Some("a1"), "2026-11-01T10:00:02.000Z", json!([
            {"type": "tool_result", "tool_use_id": "toolu_1", "content": "error[E0425]", "is_error": true}
        ])),
        cl("assistant", "a2", Some("r1"), "2026-11-01T10:00:03.000Z", json!([
            {"type": "tool_use", "id": "toolu_2", "name": "Read", "input": {"file_path": "/work/demo/src/lib.rs"}}
        ])),
        cl("user", "r2", Some("a2"), "2026-11-01T10:00:04.000Z", json!([
            {"type": "tool_result", "tool_use_id": "toolu_2", "content": [{"type": "text", "text": "fn main() {}"}]}
        ])),
        // Abandoned branch (rewind): child of r2 that the final leaf does not descend from.
        cl("assistant", "x1", Some("r2"), "2026-11-01T10:00:05.000Z", json!([{"type": "text", "text": "ABANDONED"}])),
        {
            let mut s = cl("assistant", "s1", Some("r2"), "2026-11-01T10:00:05.500Z", json!([{"type": "text", "text": "SIDECHAIN"}]));
            s["isSidechain"] = json!(true);
            s
        },
        cl("assistant", "a3", Some("r2"), "2026-11-01T10:00:06.000Z", json!([{"type": "text", "text": "Fixed: missing import."}])),
    ]
}

fn codex_line(ts: &str, kind: &str, payload: Value) -> Value {
    json!({"timestamp": ts, "type": kind, "payload": payload})
}

/// A Codex rollout: meta, injected context, prompt, reasoning, shell call,
/// output, custom tool call, output, answer.
pub(crate) fn codex_sample() -> Vec<Value> {
    vec![
        codex_line("2026-11-02T08:00:00.000Z", "session_meta", json!({
            "id": "11111111-2222-3333-4444-555555555555", "timestamp": "2026-11-02T08:00:00.000Z",
            "cwd": r"\\?\C:\work\demo", "originator": "codex_cli_rs", "cli_version": "0.50.0", "source": "cli"
        })),
        codex_line("2026-11-02T08:00:00.100Z", "turn_context", json!({"cwd": "C:\\work\\demo", "model": "gpt-5"})),
        codex_line("2026-11-02T08:00:00.200Z", "response_item", json!({
            "type": "message", "role": "developer", "content": [{"type": "input_text", "text": "dev instructions"}]
        })),
        codex_line("2026-11-02T08:00:00.300Z", "response_item", json!({
            "type": "message", "role": "user", "content": [{"type": "input_text", "text": "<environment_context>x</environment_context>"}]
        })),
        codex_line("2026-11-02T08:00:01.000Z", "response_item", json!({
            "type": "message", "role": "user", "content": [{"type": "input_text", "text": "List the files"}]
        })),
        codex_line("2026-11-02T08:00:01.100Z", "event_msg", json!({"type": "user_message", "message": "List the files"})),
        codex_line("2026-11-02T08:00:02.000Z", "response_item", json!({"type": "reasoning", "summary": [], "encrypted_content": "zzz"})),
        codex_line("2026-11-02T08:00:03.000Z", "response_item", json!({
            "type": "function_call", "name": "shell", "call_id": "call_a",
            "arguments": "{\"command\":[\"bash\",\"-lc\",\"ls -la\"],\"workdir\":\"C:\\\\work\\\\demo\"}"
        })),
        codex_line("2026-11-02T08:00:04.000Z", "response_item", json!({
            "type": "function_call_output", "call_id": "call_a", "output": "a.txt\nb.txt"
        })),
        codex_line("2026-11-02T08:00:05.000Z", "response_item", json!({
            "type": "custom_tool_call", "name": "apply_patch", "call_id": "call_b", "input": "*** Begin Patch"
        })),
        codex_line("2026-11-02T08:00:06.000Z", "response_item", json!({
            "type": "custom_tool_call_output", "call_id": "call_b", "output": [{"type": "input_text", "text": "Done"}]
        })),
        codex_line("2026-11-02T08:00:07.000Z", "response_item", json!({
            "type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Two files: a.txt, b.txt"}]
        })),
    ]
}

fn texts(items: &[Item]) -> Vec<(char, String)> {
    items
        .iter()
        .filter_map(|i| match i {
            Item::User { text, .. } => Some(('u', text.clone())),
            Item::Assistant { text, .. } => Some(('a', text.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn parses_claude_active_branch() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("src-claude.jsonl");
    write_lines(&f, &claude_sample());
    let c = claude::parse(&f);
    assert_eq!(c.id, "src-claude");
    assert_eq!(c.cwd, "/work/demo");
    assert_eq!(c.title.as_deref(), Some("Fix the build"));
    assert_eq!(c.model.as_deref(), Some("claude-opus-4"));
    assert_eq!(
        texts(&c.items),
        vec![
            ('u', "Please fix the build".to_string()),
            ('a', "Let me look.".to_string()),
            ('a', "Fixed: missing import.".to_string()),
        ]
    );
    let calls: Vec<_> = c.items.iter().filter(|i| matches!(i, Item::Call { .. })).collect();
    assert_eq!(calls.len(), 2);
    assert!(matches!(calls[0], Item::Call { shell: Some(s), .. } if s == "cargo build"));
    assert!(c.items.iter().any(|i| matches!(i, Item::Result { id, is_error: true, .. } if id == "toolu_1")));
}

#[test]
fn parses_codex_and_skips_injected_items() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("rollout-2026-11-02T08-00-00-11111111-2222-3333-4444-555555555555.jsonl");
    write_lines(&f, &codex_sample());
    let c = codex::parse(&f);
    assert_eq!(c.id, "11111111-2222-3333-4444-555555555555");
    assert_eq!(c.cwd, r"C:\work\demo");
    assert_eq!(c.model.as_deref(), Some("gpt-5"));
    assert_eq!(
        texts(&c.items),
        vec![('u', "List the files".to_string()), ('a', "Two files: a.txt, b.txt".to_string())]
    );
    assert!(c.items.iter().any(|i| matches!(i, Item::Call { shell: Some(s), .. } if s == "ls -la")));
    assert!(c.items.iter().any(|i| matches!(i, Item::Result { output, .. } if output == "Done")));
}

#[test]
fn shell_command_variants() {
    assert_eq!(codex::shell_command("shell", &json!({"command": ["bash", "-lc", "ls"]})).as_deref(), Some("ls"));
    assert_eq!(codex::shell_command("shell", &json!({"command": ["git", "status"]})).as_deref(), Some("git status"));
    assert_eq!(
        codex::shell_command("shell", &json!({"command": ["powershell.exe", "-Command", "dir"]})).as_deref(),
        Some("dir")
    );
    assert_eq!(codex::shell_command("shell_command", &json!({"command": "pwd"})).as_deref(), Some("pwd"));
    assert_eq!(codex::shell_command("exec_command", &json!({"cmd": "echo hi"})).as_deref(), Some("echo hi"));
    assert_eq!(codex::shell_command("apply_patch", &json!({"input": "x"})), None);
}

#[test]
fn plan_maps_shell_degrades_others_and_merges_text() {
    let ts = || "2026-01-01T00:00:00.000Z".to_string();
    let items = vec![
        Item::User { ts: ts(), text: "go".into() },
        Item::Call { ts: ts(), id: "c1".into(), name: "Bash".into(), input: json!({"command": "ls"}), shell: Some("ls".into()) },
        Item::Assistant { ts: ts(), text: "while waiting".into() },
        Item::Result { ts: ts(), id: "c1".into(), output: "out".into(), is_error: false },
        Item::Call { ts: ts(), id: "c2".into(), name: "Read".into(), input: json!({"file_path": "a"}), shell: None },
        Item::Result { ts: ts(), id: "c2".into(), output: "x".repeat(MAX_OUTPUT_CHARS + 10), is_error: false },
        Item::Call { ts: ts(), id: "c3".into(), name: "Bash".into(), input: json!({}), shell: Some("never answered".into()) },
        Item::Result { ts: ts(), id: "orphan".into(), output: "lonely".into(), is_error: true },
    ];
    let (out, stats) = plan(&items);
    assert_eq!(stats.tool_calls, 3);
    assert_eq!(stats.mapped_calls, 1);
    assert_eq!(stats.degraded_calls, 3);
    assert_eq!(out.len(), 3, "{out:#?}");
    assert!(matches!(&out[0], Out::User { text, .. } if text == "go"));
    assert!(matches!(&out[1], Out::Shell { command, output, .. } if command == "ls" && output == "out"));
    let Out::Assistant { text, .. } = &out[2] else { panic!("{out:#?}") };
    assert!(text.starts_with("while waiting"));
    assert!(text.contains("[Tool call: Read]"));
    assert!(text.contains("[truncated 10 chars]"));
    assert!(text.contains("[Tool call: Bash] never answered"));
    assert!(text.contains("[Tool error]\nlonely"));
}

/// Checks the Claude Code invariants a resumable file needs.
pub(crate) fn assert_valid_claude(lines: &[String], session_id: &str, cwd: &str) {
    let vals: Vec<Value> = lines.iter().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert!(!vals.is_empty());
    assert_eq!(vals[0]["type"], "user", "first message must be from the user");
    assert!(vals[0]["parentUuid"].is_null());
    let mut seen = HashSet::new();
    for (i, v) in vals.iter().enumerate() {
        assert_eq!(v["sessionId"], session_id);
        assert_eq!(v["cwd"], cwd);
        assert!(v["timestamp"].as_str().is_some_and(|t| crate::extract::sessions::parse_ts(t).is_some()));
        let uuid = v["uuid"].as_str().unwrap().to_string();
        if i > 0 {
            assert_eq!(v["parentUuid"], vals[i - 1]["uuid"], "linear parent chain");
        }
        assert!(seen.insert(uuid));
        let content = &v["message"]["content"];
        if let Some(blocks) = content.as_array() {
            for b in blocks {
                if b["type"] == "tool_use" {
                    let id = b["id"].as_str().unwrap();
                    assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
                    let next = &vals[i + 1];
                    assert_eq!(next["type"], "user");
                    assert_eq!(next["message"]["content"][0]["type"], "tool_result");
                    assert_eq!(next["message"]["content"][0]["tool_use_id"], id);
                    assert!(next["toolUseResult"]["stdout"].is_string());
                }
                assert_ne!(b["type"], "thinking");
            }
        }
    }
}

/// Checks the Codex rollout invariants.
pub(crate) fn assert_valid_codex(lines: &[String], session_id: &str) {
    let vals: Vec<Value> = lines.iter().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(vals[0]["type"], "session_meta");
    assert_eq!(vals[0]["payload"]["id"], session_id);
    let mut user_items = 0;
    let mut user_events = 0;
    for (i, v) in vals.iter().enumerate() {
        assert!(v["timestamp"].as_str().is_some_and(|t| crate::extract::sessions::parse_ts(t).is_some()));
        let p = &v["payload"];
        if v["type"] == "response_item" && p["type"] == "message" && p["role"] == "user" {
            user_items += 1;
        }
        if v["type"] == "event_msg" && p["type"] == "user_message" {
            user_events += 1;
        }
        if p["type"] == "function_call" {
            let args: Value = serde_json::from_str(p["arguments"].as_str().unwrap()).unwrap();
            assert!(args["command"].is_array());
            let next = &vals[i + 1]["payload"];
            assert_eq!(next["type"], "function_call_output");
            assert_eq!(next["call_id"], p["call_id"]);
            assert!(next["output"].is_string());
        }
    }
    assert!(user_items > 0);
    assert_eq!(user_items, user_events, "every user message has its event");
}

#[test]
fn claude_to_codex_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src-claude.jsonl");
    write_lines(&src, &claude_sample());
    let conv = claude::parse(&src);
    let (out, stats) = plan(&conv.items);
    assert_eq!(stats.mapped_calls, 1);
    assert_eq!(stats.degraded_calls, 1);
    let note = conversion_note(Tool::Claude, &conv.id);
    let meta = codex::MetaInfo {
        session_id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
        cwd: &conv.cwd,
        started_at: "2026-11-01T10:00:00.000Z",
        model_provider: "openai",
    };
    let lines = codex::render(&out, &meta, Some(&note));
    assert_valid_codex(&lines, meta.session_id);
    let file = dir.path().join("rollout-2026-11-01T10-00-00-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.jsonl");
    codex::write(&file, &lines).unwrap();
    assert!(codex::write(&file, &lines).is_err(), "never overwrites");
    let back = codex::parse(&file);
    assert_eq!(back.id, meta.session_id);
    assert_eq!(back.cwd, "/work/demo");
    let t = texts(&back.items);
    assert_eq!(t[0], ('u', "Please fix the build".to_string()));
    assert!(t.iter().any(|(_, s)| s.contains("[Tool call: Read]") && s.contains("fn main() {}")));
    assert!(t.last().unwrap().1.contains("MemDock"));
    assert!(back.items.iter().any(|i| matches!(i, Item::Call { shell: Some(s), .. } if s == "cargo build")));
    // Source untouched.
    assert_eq!(claude::parse(&src), conv);
}

#[test]
fn codex_to_claude_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("rollout-2026-11-02T08-00-00-11111111-2222-3333-4444-555555555555.jsonl");
    write_lines(&src, &codex_sample());
    let conv = codex::parse(&src);
    let (out, _) = plan(&conv.items);
    let sid = "99999999-8888-7777-6666-555555555555";
    let lines = claude::render(&out, sid, &conv.cwd, Some(&conversion_note(Tool::Codex, &conv.id)));
    assert_valid_claude(&lines, sid, &conv.cwd);
    let path = claude::write(dir.path(), sid, &lines).unwrap();
    let back = claude::parse(&path);
    assert_eq!(back.id, sid);
    assert_eq!(back.cwd, r"C:\work\demo");
    assert_eq!(back.model, None, "converted model name is not reported");
    let t = texts(&back.items);
    assert_eq!(t[0], ('u', "List the files".to_string()));
    assert!(t.iter().any(|(_, s)| s.contains("[Tool call: apply_patch]") && s.contains("Done")));
    assert!(t.iter().any(|(_, s)| s.ends_with("Two files: a.txt, b.txt")));
    let (out2, stats2) = plan(&back.items);
    assert_eq!(stats2.mapped_calls, 1, "shell call survives the round trip");
    assert!(out2.iter().any(|o| matches!(o, Out::Shell { command, output, .. } if command == "ls -la" && output == "a.txt\nb.txt")));
}

#[test]
fn claude_render_prepends_user_when_starting_with_assistant() {
    let out = vec![Out::Assistant { ts: "2026-01-01T00:00:00.000Z".into(), text: "hi".into() }];
    let lines = claude::render(&out, "s", "/w", None);
    assert_valid_claude(&lines, "s", "/w");
    assert_eq!(lines.len(), 2);
}

#[test]
fn encodes_claude_project_dir() {
    assert_eq!(claude::encode_project_dir(r"C:\codes\cc-memory"), "C--codes-cc-memory");
    assert_eq!(claude::encode_project_dir("/home/me/my_app.v2"), "-home-me-my-app-v2");
}

#[test]
fn reads_model_provider_from_config() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(codex::model_provider(dir.path()), "openai");
    fs::write(
        dir.path().join("config.toml"),
        "model = \"gpt-5\"\nmodel_provider = \"azure\" # comment\n[model_providers.azure]\nname = \"x\"\n",
    )
    .unwrap();
    assert_eq!(codex::model_provider(dir.path()), "azure");
}

const THREADS_DDL: &str = "CREATE TABLE threads (
    id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    source TEXT NOT NULL, model_provider TEXT NOT NULL, cwd TEXT NOT NULL, title TEXT NOT NULL,
    sandbox_policy TEXT NOT NULL, approval_mode TEXT NOT NULL, tokens_used INTEGER NOT NULL DEFAULT 0,
    has_user_event INTEGER NOT NULL DEFAULT 0, archived INTEGER NOT NULL DEFAULT 0, archived_at INTEGER,
    cli_version TEXT NOT NULL DEFAULT '', first_user_message TEXT NOT NULL DEFAULT '',
    created_at_ms INTEGER, updated_at_ms INTEGER, preview TEXT NOT NULL DEFAULT '',
    recency_at INTEGER NOT NULL DEFAULT 0, recency_at_ms INTEGER NOT NULL DEFAULT 0,
    history_mode TEXT NOT NULL DEFAULT 'legacy', name TEXT)";

pub(crate) fn make_state_db(codex_home: &Path) -> std::path::PathBuf {
    fs::create_dir_all(codex_home).unwrap();
    let db = codex_home.join("state_5.sqlite");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(THREADS_DDL).unwrap();
    db
}

#[test]
fn registers_thread_in_state_db() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("state_2.sqlite"), b"").unwrap();
    let db = make_state_db(dir.path());
    assert_eq!(codex::state_db_path(dir.path()).unwrap(), db, "newest version wins");
    let file = dir.path().join("rollout.jsonl");
    let row = codex::ThreadRow {
        id: "t1",
        rollout_path: &file,
        cwd: "/w",
        title: "Title",
        first_user_message: "hello",
        model_provider: "openai",
        model: Some("gpt-5"),
        created_ms: 1_700_000_000_123,
        updated_ms: 1_700_000_100_456,
    };
    assert!(codex::register_thread(&db, &row).unwrap());
    assert!(!codex::register_thread(&db, &row).unwrap(), "insert or ignore");
    let conn = rusqlite::Connection::open(&db).unwrap();
    let (path, created, has_user, mode): (String, i64, i64, String) = conn
        .query_row(
            "SELECT rollout_path, created_at, has_user_event, history_mode FROM threads WHERE id='t1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(path, file.to_string_lossy());
    assert_eq!(created, 1_700_000_000);
    assert_eq!(has_user, 1);
    assert_eq!(mode, "legacy");
}

#[test]
fn skips_state_db_with_unknown_required_column() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("state_9.sqlite");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL, mystery TEXT NOT NULL)")
        .unwrap();
    drop(conn);
    let file = dir.path().join("r.jsonl");
    let row = codex::ThreadRow {
        id: "t1",
        rollout_path: &file,
        cwd: "/w",
        title: "T",
        first_user_message: "m",
        model_provider: "openai",
        model: None,
        created_ms: 0,
        updated_ms: 0,
    };
    assert!(!codex::register_thread(&db, &row).unwrap());
}

#[test]
fn appends_session_index_line() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("session_index.jsonl"), "{\"id\":\"old\"}").unwrap();
    codex::append_session_index(dir.path(), "new", "Name").unwrap();
    let text = fs::read_to_string(dir.path().join("session_index.jsonl")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    let v: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(v["id"], "new");
    assert_eq!(v["thread_name"], "Name");
}

/// Manual check against the real local sessions (read-only; output goes to a
/// temp dir): `cargo test --lib real_sessions_dry_run -- --ignored --nocapture`.
#[test]
#[ignore]
fn real_sessions_dry_run() {
    let paths = crate::paths::AgentPaths::detect();
    let tmp = tempfile::tempdir().unwrap();
    let mut files: Vec<(Tool, std::path::PathBuf)> = crate::extract::sessions::claude_files(&paths)
        .into_iter()
        .rev()
        .take(40)
        .map(|p| (Tool::Claude, p))
        .collect();
    files.extend(crate::extract::sessions::codex_files(&paths).into_iter().map(|p| (Tool::Codex, p)));
    let mut checked = 0;
    for (tool, f) in files {
        let conv = parse(tool, &f);
        if !conv.items.iter().any(|i| matches!(i, Item::User { .. })) || conv.cwd.is_empty() {
            continue;
        }
        let (out, stats) = plan(&conv.items);
        let sid = uuid::Uuid::new_v4().to_string();
        match tool {
            Tool::Codex => {
                let lines = claude::render(&out, &sid, &conv.cwd, Some("note"));
                assert_valid_claude(&lines, &sid, &conv.cwd);
                let p = claude::write(tmp.path(), &sid, &lines).unwrap();
                let back = claude::parse(&p);
                assert_eq!(texts(&back.items).first(), texts(&conv.items).first());
            }
            Tool::Claude => {
                let meta = codex::MetaInfo {
                    session_id: &sid,
                    cwd: &conv.cwd,
                    started_at: "2026-01-01T00:00:00.000Z",
                    model_provider: "openai",
                };
                let lines = codex::render(&out, &meta, Some("note"));
                assert_valid_codex(&lines, &sid);
                let p = tmp.path().join(format!("rollout-x-{sid}.jsonl"));
                codex::write(&p, &lines).unwrap();
                let back = codex::parse(&p);
                assert_eq!(back.cwd, conv.cwd);
            }
        }
        checked += 1;
        println!("{tool:?} items={} out={} {stats:?}", conv.items.len(), out.len());
    }
    println!("checked {checked}");
}
