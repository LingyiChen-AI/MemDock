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
        self.calls.lock().unwrap().sent.push(text.to_string());
        Ok(())
    }
    fn respond(&mut self, request_id: &str, decision: Decision) -> Result<(), String> {
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
    mgr: LiveManager,
    calls: Arc<StdMutex<Calls>>,
    emits: Arc<StdMutex<Vec<Emit>>>,
    published: Arc<StdMutex<Vec<LiveEnvelope>>>,
}

fn harness() -> Harness {
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
    Harness { mgr: LiveManager::new(spawner, publish), calls, emits, published }
}

fn opts(cwd: &str) -> StartOptions {
    StartOptions {
        agent: Agent::Claude,
        cwd: cwd.into(),
        model: None,
        effort: None,
        permission: PermissionMode::Ask,
        resume_id: None,
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
