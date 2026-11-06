//! End-to-end tests against the real Claude Code and Codex CLIs.
//!
//! All `#[ignore]`: they spawn real model sessions (small cost) in the user's
//! test folder `C:\Users\Admin\Documents\test` (override with `MEMDOCK_E2E_DIR`).
//! Run with:
//! `cargo test --lib e2e -- --ignored --nocapture --test-threads=1`
#![cfg(test)]

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::commands_sessions::{list_sessions_impl, rename_session_impl, session_preview_impl};
use crate::convert::timeline::TimelineItem;
use crate::convert::SessionSummary;
use crate::extract::runner::{find_program, path_dirs};
use crate::live::{
    self, Agent, Decision, Emit, LiveEnvelope, LiveEvent, LiveManager, LiveState, NativeRegistry, PermissionMode,
    Spawner, StartOptions, TurnUsage,
};
use crate::model::Tool;
use crate::paths::AgentPaths;
use crate::pty::{PtyManager, PtyOpenOptions, PtyTool};

const TURN: Duration = Duration::from_secs(180);
const HAIKU: &str = "haiku";

static COST: Mutex<f64> = Mutex::new(0.0);

fn test_dir() -> PathBuf {
    std::env::var_os("MEMDOCK_E2E_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\Admin\Documents\test"))
}

