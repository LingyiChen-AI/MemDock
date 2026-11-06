use std::fs;

use serde_json::{json, Value};

use super::*;
use crate::convert::tests::{
    assert_valid_claude, assert_valid_codex, claude_sample, codex_sample, make_state_db, write_lines,
};

struct Env {
    _tmp: tempfile::TempDir,
    paths: AgentPaths,
    data: PathBuf,
    claude_src: PathBuf,
    codex_src: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let paths = AgentPaths::from_home(tmp.path());
    let claude_src = paths.claude_home.join("projects").join("-work-demo").join("src-claude.jsonl");
    write_lines(&claude_src, &claude_sample());
    let codex_src = paths
        .codex_home
        .join("sessions/2026/11/02/rollout-2026-11-02T08-00-00-11111111-2222-3333-4444-555555555555.jsonl");
    write_lines(&codex_src, &codex_sample());
    let data = tmp.path().join("data");
    fs::create_dir_all(&data).unwrap();
    Env { paths, data, claude_src, codex_src, _tmp: tmp }
}

fn read_lines(p: &str) -> Vec<String> {
    fs::read_to_string(p).unwrap().lines().map(String::from).collect()
}

#[test]
fn lists_both_tools_newest_first() {
    let e = env();
    let cache = Mutex::new(HashMap::new());
    let list = list_sessions_impl(&e.paths, &e.data, &cache);
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].tool, Tool::Codex, "codex sample is newer");
    assert_eq!(list[0].first_prompt, "List the files");
    assert_eq!(list[0].project_name, "demo");
    assert_eq!(list[1].tool, Tool::Claude);
    assert_eq!(list[1].title.as_deref(), Some("Fix the build"));
    assert_eq!(list[1].message_count, 3);
    assert_eq!(list[1].tool_calls, 2);
    // Cached second call gives the same result.
    assert_eq!(list_sessions_impl(&e.paths, &e.data, &cache), list);
}

#[test]
fn converts_claude_to_codex_and_records_it() {
    let e = env();
    let db = make_state_db(&e.paths.codex_home);
    let lock_ = Mutex::new(());
    let src = e.claude_src.to_string_lossy().into_owned();
    let before = fs::read(&e.claude_src).unwrap();
    let r = convert_session_impl(&e.paths, &e.data, &lock_, Tool::Claude, &src, false, true).unwrap();
    assert_eq!(r.tool, Tool::Codex);
    assert_eq!(r.cwd, "/work/demo");
    assert!(r.indexed, "{:?}", r.warnings);
    assert!(r.path.contains("rollout-") && r.path.ends_with(&format!("{}.jsonl", r.session_id)));
    assert!(Path::new(&r.path).starts_with(e.paths.codex_home.join("sessions")));
    assert_eq!(r.resume_command, format!("cd \"/work/demo\"{} codex resume {}", if cfg!(windows) { ";" } else { " &&" }, r.session_id));
    assert_valid_codex(&read_lines(&r.path), &r.session_id);
    assert_eq!(fs::read(&e.claude_src).unwrap(), before, "source untouched");

    let conn = rusqlite::Connection::open(&db).unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM threads WHERE id = ?1", [&r.session_id], |row| row.get(0))
        .unwrap();
    assert_eq!(n, 1);
    let index = fs::read_to_string(e.paths.codex_home.join("session_index.jsonl")).unwrap();
    let v: Value = serde_json::from_str(index.lines().last().unwrap()).unwrap();
    assert_eq!(v["id"], r.session_id.as_str());
    assert_eq!(v["thread_name"], "Fix the build");

    // A second conversion needs force.
    let err = convert_session_impl(&e.paths, &e.data, &lock_, Tool::Claude, &src, false, true).unwrap_err();
    assert_eq!(err.code, "already_converted");
    let again = convert_session_impl(&e.paths, &e.data, &lock_, Tool::Claude, &src, true, false).unwrap();
    assert_ne!(again.session_id, r.session_id);

    // The list labels both directions.
    let list = list_sessions_impl(&e.paths, &e.data, &Mutex::new(HashMap::new()));
    let source = list.iter().find(|s| s.id == "src-claude").unwrap();
    assert_eq!(source.converted_to.len(), 2);
    let target = list.iter().find(|s| s.id == r.session_id).unwrap();
    assert_eq!(target.converted_from, Some(SessionRef { tool: Tool::Claude, id: "src-claude".into() }));
    assert_eq!(target.first_prompt, "Please fix the build");
}

