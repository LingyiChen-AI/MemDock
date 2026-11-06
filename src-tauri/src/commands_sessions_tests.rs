use std::fs;

use serde_json::Value;

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
