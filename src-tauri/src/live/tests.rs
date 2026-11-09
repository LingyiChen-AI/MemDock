use super::*;
use std::sync::Mutex as StdMutex;

#[derive(Default)]
struct Calls {
    sent: Vec<String>,
    responded: Vec<(String, Decision)>,
    interrupted: usize,
    stopped: usize,
}

struct FakeDriver {
    calls: Arc<StdMutex<Calls>>,
    emit: Emit,
}

impl Driver for FakeDriver {
    fn send(&mut self, text: &str, _images: &[String]) -> Result<(), String> {
        match text {
            "slow" => std::thread::sleep(std::time::Duration::from_millis(1500)),
            "fail" => return Err("pipe closed".into()),
            // The turn finishes before `send` returns.
            "fast-turn" => (self.emit)(LiveEvent::TurnCompleted { usage: TurnUsage::default(), is_error: false }),
            _ => {}
        }
        self.calls.lock().unwrap().sent.push(text.to_string());
        Ok(())
    }
    fn respond(&mut self, request_id: &str, decision: Decision) -> Result<(), String> {
        if request_id == "r-fast" {
            (self.emit)(LiveEvent::TurnCompleted { usage: TurnUsage::default(), is_error: false });
        }
        self.calls.lock().unwrap().responded.push((request_id.to_string(), decision));
        Ok(())
    }
    fn interrupt(&mut self) -> Result<(), String> {
        self.calls.lock().unwrap().interrupted += 1;
        Ok(())
    }
    fn stop(&mut self) {
        self.calls.lock().unwrap().stopped += 1;
        (self.emit)(LiveEvent::Exited { code: Some(0) });
    }
}

struct Harness {
    mgr: Arc<LiveManager>,
    calls: Arc<StdMutex<Calls>>,
    emits: Arc<StdMutex<Vec<Emit>>>,
    published: Arc<StdMutex<Vec<LiveEnvelope>>>,
}

fn harness() -> Harness {
    harness_with(Limits::default())
}

fn harness_with(limits: Limits) -> Harness {
    let calls = Arc::new(StdMutex::new(Calls::default()));
    let emits: Arc<StdMutex<Vec<Emit>>> = Arc::new(StdMutex::new(vec![]));
    let published = Arc::new(StdMutex::new(vec![]));
    let (c, e) = (calls.clone(), emits.clone());
    let spawner: Spawner = Arc::new(move |_opts: &StartOptions, emit: Emit| {
        e.lock().unwrap().push(emit.clone());
        Ok(Box::new(FakeDriver { calls: c.clone(), emit }) as Box<dyn Driver>)
    });
    let p = published.clone();
    let publish: Publish = Arc::new(move |env: &LiveEnvelope| p.lock().unwrap().push(env.clone()));
    Harness { mgr: Arc::new(LiveManager::new(spawner, publish).with_limits(limits)), calls, emits, published }
}

fn opts(cwd: &str) -> StartOptions {
    StartOptions {
        agent: Agent::Claude,
        cwd: cwd.into(),
        model: None,
        effort: None,
        permission: PermissionMode::Ask,
        resume_id: None,
        fork_from: None,
        extra_args: vec![],
        prompt: None,
        prompt_images: vec![],
    }
}

fn states(h: &Harness) -> Vec<LiveState> {
    h.published
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e.event {
            LiveEvent::Status { state } => Some(state),
            _ => None,
        })
        .collect()
}

