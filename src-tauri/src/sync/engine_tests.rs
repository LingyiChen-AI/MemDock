//! Engine tests: two temporary homes ("machines" A and B), each with its own
//! clone of one local bare repository. No network, no real home.

use super::*;
use crate::memory::scan::scan_all;
use crate::model::{Project, ProjectSource};
use crate::skills::git::testutil::git;
use crate::sync::layout::map_files;
use crate::sync::repo::TreeBlob;
use tempfile::TempDir;

const P_KEY: &str = "github.com/me/p";
const GLOBAL: &str = "global/claude/CLAUDE.md";
const CODEX: &str = "global/codex/AGENTS.md";

fn p_path(tail: &str) -> String {
    format!("projects/{P_KEY}/{tail}")
}

struct World {
    t: TempDir,
    bare: PathBuf,
    url: String,
}

impl World {
    fn new() -> World {
        let t = tempfile::tempdir().unwrap();
        let bare = t.path().join("remote.git");
        fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare", "--initial-branch=main"]);
        let url = format!("file:///{}", bare.to_string_lossy().replace('\\', "/"));
        // Bootstrap: marker file only (what repo creation leaves behind).
        let g = SyncGit::new();
        let seed = t.path().join("seed");
        g.clone(&url, &seed, &None).unwrap();
        let mut m = BTreeMap::new();
        m.insert(
            "memdock.json".to_string(),
            b"{\"format\":1,\"app\":\"MemDock\"}".to_vec(),
        );
        let c = g.commit_tree(&seed, None, &m, "init", SYNC_AUTHOR).unwrap();
        g.push(&seed, &c, "main", &None).unwrap();
        World { t, bare, url }
    }

    fn head(&self) -> String {
        git(&self.bare, &["rev-parse", "main"])
    }

    fn file(&self, path: &str) -> Option<String> {
        SyncGit::new()
            .read_at(&self.bare, "main", path)
            .unwrap()
            .map(|b| String::from_utf8(b).unwrap())
    }

    /// Commits changes directly on top of `main`, as another machine would.
    fn push_raw(&self, changes: &[(&str, Option<&str>)]) {
        let ch: Vec<(&str, Option<TreeBlob>)> = changes
            .iter()
            .map(|(p, c)| (*p, c.map(|c| TreeBlob::Data(c.as_bytes().to_vec()))))
            .collect();
        self.push_entries(&ch);
    }

    /// Like `push_raw`, with raw entries (bytes, modes, gitlinks).
    fn push_entries(&self, changes: &[(&str, Option<TreeBlob>)]) {
        let g = SyncGit::new();
        let seed = self.t.path().join("seed");
        let head = g.fetch(&seed, &None).unwrap().unwrap();
        let mut tree: BTreeMap<String, TreeBlob> = g
            .tree_entries(&seed, &head)
            .unwrap()
            .into_iter()
            .map(|(p, e)| (p, TreeBlob::Existing(e)))
            .collect();
        for (p, c) in changes {
            match c {
                Some(c) => tree.insert(p.to_string(), c.clone()),
                None => tree.remove(*p),
            };
        }
        let c = g.commit_tree_blobs(&seed, Some(&head), &tree, "raw", SYNC_AUTHOR).unwrap();
        g.push(&seed, &c, "main", &None).unwrap();
    }

    /// A new machine: temporary home plus its own clone.
    fn machine(&self, name: &str) -> Pc {
        let root = self.t.path().join(name);
        let home = root.join("home");
        let paths = AgentPaths::from_home(&home);
        fs::create_dir_all(&paths.claude_home).unwrap();
        fs::create_dir_all(&paths.codex_home).unwrap();
        let repo = root.join("repo");
        let g = SyncGit::new();
        g.clone(&self.url, &repo, &None).unwrap();
        Pc {
            root: root.clone(),
            paths,
            repo,
            snaps: SnapshotStore::new(root.join("snaps")),
            machine: Machine {
                id: format!("id-{name}"),
                name: name.to_string(),
            },
            projects: Vec::new(),
            base: None,
            synced: Vec::new(),
            unsynced: BTreeSet::new(),
            held: BTreeSet::new(),
            lock: Mutex::new(()),
            git: g,
        }
    }
}

struct Pc {
    root: PathBuf,
    paths: AgentPaths,
    repo: PathBuf,
    snaps: SnapshotStore,
    machine: Machine,
    projects: Vec<Project>,
    base: Option<String>,
    synced: Vec<SyncedProject>,
    unsynced: BTreeSet<String>,
    held: BTreeSet<String>,
    lock: Mutex<()>,
    git: SyncGit,
}

impl Pc {
    /// Adds project `p` (origin github.com/me/p) and returns its directory.
    fn add_project(&mut self) -> PathBuf {
        self.add_project_at("p")
    }

    /// Adds a clone of github.com/me/p in directory `work/<name>`.
    fn add_project_at(&mut self, name: &str) -> PathBuf {
        let dir = self.root.join("work").join(name);
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(
            dir.join(".git").join("config"),
            "[remote \"origin\"]\n\turl = https://github.com/me/p.git\n",
        )
        .unwrap();
        let s = dir.to_string_lossy().into_owned();
        self.projects.push(Project {
            id: normalize_path(&s),
            path: s,
            name: "p".into(),
            exists: true,
            sources: vec![ProjectSource::Manual],
        });
        dir
    }

    fn global(&self) -> PathBuf {
        self.paths.claude_home.join("CLAUDE.md")
    }

    fn codex(&self) -> PathBuf {
        self.paths.codex_home.join("AGENTS.md")
    }

    fn ctx(&self) -> SyncCtx<'_> {
        let index = scan_all(&self.paths, &self.projects);
        SyncCtx {
            git: &self.git,
            repo_dir: &self.repo,
            auth: None,
            branch: "main".into(),
            base_commit: self.base.clone(),
            files: map_files(&index, &[]),
            projects: sync_projects(&index, &[]),
            synced_projects: self.synced.clone(),
            unsynced_paths: self.unsynced.clone(),
            held_projects: self.held.clone(),
            paths: &self.paths,
            machine: &self.machine,
            local_skills: Some(Vec::new()),
            snapshots: &self.snaps,
            write_lock: &self.lock,
        }
    }

    fn prepare(&self) -> SyncPlan {
        prepare(&self.ctx()).unwrap()
    }

    fn finish(
        &mut self,
        plan: &SyncPlan,
        res: &BTreeMap<String, Resolution>,
    ) -> Result<SyncSummary, SyncError> {
        let r = finish(&self.ctx(), plan, res);
        match &r {
            Ok(s) => {
                self.base = s.commit.clone();
                self.synced = s.synced_projects.clone();
                self.unsynced = s.unsynced_paths.clone();
                self.held = s.held_projects.clone();
            }
            // what the real caller does on failure after local writes
            Err(e) => {
                if let Some(b) = e.base_to_persist() {
                    self.base = b.commit.clone();
                    self.synced = b.synced_projects.clone();
                    self.unsynced = b.unsynced_paths.clone();
                    self.held = b.held_projects.clone();
                }
            }
        }
        r
    }

    /// A full sync that must not need user input.
    fn sync(&mut self) -> SyncSummary {
        let plan = self.prepare();
        assert!(plan.conflicts.is_empty(), "conflicts: {:?}", plan.conflicts);
        assert!(plan.secret_hits.is_empty());
        self.finish(&plan, &BTreeMap::new()).unwrap()
    }
}

