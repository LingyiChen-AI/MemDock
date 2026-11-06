use super::*;
use crate::commands_extract::{apply_suggestion_impl, list_suggestions_impl};
use crate::extract::apply::ApplyOutcome;
use crate::ghimport::fetch::{EntryKind, RepoTree};
use crate::paths::AgentPaths;
use crate::skills::model::SkillScope;
use std::fs;
use std::sync::atomic::{AtomicU32, Ordering};

/// In-memory GitHub: one repo `octo/mem`, branches `main` and `feature/x`.
#[derive(Default)]
struct FakeGh {
    files: HashMap<String, Vec<u8>>,
    /// Calls that carried a token.
    token_calls: AtomicU32,
    /// Every call with a token is rejected (stale login).
    reject_token: bool,
    /// The repo only exists for authenticated calls (private).
    private: bool,
    raw_calls: AtomicU32,
}

impl FakeGh {
    fn new(files: &[(&str, &str)]) -> Self {
        FakeGh {
            files: files.iter().map(|(p, t)| (p.to_string(), t.as_bytes().to_vec())).collect(),
            ..Default::default()
        }
    }

    fn gate(&self, o: &str, r: &str, t: Option<&str>) -> Result<(), FetchError> {
        if t.is_some() {
            self.token_calls.fetch_add(1, Ordering::SeqCst);
            if self.reject_token {
                return Err(FetchError::Unauthorized);
            }
        }
        if (o, r) != ("octo", "mem") || (self.private && t.is_none()) {
            return Err(FetchError::NotFound);
        }
        Ok(())
    }
}

impl GhFetcher for FakeGh {
    fn default_branch(&self, o: &str, r: &str, t: Option<&str>) -> Result<String, FetchError> {
        self.gate(o, r, t)?;
        Ok("main".into())
    }
    fn commit_sha(&self, o: &str, r: &str, g: &str, t: Option<&str>) -> Result<Option<String>, FetchError> {
        self.gate(o, r, t)?;
        Ok(match g {
            "main" => Some("aaaaaaa1".into()),
            "feature/x" => Some("bbbbbbb2".into()),
            _ => None,
        })
    }
    fn tree(&self, o: &str, r: &str, _s: &str, t: Option<&str>) -> Result<RepoTree, FetchError> {
        self.gate(o, r, t)?;
        let mut entries: Vec<TreeEntry> = self
            .files
            .iter()
            .map(|(p, b)| TreeEntry { path: p.clone(), kind: EntryKind::Blob, size: b.len() as u64 })
            .collect();
        // Directory entries for every parent.
        let mut dirs: Vec<String> = Vec::new();
        for p in self.files.keys() {
            let mut cur = p.as_str();
            while let Some((d, _)) = cur.rsplit_once('/') {
                dirs.push(d.to_string());
                cur = d;
            }
        }
        dirs.sort();
        dirs.dedup();
        entries.extend(dirs.into_iter().map(|d| TreeEntry { path: d, kind: EntryKind::Tree, size: 0 }));
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(RepoTree { entries, truncated: false })
    }
    fn raw(&self, o: &str, r: &str, _s: &str, p: &str, limit: u64, t: Option<&str>) -> Result<Vec<u8>, FetchError> {
        self.gate(o, r, t)?;
        self.raw_calls.fetch_add(1, Ordering::SeqCst);
        let b = self.files.get(p).cloned().ok_or(FetchError::NotFound)?;
        if b.len() as u64 > limit {
            return Err(FetchError::TooLarge);
        }
        Ok(b)
    }
}

const REPO: &[(&str, &str)] = &[
    ("CLAUDE.md", "# Team rules\r\n\r\n- Use tabs\r\n"),
    ("AGENTS.md", "Run tests first.\n"),
    ("README.md", "readme"),
    ("memory/notes.md", "remember this\n"),
    ("docs/empty/CLAUDE.md", "   \n"),
    ("skills/pdf/SKILL.md", "---\nname: pdf-tools\ndescription: PDF helpers\n---\nbody\n"),
    ("skills/pdf/scripts/run.py", "print(1)\n"),
];