#[test]
fn start_send_and_state_flow() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness();
    let mut o = opts(dir.path().to_str().unwrap());
    o.prompt = Some("\n  修一下登录页的按钮\n第二行".into());
    let info = h.mgr.start(o).unwrap();
    assert_eq!(info.title, "修一下登录页的按钮");
    assert_eq!(h.calls.lock().unwrap().sent, vec!["\n  修一下登录页的按钮\n第二行"]);
    let emit = h.emits.lock().unwrap()[0].clone();
    emit(LiveEvent::Started { native_id: "sess-1".into(), model: Some("opus".into()) });
    emit(LiveEvent::ApprovalRequest {
        tool_id: None,
        request_id: "r1".into(),
        tool_name: "Bash".into(),
        kind: ToolKind::Shell,
        summary: "ls".into(),
        detail: "ls".into(),
        can_allow_session: true,
    });
    h.mgr.respond(&info.live_id, "r1", Decision::Allow).unwrap();
    emit(LiveEvent::TurnCompleted { usage: TurnUsage::default(), is_error: false });
    assert_eq!(
        states(&h),
        vec![LiveState::Running, LiveState::WaitingApproval, LiveState::Running, LiveState::Idle]
    );
    let list = h.mgr.list();
    assert_eq!(list[0].native_id.as_deref(), Some("sess-1"));
    assert_eq!(list[0].model.as_deref(), Some("opus"));
    assert_eq!(list[0].state, LiveState::Idle);
    assert_eq!(h.calls.lock().unwrap().responded, vec![("r1".to_string(), Decision::Allow)]);
    // Transcript replays everything with increasing seq.
    let t = h.mgr.transcript(&info.live_id).unwrap();
    assert_eq!(t.len(), h.published.lock().unwrap().len());
    assert!(t.windows(2).all(|w| w[0].seq < w[1].seq));
}

#[test]
fn followups_wait_for_the_current_turn_and_keep_transcript_order() {
    for agent in [Agent::Claude, Agent::Codex] {
        let dir = tempfile::tempdir().unwrap();
        let h = harness();
        let mut options = opts(dir.path().to_str().unwrap());
        options.agent = agent;
        options.prompt = Some("first".into());
        let info = h.mgr.start(options).unwrap();
        let emit = h.emits.lock().unwrap()[0].clone();

        h.mgr.send(&info.live_id, "second", &[]).unwrap();
        h.mgr.send(&info.live_id, "fast-turn", &[]).unwrap();
        assert_eq!(h.calls.lock().unwrap().sent, ["first"]);
        emit(delta("a1", "first reply"));
        emit(LiveEvent::TurnCompleted { usage: TurnUsage::default(), is_error: false });
        wait_for_sent(&h, 2);
        assert_eq!(h.calls.lock().unwrap().sent, ["first", "second"]);
        emit(delta("a2", "second reply"));
        emit(LiveEvent::TurnCompleted { usage: TurnUsage::default(), is_error: false });
        wait_for_sent(&h, 3);

        let ordered: Vec<_> = h.mgr.transcript(&info.live_id).unwrap().into_iter().filter_map(|env| {
            match env.event {
                LiveEvent::UserMessage { text, .. } | LiveEvent::AssistantDelta { text, .. } => Some(text),
                _ => None,
            }
        }).collect();
        assert_eq!(ordered, ["first", "first reply", "second", "second reply", "fast-turn"]);
    }
}