fn write(p: &Path, s: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, s).unwrap();
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap().replace("\r\n", "\n")
}

fn keys(v: &[SyncedProject]) -> Vec<String> {
    v.iter().map(|p| p.key.clone()).collect()
}

fn text_res(path: &str, text: &str) -> BTreeMap<String, Resolution> {
    BTreeMap::from([(path.to_string(), Resolution::Text(text.to_string()))])
}

const FIVE: &str = "l1\nl2\nl3\nl4\nl5\n";

// ------------------------------------------------------------------ tests

#[test]
fn first_sync_pushes_everything() {
    let w = World::new();
    let mut a = w.machine("A");
    let p = a.add_project();
    write(&a.global(), "global rules\n");
    write(&a.codex(), "codex rules\n");
    write(&p.join("CLAUDE.md"), "project rules\r\n");
    let s = a.sync();
    assert!(s.committed);
    assert_eq!(s.pushed, 3);
    assert_eq!(keys(&s.synced_projects), vec![P_KEY.to_string()]);
    assert_eq!(s.commit.as_deref(), Some(w.head().as_str()));
    assert_eq!(w.file(GLOBAL).as_deref(), Some("global rules\n"));
    assert_eq!(w.file(CODEX).as_deref(), Some("codex rules\n"));
    // normalized to LF in the repo
    assert_eq!(w.file(&p_path("claude/CLAUDE.md")).as_deref(), Some("project rules\n"));
    let skills = w.file("skills.json").unwrap();
    assert_eq!(Manifest::from_json(&skills).unwrap(), Manifest::default());
    assert!(w.file("memdock.json").is_some());
    let msg = git(&w.bare, &["log", "-1", "--format=%s|%an", "main"]);
    assert_eq!(msg, "sync from A|MemDock");
    // second sync: nothing to do, no new commit
    let again = a.sync();
    assert!(!again.committed);
    assert_eq!(again.commit, s.commit);
    assert_eq!(w.head(), s.commit.unwrap());
}

#[test]
fn second_machine_pulls_into_local() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    write(&a.global(), "v1\n");
    write(&pa.join("CLAUDE.md"), "p1\n");
    a.sync();

    let mut b = w.machine("B");
    let pb = b.add_project();
    let s = b.sync();
    assert_eq!(s.pulled, 2);
    assert!(!s.committed, "pull-only sync must not commit");
    assert_eq!(read(&b.global()), "v1\n");
    assert_eq!(read(&pb.join("CLAUDE.md")), "p1\n");
    let raw = fs::read(b.global()).unwrap();
    // new files: platform default line endings, no BOM
    assert_eq!(raw.windows(2).any(|x| x == b"\r\n"), cfg!(windows));
    assert!(!raw.starts_with(&[0xEF, 0xBB, 0xBF]));
    // new files have no snapshot
    assert!(b.snaps.list(&b.global()).unwrap().is_empty());

    // an existing file overwritten by a pull is snapshotted first
    write(&a.global(), "v2\n");
    a.sync();
    let s = b.sync();
    assert_eq!(s.pulled, 1);
    assert_eq!(read(&b.global()), "v2\n");
    let snaps = b.snaps.list(&b.global()).unwrap();
    assert_eq!(snaps.len(), 1);
    let old = b.snaps.read(&b.global(), &snaps[0].id).unwrap();
    assert_eq!(String::from_utf8(old).unwrap().replace("\r\n", "\n"), "v1\n");
}

#[test]
fn two_machines_merge_disjoint_edits() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), FIVE);
    a.sync();
    let mut b = w.machine("B");
    b.sync();

    write(&a.global(), "L1\nl2\nl3\nl4\nl5\n");
    a.sync();
    write(&b.global(), "l1\nl2\nl3\nl4\nL5\n");
    let plan = b.prepare();
    assert_eq!(plan.merged, vec![GLOBAL.to_string()]);
    assert!(plan.conflicts.is_empty());
    let s = b.finish(&plan, &BTreeMap::new()).unwrap();
    assert_eq!(s.merged, 1);
    let both = "L1\nl2\nl3\nl4\nL5\n";
    assert_eq!(read(&b.global()), both);
    assert_eq!(w.file(GLOBAL).as_deref(), Some(both));
    a.sync();
    assert_eq!(read(&a.global()), both);
}

#[test]
fn same_line_edit_is_conflict() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), FIVE);
    a.sync();
    let mut b = w.machine("B");
    b.sync();

    write(&a.global(), "from A\nl2\nl3\nl4\nl5\n");
    a.sync();
    write(&b.global(), "from B\nl2\nl3\nl4\nl5\n");
    let plan = b.prepare();
    assert_eq!(plan.conflicts.len(), 1);
    let c = &plan.conflicts[0];
    assert_eq!(c.repo_path, GLOBAL);
    assert_eq!(c.conflicts, 1);
    assert!(c.merged.contains("<<<<<<< local"));
    assert!(c.merged.contains(">>>>>>> remote"));
    assert_eq!(c.local.as_deref(), Some("from B\nl2\nl3\nl4\nl5\n"));
    assert_eq!(c.remote.as_deref(), Some("from A\nl2\nl3\nl4\nl5\n"));
    assert_eq!(c.base.as_deref(), Some(FIVE));

    let resolved = "from A and B\nl2\nl3\nl4\nl5\n";
    b.finish(&plan, &text_res(GLOBAL, resolved)).unwrap();
    assert_eq!(read(&b.global()), resolved);
    assert_eq!(w.file(GLOBAL).as_deref(), Some(resolved));
    a.sync();
    assert_eq!(read(&a.global()), resolved);
}