struct Env {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    app: Arc<AppState>,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let home = root.join("home");
    let paths = AgentPaths::from_home(&home);
    fs::create_dir_all(&paths.claude_home).unwrap();
    fs::create_dir_all(&paths.codex_home).unwrap();
    fs::create_dir_all(root.join("data")).unwrap();
    let app = Arc::new(AppState::with_home(paths, root.join("data"), home));
    Env { _tmp: tmp, root, app }
}

fn state(e: &Env, gh: FakeGh) -> (GhImportState, Arc<FakeGh>) {
    let gh = Arc::new(gh);
    (GhImportState::new(e.app.clone(), gh.clone()), gh)
}

fn extract(e: &Env) -> ExtractState {
    ExtractState::build(e.app.clone(), vec![], |_| {})
}

fn ids(r: &ScanResultDto) -> Vec<&str> {
    r.items.iter().map(|i| i.id.as_str()).collect()
}

#[test]
fn scan_whole_repo_lists_memories_and_skills() {
    let e = env();
    let (gs, _) = state(&e, FakeGh::new(REPO));
    let r = scan_impl(&gs, "https://github.com/octo/mem", None).unwrap();
    assert_eq!(r.repo, "octo/mem");
    assert_eq!(r.git_ref, "main");
    assert_eq!(r.commit, "aaaaaaa1");
    assert!(!r.authenticated);
    assert_eq!(
        ids(&r),
        vec!["m:CLAUDE.md", "m:docs/empty/CLAUDE.md", "m:AGENTS.md", "m:memory/notes.md", "s:skills/pdf"]
    );
}

#[test]
fn scan_tree_link_with_slash_branch_and_subdir() {
    let e = env();
    let (gs, _) = state(&e, FakeGh::new(REPO));
    let r = scan_impl(&gs, "https://github.com/octo/mem/tree/feature/x/memory", None).unwrap();
    assert_eq!(r.git_ref, "feature/x");
    assert_eq!(r.scope, "memory");
    assert_eq!(ids(&r), vec!["m:memory/notes.md"]);
    // Blob link to a single file whatever its name.
    let r = scan_impl(&gs, "https://github.com/octo/mem/blob/main/README.md", None).unwrap();
    assert_eq!(ids(&r), vec!["m:README.md"]);
}

#[test]
fn scan_errors() {
    let e = env();
    let (gs, _) = state(&e, FakeGh::new(REPO));
    assert_eq!(scan_impl(&gs, "https://gitlab.com/a/b", None).unwrap_err().code, "import_bad_url");
    assert_eq!(scan_impl(&gs, "octo/other", None).unwrap_err().code, "import_not_found");
    assert_eq!(
        scan_impl(&gs, "https://github.com/octo/mem/tree/nope/x", None).unwrap_err().code,
        "import_not_found"
    );
    assert_eq!(
        scan_impl(&gs, "https://github.com/octo/mem/tree/main/missing", None).unwrap_err().code,
        "import_not_found"
    );
}

#[test]
fn token_is_used_and_falls_back_when_rejected() {
    let e = env();
    let mut gh = FakeGh::new(REPO);
    gh.private = true;
    let (gs, fake) = state(&e, gh);
    // Private repo: anonymous fails, with a token it works.
    assert_eq!(scan_impl(&gs, "octo/mem", None).unwrap_err().code, "import_not_found");
    let r = scan_impl(&gs, "octo/mem", Some("tok")).unwrap();
    assert!(r.authenticated);
    assert!(fake.token_calls.load(Ordering::SeqCst) >= 3);

    // Stale token on a public repo: every call retries anonymously.
    let mut gh = FakeGh::new(REPO);
    gh.reject_token = true;
    let (gs, _) = state(&e, gh);
    let r = scan_impl(&gs, "octo/mem", Some("old")).unwrap();
    let p = preview_impl(&gs, &r.session_id, "m:AGENTS.md", Some("old")).unwrap();
    assert_eq!(p.text, "Run tests first.\n");
}

