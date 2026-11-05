use super::*;
use crate::commands::add_manual_project_impl;
use std::path::Path;
use crate::extract::job::TurnSource;
use crate::extract::prompt::{Evidence, Suggestion, SuggestionKind, SuggestionStatus};
use crate::extract::runner::RunError;
use crate::extract::sessions::{Role, Turn};
use crate::extract::store::{ProjectFilter, RunStatus, RunTrigger, Schedule};
use crate::model::Tool;
use crate::paths::AgentPaths;
use serde_json::Value;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Blocks until the cancel flag is set; never touches a real CLI.
#[derive(Default)]
struct BlockingRunner {
    saw_cancel: Arc<AtomicBool>,
}

impl CliRunner for BlockingRunner {
    fn tool(&self) -> Tool {
        Tool::Claude
    }
    fn available(&self) -> bool {
        true
    }
    fn run(
        &self,
        _prompt: &str,
        _schema: &Value,
        _timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<String, RunError> {
        let t0 = Instant::now();
        while !cancel.load(Ordering::SeqCst) {
            if t0.elapsed() > Duration::from_secs(20) {
                return Err(RunError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.saw_cancel.store(true, Ordering::SeqCst);
        Err(RunError::Cancelled)
    }
}

type Events = Arc<Mutex<Vec<(String, Value)>>>;

fn app(tmp: &Path) -> Arc<AppState> {
    let paths = AgentPaths::from_home(&tmp.join("home"));
    fs::create_dir_all(&paths.claude_home).unwrap();
    fs::create_dir_all(&paths.codex_home).unwrap();
    fs::create_dir_all(tmp.join("data")).unwrap();
    Arc::new(AppState::with_home(paths, tmp.join("data"), tmp.join("home")))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn extract_state(app: Arc<AppState>, runners: Vec<Arc<dyn CliRunner>>) -> (ExtractState, Events) {
    let es = ExtractState::build(app, runners, |ctx| {
        let turns: TurnSource = Arc::new(|tool, _since| {
            if tool != Tool::Claude {
                return Vec::new();
            }
            vec![Turn {
                tool: Tool::Claude,
                session_id: "s1".into(),
                project_id: None,
                ts: now_ms() - 1000,
                role: Role::User,
                text: "always use tabs".into(),
            }]
        });
        ctx.turns = turns;
    });
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    es.set_emitter(Arc::new(move |name: &str, payload: Value| {
        sink.lock().unwrap().push((name.to_string(), payload));
    }));
    (es, events)
}

fn names(events: &Events) -> Vec<String> {
    events.lock().unwrap().iter().map(|(n, _)| n.clone()).collect()
}

fn sug(id: &str, path: &str, base_hash: &str, new_text: &str) -> Suggestion {
    Suggestion {
        id: id.into(),
        fingerprint: format!("fp-{id}"),
        kind: SuggestionKind::Add,
        file_id: crate::projects::normalize_path(path),
        path: path.into(),
        tool_of_file: Tool::Claude,
        old_text: None,
        new_text: Some(new_text.into()),
        reason: "r".into(),
        evidence: vec![],
        source_tool: Tool::Claude,
        created_at: 1,
        base_hash: base_hash.into(),
        status: SuggestionStatus::Pending,
    }
}

fn add(es: &ExtractState, s: Suggestion) {
    es.store().unwrap().lock().unwrap().add_suggestions(vec![s]);
}

#[test]
fn start_twice_returns_already_running() {
    let tmp = tempfile::tempdir().unwrap();
    let (es, events) = extract_state(app(tmp.path()), vec![Arc::new(BlockingRunner::default())]);
    let rec = start_extraction_impl(&es).unwrap();
    assert_eq!(rec.status, RunStatus::Running);
    let err = start_extraction_impl(&es).unwrap_err();
    assert_eq!(err.code, "already_running");
    cancel_extraction_impl(&es).unwrap();
    let t0 = Instant::now();
    while !names(&events).contains(&"extract-finished".to_string()) {
        assert!(t0.elapsed() < Duration::from_secs(10), "run did not finish");
        std::thread::sleep(Duration::from_millis(20));
    }
    let runs = list_extract_runs_impl(&es).unwrap();
    assert_eq!(runs[0].id, rec.id);
    assert_eq!(runs[0].status, RunStatus::Cancelled);
    assert!(names(&events).contains(&"suggestions-changed".to_string()));
    // The lock is released afterwards.
    let again = start_extraction_impl(&es).unwrap();
    assert_ne!(again.id, rec.id);
    cancel_extraction_impl(&es).unwrap();
}

#[test]
fn start_without_cli_is_not_installed() {
    let tmp = tempfile::tempdir().unwrap();
    let (es, _) = extract_state(app(tmp.path()), vec![]);
    assert_eq!(start_extraction_impl(&es).unwrap_err().code, "not_installed");
    assert!(list_extract_runs_impl(&es).unwrap().is_empty());
}

#[test]
fn runners_lists_both_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let (es, _) = extract_state(app(tmp.path()), vec![Arc::new(BlockingRunner::default())]);
    let r = extract_runners_impl(&es);
    assert_eq!(r.len(), 2);
    assert!(r.iter().any(|x| x.tool == Tool::Claude && x.available));
    assert!(r.iter().any(|x| x.tool == Tool::Codex && !x.available && x.program.is_none()));
}

#[test]
fn apply_command_updates_summary() {
    let tmp = tempfile::tempdir().unwrap();
    let a = app(tmp.path());
    let file = a.paths.claude_home.join("CLAUDE.md");
    fs::write(&file, "# rules\n").unwrap();
    a.rescan();
    let hash = crate::memory::io::read_memory(&file).unwrap().hash;
    let (es, events) = extract_state(a, vec![]);
    add(&es, sug("a", file.to_str().unwrap(), &hash, "- use tabs"));
    add(&es, sug("b", file.to_str().unwrap(), &hash, "- other"));

    let before = pending_summary_impl(&es).unwrap();
    assert_eq!(before.total, 2);
    assert_eq!(before.by_project.get("__global__"), Some(&2));

    let out = apply_suggestion_impl(&es, "a", None, false).unwrap();
    assert!(matches!(out, ApplyOutcome::Applied { saved: true, .. }));
    assert!(fs::read_to_string(&file).unwrap().contains("- use tabs"));
    assert_eq!(pending_summary_impl(&es).unwrap().total, 1);
    let ev = events.lock().unwrap().clone();
    let last = ev.iter().rev().find(|(n, _)| n == "suggestions-changed").unwrap();
    assert_eq!(last.1["total"], 1);
    assert!(ev.iter().any(|(n, _)| n == "memory-changed"));

    let rej = reject_suggestion_impl(&es, "b");
    assert!(rej.is_ok());
    assert_eq!(pending_summary_impl(&es).unwrap().total, 0);
    assert_eq!(reject_suggestion_impl(&es, "b").unwrap_err().code, "not_pending");
    assert_eq!(reject_suggestion_impl(&es, "zzz").unwrap_err().code, "not_found");
    let accepted = list_suggestions_impl(&es, Some(SuggestionStatus::Accepted)).unwrap();
    assert_eq!(accepted.len(), 1);
    assert_eq!(list_suggestions_impl(&es, None).unwrap().len(), 2);
}

#[test]
fn summary_keys_are_plan1_project_ids() {
    let tmp = tempfile::tempdir().unwrap();
    let a = app(tmp.path());
    let proj = tmp.path().join("work").join("proj");
    fs::create_dir_all(&proj).unwrap();
    fs::write(proj.join("CLAUDE.md"), "x\n").unwrap();
    add_manual_project_impl(&a, proj.to_str().unwrap()).unwrap();
    let pid = a.snapshot_index().projects[0].project.id.clone();
    let (es, _) = extract_state(a, vec![]);
    add(&es, sug("p", proj.join("CLAUDE.md").to_str().unwrap(), "h", "n"));
    let s = pending_summary_impl(&es).unwrap();
    assert_eq!(s.by_project.get(&pid), Some(&1), "{:?}", s.by_project);
}

#[test]
fn accept_many_reports_each() {
    let tmp = tempfile::tempdir().unwrap();
    let a = app(tmp.path());
    let file = a.paths.claude_home.join("CLAUDE.md");
    fs::write(&file, "# rules\n").unwrap();
    a.rescan();
    let hash = crate::memory::io::read_memory(&file).unwrap().hash;
    let (es, _) = extract_state(a, vec![]);
    add(&es, sug("a", file.to_str().unwrap(), &hash, "- one"));
    let res = accept_suggestions_impl(&es, &["a".into(), "nope".into()]).unwrap();
    assert_eq!(res.len(), 2);
    assert!(matches!(res[0].outcome, Some(ApplyOutcome::Applied { .. })));
    assert_eq!(res[1].error.as_ref().unwrap().code, "not_found");
}

#[test]
fn config_validation() {
    let tmp = tempfile::tempdir().unwrap();
    let (es, _) = extract_state(app(tmp.path()), vec![]);
    let ok = get_extract_config_impl(&es).unwrap();
    let bad = |f: &dyn Fn(&mut ExtractConfig)| {
        let mut c = ok.clone();
        f(&mut c);
        set_extract_config_impl(&es, c).unwrap_err().code
    };
    assert_eq!(bad(&|c| c.lookback_days = 0), "invalid");
    assert_eq!(bad(&|c| c.lookback_days = 91), "invalid");
    assert_eq!(bad(&|c| c.schedule = Schedule::Daily { hh: 24, mm: 0 }), "invalid");
    assert_eq!(bad(&|c| c.schedule = Schedule::Daily { hh: 1, mm: 60 }), "invalid");
    assert_eq!(bad(&|c| c.schedule = Schedule::Weekly { weekday: 7, hh: 1, mm: 0 }), "invalid");
    assert_eq!(bad(&|c| c.projects = ProjectFilter::Selected(vec![])), "invalid");

    let mut good = ok.clone();
    good.lookback_days = 30;
    good.schedule = Schedule::Weekly { weekday: 6, hh: 23, mm: 59 };
    good.projects = ProjectFilter::Selected(vec!["p".into()]);
    let saved = set_extract_config_impl(&es, good.clone()).unwrap();
    assert_eq!(saved, good);
    assert_eq!(get_extract_config_impl(&es).unwrap(), good);
    // Persisted to disk.
    assert!(tmp.path().join("data").join("extract-config.json").exists());
}

#[test]
fn context_for_suggestion() {
    let tmp = tempfile::tempdir().unwrap();
    let a = app(tmp.path());
    let dir = a.paths.claude_home.join("projects").join("enc");
    fs::create_dir_all(&dir).unwrap();
    let mut lines = Vec::new();
    let mut tss = Vec::new();
    for i in 0..12 {
        let stamp = format!("2026-11-05T10:00:{i:02}.000Z");
        tss.push(chrono::DateTime::parse_from_rfc3339(&stamp).unwrap().timestamp_millis());
        let text = if i == 5 { "key sk-abcdefghijklmnopqrstuvwxyz123456 here".to_string() } else { format!("turn {i}") };
        lines.push(format!(
            r#"{{"type":"user","message":{{"role":"user","content":"{text}"}},"timestamp":"{stamp}","cwd":"/x","sessionId":"s1"}}"#
        ));
    }
    fs::write(dir.join("s1.jsonl"), lines.join("\n") + "\n").unwrap();
    let file = a.paths.claude_home.join("CLAUDE.md");
    let (es, _) = extract_state(a, vec![]);
    let mut s = sug("c", file.to_str().unwrap(), "h", "n");
    s.evidence = vec![
        Evidence { tool: Tool::Claude, session_id: "s1".into(), ts: tss[5], quote: "q".into() },
        Evidence { tool: Tool::Claude, session_id: "missing".into(), ts: 1, quote: "q2".into() },
    ];
    add(&es, s);
    let ctx = suggestion_context_impl(&es, "c").unwrap();
    assert_eq!(ctx.len(), 2);
    assert_eq!(ctx[0].evidence.quote, "q");
    assert_eq!(ctx[0].turns.len(), 9, "4 before + center + 4 after");
    let joined: String = ctx[0].turns.iter().map(|t| t.text.clone()).collect::<Vec<_>>().join("|");
    assert!(joined.contains("[REDACTED]") && !joined.contains("sk-abc"), "{joined}");
    assert_eq!(ctx[0].turns[0].text, "turn 1");
    assert!(ctx[1].turns.is_empty());
    assert_eq!(suggestion_context_impl(&es, "nope").unwrap_err().code, "not_found");
}

#[test]
fn run_error_codes() {
    assert_eq!(CommandError::from(RunError::NotInstalled).code, "not_installed");
    assert_eq!(CommandError::from(RunError::NotLoggedIn("x".into())).code, "not_logged_in");
    assert_eq!(CommandError::from(RunError::Timeout).code, "io");
    assert_eq!(CommandError::from(ExtractError::AlreadyRunning).code, "already_running");
}

#[test]
fn unavailable_store_reports_io() {
    let tmp = tempfile::tempdir().unwrap();
    let a = app(tmp.path());
    fs::write(tmp.path().join("data").join("suggestions.json"), "{ not json").unwrap();
    let es = ExtractState::build(a, vec![], |_| {});
    assert_eq!(get_extract_config_impl(&es).unwrap_err().code, "io");
}

#[test]
fn shutdown_cancels_manual_run_within_bound() {
    let tmp = tempfile::tempdir().unwrap();
    let runner = Arc::new(BlockingRunner::default());
    let saw = runner.saw_cancel.clone();
    let (es, _) = extract_state(app(tmp.path()), vec![runner]);
    start_extraction_impl(&es).unwrap();
    // Wait until the runner is actually executing.
    std::thread::sleep(Duration::from_millis(200));
    let t0 = Instant::now();
    es.shutdown();
    assert!(t0.elapsed() < Duration::from_secs(5), "shutdown bounded");
    assert!(!es.inner().unwrap().ctx.is_running());
    assert!(saw.load(Ordering::SeqCst), "runner observed cancel");
}

#[test]
fn tampered_suggestion_path_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let a = app(tmp.path());
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    let target = outside.join("x.md");
    let db = a.paths.codex_home.join("memories_1.sqlite");
    fs::write(&db, b"x").unwrap();
    a.rescan();
    let (es, _) = extract_state(a, vec![]);
    add(&es, sug("t", target.to_str().unwrap(), "", "- evil"));
    add(&es, sug("d", db.to_str().unwrap(), "", "- evil"));
    // CLAUDE.md in a directory that is not a project root / claude home.
    let sneaky = outside.join("CLAUDE.md");
    add(&es, sug("s", sneaky.to_str().unwrap(), "", "- evil"));
    for id in ["t", "d", "s"] {
        assert_eq!(apply_suggestion_impl(&es, id, None, false).unwrap_err().code, "invalid", "{id}");
    }
    let res = accept_suggestions_impl(&es, &["t".into()]).unwrap();
    assert_eq!(res[0].error.as_ref().unwrap().code, "invalid");
    assert!(!target.exists() && !sneaky.exists());
    assert_eq!(fs::read(&db).unwrap(), b"x");
}

#[test]
fn placeholder_claude_md_in_claude_home_is_allowed() {
    let tmp = tempfile::tempdir().unwrap();
    let a = app(tmp.path());
    let file = a.paths.claude_home.join("CLAUDE.md");
    assert!(!file.exists());
    let (es, _) = extract_state(a, vec![]);
    add(&es, sug("n", file.to_str().unwrap(), "", "- fresh"));
    let out = apply_suggestion_impl(&es, "n", None, false).unwrap();
    assert!(matches!(out, ApplyOutcome::Applied { .. }), "{out:?}");
    assert!(fs::read_to_string(&file).unwrap().contains("- fresh"));
}

#[test]
fn accept_many_emits_one_merged_memory_changed() {
    let tmp = tempfile::tempdir().unwrap();
    let a = app(tmp.path());
    let file = a.paths.claude_home.join("CLAUDE.md");
    fs::write(&file, "# rules
").unwrap();
    a.rescan();
    let hash = crate::memory::io::read_memory(&file).unwrap().hash;
    let (es, events) = extract_state(a, vec![]);
    add(&es, sug("a", file.to_str().unwrap(), &hash, "- one"));
    add(&es, sug("b", file.to_str().unwrap(), &hash, "- two"));
    accept_suggestions_impl(&es, &["a".into(), "b".into()]).unwrap();
    let ev = events.lock().unwrap().clone();
    let changed: Vec<_> = ev.iter().filter(|(n, _)| n == "memory-changed").collect();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].1["paths"].as_array().unwrap().len(), 1);
}

#[test]
fn runners_reported_even_when_store_is_broken() {
    let tmp = tempfile::tempdir().unwrap();
    let a = app(tmp.path());
    fs::write(tmp.path().join("data").join("suggestions.json"), "{ not json").unwrap();
    let es = ExtractState::build(a, vec![Arc::new(BlockingRunner::default())], |_| {});
    let r = extract_runners_impl(&es);
    assert!(r.iter().any(|x| x.tool == Tool::Claude && x.available));
}

struct EmptyRunner;

impl CliRunner for EmptyRunner {
    fn tool(&self) -> Tool {
        Tool::Claude
    }
    fn available(&self) -> bool {
        true
    }
    fn run(&self, _p: &str, _s: &Value, _t: Duration, _c: &AtomicBool) -> Result<String, RunError> {
        Ok(r#"{"suggestions":[]}"#.into())
    }
}

#[test]
fn scheduled_run_emits_events_through_hooks() {
    let tmp = tempfile::tempdir().unwrap();
    let (es, events) = extract_state(app(tmp.path()), vec![Arc::new(EmptyRunner)]);
    set_extract_config_impl(
        &es,
        ExtractConfig { schedule: Schedule::Daily { hh: 0, mm: 0 }, ..get_extract_config_impl(&es).unwrap() },
    )
    .unwrap();
    let ctx = es.inner().unwrap().ctx.clone();
    let sched = crate::extract::job::Scheduler::with_offset(ctx, Arc::new(|_| 0));
    let rec = sched.tick(0).expect("slot is due, run happens");
    assert_eq!(rec.trigger, RunTrigger::Scheduled);
    let n = names(&events);
    for want in ["extract-progress", "extract-finished", "suggestions-changed"] {
        assert!(n.contains(&want.to_string()), "{want} in {n:?}");
    }
}