#[test]
fn first_sync_differing_global_is_conflict() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), "rules of A\n");
    a.sync();
    let b = w.machine("B");
    write(&b.global(), "rules of B\n");
    let plan = b.prepare();
    assert_eq!(plan.conflicts.len(), 1);
    let c = &plan.conflicts[0];
    assert_eq!(c.repo_path, GLOBAL);
    assert_eq!(c.local.as_deref(), Some("rules of B\n"));
    assert_eq!(c.remote.as_deref(), Some("rules of A\n"));
    assert_eq!(c.base, None);
    assert!(c.conflicts >= 1);
    assert!(plan.take_remote.is_empty() && plan.push_local.is_empty());
}

#[test]
fn remote_deletion_prompts_default_keep() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    write(&pa.join("CLAUDE.md"), "keep me\n");
    write(&pa.join("AGENTS.md"), "stays\n");
    a.sync();
    let mut b = w.machine("B");
    let pb = b.add_project();
    b.sync();
    assert_eq!(read(&pb.join("CLAUDE.md")), "keep me\n");

    fs::remove_file(pa.join("CLAUDE.md")).unwrap();
    let s = a.sync();
    assert_eq!(s.pushed, 1);
    let path = p_path("claude/CLAUDE.md");
    assert_eq!(w.file(&path), None);

    let plan = b.prepare();
    assert_eq!(plan.remote_deleted, vec![path.clone()]);
    let s = b.finish(&plan, &BTreeMap::new()).unwrap();
    assert!(s.committed);
    assert_eq!(read(&pb.join("CLAUDE.md")), "keep me\n");
    assert_eq!(w.file(&path).as_deref(), Some("keep me\n"));

    // DeleteLocal: snapshot, then delete
    a.sync(); // A pulls the re-pushed file back
    assert_eq!(read(&pa.join("CLAUDE.md")), "keep me\n");
    fs::remove_file(pa.join("CLAUDE.md")).unwrap();
    a.sync();
    let plan = b.prepare();
    assert_eq!(plan.remote_deleted, vec![path.clone()]);
    let res = BTreeMap::from([(path.clone(), Resolution::DeleteLocal)]);
    b.finish(&plan, &res).unwrap();
    assert!(!pb.join("CLAUDE.md").exists());
    assert_eq!(b.snaps.list(&pb.join("CLAUDE.md")).unwrap().len(), 1);
    assert_eq!(w.file(&path), None);
}

#[test]
fn secret_blocks_everything() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    write(&pa.join("CLAUDE.md"), "p1\n");
    a.sync();
    let mut b = w.machine("B");
    let pb = b.add_project();
    b.sync();
    write(&pa.join("CLAUDE.md"), "p2\n");
    a.sync();
    let head = w.head();

    let token = format!("ghp_{}", "a1B2c3D4e5".repeat(4));
    write(&b.global(), &format!("ok\nmy token {token}\n"));
    let plan = b.prepare();
    assert_eq!(plan.take_remote, vec![p_path("claude/CLAUDE.md")]);
    assert_eq!(plan.secret_hits.len(), 1);
    let hit = &plan.secret_hits[0];
    assert_eq!((hit.repo_path.as_str(), hit.line, hit.rule.as_str()), (GLOBAL, 2, "github_token"));
    match b.finish(&plan, &BTreeMap::new()) {
        Err(SyncError::Secrets { hits, base }) => {
            assert_eq!(hits.len(), 1);
            assert_eq!(base, None);
        }
        other => panic!("expected Secrets, got {other:?}"),
    }
    assert_eq!(w.head(), head);
    assert_eq!(read(&pb.join("CLAUDE.md")), "p1\n");

    // a secret typed into a conflict resolution is blocked too
    write(&b.global(), "clean\n");
    write(&a.global(), "other\n");
    a.sync();
    let plan = b.prepare();
    assert_eq!(plan.conflicts.len(), 1);
    let r = b.finish(&plan, &text_res(GLOBAL, &format!("x {token}\n")));
    assert!(matches!(r, Err(SyncError::Secrets { .. })));
    assert_eq!(read(&b.global()), "clean\n");
}

#[test]
fn local_edit_during_sync_aborts() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), "v1\n");
    a.sync();
    let mut b = w.machine("B");
    b.sync();
    write(&a.global(), "v2\n");
    a.sync();
    let head = w.head();

    write(&b.codex(), "new on B\n");
    let plan = b.prepare();
    assert_eq!(plan.take_remote, vec![GLOBAL.to_string()]);
    assert_eq!(plan.push_local, vec![CODEX.to_string()]);
    write(&b.global(), "edited meanwhile\n");
    match b.finish(&plan, &BTreeMap::new()) {
        Err(SyncError::LocalChanged { paths }) => assert_eq!(paths.len(), 1),
        other => panic!("expected LocalChanged, got {other:?}"),
    }
    assert_eq!(w.head(), head);
    assert_eq!(read(&b.global()), "edited meanwhile\n");
    assert_eq!(w.file(CODEX), None);
}

#[test]
fn push_race_retries_once() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), FIVE);
    a.sync();
    let mut b = w.machine("B");
    b.sync();

    write(&a.global(), "A1\nl2\nl3\nl4\nl5\n");
    let plan = a.prepare();
    assert_eq!(plan.push_local, vec![GLOBAL.to_string()]);
    // B pushes a non-conflicting change in between
    write(&b.global(), "l1\nl2\nl3\nl4\nB5\n");
    write(&b.codex(), "codex from B\n");
    b.sync();

    let s = a.finish(&plan, &BTreeMap::new()).unwrap();
    assert!(s.committed);
    let both = "A1\nl2\nl3\nl4\nB5\n";
    assert_eq!(w.file(GLOBAL).as_deref(), Some(both));
    assert_eq!(w.file(CODEX).as_deref(), Some("codex from B\n"));
    assert_eq!(read(&a.global()), both);
    assert_eq!(read(&a.codex()), "codex from B\n");
    assert_eq!(s.commit.as_deref(), Some(w.head().as_str()));
}

#[test]
fn push_race_with_new_conflict_is_remote_moved() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), FIVE);
    a.sync();
    let mut b = w.machine("B");
    b.sync();

    write(&a.global(), "A1\nl2\nl3\nl4\nl5\n");
    let plan = a.prepare();
    write(&b.global(), "B1\nl2\nl3\nl4\nl5\n");
    b.sync();
    let head = w.head();
    let r = a.finish(&plan, &BTreeMap::new());
    assert!(matches!(r, Err(SyncError::RemoteMoved { .. })), "{r:?}");
    assert_eq!(w.head(), head);
}