fn data_dir() -> PathBuf {
    let d = std::env::temp_dir().join("memdock-e2e-data");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn norm(p: &str) -> String {
    p.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

fn token(tag: &str) -> String {
    let n = uuid::Uuid::new_v4().simple().to_string();
    format!("{tag}{}", n[..6].to_uppercase())
}

macro_rules! log {
    ($($t:tt)*) => { println!("  · {}", format!($($t)*)) };
}

// ---- recorder ---------------------------------------------------------------

#[derive(Default)]
struct Rec {
    envs: Mutex<Vec<LiveEnvelope>>,
    cv: Condvar,
}

impl Rec {
    fn of(&self, id: &str) -> Vec<LiveEnvelope> {
        self.envs.lock().unwrap().iter().filter(|e| e.live_id == id).cloned().collect()
    }

    fn count(&self, id: &str) -> usize {
        self.of(id).len()
    }

    /// Index (in `of(id)`) of the first event at or after `from` matching `pred`.
    /// Fails early when the session exits first.
    fn wait(&self, id: &str, from: usize, timeout: Duration, pred: impl Fn(&LiveEvent) -> bool) -> Result<usize, String> {
        let deadline = Instant::now() + timeout;
        let mut g = self.envs.lock().unwrap();
        loop {
            let evs: Vec<&LiveEnvelope> = g.iter().filter(|e| e.live_id == id).collect();
            for (i, e) in evs.iter().enumerate().skip(from) {
                if pred(&e.event) {
                    return Ok(i);
                }
                if matches!(e.event, LiveEvent::Exited { .. }) {
                    let tail: Vec<String> = evs.iter().skip(from).map(|e| short(&e.event)).collect();
                    return Err(format!("session exited before the expected event; events: {tail:#?}"));
                }
            }
            let now = Instant::now();
            if now >= deadline {
                let tail: Vec<String> = evs.iter().skip(from).map(|e| short(&e.event)).collect();
                return Err(format!("timeout; events since {from}: {tail:#?}"));
            }
            g = self.cv.wait_timeout(g, deadline - now).unwrap().0;
        }
    }

    /// Waits for the turn's `TurnCompleted`; returns (assistant text, usage, index).
    fn turn(&self, id: &str, from: usize) -> Result<(String, TurnUsage, usize), String> {
        let i = self.wait(id, from, TURN, |e| matches!(e, LiveEvent::TurnCompleted { .. }))?;
        let evs = self.of(id);
        let LiveEvent::TurnCompleted { usage, .. } = evs[i].event.clone() else { unreachable!() };
        if let Some(c) = usage.cost_usd {
            *COST.lock().unwrap() += c;
        }
        Ok((assistant_text(&evs[from..=i]), usage, i))
    }
}

fn assistant_text(evs: &[LiveEnvelope]) -> String {
    let finals: Vec<&str> = evs
        .iter()
        .filter_map(|e| match &e.event {
            LiveEvent::AssistantMessage { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    if !finals.is_empty() {
        return finals.join("\n");
    }
    evs.iter()
        .filter_map(|e| match &e.event {
            LiveEvent::AssistantDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn short(e: &LiveEvent) -> String {
    let s = serde_json::to_string(e).unwrap_or_default();
    crate::convert::truncate_chars(&s, 300)
}

fn spawner() -> Spawner {
    Arc::new(|opts: &StartOptions, emit: Emit| match opts.agent {
        Agent::Claude => live::claude::spawn(opts, emit),
        Agent::Codex => live::codex::spawn(opts, emit),
    })
}

struct H {
    mgr: Arc<LiveManager>,
    rec: Arc<Rec>,
    pty: Arc<PtyManager>,
    prec: Arc<PtyRec>,
    registry: Arc<NativeRegistry>,
}

impl Drop for H {
    fn drop(&mut self) {
        self.mgr.stop_all();
        self.pty.close_all();
    }
}

fn harness() -> H {
    let rec = Arc::new(Rec::default());
    let r = rec.clone();
    let publish: live::Publish = Arc::new(move |env: &LiveEnvelope| {
        r.envs.lock().unwrap().push(env.clone());
        r.cv.notify_all();
    });
    let registry = Arc::new(NativeRegistry::new());
    let mgr = Arc::new(LiveManager::with_registry(spawner(), publish, registry.clone()));
    let prec = Arc::new(PtyRec::default());
    let p = prec.clone();
    let ppub: crate::pty::Publish = Arc::new(move |name: &str, payload: serde_json::Value| {
        p.items.lock().unwrap().push((name.to_string(), payload));
        p.cv.notify_all();
    });
    let pty = Arc::new(PtyManager::with_registry(ppub, registry.clone()));
    H { mgr, rec, pty, prec, registry }
}

fn opts(agent: Agent, cwd: &Path, prompt: &str) -> StartOptions {
    StartOptions {
        agent,
        cwd: cwd.to_string_lossy().into_owned(),
        model: (agent == Agent::Claude).then(|| HAIKU.to_string()),
        effort: None,
        permission: PermissionMode::Bypass,
        resume_id: None,
        extra_args: vec![],
        prompt: Some(prompt.to_string()),
        prompt_images: vec![],
    }
}

fn state_of(mgr: &LiveManager, id: &str) -> LiveState {
    mgr.list().into_iter().find(|i| i.live_id == id).map(|i| i.state).unwrap()
}

fn native_of(rec: &Rec, id: &str) -> Option<String> {
    rec.of(id).iter().rev().find_map(|e| match &e.event {
        LiveEvent::Started { native_id, .. } => Some(native_id.clone()),
        _ => None,
    })
}

fn assert_seq(rec: &Rec, id: &str) {
    let seqs: Vec<u64> = rec.of(id).iter().map(|e| e.seq).collect();
    assert!(seqs.windows(2).all(|w| w[1] > w[0]), "seq not strictly increasing: {seqs:?}");
}

/// Waits until the state settles to `want` (Status events follow the trigger event).
fn wait_state(mgr: &LiveManager, id: &str, want: LiveState, timeout: Duration) -> LiveState {
    let deadline = Instant::now() + timeout;
    loop {
        let s = state_of(mgr, id);
        if s == want || Instant::now() >= deadline {
            return s;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn cost_line(u: &TurnUsage, ms: u128) -> String {
    format!(
        "{ms} ms, in {:?} out {:?} tokens, cost ${:.4}",
        u.input_tokens,
        u.output_tokens,
        u.cost_usd.unwrap_or(0.0)
    )
}

fn list_sessions() -> Vec<SessionSummary> {
    let cache = Mutex::new(HashMap::new());
    list_sessions_impl(&AgentPaths::detect(), &data_dir(), &cache)
}

/// Polls the session list until `id` shows up (the CLI may flush late).
fn find_session(id: &str) -> SessionSummary {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(s) = list_sessions().into_iter().find(|s| s.id == id) {
            return s;
        }
        assert!(Instant::now() < deadline, "session {id} not in list_sessions");
        std::thread::sleep(Duration::from_millis(500));
    }
}

// ---- PTY recorder -----------------------------------------------------------

#[derive(Default)]
struct PtyRec {
    items: Mutex<Vec<(String, serde_json::Value)>>,
    cv: Condvar,
}

impl PtyRec {
    fn output(&self, pty_id: &str) -> String {
        self.items
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, p)| n == "pty://data" && p["pty_id"] == pty_id)
            .map(|(_, p)| p["data"].as_str().unwrap_or("").to_string())
            .collect()
    }

    fn exited(&self, pty_id: &str) -> Option<serde_json::Value> {
        self.items
            .lock()
            .unwrap()
            .iter()
            .find(|(n, p)| n == "pty://exit" && p["pty_id"] == pty_id)
            .map(|(_, p)| p.clone())
    }

    /// Waits for `pred(output)`, answering cursor position queries like xterm.js does.
    fn wait(&self, pty: &PtyManager, pty_id: &str, timeout: Duration, pred: impl Fn(&str, bool) -> bool) -> Result<String, String> {
        let deadline = Instant::now() + timeout;
        let mut answered = 0;
        loop {
            let out = self.output(pty_id);
            let exited = self.exited(pty_id).is_some();
            if pred(&out, exited) {
                return Ok(out);
            }
            let asks = out.matches("\x1b[6n").count();
            while answered < asks {
                let _ = pty.write(pty_id, "\x1b[1;1R");
                answered += 1;
            }
            if Instant::now() >= deadline {
                return Err(format!("pty timeout; exited={exited}; output: {:?}", crate::convert::truncate_chars(&out, 2000)));
            }
            let g = self.items.lock().unwrap();
            let _ = self.cv.wait_timeout(g, Duration::from_millis(200)).unwrap();
        }
    }
}

fn pty_opts(tool: PtyTool, cwd: &Path) -> PtyOpenOptions {
    PtyOpenOptions {
        tool,
        cwd: cwd.to_string_lossy().into_owned(),
        resume_id: None,
        permission: PermissionMode::Ask,
        model: None,
        effort: None,
        cols: 120,
        rows: 30,
    }
}

// ---- tiny PNG ----------------------------------------------------------------

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let mut body = ty.to_vec();
    body.extend_from_slice(data);
    out.extend(&body);
    out.extend(crc32(&body).to_be_bytes());
}

/// Solid-colour RGB PNG (uncompressed deflate).
fn solid_png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut raw = vec![];
    for _ in 0..h {
        raw.push(0);
        for _ in 0..w {
            raw.extend(rgb);
        }
    }
    assert!(raw.len() < 65535);
    let mut z = vec![0x78, 0x01, 0x01];
    let len = raw.len() as u16;
    z.extend(len.to_le_bytes());
    z.extend((!len).to_le_bytes());
    z.extend(&raw);
    z.extend(adler32(&raw).to_be_bytes());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = vec![];
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &z);
    chunk(&mut png, b"IEND", &[]);
    png
}

fn data_uri(png: &[u8]) -> String {
    use base64::Engine;
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png))
}

// ---- git ----------------------------------------------------------------------

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git").arg("-C").arg(dir).args(args).output().expect("git");
    assert!(o.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn demo_repo() -> PathBuf {
    let dir = test_dir().join("worktree-demo");
    if !dir.join(".git").exists() {
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["-c", "init.defaultBranch=main", "init", "-q"]);
        git(&dir, &["config", "user.name", "MemDock E2E"]);
        git(&dir, &["config", "user.email", "e2e@memdock.local"]);
        std::fs::write(dir.join("README.md"), "# worktree demo\n\nE2E test repo for MemDock.\n").unwrap();
        std::fs::write(dir.join("app.txt"), "line1\nline2\nline3\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", "initial"]);
    }
    dir
}

fn total_cost() -> f64 {
    *COST.lock().unwrap()
}

// =============================================================================
// Scenarios
// =============================================================================

/// 1 + 2 + 9 (Claude): new session, continue, stop, resume, registry dedupe,
/// busy_live terminal, list / preview / rename.
#[test]
#[ignore]
fn e2e_01_claude_new_continue_resume_rename() {
    println!("[S1] Claude new session");
    let h = harness();
    let dir = test_dir();
    let t1 = token("ALPHA");
    let t0 = Instant::now();
    let info = h.mgr.start(opts(Agent::Claude, &dir, &format!("[E2E] s1 Reply with exactly: {t1}"))).unwrap();
    let id = info.live_id.clone();
    let (text, usage, done) = h.rec.turn(&id, 0).unwrap();
    log!("turn 1: {} | {}", text.trim(), cost_line(&usage, t0.elapsed().as_millis()));
    assert!(text.contains(&t1), "reply {text:?} lacks {t1}");
    assert!(usage.output_tokens.is_some() && usage.cost_usd.is_some(), "usage {usage:?}");
    assert_seq(&h.rec, &id);
    let evs = h.rec.of(&id);
    let started = evs.iter().position(|e| matches!(e.event, LiveEvent::Started { .. })).expect("Started");
    let first_text = evs
        .iter()
        .position(|e| matches!(e.event, LiveEvent::AssistantDelta { .. } | LiveEvent::AssistantMessage { .. }))
        .unwrap();
    assert!(started < first_text && first_text < done);
    let native = native_of(&h.rec, &id).unwrap();
    assert_eq!(wait_state(&h.mgr, &id, LiveState::Idle, Duration::from_secs(2)), LiveState::Idle);
    log!("native id {native}, {} events", evs.len());

    let s = find_session(&native);
    log!("listed: cwd={} title={:?} first_prompt={:?}", s.cwd, s.title, s.first_prompt);
    assert_eq!(norm(&s.cwd), norm(&dir.to_string_lossy()));
    assert!(s.first_prompt.starts_with("[E2E] s1"), "first_prompt {:?}", s.first_prompt);
    let pv = session_preview_impl(&AgentPaths::detect(), &data_dir(), Tool::Claude, &s.path).unwrap();
    assert!(
        pv.timeline.iter().any(|t| matches!(t, TimelineItem::Assistant { text, .. } if text.contains(&t1))),
        "preview lacks the reply"
    );
    assert!(pv.timeline.iter().any(|t| matches!(t, TimelineItem::User { text, .. } if text.contains(&t1))));

    println!("[S2] continue, stop, resume");
    let t2 = token("BRAVO");
    let from = h.rec.count(&id);
    let t0 = Instant::now();
    h.mgr.send(&id, &format!("Reply with exactly: {t2}"), &[]).unwrap();
    let (text, usage, _) = h.rec.turn(&id, from).unwrap();
    log!("turn 2: {} | {}", text.trim(), cost_line(&usage, t0.elapsed().as_millis()));
    assert!(text.contains(&t2));
    assert_eq!(h.registry.owner(Agent::Claude, &native).map(|o| o.id), Some(id.clone()));
    h.mgr.stop(&id).unwrap();
    h.rec.wait(&id, 0, Duration::from_secs(10), |e| matches!(e, LiveEvent::Exited { .. })).unwrap();
    assert_eq!(state_of(&h.mgr, &id), LiveState::Exited);
    assert!(h.registry.owner(Agent::Claude, &native).is_none(), "registry not released on stop");

    let mut o = opts(Agent::Claude, &dir, "What was the very first token I asked you to reply with in this conversation? Reply with just that token.");
    o.resume_id = Some(native.clone());
    let t0 = Instant::now();
    let r = h.mgr.start(o.clone()).unwrap();
    assert_ne!(r.live_id, id);
    // Same resume id while it runs: the same live session comes back.
    let mut again = o.clone();
    again.prompt = None;
    let r2 = h.mgr.start(again).unwrap();
    assert_eq!(r2.live_id, r.live_id, "registry dedupe");
    let mut po = pty_opts(PtyTool::Claude, &dir);
    po.resume_id = Some(native.clone());
    let busy = h.pty.open(&po);
    assert_eq!(busy.as_ref().err().cloned(), Some(format!("busy_live:{}", r.live_id)), "pty_open: {busy:?}");
    let (text, usage, _) = h.rec.turn(&r.live_id, 0).unwrap();
    log!("resumed: {} | {}", text.trim(), cost_line(&usage, t0.elapsed().as_millis()));
    assert!(text.contains(&t1), "resumed reply {text:?} lacks {t1}");
    assert_eq!(native_of(&h.rec, &r.live_id).as_deref(), Some(native.as_str()), "resume kept the session id");
    assert_seq(&h.rec, &r.live_id);
    h.mgr.stop(&r.live_id).unwrap();

    println!("[S9] rename Claude session");
    let title = format!("[E2E] renamed {}", token("T"));
    rename_session_impl(&AgentPaths::detect(), &data_dir(), Tool::Claude, &s.path, &title).unwrap();
    let listed = find_session(&native);
    assert_eq!(listed.title.as_deref(), Some(title.as_str()));
    let native_title = crate::convert::summarize(&crate::convert::parse(Tool::Claude, Path::new(&s.path)), Path::new(&s.path)).title;
    assert_eq!(native_title.as_deref(), Some(title.as_str()), "custom-title not read back from the file");
    log!("renamed to {title:?}; total cost so far ${:.4}", total_cost());
}

/// 3: two concurrent sessions in the same folder, no cross-talk.
#[test]
#[ignore]
fn e2e_03_claude_concurrent() {
    println!("[S3] two concurrent Claude sessions");
    let h = harness();
    let dir = test_dir();
    let (a1, b1, a2, b2) = (token("APPLE"), token("BERRY"), token("ACORN"), token("BISON"));
    let t0 = Instant::now();
    let a = h.mgr.start(opts(Agent::Claude, &dir, &format!("[E2E] s3a Reply with exactly: {a1}"))).unwrap().live_id;
    let b = h.mgr.start(opts(Agent::Claude, &dir, &format!("[E2E] s3b Reply with exactly: {b1}"))).unwrap().live_id;
    let (ta, ua, _) = h.rec.turn(&a, 0).unwrap();
    let (tb, ub, _) = h.rec.turn(&b, 0).unwrap();
    log!("A: {} | {}", ta.trim(), cost_line(&ua, t0.elapsed().as_millis()));
    log!("B: {} | {}", tb.trim(), cost_line(&ub, t0.elapsed().as_millis()));
    let (fa, fb) = (h.rec.count(&a), h.rec.count(&b));
    h.mgr.send(&a, &format!("Reply with exactly: {a2}"), &[]).unwrap();
    h.mgr.send(&b, &format!("Reply with exactly: {b2}"), &[]).unwrap();
    let (ta2, _, _) = h.rec.turn(&a, fa).unwrap();
    let (tb2, _, _) = h.rec.turn(&b, fb).unwrap();
    log!("A2: {} / B2: {}", ta2.trim(), tb2.trim());
    let all_a = assistant_text(&h.rec.of(&a));
    let all_b = assistant_text(&h.rec.of(&b));
    assert!(all_a.contains(&a1) && all_a.contains(&a2), "A: {all_a}");
    assert!(all_b.contains(&b1) && all_b.contains(&b2), "B: {all_b}");
    let dump_a = format!("{:?}", h.rec.of(&a));
    let dump_b = format!("{:?}", h.rec.of(&b));
    assert!(!dump_a.contains(&b1) && !dump_a.contains(&b2), "cross-talk into A");
    assert!(!dump_b.contains(&a1) && !dump_b.contains(&a2), "cross-talk into B");
    assert_seq(&h.rec, &a);
    assert_seq(&h.rec, &b);
    assert_ne!(native_of(&h.rec, &a), native_of(&h.rec, &b));
    log!("no cross-talk; total cost so far ${:.4}", total_cost());
}

/// 4: approval flow (ask mode): Allow creates the file, Deny does not and ends Idle.
#[test]
#[ignore]
fn e2e_04_claude_permissions() {
    println!("[S4] permission flow");
    let h = harness();
    let dir = test_dir();
    for (name, decision) in [("e2e_allow.txt", Decision::Allow), ("e2e_deny.txt", Decision::Deny)] {
        let file = dir.join(name);
        let _ = std::fs::remove_file(&file);
        let mut o = opts(
            Agent::Claude,
            &dir,
            &format!("[E2E] s4 Use the Write tool to create the file {name} in the current directory with the content: ok. Do nothing else. If the write is denied, just say DENIED and stop."),
        );
        o.permission = PermissionMode::Ask;
        let t0 = Instant::now();
        let id = h.mgr.start(o).unwrap().live_id;
        let mut from = 0;
        let mut asked = 0;
        loop {
            let i = h
                .rec
                .wait(&id, from, TURN, |e| matches!(e, LiveEvent::ApprovalRequest { .. } | LiveEvent::TurnCompleted { .. }))
                .unwrap();
            let ev = h.rec.of(&id)[i].event.clone();
            match ev {
                LiveEvent::ApprovalRequest { request_id, tool_name, summary, .. } => {
                    asked += 1;
                    assert_eq!(wait_state(&h.mgr, &id, LiveState::WaitingApproval, Duration::from_secs(2)), LiveState::WaitingApproval);
                    log!("{name}: approval #{asked} {tool_name} {summary} -> {decision:?}");
                    h.mgr.respond(&id, &request_id, decision).unwrap();
                    from = i + 1;
                    assert!(asked < 4, "too many approval requests");
                }
                LiveEvent::TurnCompleted { usage, .. } => {
                    if let Some(c) = usage.cost_usd {
                        *COST.lock().unwrap() += c;
                    }
                    log!("{name}: turn done | {}", cost_line(&usage, t0.elapsed().as_millis()));
                    break;
                }
                _ => unreachable!(),
            }
        }
        assert!(asked >= 1, "no approval request");
        let st = wait_state(&h.mgr, &id, LiveState::Idle, Duration::from_secs(3));
        assert_eq!(st, LiveState::Idle, "state after {decision:?}");
        assert_eq!(file.exists(), decision == Decision::Allow, "{name} exists = {}", file.exists());
        assert_seq(&h.rec, &id);
        let _ = std::fs::remove_file(&file);
        h.mgr.stop(&id).unwrap();
    }

    // Interrupt while an approval is pending: the turn must end, not stay waiting.
    let file = dir.join("e2e_interrupt.txt");
    let _ = std::fs::remove_file(&file);
    let mut o = opts(
        Agent::Claude,
        &dir,
        "[E2E] s4c Use the Write tool to create the file e2e_interrupt.txt in the current directory with the content: ok. Do nothing else.",
    );
    o.permission = PermissionMode::Ask;
    let id = h.mgr.start(o).unwrap().live_id;
    let i = h.rec.wait(&id, 0, TURN, |e| matches!(e, LiveEvent::ApprovalRequest { .. })).unwrap();
    let t0 = Instant::now();
    h.mgr.interrupt(&id).unwrap();
    let r = h.rec.wait(&id, i, Duration::from_secs(30), |e| matches!(e, LiveEvent::TurnCompleted { .. }));
    let st = wait_state(&h.mgr, &id, LiveState::Idle, Duration::from_secs(3));
    for e in h.rec.of(&id).iter().skip(i) {
        log!("  #{} {}", e.seq, short(&e.event));
    }
    log!("interrupt during approval: {:?} after {} ms, state {st:?}", r.as_ref().map(|_| "TurnCompleted"), t0.elapsed().as_millis());
    r.unwrap();
    assert_eq!(st, LiveState::Idle);
    assert!(!file.exists());
    let tok = token("FOXTROT");
    let from = h.rec.count(&id);
    h.mgr.send(&id, &format!("Never mind. Reply with exactly: {tok}"), &[]).unwrap();
    let (text, _, _) = h.rec.turn(&id, from).unwrap();
    assert!(text.contains(&tok), "{text}");
    h.mgr.stop(&id).unwrap();
    log!("total cost so far ${:.4}", total_cost());
}

/// 5: interrupt a long turn; the session stays usable.
#[test]
#[ignore]
fn e2e_05_claude_interrupt() {
    println!("[S5] interrupt");
    let h = harness();
    let dir = test_dir();
    let id = h
        .mgr
        .start(opts(Agent::Claude, &dir, "[E2E] s5 Count from 1 to 300, one number per line, no other text."))
        .unwrap()
        .live_id;
    let i = h.rec.wait(&id, 0, TURN, |e| matches!(e, LiveEvent::AssistantDelta { .. })).unwrap();
    let t0 = Instant::now();
    h.mgr.interrupt(&id).unwrap();
    let (text, usage, _) = h.rec.turn(&id, i).unwrap();
    let ms = t0.elapsed().as_millis();
    log!("interrupted after {} chars; TurnCompleted {ms} ms after interrupt | {}", text.len(), cost_line(&usage, ms));
    for e in h.rec.of(&id).iter().skip(i) {
        log!("  #{} {}", e.seq, short(&e.event));
    }
    assert!(!text.contains("\n300"), "turn was not interrupted");
    let after = h.rec.of(&id);
    assert!(
        !after[i..].iter().any(|e| matches!(e.event, LiveEvent::Error { .. } | LiveEvent::TurnCompleted { is_error: true, .. })),
        "a user interrupt is reported as an error"
    );
    assert_eq!(wait_state(&h.mgr, &id, LiveState::Idle, Duration::from_secs(3)), LiveState::Idle);
    let tok = token("CHARLIE");
    let from = h.rec.count(&id);
    h.mgr.send(&id, &format!("Stop counting. Reply with exactly: {tok}"), &[]).unwrap();
    let (text, usage, _) = h.rec.turn(&id, from).unwrap();
    log!("after interrupt: {} | {}", text.trim(), cost_line(&usage, 0));
    assert!(text.contains(&tok));
    assert_seq(&h.rec, &id);
}

/// 6: image input.
#[test]
#[ignore]
fn e2e_06_claude_image() {
    println!("[S6] image");
    let h = harness();
    let dir = test_dir();
    let uri = data_uri(&solid_png(32, 32, [255, 0, 0]));
    let mut o = opts(Agent::Claude, &dir, "[E2E] s6 What is the dominant colour of this image? Answer with one lowercase word.");
    o.prompt_images = vec![uri.clone()];
    let t0 = Instant::now();
    let id = h.mgr.start(o).unwrap().live_id;
    let (text, usage, _) = h.rec.turn(&id, 0).unwrap();
    log!("answer: {} | {}", text.trim(), cost_line(&usage, t0.elapsed().as_millis()));
    let low = text.to_lowercase();
    assert!(low.contains("red") || low.contains("红"), "answer {text:?}");
    let published = h.rec.of(&id).into_iter().find_map(|e| match e.event {
        LiveEvent::UserMessage { images, .. } => Some(images),
        _ => None,
    });
    assert_eq!(published, Some(vec![uri]));
    let stored = h.mgr.transcript(&id).unwrap().into_iter().find_map(|e| match e.event {
        LiveEvent::UserMessage { images, .. } => Some(images),
        _ => None,
    });
    let stored = stored.unwrap();
    assert_eq!(stored.len(), 1);
    let p = stored[0].strip_prefix(live::IMAGE_REF_PREFIX).expect("stored as memdock-image:");
    assert!(Path::new(p).is_file(), "stored image file {p}");
    log!("stored as {}", stored[0]);
}

/// 7: effort + model.
#[test]
#[ignore]
fn e2e_07_effort_model() {
    println!("[S7] Claude effort low + haiku");
    let h = harness();
    let dir = test_dir();
    let tok = token("DELTA");
    let mut o = opts(Agent::Claude, &dir, &format!("[E2E] s7 Reply with exactly: {tok}"));
    o.effort = Some("low".into());
    let t0 = Instant::now();
    let id = h.mgr.start(o).unwrap().live_id;
    let (text, usage, _) = h.rec.turn(&id, 0).unwrap();
    log!("reply: {} | {}", text.trim(), cost_line(&usage, t0.elapsed().as_millis()));
    assert!(text.contains(&tok));
    let models: Vec<Option<String>> = h
        .rec
        .of(&id)
        .into_iter()
        .filter_map(|e| match e.event {
            LiveEvent::Started { model, .. } => Some(model),
            _ => None,
        })
        .collect();
    log!("Started models: {models:?}; info.model = {:?}", h.mgr.list()[0].model);
    let resolved = models.iter().flatten().any(|m| m.starts_with("claude-") && m.contains("haiku"));
    assert!(resolved, "no resolved haiku model in {models:?}");
    let info_model = h.mgr.list().into_iter().find(|i| i.live_id == id).unwrap().model.unwrap_or_default();
    assert!(info_model.starts_with("claude-") && info_model.contains("haiku"), "info.model {info_model}");
}

/// 8 + 9 (Codex): new thread, resume, dedupe, busy_live, rename.
#[test]
#[ignore]
fn e2e_08_codex() {
    println!("[S8] Codex");
    let h = harness();
    let dir = test_dir();
    let tok = token("ECHO");
    let mut o = opts(Agent::Codex, &dir, &format!("[E2E] s8 Reply with exactly: {tok}"));
    o.effort = Some("low".into());
    let t0 = Instant::now();
    let id = h.mgr.start(o.clone()).unwrap().live_id;
    let (text, usage, _) = match h.rec.turn(&id, 0) {
        Ok(x) => x,
        Err(e) => panic!("codex turn failed (network?): {e}"),
    };
    log!("reply: {} | {}", text.trim(), cost_line(&usage, t0.elapsed().as_millis()));
    assert!(text.contains(&tok), "reply {text:?}");
    let thread = native_of(&h.rec, &id).expect("thread id");
    let model = h.mgr.list().into_iter().find(|i| i.live_id == id).unwrap().model;
    log!("thread {thread}, model {model:?}, usage {usage:?}");
    assert!(usage.input_tokens.is_some() || usage.output_tokens.is_some(), "codex usage missing: {usage:?}");
    assert_seq(&h.rec, &id);
    h.mgr.stop(&id).unwrap();
    assert!(h.registry.owner(Agent::Codex, &thread).is_none());

    let s = find_session(&thread);
    log!("listed: cwd={} title={:?}", s.cwd, s.title);
    assert_eq!(norm(&s.cwd), norm(&dir.to_string_lossy()));
    let rollout = std::fs::read_to_string(&s.path).unwrap();
    assert!(rollout.contains(r#""effort":"low""#), "effort low not in the rollout's turn context");

    let mut r = o.clone();
    r.resume_id = Some(thread.clone());
    r.prompt = Some("What token did I ask you to reply with earlier in this thread? Reply with just the token.".into());
    let t0 = Instant::now();
    let rid = h.mgr.start(r.clone()).unwrap().live_id;
    let mut again = r.clone();
    again.prompt = None;
    assert_eq!(h.mgr.start(again).unwrap().live_id, rid, "codex registry dedupe");
    let mut po = pty_opts(PtyTool::Codex, &dir);
    po.resume_id = Some(thread.clone());
    assert_eq!(h.pty.open(&po).err(), Some(format!("busy_live:{rid}")));
    let (text, usage, _) = h.rec.turn(&rid, 0).unwrap();
    log!("resumed: {} | {}", text.trim(), cost_line(&usage, t0.elapsed().as_millis()));
    assert!(text.contains(&tok), "resumed reply {text:?}");
    assert_eq!(native_of(&h.rec, &rid).as_deref(), Some(thread.as_str()));
    h.mgr.stop(&rid).unwrap();

    println!("[S9] rename Codex thread");
    let title = format!("[E2E] codex renamed {}", token("T"));
    rename_session_impl(&AgentPaths::detect(), &data_dir(), Tool::Codex, &s.path, &title).unwrap();
    assert_eq!(find_session(&thread).title.as_deref(), Some(title.as_str()));
    let names = crate::convert::codex::read_session_names(&AgentPaths::detect().codex_home);
    assert_eq!(names.get(&thread).map(String::as_str), Some(title.as_str()), "session_index.jsonl");
    log!("renamed to {title:?}");
}

/// 5 for Codex: interrupt a long turn.
#[test]
#[ignore]
fn e2e_09_codex_interrupt() {
    println!("[S5c] Codex interrupt");
    let h = harness();
    let dir = test_dir();
    let mut o = opts(Agent::Codex, &dir, "[E2E] s5c Count from 1 to 300, one number per line, no other text. Do not run any commands.");
    o.effort = Some("low".into());
    let id = h.mgr.start(o).unwrap().live_id;
    let i = h.rec.wait(&id, 0, TURN, |e| matches!(e, LiveEvent::AssistantDelta { .. })).unwrap();
    let t0 = Instant::now();
    h.mgr.interrupt(&id).unwrap();
    let r = h.rec.wait(&id, i, Duration::from_secs(60), |e| matches!(e, LiveEvent::TurnCompleted { .. }));
    let ms = t0.elapsed().as_millis();
    let evs = h.rec.of(&id);
    for e in evs.iter().skip(i).filter(|e| !matches!(e.event, LiveEvent::AssistantDelta { .. })) {
        log!("  #{} {}", e.seq, short(&e.event));
    }
    let done = r.unwrap();
    log!("TurnCompleted {ms} ms after interrupt; {} chars", assistant_text(&evs[i..=done]).len());
    assert!(!evs[i..=done].iter().any(|e| matches!(e.event, LiveEvent::Error { .. } | LiveEvent::TurnCompleted { is_error: true, .. })));
    assert_eq!(wait_state(&h.mgr, &id, LiveState::Idle, Duration::from_secs(3)), LiveState::Idle);
    let tok = token("GOLF");
    let from = h.rec.count(&id);
    h.mgr.send(&id, &format!("Stop counting. Reply with exactly: {tok}"), &[]).unwrap();
    let (text, _, _) = h.rec.turn(&id, from).unwrap();
    log!("after interrupt: {}", text.trim());
    assert!(text.contains(&tok));
    assert_seq(&h.rec, &id);
}

/// 10: terminals.
#[test]
#[ignore]
fn e2e_10_pty() {
    println!("[S10] PTY");
    let h = harness();
    let dir = test_dir();
    let t0 = Instant::now();
    let sh = h.pty.open(&pty_opts(PtyTool::Shell, &dir)).unwrap();
    log!("shell: {}", sh.command);
    // Let the prompt come up before typing.
    let _ = h.prec.wait(&h.pty, &sh.pty_id, Duration::from_secs(20), |o, _| o.contains('>'));
    h.pty.write(&sh.pty_id, "echo e2e-ok\r").unwrap();
    let out = h
        .prec
        .wait(&h.pty, &sh.pty_id, Duration::from_secs(30), |o, _| o.matches("e2e-ok").count() >= 2)
        .unwrap();
    // Type the next command only once the next prompt is shown (like a person would).
    let _ = h.prec.wait(&h.pty, &sh.pty_id, Duration::from_secs(10), |o, _| {
        o.rsplit("e2e-ok").next().is_some_and(|rest| rest.contains('>'))
    });
    log!("echo seen after {} ms ({} bytes)", t0.elapsed().as_millis(), out.len());
    h.pty.write(&sh.pty_id, "exit\r").unwrap();
    h.prec.wait(&h.pty, &sh.pty_id, Duration::from_secs(20), |_, ex| ex).unwrap();
    assert!(!h.pty.is_open(&sh.pty_id));
    log!("shell exit: {:?}", h.prec.exited(&sh.pty_id));

    for (name, want) in [("claude", "Claude Code"), ("codex", "codex-cli")] {
        let program = find_program(OsStr::new(name), &path_dirs()).unwrap();
        // Through the registry like a resumed terminal: released when it exits.
        let fake = uuid::Uuid::new_v4().to_string();
        let agent = if name == "claude" { Agent::Claude } else { Agent::Codex };
        let pty_id = uuid::Uuid::new_v4().to_string();
        assert!(h.pty.claim(agent, &fake, &pty_id, name).unwrap().is_none());
        let o = pty_opts(PtyTool::Shell, &dir);
        h.pty
            .open_program_as(&pty_id, &program, &["--version".to_string()], &o, &path_dirs(), name)
            .unwrap();
        let out = h.prec.wait(&h.pty, &pty_id, Duration::from_secs(30), |_, ex| ex).unwrap();
        assert!(out.contains(want), "{name} --version output {out:?}");
        let ex = h.prec.exited(&pty_id).unwrap();
        log!("{name} --version: {:?}, exit {}", want, ex["code"]);
        assert_eq!(ex["code"], 0);
        assert!(h.registry.owner(agent, &fake).is_none(), "registry not released after pty exit");
    }
}

/// 11: worktree flow with a real Claude edit, squash merge, conflict.
#[test]
#[ignore]
fn e2e_11_worktree() {
    println!("[S11] worktree");
    use crate::worktree;
    let h = harness();
    let repo = demo_repo();
    let dd = data_dir();
    // Leftovers of an earlier failed run.
    for w in worktree::list(&dd).unwrap() {
        if norm(&w.repo_root) == norm(&repo.to_string_lossy()) {
            worktree::remove(&dd, &w.path, true).unwrap();
        }
    }
    let main_branch = git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let wt = worktree::create(&dd, &repo, "修复登录问题").unwrap();
    log!("created {} on {} (base {})", wt.path, wt.branch, wt.base_branch);
    assert!(wt.slug.starts_with("session-"), "slug {}", wt.slug);
    assert_eq!(wt.base_branch, main_branch);
    let line = format!("line2-{}", token("e2e").to_lowercase());
    let t0 = Instant::now();
    let id = h
        .mgr
        .start(opts(
            Agent::Claude,
            Path::new(&wt.path),
            &format!("[E2E] s11 In app.txt, replace the whole second line (whatever it says) with exactly `{line}` using the Edit tool. Do not ask, do nothing else, then reply DONE."),
        ))
        .unwrap()
        .live_id;
    let (text, usage, _) = h.rec.turn(&id, 0).unwrap();
    log!("agent: {} | {}", text.trim(), cost_line(&usage, t0.elapsed().as_millis()));
    // Like the UI's `beforeRemove`: stop, then merge + delete right away
    // (Windows refuses to delete a folder that is a live process's cwd).
    h.mgr.stop(&id).unwrap();
    let st = worktree::status(&dd, &wt.path).unwrap();
    log!("status: {:?}", st);
    assert!(st.files.iter().any(|f| f.path == "app.txt" && f.status == "M"), "status {st:?}");
    let d = worktree::diff(&dd, &wt.path, None).unwrap();
    assert!(d.contains(&format!("+{line}")), "diff {d}");
    let m = worktree::merge(&dd, &wt.path, "squash", "E2E squash", true).unwrap();
    log!("merge: {m:?}");
    assert!(m.merged && m.commit.is_some() && m.conflicts.is_empty());
    let main_txt = std::fs::read_to_string(repo.join("app.txt")).unwrap();
    assert!(main_txt.contains(&line), "main app.txt {main_txt}");
    assert!(!Path::new(&wt.path).exists(), "worktree folder still there");
    assert!(git(&repo, &["branch", "--list", &wt.branch]).is_empty(), "branch not deleted");

    println!("[S11b] conflict");
    let wt2 = worktree::create(&dd, &repo, "conflict test").unwrap();
    assert!(wt2.slug.starts_with("conflict-test"), "slug {}", wt2.slug);
    let p2 = Path::new(&wt2.path).join("app.txt");
    let txt = std::fs::read_to_string(&p2).unwrap();
    std::fs::write(&p2, txt.replacen("line1", "line1-from-worktree", 1)).unwrap();
    let txt = std::fs::read_to_string(repo.join("app.txt")).unwrap();
    std::fs::write(repo.join("app.txt"), txt.replacen("line1", "line1-from-main", 1)).unwrap();
    git(&repo, &["commit", "-q", "-am", "main edit"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let before = std::fs::read_to_string(repo.join("app.txt")).unwrap();
    let m = worktree::merge(&dd, &wt2.path, "squash", "E2E conflict", true).unwrap();
    log!("conflict merge: {m:?}");
    assert!(!m.merged && m.conflicts == vec!["app.txt".to_string()], "{m:?}");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
    assert_eq!(std::fs::read_to_string(repo.join("app.txt")).unwrap(), before, "main changed by a failed merge");
    assert!(git(&repo, &["status", "--porcelain", "--untracked-files=no"]).is_empty(), "main left dirty");
    assert!(Path::new(&wt2.path).exists(), "worktree removed despite the conflict");
    worktree::remove(&dd, &wt2.path, true).unwrap();
    assert!(!Path::new(&wt2.path).exists());
    assert!(git(&repo, &["branch", "--list", &wt2.branch]).is_empty());
}

/// 12: model / command catalogs and project files.
#[test]
#[ignore]
fn e2e_12_models_files() {
    println!("[S12] models / commands / project files");
    let codex_home = AgentPaths::detect().codex_home;
    for agent in [Agent::Claude, Agent::Codex] {
        let t0 = Instant::now();
        let ms = live::models::models(agent, &codex_home, true).unwrap();
        let cs = live::models::commands(agent, &codex_home, false).unwrap();
        log!(
            "{agent:?}: {} models ({:?}), {} commands, {} ms",
            ms.len(),
            ms.iter().map(|m| format!("{}{:?}", m.value, m.efforts)).collect::<Vec<_>>(),
            cs.len(),
            t0.elapsed().as_millis()
        );
        assert!(!ms.is_empty() && !cs.is_empty());
        assert!(ms.iter().any(|m| !m.efforts.is_empty()), "{agent:?}: no model with efforts");
    }
    let repo = demo_repo();
    let found = crate::project_files::search(&repo, "app", 50).unwrap();
    log!("project_files(app): {:?}", found.iter().map(|f| &f.path).collect::<Vec<_>>());
    assert!(found.iter().any(|f| f.path == "app.txt"));
    let all = crate::project_files::list(&repo).unwrap();
    assert!(all.iter().any(|f| f.path == "README.md"));
    assert!(!all.iter().any(|f| f.path.starts_with(".memdock")), "managed worktrees listed");
}
