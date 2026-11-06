//! Command-level sync tests: fake GitHub API, in-memory token store and a
//! local bare repository (`file://`). No network, no keychain, no real home.

use super::*;
use crate::paths::AgentPaths;
use crate::skills::git::testutil::git;
use crate::sync::engine::SYNC_AUTHOR;
use crate::sync::repo::TreeBlob;
use crate::sync::token::MemoryTokenStore;
use std::collections::VecDeque;
use std::fs;
use tempfile::TempDir;

const GLOBAL: &str = "global/claude/CLAUDE.md";
const CODEX: &str = "global/codex/AGENTS.md";

/// Token store that counts reads (status must never read the keychain).
#[derive(Default)]
struct CountingTokens {
    inner: MemoryTokenStore,
    gets: std::sync::atomic::AtomicUsize,
}

impl TokenStore for CountingTokens {
    fn get(&self) -> Result<Option<String>, String> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        self.inner.get()
    }
    fn set(&self, t: &str) -> Result<(), String> {
        self.inner.set(t)
    }
    fn clear(&self) -> Result<(), String> {
        self.inner.clear()
    }
}

#[derive(Default)]
struct FakeGitHub {
    login: Mutex<Option<String>>,
    polls: Mutex<VecDeque<PollResult>>,
    repo: Mutex<Option<GhRepo>>,
    marker: Mutex<Option<Vec<u8>>>,
    unauthorized: AtomicBool,
    calls: Mutex<Vec<String>>,
}

impl FakeGitHub {
    fn log(&self, s: &str) {
        self.calls.lock().unwrap().push(s.to_string());
    }
    fn auth(&self) -> Result<(), ApiError> {
        if self.unauthorized.load(Ordering::SeqCst) {
            Err(ApiError::Unauthorized)
        } else {
            Ok(())
        }
    }
    fn called(&self, s: &str) -> bool {
        self.calls.lock().unwrap().iter().any(|c| c == s)
    }
}

impl GitHubApi for FakeGitHub {
    fn device_code(&self, _: &str) -> Result<DeviceCode, ApiError> {
        self.log("device_code");
        Ok(DeviceCode {
            device_code: "dc".into(),
            user_code: "ABCD-1234".into(),
            verification_uri: VERIFICATION_URL.into(),
            interval: 5,
            expires_in: 900,
        })
    }
    fn poll_token(&self, _: &str, _: &str) -> Result<PollResult, ApiError> {
        self.log("poll");
        Ok(self
            .polls
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(PollResult::Token("tok-1".into())))
    }
    fn user(&self, _: &str) -> Result<GhUser, ApiError> {
        self.auth()?;
        Ok(GhUser {
            login: self.login.lock().unwrap().clone().unwrap_or_else(|| "octo".into()),
        })
    }
    fn get_repo(&self, _: &str, _: &str, _: &str) -> Result<Option<GhRepo>, ApiError> {
        self.auth()?;
        self.log("get_repo");
        Ok(self.repo.lock().unwrap().clone())
    }
    fn create_repo(&self, _: &str, name: &str) -> Result<GhRepo, ApiError> {
        self.auth()?;
        self.log("create_repo");
        let r = GhRepo {
            private: true,
            default_branch: "main".into(),
            clone_url: String::new(),
            description: Some("MemDock memory sync".into()),
            full_name: format!("octo/{name}"),
        };
        *self.repo.lock().unwrap() = Some(r.clone());
        Ok(r)
    }
    fn get_file(&self, _: &str, _: &str, _: &str, _: &str) -> Result<Option<Vec<u8>>, ApiError> {
        self.auth()?;
        Ok(self.marker.lock().unwrap().clone())
    }
    fn put_file(
        &self,
        _: &str,
        _: &str,
        _: &str,
        p: &str,
        b: &[u8],
        _: &str,
    ) -> Result<(), ApiError> {
        self.auth()?;
        self.log(&format!("put_file {p}"));
        *self.marker.lock().unwrap() = Some(b.to_vec());
        Ok(())
    }
    fn list_root(&self, _: &str, _: &str, _: &str) -> Result<Vec<String>, ApiError> {
        self.auth()?;
        Ok(vec![])
    }
}

type Events = Arc<Mutex<Vec<(String, Value)>>>;

struct Env {
    t: TempDir,
    bare: PathBuf,
    url: String,
    api: Arc<FakeGitHub>,
    tokens: Arc<CountingTokens>,
    events: Events,
    state: Arc<AppState>,
}

fn env() -> Env {
    let t = tempfile::tempdir().unwrap();
    let bare = t.path().join("remote.git");
    fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "-q", "--bare", "--initial-branch=main"]);
    let url = format!("file:///{}", bare.to_string_lossy().replace('\\', "/"));
    let home = t.path().join("home");
    let paths = AgentPaths::from_home(&home);
    fs::create_dir_all(&paths.claude_home).unwrap();
    fs::create_dir_all(&paths.codex_home).unwrap();
    let mut st = AppState::with_home(paths, t.path().join("data"), home);
    let api = Arc::new(FakeGitHub::default());
    let tokens = Arc::new(CountingTokens::default());
    let mut rt = SyncRuntime::new(api.clone(), tokens.clone(), SyncGit::new());
    let u = url.clone();
    rt.clone_url = Box::new(move |_, _| u.clone());
    // Short real sleeps: polling stays fast but a cancel can win the race.
    rt.sleep = Arc::new(|d| std::thread::sleep(d.min(Duration::from_millis(5))));
    rt.status_debounce = Duration::from_millis(50);
    let events: Events = Arc::default();
    let ev = events.clone();
    rt.set_emitter(Arc::new(move |n, v| ev.lock().unwrap().push((n.to_string(), v))));
    st.sync = rt;
    Env {
        t,
        bare,
        url,
        api,
        tokens,
        events,
        state: Arc::new(st),
    }
}

