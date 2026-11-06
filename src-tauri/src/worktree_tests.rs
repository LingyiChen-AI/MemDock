use super::*;
use tempfile::TempDir;

struct Fx {
    _tmp: TempDir,
    repo: PathBuf,
    data: PathBuf,
}

fn sh(dir: &Path, args: &[&str]) -> String {
    let mut all = vec!["-c", "user.name=Test", "-c", "user.email=t@example.com"];
    all.extend_from_slice(args);
    git(dir, &all).unwrap_or_else(|e| panic!("git {args:?}: {e}"))
}

fn write(p: &Path, s: &str) {
    if let Some(d) = p.parent() {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(p, s).unwrap();
}

fn setup() -> Fx {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    // Commits made by the code under test need an identity too.
    sh(&repo, &["config", "user.name", "Test"]);
    sh(&repo, &["config", "user.email", "t@example.com"]);
    sh(&repo, &["config", "core.autocrlf", "false"]);
    write(&repo.join("a.txt"), "one\ntwo\n");
    write(&repo.join("b.txt"), "keep\n");
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-q", "-m", "init"]);
    let data = tmp.path().join("data");
    Fx { _tmp: tmp, repo, data }
}

fn mk(fx: &Fx, hint: &str) -> WorktreeInfo {
    create(&fx.data, &fx.repo, hint).unwrap()
}

#[test]
fn slugify_rules() {
    assert_eq!(slugify("Fix the Login Bug!"), "fix-the-login-bug");
    assert_eq!(slugify("修复登录"), "");
    assert_eq!(slugify("修复 login 问题"), "login");
    assert_eq!(slugify(&"x".repeat(60)).len(), 40);
}

#[test]
fn create_sets_branch_path_and_exclude() {
    let fx = setup();
    let w = mk(&fx, "Add feature");
    assert_eq!(w.slug, "add-feature");
    assert_eq!(w.branch, "memdock/add-feature");
    assert_eq!(w.base_branch, "main");
    assert_eq!(w.base_commit, sh(&fx.repo, &["rev-parse", "HEAD"]));
    assert!(norm(&w.path).ends_with("/.memdock/worktrees/add-feature"));
    assert!(Path::new(&w.path).join("a.txt").is_file());
    let ex = fs::read_to_string(fx.repo.join(".git/info/exclude")).unwrap();
    assert_eq!(ex.matches(".memdock/").count(), 1);
    // Second create: unique slug, exclude not duplicated, main stays clean.
    let w2 = mk(&fx, "Add feature");
    assert_eq!(w2.slug, "add-feature-2");
    let ex = fs::read_to_string(fx.repo.join(".git/info/exclude")).unwrap();
    assert_eq!(ex.matches(".memdock/").count(), 1);
    assert_eq!(sh(&fx.repo, &["status", "--porcelain"]), "");
    // Chinese hint falls back to session-<date>.
    let w3 = mk(&fx, "中文提示");
    assert!(w3.slug.starts_with("session-"), "{}", w3.slug);
    // Created from inside a worktree resolves to the main repo.
    let w4 = create(&fx.data, Path::new(&w.path), "nested").unwrap();
    assert_eq!(norm(&w4.repo_root), norm(&w.repo_root));
    assert_eq!(list(&fx.data).unwrap().len(), 4);
    let c = check(Path::new(&w.path)).unwrap();
    assert_eq!(c.base_branch.as_deref(), Some("main"));
}

#[test]
fn status_and_diff_cover_committed_uncommitted_untracked() {
    let fx = setup();
    let w = mk(&fx, "work");
    let wt = PathBuf::from(&w.path);
    write(&wt.join("c.txt"), "c1\nc2\nc3\n");
    sh(&wt, &["add", "c.txt"]);
    sh(&wt, &["commit", "-q", "-m", "add c"]);
    write(&wt.join("a.txt"), "one\nTWO\nthree\n");
    write(&wt.join("new.txt"), "n1\nn2\n");
    fs::remove_file(wt.join("b.txt")).unwrap();
    let s = status(&fx.data, &w.path).unwrap();
    assert_eq!(s.ahead, 1);
    assert!(s.dirty);
    let get = |p: &str| s.files.iter().find(|f| f.path == p).cloned().unwrap();
    assert_eq!(get("c.txt"), ChangedFile { path: "c.txt".into(), status: "A".into(), added: 3, deleted: 0 });
    assert_eq!(get("a.txt"), ChangedFile { path: "a.txt".into(), status: "M".into(), added: 2, deleted: 1 });
    assert_eq!(get("b.txt").status, "D");
    assert_eq!(get("new.txt"), ChangedFile { path: "new.txt".into(), status: "A".into(), added: 2, deleted: 0 });
    assert_eq!(s.files.len(), 4);

    let d = diff(&fx.data, &w.path, None).unwrap();
    assert!(d.contains("+TWO"), "{d}");
    assert!(d.contains("+c3"));
    assert!(d.contains("+n2"));
    assert!(d.contains("-keep"));
    let one = diff(&fx.data, &w.path, Some("new.txt")).unwrap();
    assert!(one.contains("+n1") && !one.contains("TWO"), "{one}");
    let one = diff(&fx.data, &w.path, Some("a.txt")).unwrap();
    assert!(one.contains("+TWO") && !one.contains("n1"));
    assert!(diff(&fx.data, &w.path, Some("../x")).is_err());
}

#[test]
fn squash_merge_autocommits_and_deletes() {
    let fx = setup();
    let w = mk(&fx, "sq");
    let wt = PathBuf::from(&w.path);
    write(&wt.join("a.txt"), "one\ntwo\nthree\n");
    let r = merge(&fx.data, &w.path, "squash", "Squash it", true).unwrap();
    assert!(r.merged && r.auto_committed && r.conflicts.is_empty());
    let head = sh(&fx.repo, &["rev-parse", "HEAD"]);
    assert_eq!(r.commit.as_deref(), Some(head.as_str()));
    assert_eq!(sh(&fx.repo, &["log", "-1", "--format=%s"]), "Squash it");
    assert_eq!(sh(&fx.repo, &["rev-list", "--count", "HEAD"]), "2");
    assert_eq!(fs::read_to_string(fx.repo.join("a.txt")).unwrap(), "one\ntwo\nthree\n");
    assert!(!wt.exists());
    assert!(!branch_exists(&fx.repo, &w.branch).unwrap());
    assert!(list(&fx.data).unwrap().is_empty());
}

#[test]
fn squash_with_nothing_to_commit() {
    let fx = setup();
    let w = mk(&fx, "empty");
    let r = merge(&fx.data, &w.path, "squash", "noop", false).unwrap();
    assert!(r.merged && !r.auto_committed && r.commit.is_none());
    assert!(Path::new(&w.path).exists());
}

#[test]
fn no_ff_merge_and_keep() {
    let fx = setup();
    let w = mk(&fx, "mf");
    let wt = PathBuf::from(&w.path);
    write(&wt.join("z.txt"), "z\n");
    sh(&wt, &["add", "-A"]);
    sh(&wt, &["commit", "-q", "-m", "z"]);
    let r = merge(&fx.data, &w.path, "merge", "Merge mf", false).unwrap();
    assert!(r.merged && !r.auto_committed && r.commit.is_some());
    assert_eq!(sh(&fx.repo, &["log", "-1", "--format=%s %p"]).split(' ').count(), 4);
    assert!(fx.repo.join("z.txt").is_file());
    assert!(wt.exists() && branch_exists(&fx.repo, &w.branch).unwrap());

    let k = mk(&fx, "kp");
    let kt = PathBuf::from(&k.path);
    write(&kt.join("k.txt"), "k\n");
    let before = sh(&fx.repo, &["rev-parse", "HEAD"]);
    let r = merge(&fx.data, &k.path, "keep", "keep it", true).unwrap();
    assert!(r.merged && r.auto_committed && r.commit.is_none());
    assert_eq!(sh(&fx.repo, &["rev-parse", "HEAD"]), before);
    assert!(!kt.exists());
    assert!(branch_exists(&fx.repo, &k.branch).unwrap());
    assert_eq!(list(&fx.data).unwrap().len(), 1);
}

#[test]
fn conflicts_leave_main_untouched() {
    for mode in ["squash", "merge"] {
        let fx = setup();
        let w = mk(&fx, "cf");
        let wt = PathBuf::from(&w.path);
        write(&wt.join("a.txt"), "one\nWT\n");
        write(&fx.repo.join("a.txt"), "one\nMAIN\n");
        sh(&fx.repo, &["commit", "-q", "-am", "main change"]);
        let head = sh(&fx.repo, &["rev-parse", "HEAD"]);
        // Base moved: status measures against the merge-base.
        let s = status(&fx.data, &w.path).unwrap();
        assert_eq!(s.files.len(), 1);
        let r = merge(&fx.data, &w.path, mode, "try", true).unwrap();
        assert!(!r.merged, "{mode}");
        assert_eq!(r.conflicts, vec!["a.txt".to_string()]);
        assert_eq!(sh(&fx.repo, &["rev-parse", "HEAD"]), head);
        assert_eq!(sh(&fx.repo, &["status", "--porcelain"]), "");
        assert!(!fx.repo.join(".git/MERGE_HEAD").exists());
        assert_eq!(fs::read_to_string(fx.repo.join("a.txt")).unwrap(), "one\nMAIN\n");
        assert!(wt.exists(), "not deleted on conflict");
    }
}

#[test]
fn dirty_main_or_wrong_branch_is_refused() {
    let fx = setup();
    let w = mk(&fx, "d");
    write(&Path::new(&w.path).join("x.txt"), "x\n");
    write(&fx.repo.join("b.txt"), "dirty\n");
    let e = merge(&fx.data, &w.path, "squash", "m", false).unwrap_err();
    assert!(e.contains("未提交"), "{e}");
    sh(&fx.repo, &["checkout", "-q", "--", "b.txt"]);
    sh(&fx.repo, &["checkout", "-q", "-b", "other"]);
    let e = merge(&fx.data, &w.path, "merge", "m", false).unwrap_err();
    assert!(e.contains("other"), "{e}");
}

#[test]
fn detached_head_is_refused() {
    let fx = setup();
    sh(&fx.repo, &["checkout", "-q", "--detach"]);
    let c = check(&fx.repo).unwrap();
    assert_eq!(c.base_branch, None);
    let e = create(&fx.data, &fx.repo, "x").unwrap_err();
    assert!(e.contains("detached"), "{e}");
    let tmp = TempDir::new().unwrap();
    assert!(check(tmp.path()).is_err());
}

#[test]
fn remove_tolerates_missing_dir_and_list_drops_missing() {
    let fx = setup();
    let a = mk(&fx, "a");
    let b = mk(&fx, "b");
    fs::remove_dir_all(&a.path).unwrap();
    let l = list(&fx.data).unwrap();
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].slug, "b");
    // b: directory removed by hand, then remove() prunes and deletes branch.
    fs::remove_dir_all(&b.path).unwrap();
    // Re-register b so remove can find it (list dropped it).
    let _g = STORE_LOCK.lock().unwrap();
    save_store(&fx.data, std::slice::from_ref(&b)).unwrap();
    drop(_g);
    remove(&fx.data, &b.path, true).unwrap();
    assert!(!branch_exists(&fx.repo, &b.branch).unwrap());
    let wl = sh(&fx.repo, &["worktree", "list", "--porcelain"]);
    assert!(!wl.contains("/b\n") && !wl.ends_with("/b"), "{wl}");
    // Normal remove keeping the branch.
    let c = mk(&fx, "c");
    remove(&fx.data, &c.path, false).unwrap();
    assert!(!Path::new(&c.path).exists());
    assert!(branch_exists(&fx.repo, &c.branch).unwrap());
    assert!(list(&fx.data).unwrap().is_empty());
}