#[test]
fn unresolved_conflict_rejected() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), "a\n");
    a.sync();
    let mut b = w.machine("B");
    write(&b.global(), "b\n");
    let head = w.head();
    let plan = b.prepare();
    assert_eq!(
        b.finish(&plan, &BTreeMap::new()),
        Err(SyncError::Unresolved {
            paths: vec![GLOBAL.to_string()]
        })
    );
    let keep = BTreeMap::from([(GLOBAL.to_string(), Resolution::KeepLocal)]);
    assert!(matches!(b.finish(&plan, &keep), Err(SyncError::Unresolved { .. })));
    assert_eq!(w.head(), head);
    assert_eq!(read(&b.global()), "b\n");
}

#[test]
fn marker_text_rejected() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), "a\n");
    a.sync();
    let mut b = w.machine("B");
    write(&b.global(), "b\n");
    let head = w.head();
    let plan = b.prepare();
    let merged = plan.conflicts[0].merged.clone();
    assert!(matches!(
        b.finish(&plan, &text_res(GLOBAL, &merged)),
        Err(SyncError::Unresolved { .. })
    ));
    assert_eq!(w.head(), head);
    assert_eq!(read(&b.global()), "b\n");
    // a clean resolution of the same plan goes through
    b.finish(&plan, &text_res(GLOBAL, "a\nb\n")).unwrap();
    assert_eq!(w.file(GLOBAL).as_deref(), Some("a\nb\n"));
}

#[test]
fn project_absent_here_left_untouched() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    write(&pa.join("CLAUDE.md"), "only on A\n");
    a.sync();

    let mut b = w.machine("B");
    write(&b.global(), "global of B\n");
    let plan = b.prepare();
    let all: Vec<&String> = plan
        .take_remote
        .iter()
        .chain(&plan.push_local)
        .chain(&plan.merged)
        .chain(&plan.remote_deleted)
        .collect();
    assert!(all.iter().all(|p| !p.starts_with("projects/")), "{all:?}");
    assert!(plan.conflicts.is_empty());
    let s = b.finish(&plan, &BTreeMap::new()).unwrap();
    assert!(s.committed);
    assert!(s.synced_projects.is_empty());
    assert_eq!(w.file(&p_path("claude/CLAUDE.md")).as_deref(), Some("only on A\n"));
    assert_eq!(w.file(GLOBAL).as_deref(), Some("global of B\n"));
    assert!(w.file("memdock.json").is_some());
}

#[test]
fn project_appearing_later_pulls_not_deletes() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    write(&pa.join("CLAUDE.md"), "project memory\n");
    write(&pa.join("AGENTS.md"), "agents\n");
    a.sync();

    let mut b = w.machine("B");
    write(&b.global(), "g\n");
    b.sync();
    assert!(b.synced.is_empty());

    // P shows up on B later, without memory files
    let pb = b.add_project();
    let plan = b.prepare();
    let mut pulled = plan.take_remote.clone();
    pulled.sort();
    assert_eq!(pulled, vec![p_path("claude/CLAUDE.md"), p_path("codex/AGENTS.md")]);
    assert!(plan.push_local.is_empty() && plan.remote_deleted.is_empty());
    let s = b.finish(&plan, &BTreeMap::new()).unwrap();
    assert_eq!(keys(&s.synced_projects), vec![P_KEY.to_string()]);
    assert_eq!(read(&pb.join("CLAUDE.md")), "project memory\n");
    assert_eq!(read(&pb.join("AGENTS.md")), "agents\n");
    assert_eq!(w.file(&p_path("claude/CLAUDE.md")).as_deref(), Some("project memory\n"));
}

#[test]
fn line_endings_preserved() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), "x\ny\n");
    a.sync();
    let mut b = w.machine("B");
    b.sync();
    // B's copy uses CRLF + BOM
    fs::write(b.global(), b"\xEF\xBB\xBFx\r\ny\r\n").unwrap();
    assert_eq!(b.prepare().take_remote.len() + b.prepare().push_local.len(), 0);

    write(&a.global(), "x\ny\nz\n");
    a.sync();
    b.sync();
    assert_eq!(fs::read(b.global()).unwrap(), b"\xEF\xBB\xBFx\r\ny\r\nz\r\n");
    assert_eq!(w.file(GLOBAL).as_deref(), Some("x\ny\nz\n"));

    // an LF file stays LF
    write(&a.codex(), "c1\n");
    a.sync();
    b.sync();
    fs::write(b.codex(), "c1\n").unwrap();
    write(&a.codex(), "c1\nc2\n");
    a.sync();
    b.sync();
    assert_eq!(fs::read(b.codex()).unwrap(), b"c1\nc2\n");
}

#[test]
fn pending_local_counts_changes() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    write(&a.global(), "g\n");
    write(&pa.join("CLAUDE.md"), "p\n");
    assert_eq!(pending_local(&a.ctx()).unwrap(), 2);
    a.sync();
    assert_eq!(pending_local(&a.ctx()).unwrap(), 0);
    write(&a.global(), "g2\n");
    write(&a.codex(), "new\n");
    fs::remove_file(pa.join("CLAUDE.md")).unwrap();
    assert_eq!(pending_local(&a.ctx()).unwrap(), 3);
    // line-ending-only change is not a change
    write(&a.global(), "g\r\n");
    assert_eq!(pending_local(&a.ctx()).unwrap(), 2);
}

#[test]
fn non_utf8_local_is_skipped() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), "g\n");
    fs::write(a.codex(), [0xFF, 0xFE, b'a', 0, b'b', 0]).unwrap();
    let plan = a.prepare();
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].repo_path, CODEX);
    a.finish(&plan, &BTreeMap::new()).unwrap();
    assert_eq!(w.file(CODEX), None);
    assert_eq!(w.file(GLOBAL).as_deref(), Some("g\n"));
    assert_eq!(fs::read(a.codex()).unwrap(), [0xFF, 0xFE, b'a', 0, b'b', 0]);
}

#[test]
fn history_and_file_at() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), "one\n");
    let s1 = a.sync();
    write(&a.global(), "two\n");
    a.sync();
    a.prepare(); // refresh origin/main
    let ctx = a.ctx();
    let h = history(&ctx, 10).unwrap();
    assert_eq!(h.len(), 3);
    assert_eq!(h[0].message, "sync from A");
    let c1 = s1.commit.unwrap();
    assert_eq!(file_at(&ctx, &c1, GLOBAL).unwrap().as_deref(), Some("one\n"));
    assert_eq!(file_at(&ctx, &c1, CODEX).unwrap(), None);
}