fn wait_for_sent(h: &Harness, count: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while h.calls.lock().unwrap().sent.len() < count {
        assert!(std::time::Instant::now() < deadline, "queued message was not dispatched");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn stop_marks_exited_and_rejects_further_sends() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness();
    let info = h.mgr.start(opts(dir.path().to_str().unwrap())).unwrap();
    h.mgr.interrupt(&info.live_id).unwrap();
    h.mgr.stop(&info.live_id).unwrap();
    assert_eq!(h.calls.lock().unwrap().interrupted, 1);
    assert_eq!(h.calls.lock().unwrap().stopped, 1);
    assert_eq!(h.mgr.list()[0].state, LiveState::Exited);
    let exits = h.published.lock().unwrap().iter().filter(|e| matches!(e.event, LiveEvent::Exited { .. })).count();
    assert_eq!(exits, 1, "the driver's own Exited after stop() is dropped");
    assert!(h.mgr.send(&info.live_id, "hi", &[]).is_err());
    h.mgr.close(&info.live_id).unwrap();
    assert!(h.mgr.list().is_empty());
}

#[test]
fn missing_dir_and_spawn_failure_are_errors() {
    let h = harness();
    assert!(h.mgr.start(opts("Z:/definitely/not/here")).is_err());
    let failing: Spawner = Arc::new(|_: &StartOptions, _: Emit| Err("boom".to_string()));
    let mgr = LiveManager::new(failing, Arc::new(|_: &LiveEnvelope| {}));
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(mgr.start(opts(dir.path().to_str().unwrap())).unwrap_err(), "boom");
    assert_eq!(mgr.list()[0].state, LiveState::Exited);
}

#[test]
fn image_files_become_data_uris() {
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("Shot.PNG");
    std::fs::write(&png, b"ABC").unwrap();
    assert_eq!(image_data_uri(&png).unwrap(), "data:image/png;base64,QUJD");
    let txt = dir.path().join("a.txt");
    std::fs::write(&txt, b"x").unwrap();
    assert!(image_data_uri(&txt).is_err());
    assert_eq!(image_media_type(std::path::Path::new("a.jpeg")), Some("image/jpeg"));
    let big = dir.path().join("big.webp");
    std::fs::write(&big, vec![0u8; (MAX_IMAGE_BYTES + 1) as usize]).unwrap();
    assert!(image_data_uri(&big).unwrap_err().contains("5 MB"));
}

#[test]
fn data_uris_are_checked_before_sending() {
    assert_eq!(decode_data_uri("data:image/png;base64,AAAA"), Some(("image/png", "AAAA")));
    assert_eq!(decode_data_uri("data:text/plain;base64,AAAA"), None);
    assert_eq!(decode_data_uri("data:image/png,AAAA"), None);
    assert_eq!(decode_data_uri("C:/x.png"), None);
    let dir = tempfile::tempdir().unwrap();
    let h = harness();
    let info = h.mgr.start(opts(dir.path().to_str().unwrap())).unwrap();
    assert!(h.mgr.send(&info.live_id, "see", &["not-an-image".into()]).is_err());
    assert!(h.calls.lock().unwrap().sent.is_empty());
    h.mgr.send(&info.live_id, "see", &["data:image/png;base64,AAAA".into()]).unwrap();
    let last = h.published.lock().unwrap().iter().rev().find_map(|e| match &e.event {
        LiveEvent::UserMessage { images, .. } => Some(images.clone()),
        _ => None,
    });
    assert_eq!(last, Some(vec!["data:image/png;base64,AAAA".to_string()]));
}

// ---- session isolation ------------------------------------------------------

fn state_of(h: &Harness, live_id: &str) -> LiveState {
    h.mgr.list().into_iter().find(|i| i.live_id == live_id).unwrap().state
}

fn delta(id: &str, text: &str) -> LiveEvent {
    LiveEvent::AssistantDelta { id: id.into(), text: text.into() }
}

#[test]
fn second_start_with_same_resume_id_reuses_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness();
    let mut o = opts(dir.path().to_str().unwrap());
    o.resume_id = Some("sess-r".into());
    o.prompt = Some("a".into());
    let first = h.mgr.start(o.clone()).unwrap();
    assert_eq!(h.mgr.registry().owner(Agent::Claude, "sess-r"), Some(Owner::live(&first.live_id)));
    o.prompt = Some("b".into());
    let again = h.mgr.start(o.clone()).unwrap();
    assert_eq!(again.live_id, first.live_id);
    assert_eq!(h.emits.lock().unwrap().len(), 1, "no second process");
    assert_eq!(h.calls.lock().unwrap().sent, ["a"]);
    let emit = h.emits.lock().unwrap()[0].clone();
    emit(LiveEvent::TurnCompleted { usage: TurnUsage::default(), is_error: false });
    wait_for_sent(&h, 2);
    assert_eq!(h.calls.lock().unwrap().sent, ["a", "b"], "the next turn uses the same session");
    // Released after the process ends; the next start is a new process.
    h.mgr.stop(&first.live_id).unwrap();
    assert_eq!(h.mgr.registry().owner(Agent::Claude, "sess-r"), None);
    let third = h.mgr.start(o).unwrap();
    assert_ne!(third.live_id, first.live_id);
    assert_eq!(h.emits.lock().unwrap().len(), 2);
    h.mgr.close(&third.live_id).unwrap();
    assert_eq!(h.mgr.registry().owner(Agent::Claude, "sess-r"), None);
}