#[test]
fn preview_memory_and_skill() {
    let e = env();
    let (gs, fake) = state(&e, FakeGh::new(REPO));
    let r = scan_impl(&gs, "octo/mem", None).unwrap();
    let p = preview_impl(&gs, &r.session_id, "m:CLAUDE.md", None).unwrap();
    assert_eq!(p.text, "# Team rules\n\n- Use tabs\n");
    assert!(!p.binary);
    // Cached: no second download.
    preview_impl(&gs, &r.session_id, "m:CLAUDE.md", None).unwrap();
    assert_eq!(fake.raw_calls.load(Ordering::SeqCst), 1);
    let p = preview_impl(&gs, &r.session_id, "s:skills/pdf", None).unwrap();
    assert!(p.text.contains("name: pdf-tools"));
    let mut files: Vec<&str> = p.files.iter().map(|f| f.path.as_str()).collect();
    files.sort();
    assert_eq!(files, vec!["SKILL.md", "scripts/run.py"]);
    assert_eq!(preview_impl(&gs, "nope", "m:CLAUDE.md", None).unwrap_err().code, "import_expired");
    assert_eq!(preview_impl(&gs, &r.session_id, "m:zzz", None).unwrap_err().code, "invalid");
}

#[test]
fn import_memories_creates_reviewable_suggestions() {
    let e = env();
    let (gs, _) = state(&e, FakeGh::new(REPO));
    let es = extract(&e);
    let project = e.root.join("proj");
    fs::create_dir_all(&project).unwrap();
    crate::commands::add_manual_project_impl(&e.app, &project.to_string_lossy()).unwrap();
    let pid = e.app.snapshot_index().projects[0].project.id.clone();

    let r = scan_impl(&gs, "octo/mem", None).unwrap();
    let picks = vec![
        MemoryPick { item_id: "m:CLAUDE.md".into(), target: ImportTarget::GlobalClaude },
        MemoryPick { item_id: "m:AGENTS.md".into(), target: ImportTarget::ProjectAgents { project_id: pid.clone() } },
        MemoryPick { item_id: "m:docs/empty/CLAUDE.md".into(), target: ImportTarget::GlobalClaude },
    ];
    let res = import_memories_impl(&gs, &es, &r.session_id, &picks, None).unwrap();
    assert_eq!(res.added, 2);
    assert_eq!(res.duplicates, 0);
    assert_eq!(res.skipped, vec![SkippedItem { item_id: "m:docs/empty/CLAUDE.md".into(), reason: "empty".into() }]);

    // Nothing was written yet.
    let global = e.app.paths.claude_home.join("CLAUDE.md");
    assert!(!global.exists());
    assert!(!project.join("AGENTS.md").exists());

    let list = list_suggestions_impl(&es, Some(SuggestionStatus::Pending)).unwrap();
    assert_eq!(list.len(), 2);
    let g = list.iter().find(|s| s.path.ends_with("CLAUDE.md")).unwrap();
    assert_eq!(g.kind, SuggestionKind::Add);
    assert_eq!(g.tool_of_file, Tool::Claude);
    assert_eq!(g.new_text.as_deref(), Some("# Team rules\n\n- Use tabs"));
    match &g.origin {
        Some(SuggestionOrigin::Github { repo, path, git_ref, commit, url }) => {
            assert_eq!((repo.as_str(), path.as_str(), git_ref.as_str(), commit.as_str()), ("octo/mem", "CLAUDE.md", "main", "aaaaaaa1"));
            assert_eq!(url, "https://github.com/octo/mem/blob/aaaaaaa1/CLAUDE.md");
        }
        o => panic!("origin {o:?}"),
    }
    let a = list.iter().find(|s| s.path.ends_with("AGENTS.md")).unwrap();
    assert_eq!(a.tool_of_file, Tool::Codex);

    // Importing the same thing again does not duplicate.
    let res = import_memories_impl(&gs, &es, &r.session_id, &picks[..2], None).unwrap();
    assert_eq!((res.added, res.duplicates), (0, 2));

    // Review: accepting writes (creates) the files.
    for s in &list {
        let out = apply_suggestion_impl(&es, &s.id, None, false).unwrap();
        assert!(matches!(out, ApplyOutcome::Applied { .. }), "{out:?}");
    }
    assert_eq!(fs::read_to_string(&global).unwrap(), "# Team rules\n\n- Use tabs\n");
    assert_eq!(fs::read_to_string(project.join("AGENTS.md")).unwrap(), "Run tests first.\n");
}