#[test]
fn manifest_merges_local_skills() {
    use crate::model::Tool;
    use crate::sync::manifest::ManifestScope;
    let w = World::new();
    let mut a = w.machine("A");
    let skill = ManifestSkill {
        name: "s".into(),
        url: "https://github.com/x/skills".into(),
        git_ref: None,
        commit: Some("abc".into()),
        skill_path: "s".into(),
        tool: Tool::Claude,
        scope: ManifestScope::Global,
        installed_on: Default::default(),
        removed_on: Default::default(),
    };
    let ctx = SyncCtx {
        local_skills: Some(vec![skill.clone()]),
        ..a.ctx()
    };
    let plan = prepare(&ctx).unwrap();
    let s = finish(&ctx, &plan, &BTreeMap::new()).unwrap();
    drop(ctx);
    a.base = s.commit.clone();
    let m = Manifest::from_json(&w.file("skills.json").unwrap()).unwrap();
    assert_eq!(m.skills.len(), 1);
    assert!(m.skills[0].installed_on.contains("id-A"));

    let b = w.machine("B");
    let plan = b.prepare();
    assert_eq!(plan.missing_skills.len(), 1);

    // unreadable skill lock: skills.json kept as is
    let head = w.head();
    let ctx = SyncCtx {
        local_skills: None,
        ..b.ctx()
    };
    let plan = prepare(&ctx).unwrap();
    let s = finish(&ctx, &plan, &BTreeMap::new()).unwrap();
    assert!(!s.committed);
    assert_eq!(w.head(), head);
}

// ------------------------------------------------------ fix round 1 tests

/// Fakes a Claude session so the project at `dir` has an auto-memory folder;
/// returns that `memory` directory.
fn fake_session(pc: &Pc, dir: &Path) -> PathBuf {
    let enc = pc.paths.claude_home.join("projects").join("enc");
    fs::create_dir_all(enc.join("memory")).unwrap();
    let cwd = dir.to_string_lossy().replace('\\', "\\\\");
    fs::write(
        enc.join("s.jsonl"),
        format!("{{\"type\":\"user\",\"cwd\":\"{cwd}\"}}\n"),
    )
    .unwrap();
    enc.join("memory")
}

#[test]
fn race_after_keep_local_retries_without_prompt() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    write(&pa.join("CLAUDE.md"), "keep me\n");
    write(&pa.join("AGENTS.md"), "stays\n");
    a.sync();
    let mut b = w.machine("B");
    let pb = b.add_project();
    b.sync();
    fs::remove_file(pb.join("CLAUDE.md")).unwrap();
    b.sync();

    let path = p_path("claude/CLAUDE.md");
    let plan = a.prepare();
    assert_eq!(plan.remote_deleted, vec![path.clone()]);
    w.push_raw(&[(CODEX, Some("raced\n"))]);
    let res = BTreeMap::from([(path.clone(), Resolution::KeepLocal)]);
    let s = a.finish(&plan, &res).unwrap();
    assert!(s.committed);
    assert_eq!(w.file(&path).as_deref(), Some("keep me\n"));
    assert_eq!(w.file(CODEX).as_deref(), Some("raced\n"));
    assert_eq!(read(&a.codex()), "raced\n");
    // nothing left over for the next sync
    let next = a.prepare();
    assert!(next.remote_deleted.is_empty() && next.push_local.is_empty());
}

#[test]
fn race_on_pulled_file_has_no_false_conflict() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), FIVE);
    a.sync();
    w.push_raw(&[(GLOBAL, Some("R1\nl2\nl3\nl4\nl5\n"))]);
    write(&a.codex(), "codex of A\n");
    let plan = a.prepare();
    assert_eq!(plan.take_remote, vec![GLOBAL.to_string()]);
    assert_eq!(plan.push_local, vec![CODEX.to_string()]);
    // the same line changes again remotely before A pushes
    w.push_raw(&[(GLOBAL, Some("R2\nl2\nl3\nl4\nl5\n"))]);
    let s = a.finish(&plan, &BTreeMap::new()).unwrap();
    assert!(s.committed);
    assert_eq!(read(&a.global()), "R2\nl2\nl3\nl4\nl5\n");
    assert_eq!(w.file(GLOBAL).as_deref(), Some("R2\nl2\nl3\nl4\nl5\n"));
    assert_eq!(w.file(CODEX).as_deref(), Some("codex of A\n"));
    assert_eq!(s.pulled, 1, "a file pulled twice counts once");
}

#[test]
fn race_counts_merged_once() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), FIVE);
    a.sync();
    w.push_raw(&[(GLOBAL, Some("l1\nl2\nl3\nl4\nR5\n"))]);
    write(&a.global(), "A1\nl2\nl3\nl4\nl5\n");
    let plan = a.prepare();
    assert_eq!(plan.merged, vec![GLOBAL.to_string()]);
    w.push_raw(&[(CODEX, Some("x\n"))]);
    let s = a.finish(&plan, &BTreeMap::new()).unwrap();
    assert_eq!(s.merged, 1);
    assert_eq!(w.file(GLOBAL).as_deref(), Some("A1\nl2\nl3\nl4\nR5\n"));
}

#[test]
fn race_with_secret_returns_secrets_and_base() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), FIVE);
    a.sync();
    write(&a.global(), "A1\nl2\nl3\nl4\nl5\n");
    let plan = a.prepare();
    let r1 = plan.remote_commit.clone();
    let token = format!("ghp_{}", "a1B2c3D4e5".repeat(4));
    w.push_raw(&[(GLOBAL, Some(&format!("l1\nl2\nl3\nl4\nkey {token}\n")))]);
    let head = w.head();
    match a.finish(&plan, &BTreeMap::new()) {
        Err(SyncError::Secrets { hits, base }) => {
            assert_eq!(hits[0].repo_path, GLOBAL);
            assert_eq!(base.unwrap().commit, r1);
        }
        other => panic!("expected Secrets, got {other:?}"),
    }
    assert_eq!(w.head(), head);
}