impl Env {
    fn login(&self) -> DeviceCodeDto {
        let (dto, h) = login_start_impl(&self.state).unwrap();
        h.join().unwrap();
        dto
    }

    fn global(&self) -> PathBuf {
        self.state.paths.claude_home.join("CLAUDE.md")
    }

    fn sync_state(&self) -> SyncState {
        load(&self.state).unwrap()
    }

    fn events_named(&self, name: &str) -> Vec<Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .collect()
    }

    fn head(&self) -> String {
        git(&self.bare, &["rev-parse", "main"])
    }

    fn remote_file(&self, path: &str) -> Option<String> {
        SyncGit::new()
            .read_at(&self.bare, "main", path)
            .unwrap()
            .map(|b| String::from_utf8(b).unwrap())
    }

    /// Commits on top of `main` as another machine would.
    fn push_remote(&self, changes: &[(&str, &str)]) {
        let raw: Vec<(&str, Vec<u8>)> =
            changes.iter().map(|(p, c)| (*p, c.as_bytes().to_vec())).collect();
        self.push_bytes(&raw);
    }

    fn push_bytes(&self, changes: &[(&str, Vec<u8>)]) {
        let g = SyncGit::new();
        let seed = self.t.path().join("seed");
        if !seed.join(".git").exists() {
            g.clone(&self.url, &seed, &None).unwrap();
        }
        let head = g.fetch(&seed, &None).unwrap();
        let mut tree: BTreeMap<String, TreeBlob> = match &head {
            Some(h) => g
                .tree_entries(&seed, h)
                .unwrap()
                .into_iter()
                .map(|(p, e)| (p, TreeBlob::Existing(e)))
                .collect(),
            None => BTreeMap::new(),
        };
        for (p, c) in changes {
            tree.insert(p.to_string(), TreeBlob::Data(c.clone()));
        }
        let c = g
            .commit_tree_blobs(&seed, head.as_deref(), &tree, "other", SYNC_AUTHOR)
            .unwrap();
        g.push(&seed, &c, "main", &None).unwrap();
    }

    fn done(&self) -> SyncSummaryDto {
        match sync_start_impl(&self.state).unwrap() {
            SyncStartDto::Done { summary, .. } => summary,
            other => panic!("expected done, got {other:?}"),
        }
    }
}

fn write(p: &Path, s: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, s).unwrap();
}

#[test]
fn login_flow_creates_repo_and_clones() {
    let e = env();
    *e.api.polls.lock().unwrap() = VecDeque::from([PollResult::Pending, PollResult::Pending]);
    let st0 = sync_status_impl(&e.state);
    assert!(!st0.logged_in);
    assert!(st0.repo.is_none());

    let dto = e.login();
    assert_eq!(dto.user_code, "ABCD-1234");
    assert_eq!(dto.verification_uri, VERIFICATION_URL);
    assert!(e.api.called("create_repo"));
    assert!(e.api.called("put_file memdock.json"));
    assert_eq!(e.tokens.get().unwrap().as_deref(), Some("tok-1"));
    let s = e.sync_state();
    assert_eq!(s.login.as_deref(), Some("octo"));
    assert_eq!(s.repo_owner.as_deref(), Some("octo"));
    assert_eq!(s.repo_name, "MemDockHub");
    assert_eq!(s.clone_of.as_deref(), Some("octo/MemDockHub"));
    assert_eq!(s.branch.as_deref(), Some("main"));
    assert!(e.state.data_dir.join("sync/repo/.git").exists());
    // The token never lands in state.json.
    let raw = fs::read_to_string(e.state.data_dir.join("sync/state.json")).unwrap();
    assert!(!raw.contains("tok-1"));

    let login = e.events_named(LOGIN_EVENT);
    assert_eq!(login.len(), 1);
    assert_eq!(login[0]["status"], "ok");
    assert!(login[0].get("needs").is_none());
    assert!(!login[0].to_string().contains("tok-1"));
    assert!(!e.events_named(STATUS_EVENT).is_empty());

    let st = sync_status_impl(&e.state);
    assert!(st.logged_in);
    assert_eq!(st.login.as_deref(), Some("octo"));
    let repo = st.repo.unwrap();
    assert_eq!(repo.url, "https://github.com/octo/MemDockHub");
    assert_eq!(repo.private, Some(true));
    assert_eq!(st.needs, None);
}