#[test]
fn import_memories_validates_input() {
    let e = env();
    let (gs, _) = state(&e, FakeGh::new(REPO));
    let es = extract(&e);
    let r = scan_impl(&gs, "octo/mem", None).unwrap();
    let pick = |id: &str, t: ImportTarget| vec![MemoryPick { item_id: id.into(), target: t }];
    assert_eq!(import_memories_impl(&gs, &es, &r.session_id, &[], None).unwrap_err().code, "invalid");
    assert_eq!(
        import_memories_impl(&gs, &es, &r.session_id, &pick("s:skills/pdf", ImportTarget::GlobalClaude), None).unwrap_err().code,
        "invalid"
    );
    assert_eq!(
        import_memories_impl(&gs, &es, &r.session_id, &pick("m:CLAUDE.md", ImportTarget::ProjectClaude { project_id: "x".into() }), None)
            .unwrap_err()
            .code,
        "invalid"
    );
    assert_eq!(
        import_memories_impl(&gs, &es, "gone", &pick("m:CLAUDE.md", ImportTarget::GlobalClaude), None).unwrap_err().code,
        "import_expired"
    );
}

#[test]
fn import_skill_installs_through_folder_flow() {
    let e = env();
    let (gs, _) = state(&e, FakeGh::new(REPO));
    let r = scan_impl(&gs, "octo/mem", None).unwrap();
    let targets = || vec![InstallTargetDto { tool: Tool::Claude, scope: SkillScope::Global }];
    let installed = import_skill_impl(&gs, &r.session_id, "s:skills/pdf", targets(), false, None).unwrap();
    assert_eq!(installed.len(), 1);
    let dir = e.app.paths.claude_home.join("skills").join("pdf-tools");
    assert!(dir.join("SKILL.md").is_file());
    assert_eq!(fs::read_to_string(dir.join("scripts").join("run.py")).unwrap(), "print(1)\n");
    // Staging is cleaned up.
    let tmp = e.app.data_dir.join("skill-tmp");
    let left: Vec<_> = fs::read_dir(&tmp).map(|d| d.flatten().map(|x| x.file_name()).collect()).unwrap_or_default();
    assert!(left.iter().all(|n| !n.to_string_lossy().starts_with("gh-")), "{left:?}");
    // Second install without overwrite reports the conflict.
    let err = import_skill_impl(&gs, &r.session_id, "s:skills/pdf", targets(), false, None).unwrap_err();
    assert_eq!(err.code, "exists");
    import_skill_impl(&gs, &r.session_id, "s:skills/pdf", targets(), true, None).unwrap();
    // A memory item is not a skill.
    assert_eq!(
        import_skill_impl(&gs, &r.session_id, "m:CLAUDE.md", targets(), false, None).unwrap_err().code,
        "invalid"
    );
}

#[test]
fn helpers() {
    assert_eq!(decode_text(b"\xEF\xBB\xBFa\r\nb"), Some("a\nb".into()));
    assert_eq!(decode_text(b"a\0b"), None);
    assert_eq!(decode_text(&[0xff, 0xfe]), None);
    assert!(safe_rel("a/b.md").is_some());
    for bad in ["../x", "a/../b", "a\\b", "c:x", "", "a//b", "a/b."] {
        assert!(safe_rel(bad).is_none(), "{bad}");
    }
    assert_eq!(folder_name("My Skill", "r"), "my-skill");
    assert_eq!(folder_name("", "Repo.Name"), "repo.name");
    assert_eq!(folder_name("__", "r"), "skill");
}