#[test]
fn push_failure_persists_base_and_next_sync_is_clean() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), FIVE);
    a.sync();
    w.push_raw(&[(GLOBAL, Some("R1\nl2\nl3\nl4\nl5\n"))]);
    write(&a.global(), "A1\nl2\nl3\nl4\nl5\n");
    let plan = a.prepare();
    assert_eq!(plan.conflicts.len(), 1);
    let r1 = plan.remote_commit.clone();

    // the remote becomes unreachable between prepare and push
    let moved = w.bare.with_extension("away");
    fs::rename(&w.bare, &moved).unwrap();
    let resolved = "A1 R1\nl2\nl3\nl4\nl5\n";
    let r = a.finish(&plan, &text_res(GLOBAL, resolved));
    fs::rename(&moved, &w.bare).unwrap();
    match &r {
        Err(SyncError::PushFailed { base, .. }) => assert_eq!(base.commit, r1),
        other => panic!("expected PushFailed, got {other:?}"),
    }
    assert_eq!(read(&a.global()), resolved);
    assert_eq!(a.base, r1, "caller persisted the new base");

    // next sync: no replayed conflict, the resolution is pushed
    let plan = a.prepare();
    assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);
    assert_eq!(plan.push_local, vec![GLOBAL.to_string()]);
    a.finish(&plan, &BTreeMap::new()).unwrap();
    assert_eq!(w.file(GLOBAL).as_deref(), Some(resolved));
}

#[test]
fn moved_project_dir_pulls_not_deletes() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    write(&pa.join("CLAUDE.md"), "project memory\n");
    a.sync();
    let mut b = w.machine("B");
    let old = b.add_project();
    b.sync();
    assert_eq!(read(&old.join("CLAUDE.md")), "project memory\n");

    // the project is re-cloned somewhere else on B, without memory files
    fs::remove_dir_all(&old).unwrap();
    b.projects.clear();
    let new = b.add_project_at("p-moved");
    let plan = b.prepare();
    assert!(plan.push_local.is_empty() && plan.local_all_deleted.is_empty());
    assert_eq!(plan.take_remote, vec![p_path("claude/CLAUDE.md")]);
    let s = b.finish(&plan, &BTreeMap::new()).unwrap();
    assert_eq!(read(&new.join("CLAUDE.md")), "project memory\n");
    assert_eq!(w.file(&p_path("claude/CLAUDE.md")).as_deref(), Some("project memory\n"));
    assert_eq!(s.synced_projects[0].dir, normalize_path(&new.to_string_lossy()));
}

#[test]
fn all_files_deleted_in_place_is_held() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    write(&pa.join("CLAUDE.md"), "c\n");
    write(&pa.join("AGENTS.md"), "g\n");
    a.sync();
    let mut b = w.machine("B");
    let pb = b.add_project();
    b.sync();
    fs::remove_file(pb.join("CLAUDE.md")).unwrap();
    fs::remove_file(pb.join("AGENTS.md")).unwrap();
    let head = w.head();

    let (claude, agents) = (p_path("claude/CLAUDE.md"), p_path("codex/AGENTS.md"));
    let plan = b.prepare();
    let mut held = plan.local_all_deleted.clone();
    held.sort();
    assert_eq!(held, vec![claude.clone(), agents.clone()]);
    assert!(plan.push_local.is_empty() && plan.take_remote.is_empty());
    // default: nothing happens on either side
    let s = b.finish(&plan, &BTreeMap::new()).unwrap();
    assert!(!s.committed);
    assert_eq!(w.head(), head);
    assert!(!pb.join("CLAUDE.md").exists());

    let plan = b.prepare();
    assert_eq!(plan.local_all_deleted.len(), 2);
    let res = BTreeMap::from([
        (claude.clone(), Resolution::KeepRemote),
        (agents.clone(), Resolution::DeleteRemote),
    ]);
    b.finish(&plan, &res).unwrap();
    assert_eq!(read(&pb.join("CLAUDE.md")), "c\n");
    assert_eq!(w.file(&claude).as_deref(), Some("c\n"));
    assert_eq!(w.file(&agents), None);
    assert!(!pb.join("AGENTS.md").exists());
}

#[test]
fn conflict_with_deleted_side_accepts_delete_local() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    write(&pa.join("CLAUDE.md"), "v1\n");
    write(&pa.join("AGENTS.md"), "keep project synced\n");
    a.sync();
    // remote modified, local deleted
    w.push_raw(&[(&p_path("claude/CLAUDE.md"), Some("v2\n"))]);
    fs::remove_file(pa.join("CLAUDE.md")).unwrap();
    let plan = a.prepare();
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].local, None);
    let res = BTreeMap::from([(p_path("claude/CLAUDE.md"), Resolution::DeleteLocal)]);
    a.finish(&plan, &res).unwrap();
    assert_eq!(w.file(&p_path("claude/CLAUDE.md")), None);
    assert!(!pa.join("CLAUDE.md").exists());

    // remote deleted, local modified
    w.push_raw(&[(&p_path("codex/AGENTS.md"), None)]);
    write(&pa.join("AGENTS.md"), "changed here\n");
    let plan = a.prepare();
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].remote, None);
    let res = BTreeMap::from([(p_path("codex/AGENTS.md"), Resolution::DeleteLocal)]);
    a.finish(&plan, &res).unwrap();
    assert!(!pa.join("AGENTS.md").exists());
    assert_eq!(a.snaps.list(&pa.join("AGENTS.md")).unwrap().len(), 1);
    assert_eq!(w.file(&p_path("codex/AGENTS.md")), None);
}

#[test]
fn case_collision_is_skipped() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    let mem = fake_session(&a, &pa);
    write(&a.global(), "g\n");
    a.sync();
    let (up, low) = (p_path("claude/memory/Note.md"), p_path("claude/memory/note.md"));
    w.push_raw(&[(&up, Some("1\n")), (&low, Some("2\n"))]);
    let plan = a.prepare();
    let mut skipped: Vec<(String, String)> = plan
        .skipped
        .iter()
        .map(|s| (s.repo_path.clone(), s.reason.clone()))
        .collect();
    skipped.sort();
    assert_eq!(
        skipped,
        vec![(up.clone(), "case_collision".into()), (low.clone(), "case_collision".into())]
    );
    a.finish(&plan, &BTreeMap::new()).unwrap();
    assert!(fs::read_dir(&mem).unwrap().next().is_none());
    assert!(w.file(&up).is_some() && w.file(&low).is_some());
}

#[test]
fn skills_json_is_scanned() {
    use crate::model::Tool;
    use crate::sync::manifest::ManifestScope;
    let w = World::new();
    let a = w.machine("A");
    let token = format!("ghp_{}", "a1B2c3D4e5".repeat(4));
    let skill = ManifestSkill {
        name: token,
        url: "https://github.com/x/skills".into(),
        git_ref: None,
        commit: None,
        skill_path: "s".into(),
        tool: Tool::Claude,
        scope: ManifestScope::Global,
        installed_on: Default::default(),
        removed_on: Default::default(),
    };
    let ctx = SyncCtx {
        local_skills: Some(vec![skill]),
        ..a.ctx()
    };
    let head = w.head();
    let plan = prepare(&ctx).unwrap();
    assert!(plan.secret_hits.iter().any(|h| h.repo_path == SKILLS_JSON));
    assert!(matches!(
        finish(&ctx, &plan, &BTreeMap::new()),
        Err(SyncError::Secrets { .. })
    ));
    assert_eq!(w.head(), head);
}