#[test]
fn live_start_on_a_terminal_owned_id_is_busy_pty() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness();
    h.mgr.registry().claim(Agent::Codex, "thr-1", Owner::pty("p1")).unwrap();
    let mut o = opts(dir.path().to_str().unwrap());
    o.agent = Agent::Codex;
    o.resume_id = Some(" thr-1 ".into());
    assert_eq!(h.mgr.start(o.clone()).unwrap_err(), "busy_pty:p1");
    assert!(h.mgr.list().is_empty());
    assert!(h.emits.lock().unwrap().is_empty());
    // Same id for the other agent is a different session.
    o.agent = Agent::Claude;
    assert!(h.mgr.start(o).is_ok());
}

#[test]
fn started_registers_the_native_id_and_exit_releases_it() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness();
    let info = h.mgr.start(opts(dir.path().to_str().unwrap())).unwrap();
    let emit = h.emits.lock().unwrap()[0].clone();
    emit(LiveEvent::Started { native_id: "n-1".into(), model: None });
    assert_eq!(h.mgr.registry().owner(Agent::Claude, "n-1"), Some(Owner::live(&info.live_id)));
    // A fork reports another id: the old one is released.
    emit(LiveEvent::Started { native_id: "n-2".into(), model: None });
    assert_eq!(h.mgr.registry().owner(Agent::Claude, "n-1"), None);
    assert_eq!(h.mgr.registry().owner(Agent::Claude, "n-2"), Some(Owner::live(&info.live_id)));
    // The process dies on its own.
    emit(LiveEvent::Exited { code: Some(1) });
    assert_eq!(h.mgr.registry().owner(Agent::Claude, "n-2"), None);
    assert_eq!(state_of(&h, &info.live_id), LiveState::Exited);
}

#[test]
fn publish_order_is_seq_order_under_concurrency() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness();
    let info = h.mgr.start(opts(dir.path().to_str().unwrap())).unwrap();
    let emit = h.emits.lock().unwrap()[0].clone();
    emit(LiveEvent::Started { native_id: "s".into(), model: None });
    let mut handles = vec![];
    for t in 0..8 {
        let emit = emit.clone();
        handles.push(std::thread::spawn(move || {
            for i in 0..200 {
                emit(delta(&format!("m{t}"), &i.to_string()));
            }
        }));
    }
    for _ in 0..2 {
        let (mgr, id) = (h.mgr.clone(), info.live_id.clone());
        handles.push(std::thread::spawn(move || {
            for _ in 0..50 {
                mgr.send(&id, "fast-turn", &[]).unwrap();
            }
        }));
    }
    for th in handles {
        th.join().unwrap();
    }
    let seqs: Vec<u64> = h.published.lock().unwrap().iter().map(|e| e.seq).collect();
    assert!(seqs.len() > 1700);
    assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "published in seq order without gaps");
}

