use super::*;

fn opts(tool: Agent, resume: Option<&str>, permission: PermissionMode, model: Option<&str>) -> PtyOpenOptions {
    PtyOpenOptions {
        tool,
        cwd: ".".into(),
        resume_id: resume.map(String::from),
        permission,
        model: model.map(String::from),
        cols: 80,
        rows: 24,
    }
}

fn args(o: &PtyOpenOptions) -> String {
    build_args(o).join(" ")
}

#[test]
fn claude_args_per_mode() {
    use PermissionMode::*;
    assert_eq!(args(&opts(Agent::Claude, None, Ask, None)), "");
    assert_eq!(args(&opts(Agent::Claude, Some("abc"), Ask, Some("opus"))), "--resume abc --model opus");
    assert_eq!(args(&opts(Agent::Claude, None, AutoEdit, None)), "--permission-mode acceptEdits");
    assert_eq!(args(&opts(Agent::Claude, None, Plan, None)), "--permission-mode plan");
    assert_eq!(args(&opts(Agent::Claude, Some("x"), Bypass, None)), "--resume x --dangerously-skip-permissions");
    // Blank values are ignored.
    assert_eq!(args(&opts(Agent::Claude, Some("  "), Ask, Some(""))), "");
}

#[test]
fn codex_args_per_mode() {
    use PermissionMode::*;
    assert_eq!(args(&opts(Agent::Codex, None, Ask, None)), "-a untrusted -s workspace-write");
    assert_eq!(
        args(&opts(Agent::Codex, Some("t-1"), AutoEdit, Some("gpt-5"))),
        "resume -a on-request -s workspace-write -m gpt-5 t-1"
    );
    assert_eq!(args(&opts(Agent::Codex, Some("t-1"), Plan, None)), "resume -a on-request -s read-only t-1");
    assert_eq!(args(&opts(Agent::Codex, None, Bypass, None)), "--dangerously-bypass-approvals-and-sandbox");
}

#[test]
fn display_command_shortens_the_id() {
    let o = opts(Agent::Claude, Some("0123456789abcdef"), PermissionMode::Ask, None);
    assert_eq!(display_command(&o), "claude --resume 01234567…");
    let o = opts(Agent::Codex, Some("0123456789abcdef"), PermissionMode::Plan, None);
    assert_eq!(display_command(&o), "codex resume -a on-request -s read-only 01234567…");
}

#[test]
fn batch_shims_run_through_cmd() {
    let a = vec!["--resume".to_string(), "x".into()];
    assert_eq!(
        launch_argv(Path::new("C:/n/claude.cmd"), &a),
        vec!["cmd.exe", "/d", "/c", "C:/n/claude.cmd", "--resume", "x"]
    );
    assert_eq!(launch_argv(Path::new("C:/n/claude.EXE"), &a), vec!["C:/n/claude.EXE", "--resume", "x"]);
}

#[test]
fn deserializes_frontend_options() {
    let o: PtyOpenOptions = serde_json::from_value(serde_json::json!({
        "tool": "codex", "cwd": "/p", "resume_id": null, "permission": "auto_edit", "cols": 100, "rows": 30
    }))
    .unwrap();
    assert_eq!(o.tool, Agent::Codex);
    assert_eq!(o.permission, PermissionMode::AutoEdit);
    assert_eq!(o.model, None);
}

#[test]
fn utf8_chunker_carries_split_sequences() {
    let text = "终端 ok 🎉";
    let bytes = text.as_bytes();
    // Every split point gives back the same text.
    for i in 0..=bytes.len() {
        let mut d = Utf8Chunker::default();
        let mut s = d.push(&bytes[..i]);
        s.push_str(&d.push(&bytes[i..]));
        s.push_str(&d.finish());
        assert_eq!(s, text, "split at {i}");
    }
    // Byte by byte.
    let mut d = Utf8Chunker::default();
    let s: String = bytes.iter().map(|b| d.push(&[*b])).collect();
    assert_eq!(s, text);
}

#[test]
fn utf8_chunker_replaces_invalid_bytes() {
    let mut d = Utf8Chunker::default();
    assert_eq!(d.push(b"a\xFFb"), "a\u{FFFD}b");
    // A truncated sequence at the end of the stream.
    assert_eq!(d.push(b"c\xE7\xBB"), "c");
    assert_eq!(d.finish(), "\u{FFFD}");
}

/// Collects events from a real PTY until `pty://exit`.
fn run_real(program: &Path, args: &[String]) -> (String, Option<i64>) {
    let (tx, rx) = mpsc::channel::<(String, serde_json::Value)>();
    let tx = Mutex::new(tx);
    let mgr = PtyManager::new(Arc::new(move |name: &str, v: serde_json::Value| {
        let _ = tx.lock().unwrap().send((name.to_string(), v));
    }));
    let o = opts(Agent::Claude, None, PermissionMode::Ask, None);
    let id = mgr.open_program(program, args, &o, &path_dirs()).expect("open");
    let mut out = String::new();
    loop {
        let (name, v) = rx.recv_timeout(Duration::from_secs(60)).expect("pty event");
        assert_eq!(v["pty_id"], id);
        if name == "pty://exit" {
            assert!(!mgr.is_open(&id));
            return (out, v["code"].as_i64());
        }
        let d = v["data"].as_str().unwrap_or_default();
        answer_cursor_query(&mgr, &id, d);
        out.push_str(d);
    }
}