#[test]
fn many_files_use_few_git_processes() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    let mem = fake_session(&a, &pa);
    for i in 0..200 {
        write(&mem.join(format!("m{i:03}.md")), &format!("memory {i}\n"));
    }
    let t0 = std::time::Instant::now();
    let before = a.git.spawn_count();
    let s = a.sync();
    let used = a.git.spawn_count() - before;
    assert_eq!(s.pushed, 200);
    assert!(used <= 20, "first sync of 200 files used {used} git processes");

    let mut b = w.machine("B");
    let pb = b.add_project();
    let mem_b = fake_session(&b, &pb);
    let before = b.git.spawn_count();
    let s = b.sync();
    let used = b.git.spawn_count() - before;
    assert_eq!(s.pulled, 200);
    assert!(used <= 20, "pull of 200 files used {used} git processes");
    assert_eq!(read(&mem_b.join("m199.md")), "memory 199\n");

    // nothing changed: still bounded
    let before = a.git.spawn_count();
    a.sync();
    assert!(a.git.spawn_count() - before <= 12);
    eprintln!("many_files_use_few_git_processes: {:?}", t0.elapsed());
}

// ------------------------------------------------------ fix round 2 tests

fn bare_bytes(w: &World, path: &str) -> Option<Vec<u8>> {
    SyncGit::new().read_at(&w.bare, "main", path).unwrap()
}

#[test]
fn remote_non_utf8_files_survive_repeat_syncs() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    write(&pa.join("CLAUDE.md"), "project\n");
    write(&a.global(), "mine\n");
    a.sync();
    let utf16: Vec<u8> = vec![0xFF, 0xFE, b'h', 0, b'i', 0];
    let gbk: Vec<u8> = vec![0xC4, 0xE3, 0xBA, 0xC3, b'\n'];
    let agents = p_path("codex/AGENTS.md");
    w.push_entries(&[
        (agents.as_str(), Some(TreeBlob::Data(utf16.clone()))),
        (CODEX, Some(TreeBlob::Data(gbk.clone()))),
        (GLOBAL, Some(TreeBlob::Data(utf16.clone()))),
    ]);
    for round in 0..3 {
        let plan = a.prepare();
        let mut skipped: Vec<String> = plan.skipped.iter().map(|s| s.repo_path.clone()).collect();
        skipped.sort();
        let mut want = vec![agents.clone(), CODEX.to_string(), GLOBAL.to_string()];
        want.sort();
        assert_eq!(skipped, want, "round {round}");
        assert!(plan.push_local.is_empty(), "round {round}: {:?}", plan.push_local);
        a.finish(&plan, &BTreeMap::new()).unwrap();
        assert_eq!(bare_bytes(&w, &agents), Some(utf16.clone()), "round {round}");
        assert_eq!(bare_bytes(&w, CODEX), Some(gbk.clone()), "round {round}");
        assert_eq!(bare_bytes(&w, GLOBAL), Some(utf16.clone()), "round {round}");
        assert_eq!(read(&a.global()), "mine\n");
        assert!(!pa.join("AGENTS.md").exists());
    }
}

#[test]
fn case_collision_survivor_is_not_deleted() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    let mem = fake_session(&a, &pa);
    write(&pa.join("CLAUDE.md"), "c\n");
    a.sync();
    let (up, low) = (p_path("claude/memory/Note.md"), p_path("claude/memory/note.md"));
    w.push_raw(&[(&up, Some("1\n")), (&low, Some("2\n"))]);
    a.sync();
    // another machine removes one of the two
    w.push_raw(&[(&low, None)]);
    let plan = a.prepare();
    assert!(plan.push_local.is_empty(), "{:?}", plan.push_local);
    assert_eq!(plan.take_remote, vec![up.clone()]);
    a.finish(&plan, &BTreeMap::new()).unwrap();
    assert_eq!(w.file(&up).as_deref(), Some("1\n"));
    assert_eq!(read(&mem.join("Note.md")), "1\n");
}

#[test]
fn converted_local_file_is_conflict_not_overwrite() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), "g\n");
    a.sync();
    w.push_raw(&[(CODEX, Some("remote codex\n"))]);
    fs::write(a.codex(), [0xFF, 0xFE, b'x', 0]).unwrap();
    let plan = a.prepare();
    assert_eq!(plan.skipped.len(), 1);
    a.finish(&plan, &BTreeMap::new()).unwrap();
    // the user converts the file to UTF-8 later
    write(&a.codex(), "local codex\n");
    let plan = a.prepare();
    assert!(plan.push_local.is_empty(), "silent overwrite: {:?}", plan.push_local);
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].repo_path, CODEX);
    assert_eq!(plan.conflicts[0].base, None);
}

#[test]
fn held_project_stays_held_with_new_local_file() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    write(&pa.join("CLAUDE.md"), "c\n");
    write(&pa.join("AGENTS.md"), "g\n");
    a.sync();
    let mut b = w.machine("B");
    let pb = b.add_project();
    b.sync();
    fs::remove_file(pb.join("CLAUDE.md")).unwrap();
    fs::remove_file(pb.join("AGENTS.md")).unwrap();
    let plan = b.prepare();
    assert_eq!(plan.local_all_deleted.len(), 2);
    b.finish(&plan, &BTreeMap::new()).unwrap();

    // a new memory file appears in the held project
    write(&pb.join("CLAUDE.local.md"), "local notes\n");
    let plan = b.prepare();
    assert_eq!(plan.push_local, vec![p_path("claude/CLAUDE.local.md")]);
    assert_eq!(plan.local_all_deleted.len(), 2, "still held");
    b.finish(&plan, &BTreeMap::new()).unwrap();
    assert_eq!(w.file(&p_path("claude/CLAUDE.md")).as_deref(), Some("c\n"));
    assert_eq!(w.file(&p_path("codex/AGENTS.md")).as_deref(), Some("g\n"));
    assert_eq!(w.file(&p_path("claude/CLAUDE.local.md")).as_deref(), Some("local notes\n"));
    let plan = b.prepare();
    assert_eq!(plan.local_all_deleted.len(), 2, "held across syncs");
}