#[test]
fn slow_driver_io_does_not_block_other_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness();
    let a = h.mgr.start(opts(dir.path().to_str().unwrap())).unwrap();
    let b = h.mgr.start(opts(dir.path().to_str().unwrap())).unwrap();
    let emit_a = h.emits.lock().unwrap()[0].clone();
    let emit_b = h.emits.lock().unwrap()[1].clone();
    let (mgr, id) = (h.mgr.clone(), a.live_id.clone());
    let slow = std::thread::spawn(move || mgr.send(&id, "slow", &[]));
    std::thread::sleep(std::time::Duration::from_millis(150));
    let t0 = std::time::Instant::now();
    emit_b(delta("x", "hello"));
    h.mgr.send(&b.live_id, "fast", &[]).unwrap();
    let _ = h.mgr.list();
    let _ = h.mgr.transcript(&b.live_id).unwrap();
    // The slow session itself still records its reader's events.
    emit_a(delta("y", "still streaming"));
    let _ = h.mgr.transcript(&a.live_id).unwrap();
    let took = t0.elapsed();
    assert!(took < std::time::Duration::from_millis(500), "blocked for {took:?}");
    slow.join().unwrap().unwrap();
}

#[test]
fn user_message_and_approval_are_recorded_before_the_driver_write() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness();
    let info = h.mgr.start(opts(dir.path().to_str().unwrap())).unwrap();
    let emit = h.emits.lock().unwrap()[0].clone();
    emit(LiveEvent::Started { native_id: "s".into(), model: None });
    // The turn completes inside `send`: it must come after the user message.
    h.mgr.send(&info.live_id, "fast-turn", &[]).unwrap();
    assert_eq!(state_of(&h, &info.live_id), LiveState::Idle);
    emit(LiveEvent::UserMessage { id: "u".into(), text: "x".into(), images: vec![] });
    emit(LiveEvent::ApprovalRequest {
        tool_id: None,
        request_id: "r-fast".into(),
        tool_name: "Bash".into(),
        kind: ToolKind::Shell,
        summary: "ls".into(),
        detail: "ls".into(),
        can_allow_session: false,
    });
    h.mgr.respond(&info.live_id, "r-fast", Decision::Allow).unwrap();
    assert_eq!(state_of(&h, &info.live_id), LiveState::Idle, "TurnCompleted right after ApprovalResolved");
    let kinds: Vec<String> = h
        .published
        .lock()
        .unwrap()
        .iter()
        .map(|e| serde_json::to_value(&e.event).unwrap()["type"].as_str().unwrap().to_string())
        .collect();
    let pos = |k: &str| kinds.iter().position(|x| x == k).unwrap();
    assert!(pos("user_message") < pos("turn_completed"));
    let resolved = pos("approval_resolved");
    assert!(kinds[resolved..].contains(&"turn_completed".to_string()));
    // A late answer to an approval doesn't move an idle session back to running.
    h.mgr.respond(&info.live_id, "r-old", Decision::Deny).unwrap();
    assert_eq!(state_of(&h, &info.live_id), LiveState::Idle);
}

#[test]
fn failed_write_rolls_back_with_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness();
    let info = h.mgr.start(opts(dir.path().to_str().unwrap())).unwrap();
    let emit = h.emits.lock().unwrap()[0].clone();
    emit(LiveEvent::Started { native_id: "s".into(), model: None });
    assert!(h.mgr.send(&info.live_id, "fail", &[]).is_err());
    assert_eq!(state_of(&h, &info.live_id), LiveState::Idle);
    let last = h.published.lock().unwrap().iter().rev().find_map(|e| match &e.event {
        LiveEvent::Error { message, fatal } => Some((message.clone(), *fatal)),
        _ => None,
    });
    assert!(matches!(last, Some((m, false)) if m.contains("pipe closed")));
}