/// ConPTY asks for the cursor position (`ESC[6n`) at start and waits for the
/// answer; xterm.js replies in the app, the tests reply themselves.
fn answer_cursor_query(mgr: &PtyManager, id: &str, data: &str) {
    if data.contains("\x1b[6n") {
        let _ = mgr.write(id, "\x1b[1;1R");
    }
}

/// Real smoke test: `claude --version` and `codex --version` under the native PTY
/// (ConPTY on Windows, through the npm `.cmd` shims). No model calls.
#[test]
#[ignore]
fn real_cli_versions_in_pty() {
    for (name, want) in [("claude", "Claude Code"), ("codex", "codex-cli")] {
        let Some(p) = find_program(OsStr::new(name), &path_dirs()) else {
            eprintln!("{name} not installed, skipped");
            continue;
        };
        let (out, code) = run_real(&p, &["--version".to_string()]);
        eprintln!("{name} ({}) -> {code:?}: {out:?}", p.display());
        assert!(out.contains(want), "{name} output: {out:?}");
        assert_eq!(code, Some(0));
    }
}

/// Real: the interactive Claude Code TUI starts in the PTY (it shows its first
/// screen, e.g. the folder trust prompt; nothing is sent to the model), close kills it.
#[test]
#[ignore]
fn real_claude_tui_starts_and_closes() {
    let Some(p) = find_program(OsStr::new("claude"), &path_dirs()) else { return };
    let (tx, rx) = mpsc::channel::<(String, serde_json::Value)>();
    let tx = Mutex::new(tx);
    let mgr = PtyManager::new(Arc::new(move |name: &str, v: serde_json::Value| {
        let _ = tx.lock().unwrap().send((name.to_string(), v));
    }));
    let dir = tempfile::tempdir().unwrap();
    let mut o = opts(Agent::Claude, None, PermissionMode::Ask, None);
    o.cwd = dir.path().to_string_lossy().into_owned();
    o.cols = 100;
    o.rows = 30;
    let id = mgr.open_program(&p, &build_args(&o), &o, &path_dirs()).expect("open");
    let mut out = String::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while out.chars().filter(|c| c.is_alphabetic()).count() < 40 && std::time::Instant::now() < deadline {
        if let Ok((name, v)) = rx.recv_timeout(Duration::from_millis(200)) {
            assert_ne!(name, "pty://exit", "claude exited early: {out:?}");
            let d = v["data"].as_str().unwrap_or_default();
            answer_cursor_query(&mgr, &id, d);
            out.push_str(d);
        }
    }
    eprintln!("claude TUI: {} bytes, tail {:?}", out.len(), out.chars().rev().take(300).collect::<String>().chars().rev().collect::<String>());
    assert!(out.chars().filter(|c| c.is_alphabetic()).count() >= 40, "{out:?}");
    mgr.close(&id);
    loop {
        let (name, _) = rx.recv_timeout(Duration::from_secs(20)).expect("exit after close");
        if name == "pty://exit" {
            break;
        }
    }
}

/// Real: input written to the PTY reaches the child, close kills it.
#[test]
#[ignore]
fn real_write_and_close() {
    let (tx, rx) = mpsc::channel::<(String, serde_json::Value)>();
    let tx = Mutex::new(tx);
    let mgr = PtyManager::new(Arc::new(move |name: &str, v: serde_json::Value| {
        let _ = tx.lock().unwrap().send((name.to_string(), v));
    }));
    let o = opts(Agent::Claude, None, PermissionMode::Ask, None);
    #[cfg(windows)]
    let (prog, a) = (PathBuf::from("cmd.exe"), vec![]);
    #[cfg(not(windows))]
    let (prog, a) = (PathBuf::from("/bin/sh"), vec![]);
    let id = mgr.open_program(&prog, &a, &o, &path_dirs()).expect("open");
    let mut out = String::new();
    let mut sent = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !out.contains("hello-pty\r\n") && std::time::Instant::now() < deadline {
        if let Ok((_, v)) = rx.recv_timeout(Duration::from_millis(200)) {
            let d = v["data"].as_str().unwrap_or_default();
            answer_cursor_query(&mgr, &id, d);
            out.push_str(d);
        }
        // Once the prompt is up.
        if !sent && out.contains('>') {
            mgr.resize(&id, 100, 30).unwrap();
            mgr.write(&id, "echo hello-pty\r").unwrap();
            sent = true;
        }
    }
    assert!(out.contains("hello-pty"), "{out:?}");
    mgr.close(&id);
    loop {
        let (name, _) = rx.recv_timeout(Duration::from_secs(20)).expect("exit after close");
        if name == "pty://exit" {
            break;
        }
    }
    assert!(!mgr.is_open(&id));
}