#[test]
fn delete_remote_choice_survives_race() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    write(&pa.join("CLAUDE.md"), "c\n");
    write(&pa.join("AGENTS.md"), "g\n");
    a.sync();
    let mut b = w.machine("B");
    let pb = b.add_project();
    b.sync();
    fs::remove_file(pb.join("CLAUDE.md")).unwrap();
    fs::remove_file(pb.join("AGENTS.md")).unwrap();
    let plan = b.prepare();
    let res: BTreeMap<String, Resolution> = plan
        .local_all_deleted
        .iter()
        .map(|p| (p.clone(), Resolution::DeleteRemote))
        .collect();
    w.push_raw(&[(GLOBAL, Some("raced\n"))]);
    let s = b.finish(&plan, &res).unwrap();
    assert!(s.committed);
    assert_eq!(w.file(&p_path("claude/CLAUDE.md")), None);
    assert_eq!(w.file(&p_path("codex/AGENTS.md")), None);
    assert_eq!(w.file(GLOBAL).as_deref(), Some("raced\n"));
    let plan = b.prepare();
    assert!(plan.local_all_deleted.is_empty());
}

#[test]
fn changed_choice_target_after_race_is_remote_moved() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    write(&pa.join("CLAUDE.md"), "c\n");
    write(&pa.join("AGENTS.md"), "g\n");
    a.sync();
    let mut b = w.machine("B");
    let pb = b.add_project();
    b.sync();
    fs::remove_file(pb.join("CLAUDE.md")).unwrap();
    fs::remove_file(pb.join("AGENTS.md")).unwrap();
    let plan = b.prepare();
    let res: BTreeMap<String, Resolution> = plan
        .local_all_deleted
        .iter()
        .map(|p| (p.clone(), Resolution::DeleteRemote))
        .collect();
    // the file the user chose to delete changes remotely in between
    w.push_raw(&[(&p_path("claude/CLAUDE.md"), Some("c2\n"))]);
    let head = w.head();
    let r = b.finish(&plan, &res);
    assert!(matches!(r, Err(SyncError::RemoteMoved { .. })), "{r:?}");
    assert_eq!(w.head(), head);
    assert_eq!(w.file(&p_path("claude/CLAUDE.md")).as_deref(), Some("c2\n"));
}

#[test]
fn exec_and_gitlink_entries_survive_sync() {
    let w = World::new();
    let mut a = w.machine("A");
    write(&a.global(), "g\n");
    a.sync();
    let head = w.head();
    let g = SyncGit::new();
    let exe_sha = blob_id(b"#!/bin/sh\n");
    w.push_entries(&[("tools/run.sh", Some(TreeBlob::Data(b"#!/bin/sh\n".to_vec())))]);
    let exe = crate::sync::repo::TreeEntry {
        mode: "100755".into(),
        kind: "blob".into(),
        sha: exe_sha,
    };
    let link = crate::sync::repo::TreeEntry {
        mode: "160000".into(),
        kind: "commit".into(),
        sha: head,
    };
    w.push_entries(&[
        ("tools/run.sh", Some(TreeBlob::Existing(exe.clone()))),
        ("vendor/sub", Some(TreeBlob::Existing(link.clone()))),
    ]);
    write(&a.global(), "g2\n");
    let s = a.sync();
    assert!(s.committed);
    let e = g.tree_entries(&w.bare, "main").unwrap();
    assert_eq!(e["tools/run.sh"], exe);
    assert_eq!(e["vendor/sub"], link);
    assert_eq!(e[GLOBAL].mode, "100644");
}

#[test]
fn duplicate_local_mapping_is_skipped() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    fake_session(&a, &pa);
    write(&pa.join("CLAUDE.md"), "c\n");
    a.sync();
    // NFC and NFD spellings of the same file name map to one local file
    let nfc = p_path("claude/memory/caf\u{e9}.md");
    let nfd = p_path("claude/memory/cafe\u{301}.md");
    w.push_raw(&[(&nfc, Some("one\n")), (&nfd, Some("two\n"))]);
    let plan = a.prepare();
    let dups: Vec<&SkippedFile> = plan.skipped.iter().filter(|s| s.reason == "duplicate_local").collect();
    assert_eq!(dups.len(), 1, "{:?}", plan.skipped);
    assert_eq!(plan.take_remote.len(), 1);
    a.finish(&plan, &BTreeMap::new()).unwrap();
    assert!(w.file(&nfc).is_some() && w.file(&nfd).is_some());
}

// ------------------------------------------------------ fix round 3 tests

#[test]
fn memory_dir_appearing_later_pulls_not_deletes() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    write(&pa.join("CLAUDE.md"), "c\n");
    a.sync();
    let foo = p_path("claude/memory/foo.md");
    w.push_raw(&[(&foo, Some("remote memory\n"))]);
    // no Claude session for the project yet: memory files cannot map here
    let plan = a.prepare();
    assert!(plan.take_remote.is_empty());
    a.finish(&plan, &BTreeMap::new()).unwrap();
    // the first Claude session creates the enc dir with an empty memory dir
    let mem = fake_session(&a, &pa);
    let plan = a.prepare();
    assert!(plan.push_local.is_empty(), "would delete: {:?}", plan.push_local);
    assert_eq!(plan.take_remote, vec![foo.clone()]);
    a.finish(&plan, &BTreeMap::new()).unwrap();
    assert_eq!(w.file(&foo).as_deref(), Some("remote memory\n"));
    assert_eq!(read(&mem.join("foo.md")), "remote memory\n");
}

#[test]
fn memory_dir_wiped_and_recreated_pulls_not_deletes() {
    let w = World::new();
    let mut a = w.machine("A");
    let pa = a.add_project();
    let mem = fake_session(&a, &pa);
    write(&pa.join("CLAUDE.md"), "c\n");
    write(&mem.join("foo.md"), "memory\n");
    a.sync();
    let foo = p_path("claude/memory/foo.md");
    assert_eq!(w.file(&foo).as_deref(), Some("memory\n"));
    // session cleanup removes the whole enc dir, a sync runs in between
    fs::remove_dir_all(mem.parent().unwrap()).unwrap();
    a.sync();
    assert_eq!(w.file(&foo).as_deref(), Some("memory\n"));
    // a new session recreates it empty
    let mem = fake_session(&a, &pa);
    let plan = a.prepare();
    assert!(plan.push_local.is_empty(), "would delete: {:?}", plan.push_local);
    a.finish(&plan, &BTreeMap::new()).unwrap();
    assert_eq!(w.file(&foo).as_deref(), Some("memory\n"));
    assert_eq!(read(&mem.join("foo.md")), "memory\n");
}