#[test]
fn turn_completed_compacts_deltas_and_tool_output() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness();
    let info = h.mgr.start(opts(dir.path().to_str().unwrap())).unwrap();
    let emit = h.emits.lock().unwrap()[0].clone();
    emit(LiveEvent::Started { native_id: "s".into(), model: None });
    h.mgr.send(&info.live_id, "q", &["data:image/png;base64,AAAA".into()]).unwrap();
    for t in ["Hel", "lo", "!"] {
        emit(delta("m1", t));
    }
    emit(LiveEvent::ToolStarted { id: "t1".into(), name: "Bash".into(), kind: ToolKind::Shell, summary: "ls".into(), input: "ls".into() });
    emit(LiveEvent::ToolOutput { id: "t1".into(), text: "a\n".into() });
    emit(LiveEvent::ToolOutput { id: "t2".into(), text: "kept\n".into() });
    emit(LiveEvent::ToolFinished { id: "t1".into(), output: Some("a\n".into()), is_error: false });
    emit(LiveEvent::ToolFinished { id: "t2".into(), output: None, is_error: false });
    emit(delta("m2", "no final"));
    emit(LiveEvent::AssistantMessage { id: "m1".into(), text: "Hello!".into() });
    emit(LiveEvent::TurnCompleted { usage: TurnUsage::default(), is_error: false });

    let published = h.published.lock().unwrap().clone();
    let t = h.mgr.transcript(&info.live_id).unwrap();
    assert!(t.len() < published.len());
    assert!(t.windows(2).all(|w| w[0].seq < w[1].seq));
    let first_delta = published.iter().find(|e| matches!(&e.event, LiveEvent::AssistantDelta { id, .. } if id == "m1")).unwrap().seq;
    // The final message takes the first delta's place (and seq).
    let m1: Vec<_> = t.iter().filter(|e| matches!(&e.event, LiveEvent::AssistantDelta { id, .. } | LiveEvent::AssistantMessage { id, .. } if id == "m1")).collect();
    assert_eq!(m1.len(), 1);
    assert_eq!(m1[0].seq, first_delta);
    assert!(matches!(&m1[0].event, LiveEvent::AssistantMessage { text, .. } if text == "Hello!"));
    assert!(!t.iter().any(|e| matches!(&e.event, LiveEvent::ToolOutput { id, .. } if id == "t1")));
    assert!(t.iter().any(|e| matches!(&e.event, LiveEvent::ToolOutput { id, .. } if id == "t2")), "no final output: kept");
    assert!(t.iter().any(|e| matches!(&e.event, LiveEvent::AssistantDelta { id, .. } if id == "m2")));
    // Every kept envelope has its original seq.
    for e in &t {
        let p = published.iter().find(|p| p.seq == e.seq).unwrap();
        if !matches!(e.event, LiveEvent::AssistantMessage { .. } | LiveEvent::UserMessage { .. }) {
            assert_eq!(p.event, e.event);
        }
    }
    // Images: data URI to the UI, a temp file reference in the log.
    let pub_img = published.iter().find_map(|e| match &e.event {
        LiveEvent::UserMessage { images, .. } => Some(images.clone()),
        _ => None,
    });
    assert_eq!(pub_img, Some(vec!["data:image/png;base64,AAAA".to_string()]));
    let log_img = t.iter().find_map(|e| match &e.event {
        LiveEvent::UserMessage { images, .. } => Some(images.clone()),
        _ => None,
    });
    let log_img = log_img.unwrap();
    let path = log_img[0].strip_prefix(IMAGE_REF_PREFIX).expect("memdock-image: prefix");
    assert!(std::path::Path::new(path).is_file());
    assert!(image_data_uri(std::path::Path::new(path)).unwrap().ends_with("AAAA"));
}