#[test]
fn converts_codex_to_claude_into_project_dir() {
    let e = env();
    let lock_ = Mutex::new(());
    let src = e.codex_src.to_string_lossy().into_owned();
    let dir = convert_target_dir_impl(&e.paths, Tool::Codex, &src).unwrap();
    assert!(dir.ends_with("C--work-demo"), "{dir}");
    let r = convert_session_impl(&e.paths, &e.data, &lock_, Tool::Codex, &src, false, true).unwrap();
    assert_eq!(r.tool, Tool::Claude);
    assert!(!r.indexed);
    assert_eq!(Path::new(&r.path).parent().unwrap(), Path::new(&dir));
    assert_eq!(r.resume_command, format!("cd \"C:\\work\\demo\"{} claude --resume {}", if cfg!(windows) { ";" } else { " &&" }, r.session_id));
    assert_valid_claude(&read_lines(&r.path), &r.session_id, r"C:\work\demo");
    // Missing state DB is a warning only (Codex -> Claude never touches it).
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn codex_target_without_state_db_warns() {
    let e = env();
    let src = e.claude_src.to_string_lossy().into_owned();
    let r = convert_session_impl(&e.paths, &e.data, &Mutex::new(()), Tool::Claude, &src, false, true).unwrap();
    assert!(!r.indexed);
    assert_eq!(r.warnings, vec!["state_db_missing".to_string()]);
}

#[test]
fn refuses_files_outside_session_folders() {
    let e = env();
    let outside = e.data.join("evil.jsonl");
    write_lines(&outside, &claude_sample());
    let err = convert_session_impl(
        &e.paths,
        &e.data,
        &Mutex::new(()),
        Tool::Claude,
        &outside.to_string_lossy(),
        false,
        true,
    )
    .unwrap_err();
    assert_eq!(err.code, "forbidden");
    // Wrong tool for the file.
    let err = session_preview_impl(&e.paths, &e.data, Tool::Codex, &e.claude_src.to_string_lossy()).unwrap_err();
    assert_eq!(err.code, "forbidden");
    let err = session_preview_impl(&e.paths, &e.data, Tool::Claude, "/nope/x.jsonl").unwrap_err();
    assert_eq!(err.code, "not_found");
}

#[test]
fn preview_reports_plan() {
    let e = env();
    let p = session_preview_impl(&e.paths, &e.data, Tool::Claude, &e.claude_src.to_string_lossy()).unwrap();
    assert_eq!(p.stats.mapped_calls, 1);
    assert_eq!(p.stats.degraded_calls, 1);
    assert_eq!(p.messages[0].role, PreviewRole::User);
    assert!(p.messages.iter().any(|m| m.role == PreviewRole::Shell && m.text == "cargo build"));
    assert_eq!(p.total_messages, p.messages.len());
}

/// Long sessions keep the newest PREVIEW_LIMIT entries, for both tools.
#[test]
fn preview_of_long_session_keeps_the_newest_entries() {
    use crate::convert::tests::{cl, codex_line};
    use crate::convert::timeline::TimelineItem;
    const N: usize = 300; // 300 prompts + 300 replies = 600 entries, over the 400 cap
    let e = env();
    let ts = |i: usize| format!("2026-11-03T{:02}:{:02}:{:02}.000Z", i / 3600, i / 60 % 60, i % 60);

    let mut claude = Vec::new();
    for i in 0..N {
        let (u, a) = (format!("u{i}"), format!("a{i}"));
        let parent = if i == 0 { None } else { Some(format!("a{}", i - 1)) };
        claude.push(cl("user", &u, parent.as_deref(), &ts(2 * i), json!(format!("ask {i}"))));
        claude.push(cl("assistant", &a, Some(&u), &ts(2 * i + 1), json!([{"type": "text", "text": format!("reply {i}")}])));
    }
    let claude_path = e.paths.claude_home.join("projects").join("-work-demo").join("long-claude.jsonl");
    write_lines(&claude_path, &claude);

    let mut codex = vec![codex_line(&ts(0), "session_meta", json!({
        "id": "99999999-2222-3333-4444-555555555555", "timestamp": ts(0), "cwd": "C:\\work\\demo", "originator": "codex_cli_rs", "source": "cli"
    }))];
    for i in 0..N {
        codex.push(codex_line(&ts(2 * i), "response_item", json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": format!("ask {i}")}]})));
        codex.push(codex_line(&ts(2 * i + 1), "response_item", json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": format!("reply {i}")}]})));
    }
    let codex_path = e
        .paths
        .codex_home
        .join("sessions/2026/11/03/rollout-2026-11-03T00-00-00-99999999-2222-3333-4444-555555555555.jsonl");
    write_lines(&codex_path, &codex);

    for (tool, path) in [(Tool::Claude, &claude_path), (Tool::Codex, &codex_path)] {
        let p = session_preview_impl(&e.paths, &e.data, tool, &path.to_string_lossy()).unwrap();
        let text = |t: &TimelineItem| match t {
            TimelineItem::User { text, .. } | TimelineItem::Assistant { text, .. } => text.clone(),
            _ => String::new(),
        };
        assert_eq!(p.timeline_total, 2 * N, "{tool:?}");
        assert_eq!(p.timeline.len(), PREVIEW_LIMIT, "{tool:?}");
        assert_eq!(text(p.timeline.last().unwrap()), format!("reply {}", N - 1), "{tool:?} newest kept");
        assert_eq!(text(&p.timeline[0]), format!("ask {}", N - PREVIEW_LIMIT / 2), "{tool:?} oldest dropped");
        assert_eq!(p.total_messages, 2 * N, "{tool:?}");
        assert_eq!(p.messages.len(), PREVIEW_LIMIT, "{tool:?}");
        assert_eq!(p.messages.last().unwrap().text, format!("reply {}", N - 1), "{tool:?}");
        assert_eq!(p.messages[0].text, format!("ask {}", N - PREVIEW_LIMIT / 2), "{tool:?}");
    }
}

#[test]
fn empty_session_is_rejected() {
    let e = env();
    let f = e.paths.claude_home.join("projects").join("-work-demo").join("empty.jsonl");
    write_lines(&f, &[serde_json::json!({"type": "summary", "summary": "x"})]);
    let err = convert_session_impl(&e.paths, &e.data, &Mutex::new(()), Tool::Claude, &f.to_string_lossy(), false, true)
        .unwrap_err();
    assert_eq!(err.code, "empty_session");
}

/// Timing on this machine's real sessions: `cargo test --lib real_session_timing -- --ignored --nocapture`.
#[test]
#[ignore]
fn real_session_timing() {
    let paths = AgentPaths::detect();
    let data = tempfile::tempdir().unwrap();
    let cache = Mutex::new(HashMap::new());
    let t = std::time::Instant::now();
    let list = list_sessions_impl(&paths, data.path(), &cache);
    println!("cold list: {} sessions in {:?}", list.len(), t.elapsed());
    let t = std::time::Instant::now();
    let list = list_sessions_impl(&paths, data.path(), &cache);
    println!("warm list: {} sessions in {:?}", list.len(), t.elapsed());
    let mut by_size: Vec<_> = list.iter().map(|s| (std::fs::metadata(&s.path).map(|m| m.len()).unwrap_or(0), s)).collect();
    by_size.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
    for (n, s) in by_size.iter().take(3) {
        let t = std::time::Instant::now();
        let p = session_preview_impl(&paths, data.path(), s.tool, &s.path).unwrap();
        println!("preview {:?} {} KB: {} items in {:?}", s.tool, n / 1024, p.timeline.len(), t.elapsed());
    }
}

#[test]
fn rename_writes_native_titles_and_lists_them() {
    let e = env();
    let db = make_state_db(&e.paths.codex_home);
    let cache = Mutex::new(HashMap::new());
    let claude = e.claude_src.to_string_lossy().into_owned();
    let codex_p = e.codex_src.to_string_lossy().into_owned();
    rename_session_impl(&e.paths, &e.data, Tool::Claude, &claude, "  登录页修复  ").unwrap();
    let last = read_lines(&claude).pop().unwrap();
    let v: Value = serde_json::from_str(&last).unwrap();
    assert_eq!(v["type"], "custom-title");
    assert_eq!(v["customTitle"], "登录页修复");
    rename_session_impl(&e.paths, &e.data, Tool::Codex, &codex_p, "列文件").unwrap();
    let idx = fs::read_to_string(e.paths.codex_home.join("session_index.jsonl")).unwrap();
    assert!(idx.contains("\"thread_name\":\"列文件\""));
    let _ = db;
    let list = list_sessions_impl(&e.paths, &e.data, &cache);
    let t = |tool: Tool| list.iter().find(|s| s.tool == tool).unwrap().title.clone();
    assert_eq!(t(Tool::Claude).as_deref(), Some("登录页修复"));
    assert_eq!(t(Tool::Codex).as_deref(), Some("列文件"));
    assert!(rename_session_impl(&e.paths, &e.data, Tool::Claude, &claude, "   ").is_err());
}

#[test]
fn summaries_persist_across_restarts() {
    let e = env();
    let cache = Mutex::new(HashMap::new());
    let first = list_sessions_impl(&e.paths, &e.data, &cache);
    assert!(e.data.join("session-index.json").is_file());
    // A new process starts from the saved index and gives the same list.
    let reloaded = Mutex::new(load_index(&e.data));
    assert_eq!(lock(&reloaded).len(), 2);
    assert_eq!(list_sessions_impl(&e.paths, &e.data, &reloaded), first);
    // A changed file is parsed again.
    let mut lines = read_lines(&e.claude_src.to_string_lossy());
    lines.push(r#"{"type":"custom-title","customTitle":"New name","sessionId":"x"}"#.into());
    fs::write(&e.claude_src, lines.join("\n") + "\n").unwrap();
    let after = list_sessions_impl(&e.paths, &e.data, &reloaded);
    assert_eq!(after.iter().find(|s| s.tool == Tool::Claude).unwrap().title.as_deref(), Some("New name"));
}

#[test]
fn context_window_by_model() {
    assert_eq!(claude_context_window("claude-opus-5-5"), 1_000_000);
    assert_eq!(claude_context_window("claude-fable-5-1"), 1_000_000);
    assert_eq!(claude_context_window("claude-haiku-5-5"), 1_000_000);
    assert_eq!(claude_context_window("opus"), 1_000_000);
    assert_eq!(claude_context_window("claude-sonnet-4-5"), 200_000);
    assert_eq!(claude_context_window("claude-haiku-4-5-20251001"), 200_000);
    assert_eq!(claude_context_window("claude-sonnet-4-6[1m]"), 1_000_000);
    assert_eq!(claude_context_window(""), 200_000);
}

#[test]
fn context_from_last_usage_records() {
    let claude = vec![
        r#"{"type":"assistant","message":{"model":"claude-opus-5-5","usage":{"input_tokens":2,"cache_read_input_tokens":100,"cache_creation_input_tokens":8}}}"#.to_string(),
        r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-haiku-5-5","usage":{"input_tokens":9,"cache_read_input_tokens":999}}}"#.to_string(),
        r#"{"type":"user","message":{"content":"hi"}}"#.to_string(),
    ];
    let c = claude_context(&claude).unwrap();
    assert_eq!((c.used, c.window, c.model.as_deref()), (110, 1_000_000, Some("claude-opus-5-5")));
    let codex = vec![
        r#"{"type":"turn_context","payload":{"model":"gpt-6.1-sol"}}"#.to_string(),
        r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":17073},"model_context_window":475000}}}"#.to_string(),
        r#"{"type":"event_msg","payload":{"type":"token_count","info":null}}"#.to_string(),
    ];
    let x = codex_context(&codex).unwrap();
    assert_eq!((x.used, x.window, x.model.as_deref()), (17073, 475000, Some("gpt-6.1-sol")));
    assert_eq!(claude_context(&[]), None);
}

/// `cargo test --lib real_session_context -- --ignored --nocapture`: the newest real session of each tool.
#[test]
#[ignore]
fn real_session_context() {
    let paths = AgentPaths::detect();
    let cache = Mutex::new(HashMap::new());
    let data = tempfile::tempdir().unwrap();
    let list = list_sessions_impl(&paths, data.path(), &cache);
    for tool in [Tool::Claude, Tool::Codex] {
        if let Some(s) = list.iter().find(|s| s.tool == tool) {
            let c = session_context_impl(&paths, tool, &s.path).unwrap();
            println!("{tool:?} {}: {c:?} -> {:.0}%", s.title.clone().unwrap_or_default(), c.as_ref().map(|c| c.used as f64 * 100.0 / c.window as f64).unwrap_or(0.0));
        }
    }
}


#[test]
fn context_after_compaction_uses_post_tokens() {
    let lines = vec![
        r#"{"type":"assistant","message":{"model":"claude-opus-5-5","usage":{"input_tokens":1,"cache_read_input_tokens":810000}}}"#.to_string(),
        r#"{"type":"system","subtype":"compact_boundary","compactMetadata":{"trigger":"manual","preTokens":810001,"postTokens":9575}}"#.to_string(),
    ];
    let c = claude_context(&lines).unwrap();
    assert_eq!((c.used, c.window), (9575, 1_000_000));
}

/// `MEMDOCK_PREVIEW_FILE=<jsonl> cargo test --lib real_preview_tail -- --ignored --nocapture`
#[test]
#[ignore]
fn real_preview_tail() {
    let Ok(f) = std::env::var("MEMDOCK_PREVIEW_FILE") else { return };
    let paths = AgentPaths::detect();
    let data = tempfile::tempdir().unwrap();
    let p = session_preview_impl(&paths, data.path(), Tool::Claude, &f).unwrap();
    println!("timeline_total={} shown={} counts={:?}", p.timeline_total, p.timeline.len(), p.counts);
    for it in p.timeline.iter().rev().take(4) {
        let s = serde_json::to_string(it).unwrap();
        println!("  {}", s.chars().take(160).collect::<String>());
    }
}

#[test]
fn preview_limit_keeps_the_newest_entries() {
    let e = env();
    let src = e.claude_src.to_string_lossy().into_owned();
    let all = session_preview_limited_impl(&e.paths, &e.data, Tool::Claude, &src, 10_000).unwrap();
    let two = session_preview_limited_impl(&e.paths, &e.data, Tool::Claude, &src, 2).unwrap();
    assert_eq!(two.timeline_total, all.timeline.len());
    assert_eq!(two.timeline.len(), 2);
    assert_eq!(two.timeline, all.timeline[all.timeline.len() - 2..]);
}