#[test]
fn login_cancel_and_public_repo_need_ack() {
    let e = env();
    // Cancel: the login thread stops at the next sleep slice.
    *e.api.polls.lock().unwrap() = VecDeque::from(vec![PollResult::Pending; 1000]);
    let (_, h) = login_start_impl(&e.state).unwrap();
    login_cancel_impl(&e.state);
    h.join().unwrap();
    let ev = e.events_named(LOGIN_EVENT);
    assert_eq!(ev.last().unwrap()["status"], "cancelled");
    assert_eq!(e.tokens.get().unwrap(), None);

    // A public MemDock repository needs explicit confirmation.
    e.api.polls.lock().unwrap().clear();
    *e.api.repo.lock().unwrap() = Some(GhRepo {
        private: false,
        default_branch: "main".into(),
        clone_url: String::new(),
        description: None,
        full_name: "octo/MemDockHub".into(),
    });
    *e.api.marker.lock().unwrap() = Some(br#"{"format":1,"app":"MemDock"}"#.to_vec());
    e.login();
    let ev = e.events_named(LOGIN_EVENT);
    assert_eq!(ev.last().unwrap()["needs"], "public_ack");
    assert_eq!(sync_status_impl(&e.state).needs.as_deref(), Some("public_ack"));
    assert_eq!(sync_start_impl(&e.state).unwrap_err().code, "public_not_acked");
    let st = ack_public_repo_impl(&e.state).unwrap();
    assert_eq!(st.needs, None);
    assert_eq!(st.repo.unwrap().private, Some(false));
    assert!(e.state.data_dir.join("sync/repo/.git").exists());
}

#[test]
fn not_ours_then_rename() {
    let e = env();
    *e.api.repo.lock().unwrap() = Some(GhRepo {
        private: true,
        default_branch: "main".into(),
        clone_url: String::new(),
        description: Some("someone else's".into()),
        full_name: "octo/MemDockHub".into(),
    });
    e.login();
    assert_eq!(e.events_named(LOGIN_EVENT).last().unwrap()["needs"], "not_ours");
    assert_eq!(sync_start_impl(&e.state).unwrap_err().code, "repo_not_ours");
    assert_eq!(set_repo_name_impl(&e.state, "bad name").unwrap_err().code, "invalid");
    assert_eq!(set_repo_name_impl(&e.state, "..").unwrap_err().code, "invalid");
    *e.api.repo.lock().unwrap() = None;
    let st = set_repo_name_impl(&e.state, "My.Memories_2").unwrap();
    assert_eq!(st.needs, None);
    assert_eq!(st.repo.unwrap().name, "My.Memories_2");
    assert_eq!(e.sync_state().clone_of.as_deref(), Some("octo/My.Memories_2"));
}

#[test]
fn unauthorized_clears_token() {
    let e = env();
    e.login();
    // Force a repository check on the next sync, which GitHub now refuses.
    let mut s = e.sync_state();
    s.clone_of = None;
    store(&e.state, &s).unwrap();
    e.api.unauthorized.store(true, Ordering::SeqCst);
    let err = sync_start_impl(&e.state).unwrap_err();
    assert_eq!(err.code, "unauthorized");
    assert_eq!(e.tokens.get().unwrap(), None);
    let s = e.sync_state();
    assert_eq!(s.login, None);
    // Clone and base survive.
    assert!(e.state.data_dir.join("sync/repo/.git").exists());
    assert_eq!(e.events_named(LOGIN_EVENT).last().unwrap()["status"], "relogin");
    let st = sync_status_impl(&e.state);
    assert!(!st.logged_in);
    assert_eq!(st.needs.as_deref(), Some("relogin"));
    assert_eq!(sync_start_impl(&e.state).unwrap_err().code, "not_logged_in");
}

#[test]
fn logout_keeps_clone_and_base() {
    let e = env();
    e.login();
    write(&e.global(), "g\n");
    e.done();
    let commit = e.sync_state().last_sync_commit.clone();
    assert!(commit.is_some());
    let st = logout_impl(&e.state).unwrap();
    assert!(!st.logged_in);
    assert_eq!(e.tokens.get().unwrap(), None);
    let s = e.sync_state();
    assert_eq!(s.login, None);
    assert_eq!(s.last_sync_commit, commit);
    assert!(e.state.data_dir.join("sync/repo/.git").exists());
}

#[test]
fn sync_start_done_without_conflicts() {
    let e = env();
    e.login();
    assert_eq!(sync_status_impl(&e.state).pending, 0);
    write(&e.global(), "hello\n");
    e.state.rescan();
    assert_eq!(sync_status_impl(&e.state).pending, 1);
    e.events.lock().unwrap().clear();

    let sum = e.done();
    assert!(sum.committed);
    assert_eq!(sum.pushed, 1);
    assert_eq!(sum.commit.as_deref(), Some(e.head().as_str()));
    assert_eq!(e.remote_file(GLOBAL).as_deref(), Some("hello\n"));
    let s = e.sync_state();
    assert_eq!(s.last_sync_commit.as_deref(), Some(e.head().as_str()));
    assert!(s.last_sync_at.is_some());
    let st = e.events_named(STATUS_EVENT);
    assert_eq!(st.last().unwrap()["pending"], 0);
    assert_eq!(sync_status_impl(&e.state).pending, 0);

    // Remote-only change is pulled straight in.
    e.push_remote(&[(CODEX, "codex\n")]);
    let sum = e.done();
    assert_eq!(sum.pulled, 1);
    assert!(!sum.committed);
    let codex = e.state.paths.codex_home.join("AGENTS.md");
    assert_eq!(fs::read_to_string(&codex).unwrap().replace("\r\n", "\n"), "codex\n");
    let mem = e.events_named(MEMORY_EVENT);
    let id = normalize_path(&codex.to_string_lossy());
    assert!(mem.iter().any(|m| m["paths"].as_array().unwrap().iter().any(|p| p == &id)));

    let hist = sync_history_impl(&e.state, 10).unwrap();
    assert_eq!(hist.len(), 2);
}

#[test]
fn secrets_block_sync() {
    let e = env();
    e.login();
    write(&e.global(), "key = AKIAIOSFODNN7EXAMPLE\n");
    match sync_start_impl(&e.state).unwrap() {
        SyncStartDto::Blocked { secret_hits } => {
            assert!(!secret_hits.is_empty());
            assert_eq!(secret_hits[0].repo_path, GLOBAL);
        }
        other => panic!("expected blocked, got {other:?}"),
    }
    // Nothing was pushed: the remote is still empty.
    assert_eq!(git(&e.bare, &["branch", "--list"]), "");
    assert_eq!(e.sync_state().last_sync_commit, None);
}

#[test]
fn secrets_can_be_skipped_or_allowed() {
    let e = env();
    e.login();
    write(&e.global(), "key = AKIAIOSFODNN7EXAMPLE
");
    match sync_start_with(&e.state, "skip").unwrap() {
        SyncStartDto::Done { skipped, .. } => {
            assert!(skipped.iter().any(|s| s.repo_path == GLOBAL && s.reason == "secret"), "{skipped:?}");
        }
        other => panic!("expected done, got {other:?}"),
    }
    // The skipped file is not on the remote; allowing it pushes it.
    assert!(!git(&e.bare, &["ls-tree", "-r", "--name-only", "HEAD"]).contains(GLOBAL));
    match sync_start_with(&e.state, "allow").unwrap() {
        SyncStartDto::Done { .. } => {}
        other => panic!("expected done, got {other:?}"),
    }
    assert!(git(&e.bare, &["ls-tree", "-r", "--name-only", "HEAD"]).contains(GLOBAL));
}

fn conflict_setup(e: &Env) -> SyncPlanDto {
    e.login();
    write(&e.global(), "l1\nl2\nl3\nl4\nl5\n");
    e.done();
    e.push_remote(&[(GLOBAL, "R1\nl2\nl3\nl4\nl5\n")]);
    write(&e.global(), "L1\nl2\nl3\nl4\nl5\n");
    match sync_start_impl(&e.state).unwrap() {
        SyncStartDto::NeedsResolution { plan } => plan,
        other => panic!("expected needs_resolution, got {other:?}"),
    }
}

#[test]
fn sync_start_needs_resolution_then_finish() {
    let e = env();
    let plan = conflict_setup(&e);
    assert_eq!(plan.counts.conflicts, 1);
    let c = &plan.conflicts[0];
    assert_eq!(c.repo_path, GLOBAL);
    assert_eq!(c.local.as_deref(), Some("L1\nl2\nl3\nl4\nl5\n"));
    assert_eq!(c.remote.as_deref(), Some("R1\nl2\nl3\nl4\nl5\n"));
    assert!(c.conflicts >= 1);
    // The DTO carries no internal plan state.
    let v = serde_json::to_value(&plan).unwrap();
    assert!(v.get("entries").is_none());

    // Unresolved: the plan stays available.
    let err = sync_finish_impl(&e.state, &plan.id, HashMap::new()).unwrap_err();
    assert_eq!(err.code, "unresolved");
    assert_eq!(err.paths.as_deref(), Some(&[GLOBAL.to_string()][..]));

    let res: HashMap<String, ResolutionDto> = serde_json::from_value(serde_json::json!({
        GLOBAL: { "kind": "text", "text": "M1\nl2\nl3\nl4\nl5\n" }
    }))
    .unwrap();
    let sum = sync_finish_impl(&e.state, &plan.id, res).unwrap();
    assert!(sum.committed);
    assert_eq!(sum.merged, 1);
    assert_eq!(e.remote_file(GLOBAL).as_deref(), Some("M1\nl2\nl3\nl4\nl5\n"));
    assert_eq!(fs::read_to_string(e.global()).unwrap(), "M1\nl2\nl3\nl4\nl5\n");
    assert_eq!(e.sync_state().last_sync_commit.as_deref(), Some(e.head().as_str()));
    // The plan is consumed.
    let err = sync_finish_impl(&e.state, &plan.id, HashMap::new()).unwrap_err();
    assert_eq!(err.code, "plan_not_found");
}

#[test]
fn unknown_plan_id_is_plan_not_found() {
    let e = env();
    let plan = conflict_setup(&e);
    let err = sync_finish_impl(&e.state, "nope", HashMap::new()).unwrap_err();
    assert_eq!(err.code, "plan_not_found");
    sync_discard_impl(&e.state, &plan.id);
    let err = sync_finish_impl(&e.state, &plan.id, HashMap::new()).unwrap_err();
    assert_eq!(err.code, "plan_not_found");
}

#[test]
fn base_persisted_on_remote_moved() {
    let e = env();
    let plan = conflict_setup(&e);
    let head1 = e.head();
    let base0 = e.sync_state().last_sync_commit.clone();
    assert_ne!(base0.as_deref(), Some(head1.as_str()));
    // Local adds a new codex file; another machine pushes a different one
    // before the user finishes: the retry finds a new conflict.
    write(&e.state.paths.codex_home.join("AGENTS.md"), "mine\n");
    e.push_remote(&[(CODEX, "theirs\n")]);
    let head2 = e.head();
    e.events.lock().unwrap().clear();
    let res: HashMap<String, ResolutionDto> = serde_json::from_value(serde_json::json!({
        GLOBAL: { "kind": "text", "text": "M1\nl2\nl3\nl4\nl5\n" }
    }))
    .unwrap();
    let err = sync_finish_impl(&e.state, &plan.id, res).unwrap_err();
    assert_eq!(err.code, "remote_moved");
    assert_eq!(e.head(), head2);
    // Local files were written, so the base moved to the plan's remote head.
    assert_eq!(e.sync_state().last_sync_commit.as_deref(), Some(head1.as_str()));
    assert_eq!(fs::read_to_string(e.global()).unwrap(), "M1\nl2\nl3\nl4\nl5\n");
    assert!(!e.events_named(STATUS_EVENT).is_empty());
}

#[test]
fn second_sync_while_busy_rejected() {
    let e = env();
    e.login();
    let g = e.state.sync.lock.lock().unwrap();
    assert_eq!(sync_start_impl(&e.state).unwrap_err().code, "sync_busy");
    assert_eq!(sync_finish_impl(&e.state, "x", HashMap::new()).unwrap_err().code, "sync_busy");
    assert_eq!(set_included_local_impl(&e.state, vec![]).unwrap_err().code, "sync_busy");
    assert_eq!(logout_impl(&e.state).unwrap_err().code, "sync_busy");
    // Status never waits for the sync lock.
    assert!(sync_status_impl(&e.state).logged_in);
    drop(g);
    e.done();
}

#[test]
fn restore_from_history_writes_snapshot() {
    let e = env();
    e.login();
    write(&e.global(), "one\n");
    let c1 = e.done().commit.unwrap();
    write(&e.global(), "two\n");
    e.done();
    let hist = sync_history_impl(&e.state, 10).unwrap();
    assert_eq!(hist.len(), 2);
    assert_eq!(hist[1].id, c1);
    assert_eq!(sync_file_at_impl(&e.state, &c1, GLOBAL).unwrap().as_deref(), Some("one\n"));
    assert_eq!(sync_file_at_impl(&e.state, &c1, CODEX).unwrap(), None);
    assert_eq!(sync_file_at_impl(&e.state, "--x", GLOBAL).unwrap_err().code, "invalid");

    restore_from_history_impl(&e.state, &c1, GLOBAL).unwrap();
    assert_eq!(fs::read_to_string(e.global()).unwrap(), "one\n");
    let snaps = e.state.snapshots.list(&e.global()).unwrap();
    let texts: Vec<Vec<u8>> = snaps
        .iter()
        .map(|s| e.state.snapshots.read(&e.global(), &s.id).unwrap())
        .collect();
    assert!(texts.iter().any(|t| t == b"two\n"), "{texts:?}");
    // A local change now, pushed by the next sync.
    assert_eq!(sync_status_impl(&e.state).pending, 1);
    assert!(!e.events_named(MEMORY_EVENT).is_empty());
    assert_eq!(e.done().pushed, 1);
    assert_eq!(e.remote_file(GLOBAL).as_deref(), Some("one\n"));
}

#[test]
fn projects_and_included_local() {
    let e = env();
    let proj = e.t.path().join("work").join("plain");
    fs::create_dir_all(&proj).unwrap();
    write(&proj.join("CLAUDE.md"), "p\n");
    crate::commands::add_manual_project_impl(&e.state, &proj.to_string_lossy()).unwrap();
    let list = sync_projects_impl(&e.state).unwrap();
    let p = list.iter().find(|p| p.name == "plain").unwrap();
    assert_eq!(p.key, None);
    assert!(!p.included);
    let list = set_included_local_impl(&e.state, vec![p.dir_key.clone()]).unwrap();
    assert!(list.iter().find(|p| p.name == "plain").unwrap().included);
    assert_eq!(e.sync_state().included_local, vec![p.dir_key.clone()]);
}

#[test]
fn memory_change_emits_debounced_status() {
    let e = env();
    start_status_watch(&e.state);
    e.state.sync.last_pending.store(0, Ordering::SeqCst);
    write(&e.global(), "x\n");
    e.state.rescan();
    for _ in 0..5 {
        e.state.sync.memory_changed();
    }
    let mut got = Vec::new();
    for _ in 0..100 {
        got = e.events_named(STATUS_EVENT);
        if !got.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["pending"], 1);
    // No change in pending -> no new event.
    e.state.sync.memory_changed();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(e.events_named(STATUS_EVENT).len(), 1);
}

#[test]
fn not_logged_in_errors() {
    let e = env();
    assert_eq!(sync_start_impl(&e.state).unwrap_err().code, "not_logged_in");
    assert!(sync_history_impl(&e.state, 5).unwrap().is_empty());
}

fn set_public_marked_repo(e: &Env, owner: &str) {
    *e.api.repo.lock().unwrap() = Some(GhRepo {
        private: false,
        default_branch: "main".into(),
        clone_url: String::new(),
        description: None,
        full_name: format!("{owner}/MemDockHub"),
    });
    *e.api.marker.lock().unwrap() = Some(br#"{"format":1,"app":"MemDock"}"#.to_vec());
}

#[test]
fn done_reports_skipped_and_missing_skills() {
    let e = env();
    e.login();
    let skills = r#"{"format":1,"skills":[{"name":"demo","url":"https://github.com/me/skills","skill_path":"demo","tool":"claude","scope":{"type":"global"},"installed_on":["other-machine"]}]}"#;
    e.push_bytes(&[
        (CODEX, vec![0xff, 0xfe, 0x00, 0x41]),
        ("skills.json", skills.as_bytes().to_vec()),
    ]);
    match sync_start_impl(&e.state).unwrap() {
        SyncStartDto::Done {
            skipped,
            missing_skills,
            ..
        } => {
            assert!(skipped.iter().any(|s| s.repo_path == CODEX), "{skipped:?}");
            assert_eq!(missing_skills.len(), 1);
            assert_eq!(missing_skills[0].name, "demo");
        }
        other => panic!("expected done, got {other:?}"),
    }
    let v = serde_json::to_value(sync_start_impl(&e.state).unwrap()).unwrap();
    assert_eq!(v["kind"], "done");
    assert!(v["skipped"].is_array() && v["missing_skills"].is_array());
    assert!(v["removed_elsewhere"].is_array());
}

#[test]
fn account_switch_requires_new_public_ack() {
    let e = env();
    set_public_marked_repo(&e, "octo");
    e.login();
    ack_public_repo_impl(&e.state).unwrap();
    assert_eq!(e.sync_state().public_ack_for.as_deref(), Some("octo/MemDockHub"));
    e.done();
    logout_impl(&e.state).unwrap();
    assert_eq!(e.sync_state().public_ack_for, None);

    // Another account with its own public MemDockHub must confirm again.
    *e.api.login.lock().unwrap() = Some("other".into());
    set_public_marked_repo(&e, "other");
    e.login();
    assert_eq!(e.events_named(LOGIN_EVENT).last().unwrap()["needs"], "public_ack");
    assert_eq!(sync_start_impl(&e.state).unwrap_err().code, "public_not_acked");
    ack_public_repo_impl(&e.state).unwrap();
    assert_eq!(e.sync_state().public_ack_for.as_deref(), Some("other/MemDockHub"));
}

#[test]
fn repo_made_public_after_binding_requires_ack() {
    let e = env();
    e.login();
    write(&e.global(), "one\n");
    e.done();
    let head = e.head();
    // Someone flips the repository to public on GitHub.
    e.api.repo.lock().unwrap().as_mut().unwrap().private = false;
    write(&e.global(), "two\n");
    assert_eq!(sync_start_impl(&e.state).unwrap_err().code, "public_not_acked");
    assert_eq!(e.head(), head, "nothing synced");
    assert_eq!(fs::read_to_string(e.global()).unwrap(), "two\n");
    let st = sync_status_impl(&e.state);
    assert_eq!(st.needs.as_deref(), Some("public_ack"));
    assert_eq!(st.repo.unwrap().private, Some(false));
    ack_public_repo_impl(&e.state).unwrap();
    assert_eq!(e.done().pushed, 1);
}

#[test]
fn status_never_reads_the_keychain() {
    let e = env();
    e.login();
    start_status_watch(&e.state);
    e.tokens.gets.store(0, Ordering::SeqCst);
    let st = sync_status_impl(&e.state);
    assert!(st.logged_in);
    emit_status(&e.state);
    write(&e.global(), "x\n");
    e.state.sync.memory_changed();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(e.tokens.gets.load(Ordering::SeqCst), 0);
    // Logout updates the cache without a read.
    logout_impl(&e.state).unwrap();
    assert!(!sync_status_impl(&e.state).logged_in);
    assert_eq!(e.tokens.gets.load(Ordering::SeqCst), 0);
}

#[test]
fn git_unauthorized_after_local_writes_relogs_and_keeps_base() {
    let e = env();
    let plan = conflict_setup(&e);
    let head1 = e.head();
    // The remote refuses the push as GitHub does for a revoked token.
    let hook = e.bare.join("hooks").join("pre-receive");
    fs::write(&hook, "#!/bin/sh\necho 'fatal: Authentication failed' >&2\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let res: HashMap<String, ResolutionDto> = serde_json::from_value(serde_json::json!({
        GLOBAL: { "kind": "text", "text": "M1\nl2\nl3\nl4\nl5\n" }
    }))
    .unwrap();
    let err = sync_finish_impl(&e.state, &plan.id, res).unwrap_err();
    assert_eq!(err.code, "unauthorized", "{err:?}");
    assert_eq!(e.tokens.get().unwrap(), None);
    assert_eq!(e.events_named(LOGIN_EVENT).last().unwrap()["status"], "relogin");
    let s = e.sync_state();
    assert_eq!(s.login, None);
    assert_eq!(s.last_sync_commit.as_deref(), Some(head1.as_str()));
    assert_eq!(fs::read_to_string(e.global()).unwrap(), "M1\nl2\nl3\nl4\nl5\n");
}

#[test]
fn finish_keeps_plan_on_pre_write_errors() {
    let e = env();
    let plan = conflict_setup(&e);
    e.tokens.inner.clear().unwrap();
    let res = || -> HashMap<String, ResolutionDto> {
        serde_json::from_value(serde_json::json!({
            GLOBAL: { "kind": "text", "text": "M1\nl2\nl3\nl4\nl5\n" }
        }))
        .unwrap()
    };
    assert_eq!(sync_finish_impl(&e.state, &plan.id, res()).unwrap_err().code, "not_logged_in");
    e.tokens.inner.set("tok-1").unwrap();
    let sum = sync_finish_impl(&e.state, &plan.id, res()).unwrap();
    assert!(sum.committed);
}

#[test]
fn skipped_secret_files_are_recorded_until_synced() {
    let e = env();
    e.login();
    write(&e.global(), "key = AKIAIOSFODNN7EXAMPLE\n");
    match sync_start_with(&e.state, "skip").unwrap() {
        SyncStartDto::Done { .. } => {}
        other => panic!("expected done, got {other:?}"),
    }
    let recs = sync_skipped_files_impl(&e.state).unwrap();
    assert_eq!(recs.len(), 1, "{recs:?}");
    assert_eq!(recs[0].repo_path, GLOBAL);
    assert_eq!(recs[0].reason, "secret");
    assert!(recs[0].at > 0);
    let local = normalize_path(&e.global().to_string_lossy());
    assert_eq!(recs[0].local_path.as_deref().map(normalize_path), Some(local.clone()));
    assert_eq!(recs[0].file_id.as_deref(), Some(local.as_str()));
    // persisted
    assert!(e.sync_state().skipped_files.contains_key(GLOBAL));

    // a download does not upload it: still recorded
    match sync_start_dir(&e.state, "block", "down").unwrap() {
        SyncStartDto::Done { download, .. } => assert!(download),
        other => panic!("expected done, got {other:?}"),
    }
    assert_eq!(sync_skipped_files_impl(&e.state).unwrap().len(), 1);

    // the secret is removed and the file uploaded: the record goes away
    write(&e.global(), "clean\n");
    e.state.rescan();
    e.done();
    assert!(sync_skipped_files_impl(&e.state).unwrap().is_empty());
    assert!(e.sync_state().skipped_files.is_empty());
}

#[test]
fn download_to_new_machine_then_upload_is_clean() {
    let e = env();
    e.push_remote(&[(GLOBAL, "global from cloud\n"), (CODEX, "codex from cloud\n")]);
    e.login();
    let st = sync_status_impl(&e.state);
    assert!(st.remote_has_memory, "a fresh machine with cloud memory is offered a download");
    let head = e.head();
    e.events.lock().unwrap().clear();

    match sync_start_dir(&e.state, "block", "down").unwrap() {
        SyncStartDto::Done {
            summary,
            download,
            kept_local,
            absent,
            ..
        } => {
            assert!(download);
            assert_eq!(summary.pulled, 2);
            assert_eq!(summary.pushed, 0);
            assert!(!summary.committed);
            assert!(kept_local.is_empty());
            assert!(absent.is_empty());
        }
        other => panic!("expected done, got {other:?}"),
    }
    assert_eq!(e.head(), head, "a download pushes nothing");
    assert_eq!(
        fs::read_to_string(e.global()).unwrap().replace("\r\n", "\n"),
        "global from cloud\n"
    );
    let codex = e.state.paths.codex_home.join("AGENTS.md");
    assert_eq!(fs::read_to_string(&codex).unwrap().replace("\r\n", "\n"), "codex from cloud\n");
    let mem = e.events_named(MEMORY_EVENT);
    let id = normalize_path(&codex.to_string_lossy());
    assert!(mem.iter().any(|m| m["paths"].as_array().unwrap().iter().any(|p| p == &id)));
    let st = sync_status_impl(&e.state);
    assert!(!st.remote_has_memory);
    assert_eq!(st.pending, 0);
    assert_eq!(e.sync_state().last_sync_commit.as_deref(), Some(head.as_str()));

    // uploading right after: nothing to push, no conflicts, nothing deleted
    let sum = e.done();
    assert_eq!(sum.pushed, 0);
    assert_eq!(sum.pulled, 0);
    assert_eq!(e.remote_file(GLOBAL).as_deref(), Some("global from cloud\n"));
    assert_eq!(e.remote_file(CODEX).as_deref(), Some("codex from cloud\n"));
}

#[test]
fn download_conflict_is_resolved_locally_and_not_pushed() {
    let e = env();
    e.push_remote(&[(GLOBAL, "cloud\n")]);
    e.login();
    write(&e.global(), "local\n");
    e.state.rescan();
    let head = e.head();
    let plan = match sync_start_dir(&e.state, "block", "down").unwrap() {
        SyncStartDto::NeedsResolution { plan } => plan,
        other => panic!("expected resolution, got {other:?}"),
    };
    assert!(plan.download);
    assert_eq!(plan.conflicts.len(), 1);
    let res = HashMap::from([(
        GLOBAL.to_string(),
        ResolutionDto::Text {
            text: "local and cloud\n".into(),
        },
    )]);
    let sum = sync_finish_impl(&e.state, &plan.id, res).unwrap();
    assert!(!sum.committed);
    assert_eq!(e.head(), head);
    assert_eq!(
        fs::read_to_string(e.global()).unwrap().replace("\r\n", "\n"),
        "local and cloud\n"
    );
    // the next upload pushes the resolved text without asking again
    let sum = e.done();
    assert_eq!(sum.pushed, 1);
    assert_eq!(e.remote_file(GLOBAL).as_deref(), Some("local and cloud\n"));
}

#[test]
fn invalid_direction_is_rejected() {
    let e = env();
    e.login();
    let err = sync_start_dir(&e.state, "block", "sideways").unwrap_err();
    assert_eq!(err.code, "invalid");
}

// ------------------------------------------------------------ preferences

const PREFS: &str = "memdock_settings.json";

fn frontend_prefs() -> Value {
    serde_json::json!({
        "memdock.chat.prefs": {"agent": "codex", "cwd": "C:\\codes\\private"},
        "memdock.diffLayout": "unified",
        "memdock.treeWidth": "320"
    })
}

#[test]
fn prefs_upload_then_download_roundtrip() {
    let e = env();
    e.login();
    crate::commands::update_ui_prefs_impl(&e.state, None, Some("light".into()), Some("blue".into())).unwrap();
    let info = prefs_sync_info_impl(&e.state).unwrap();
    assert!(info.with_memory);
    assert!(info.remote.is_none());

    let up = prefs_upload_impl(&e.state, frontend_prefs()).unwrap();
    assert!(up.committed);
    let text = e.remote_file(PREFS).unwrap();
    assert!(text.contains("\"theme\": \"light\""));
    assert!(text.contains("\"memdock.diffLayout\": \"unified\""));
    assert!(!text.contains("private"), "no local paths: {text}");
    assert!(!text.contains("treeWidth"));
    assert!(!text.contains("tok-1"), "no token");
    let msg = git(&e.bare, &["log", "-1", "--format=%s", "main"]);
    assert_eq!(msg, "MemDock: update preferences");
    let info = up.info;
    assert!(info.remote.is_some());
    assert!(!info.newer, "this machine's own upload is not newer");

    // unchanged preferences: no second commit
    let head = e.head();
    let again = prefs_upload_impl(&e.state, frontend_prefs()).unwrap();
    assert!(!again.committed);
    assert_eq!(e.head(), head);

    // another machine writes newer preferences
    let newer = text
        .replace("\"light\"", "\"dark\"")
        .replace(&prefs::parse(&text).unwrap().updated_at, "2099-01-01T00:00:00Z");
    e.push_remote(&[(PREFS, newer.as_str())]);
    let f = prefs_download_impl(&e.state).unwrap();
    assert_eq!(f.backend.theme.as_deref(), Some("dark"));
    assert_eq!(f.frontend["memdock.diffLayout"], "unified");
    assert!(prefs_sync_info_impl(&e.state).unwrap().newer);
    let info = prefs_mark_applied_impl(&e.state, &f.updated_at).unwrap();
    assert!(!info.newer);
    assert!(prefs_mark_applied_impl(&e.state, "yesterday").is_err());
}

#[test]
fn prefs_download_errors() {
    let e = env();
    e.login();
    write(&e.global(), "g\n");
    e.done();
    assert_eq!(prefs_download_impl(&e.state).unwrap_err().code, "prefs_missing");
    e.push_remote(&[(PREFS, "{\"version\": 9}")]);
    assert_eq!(prefs_download_impl(&e.state).unwrap_err().code, "prefs_too_new");
    e.push_remote(&[(PREFS, "nope")]);
    assert_eq!(prefs_download_impl(&e.state).unwrap_err().code, "prefs_invalid");
}

#[test]
fn memory_upload_carries_preferences_when_switched_on() {
    let e = env();
    e.login();
    write(&e.global(), "g\n");
    e.state.rescan();
    // no frontend part sent: the file is not written
    e.done();
    assert!(e.remote_file(PREFS).is_none());
    // with the frontend part: written in the same sync commit
    write(&e.global(), "g2\n");
    match sync_start_full(&e.state, "block", "up", Some(frontend_prefs())).unwrap() {
        SyncStartDto::Done { summary, .. } => assert_eq!(summary.pushed, 1),
        other => panic!("expected done, got {other:?}"),
    }
    assert!(e.remote_file(PREFS).unwrap().contains("unified"));
    let msg = git(&e.bare, &["log", "-1", "--format=%s", "main"]);
    assert!(msg.starts_with("sync from"));
    assert!(e.sync_state().prefs_seen.is_some());
    // a download never writes it
    let head = e.head();
    sync_start_full(&e.state, "block", "down", Some(serde_json::json!({"memdock.diffLayout": "split"}))).unwrap();
    assert_eq!(e.head(), head);
    // switched off: left as it is
    let info = prefs_set_with_memory_impl(&e.state, false).unwrap();
    assert!(!info.with_memory);
    write(&e.global(), "g3\n");
    sync_start_full(&e.state, "block", "up", Some(serde_json::json!({"memdock.diffLayout": "split"}))).unwrap();
    assert!(e.remote_file(PREFS).unwrap().contains("unified"));
    assert_eq!(e.remote_file(GLOBAL).as_deref(), Some("g3\n"));
}