#[test]
fn log_is_capped_keeping_a_leading_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness_with(Limits { max_events: 10, ..Limits::default() });
    let info = h.mgr.start(opts(dir.path().to_str().unwrap())).unwrap();
    let emit = h.emits.lock().unwrap()[0].clone();
    emit(LiveEvent::Started { native_id: "s".into(), model: None });
    for i in 0..50 {
        emit(delta("m", &i.to_string()));
    }
    let t = h.mgr.transcript(&info.live_id).unwrap();
    assert!(t.len() <= 10, "{}", t.len());
    assert!(matches!(t[0].event, LiveEvent::Started { .. }));
    assert!(matches!(t[1].event, LiveEvent::Status { state: LiveState::Idle }));
    assert_eq!(t.last().unwrap().seq, h.published.lock().unwrap().last().unwrap().seq);
    assert!(t.windows(2).all(|w| w[0].seq < w[1].seq));

    let h = harness_with(Limits { max_bytes: 4000, ..Limits::default() });
    let info = h.mgr.start(opts(dir.path().to_str().unwrap())).unwrap();
    let emit = h.emits.lock().unwrap()[0].clone();
    let big = "x".repeat(1000);
    for _ in 0..20 {
        emit(delta("m", &big));
    }
    let t = h.mgr.transcript(&info.live_id).unwrap();
    let bytes: usize = t.iter().map(|e| serde_json::to_string(&e.event).unwrap().len()).sum();
    assert!(bytes <= 4000, "{bytes}");
    assert!(t.len() >= 3);
}

#[test]
fn idle_reaper_stops_only_idle_unattached_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness_with(Limits { idle_timeout_ms: 1000, ..Limits::default() });
    let cwd = dir.path().to_str().unwrap();
    let idle = h.mgr.start(opts(cwd)).unwrap();
    let attached = h.mgr.start(opts(cwd)).unwrap();
    let running = h.mgr.start(opts(cwd)).unwrap();
    let emits = h.emits.lock().unwrap().clone();
    for e in &emits {
        e(LiveEvent::Started { native_id: uuid::Uuid::new_v4().to_string(), model: None });
    }
    h.mgr.attach(&attached.live_id, true).unwrap();
    h.mgr.send(&running.live_id, "work", &[]).unwrap();
    assert!(h.mgr.reap_idle(now_ms()).is_empty(), "not idle long enough");
    let reaped = h.mgr.reap_idle(now_ms() + 2000);
    assert_eq!(reaped, vec![idle.live_id.clone()]);
    assert_eq!(state_of(&h, &idle.live_id), LiveState::Exited);
    assert_eq!(state_of(&h, &attached.live_id), LiveState::Idle);
    assert_eq!(state_of(&h, &running.live_id), LiveState::Running);
    assert_eq!(h.calls.lock().unwrap().stopped, 1);
    // Detached: reaped on the next pass.
    h.mgr.attach(&attached.live_id, false).unwrap();
    assert_eq!(h.mgr.reap_idle(now_ms() + 2000), vec![attached.live_id.clone()]);
}

#[test]
fn account_names_from_local_login_files() {
    use base64::Engine;
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    assert_eq!(claude_account_name(home), None);
    std::fs::write(home.join(".claude.json"), r#"{"oauthAccount":{"displayName":"  Ming ","fullName":"Li Ming"}}"#).unwrap();
    assert_eq!(claude_account_name(home).as_deref(), Some("Ming"));
    std::fs::write(home.join(".claude.json"), r#"{"oauthAccount":{"displayName":"","fullName":"Li Ming"}}"#).unwrap();
    assert_eq!(claude_account_name(home).as_deref(), Some("Li Ming"));

    let codex = home.join(".codex");
    std::fs::create_dir_all(&codex).unwrap();
    assert_eq!(codex_account_name(&codex), None);
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"name":"Laura","email":"x@y.z"}"#);
    let auth = format!(r#"{{"tokens":{{"id_token":"aaa.{payload}.sig","access_token":"t"}}}}"#);
    std::fs::write(codex.join("auth.json"), auth).unwrap();
    assert_eq!(codex_account_name(&codex).as_deref(), Some("Laura"));
    std::fs::write(codex.join("auth.json"), r#"{"OPENAI_API_KEY":"sk-x","tokens":null}"#).unwrap();
    assert_eq!(codex_account_name(&codex), None);
}
