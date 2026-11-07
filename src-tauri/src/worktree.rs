//! Worktree sessions: isolated `git worktree` checkouts under
//! `<repo_root>/.memdock/worktrees/<slug>` on branches `memdock/<slug>`,
//! with status / diff / merge back into the base branch. Registered
//! worktrees persist in `<data_dir>/worktrees.json`.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::AppState;

const GIT_TIMEOUT: Duration = Duration::from_secs(60);
const STORE_FILE: &str = "worktrees.json";
const BRANCH_PREFIX: &str = "memdock/";
const MAX_SLUG: usize = 40;

/// Serializes load-modify-save of `worktrees.json`.
static STORE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: String,
    pub slug: String,
    pub repo_root: String,
    pub base_branch: String,
    pub base_commit: String,
    /// Unix epoch milliseconds.
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorktreeCheck {
    pub repo_root: String,
    pub base_branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChangedFile {
    pub path: String,
    /// "A" | "M" | "D" | "R"
    pub status: String,
    pub added: u32,
    pub deleted: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorktreeStatus {
    pub files: Vec<ChangedFile>,
    pub ahead: u32,
    pub dirty: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MergeResult {
    pub merged: bool,
    pub conflicts: Vec<String>,
    pub auto_committed: bool,
    pub commit: Option<String>,
}

// ---------- git plumbing ----------

struct GitOut {
    ok: bool,
    stdout: String,
    stderr: String,
}

/// Runs `git -C <dir> <args>` hidden with a timeout. Err only when git could
/// not run (missing / timeout); a non-zero exit is reported via `ok`.
fn git_raw(dir: &Path, args: &[&str]) -> Result<GitOut, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .args(["-c", "core.quotepath=false", "-c", "color.ui=never"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = crate::proc::spawn_hidden(&mut cmd).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "未找到 git，请先安装 Git".to_string()
        } else {
            format!("无法运行 git：{e}")
        }
    })?;
    let out = crate::proc::wait_with_timeout(child, GIT_TIMEOUT, None).map_err(|e| match e {
        crate::proc::WaitError::Timeout => format!("git {} 超时", args.join(" ")),
        crate::proc::WaitError::Cancelled => "git 已取消".to_string(),
        crate::proc::WaitError::Io(m) => format!("git 运行失败：{m}"),
    })?;
    Ok(GitOut {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Runs git and returns trimmed stdout, failing on a non-zero exit.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let o = git_raw(dir, args)?;
    if o.ok {
        Ok(o.stdout.trim().to_string())
    } else {
        let msg = o.stderr.trim();
        let msg = if msg.is_empty() { o.stdout.trim() } else { msg };
        Err(format!("git {} 失败：{}", args.first().unwrap_or(&""), msg))
    }
}

fn split_z(s: &str) -> Vec<&str> {
    s.split('\0').filter(|p| !p.is_empty()).collect()
}

/// Comparable form of a path: forward slashes, no trailing slash, and
/// case-insensitive on Windows.
fn norm(p: &str) -> String {
    let s = p.replace('\\', "/");
    let s = s.trim_end_matches('/').to_string();
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s
    }
}

// ---------- repo helpers ----------

/// Root of the main worktree (first entry of `git worktree list`).
fn main_root(cwd: &Path) -> Result<PathBuf, String> {
    if !cwd.is_dir() {
        return Err(format!("目录不存在：{}", cwd.display()));
    }
    let o = git_raw(cwd, &["worktree", "list", "--porcelain"])?;
    if !o.ok {
        return Err("该目录不是 git 仓库".into());
    }
    o.stdout
        .lines()
        .find_map(|l| l.strip_prefix("worktree "))
        .map(PathBuf::from)
        .ok_or_else(|| "该目录不是 git 仓库".into())
}

fn current_branch(dir: &Path) -> Result<Option<String>, String> {
    let o = git_raw(dir, &["symbolic-ref", "--short", "-q", "HEAD"])?;
    let b = o.stdout.trim();
    Ok(if o.ok && !b.is_empty() { Some(b.to_string()) } else { None })
}

fn branch_exists(root: &Path, branch: &str) -> Result<bool, String> {
    let r = format!("refs/heads/{branch}");
    Ok(git_raw(root, &["show-ref", "--verify", "--quiet", &r])?.ok)
}

/// Adds `.memdock/` to `<git-common-dir>/info/exclude` once.
fn ensure_excluded(root: &Path) -> Result<(), String> {
    let common = git(root, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let info = PathBuf::from(common).join("info");
    fs::create_dir_all(&info).map_err(|e| format!("创建 info 目录失败：{e}"))?;
    let file = info.join("exclude");
    let cur = fs::read_to_string(&file).unwrap_or_default();
    if cur.lines().any(|l| matches!(l.trim(), ".memdock/" | ".memdock" | "/.memdock/" | "/.memdock")) {
        return Ok(());
    }
    let mut next = cur;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(".memdock/\n");
    fs::write(&file, next).map_err(|e| format!("写入 info/exclude 失败：{e}"))
}

/// ASCII slug from `hint`: `[a-z0-9-]`, at most 40 chars; empty when the
/// hint has no usable ASCII.
pub fn slugify(hint: &str) -> String {
    let mut s = String::new();
    for c in hint.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.is_empty() && !s.ends_with('-') {
            s.push('-');
        }
    }
    let mut s: String = s.chars().take(MAX_SLUG).collect();
    while s.ends_with('-') {
        s.pop();
    }
    s
}

fn worktrees_dir(root: &Path) -> PathBuf {
    root.join(".memdock").join("worktrees")
}

/// Picks a slug unused as both a branch and a directory (suffix -2, -3 ...).
fn unique_slug(root: &Path, base: &str) -> Result<String, String> {
    let dir = worktrees_dir(root);
    for n in 1..1000 {
        let cand = if n == 1 {
            base.to_string()
        } else {
            let suffix = format!("-{n}");
            let keep = MAX_SLUG.saturating_sub(suffix.len()).min(base.len());
            format!("{}{suffix}", base[..keep].trim_end_matches('-'))
        };
        if !dir.join(&cand).exists() && !branch_exists(root, &format!("{BRANCH_PREFIX}{cand}"))? {
            return Ok(cand);
        }
    }
    Err("无法生成唯一的工作区名称".into())
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

// ---------- store ----------

fn store_path(data_dir: &Path) -> PathBuf {
    data_dir.join(STORE_FILE)
}

fn load_store(data_dir: &Path) -> Vec<WorktreeInfo> {
    fs::read(store_path(data_dir))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_store(data_dir: &Path, items: &[WorktreeInfo]) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|e| format!("创建数据目录失败：{e}"))?;
    crate::fsutil::save_json_atomic(&store_path(data_dir), &items)
        .map_err(|e| format!("保存 worktrees.json 失败：{e}"))
}

fn find_info(data_dir: &Path, path: &str) -> Result<WorktreeInfo, String> {
    let key = norm(path);
    load_store(data_dir)
        .into_iter()
        .find(|w| norm(&w.path) == key)
        .ok_or_else(|| format!("不是 MemDock 管理的工作区：{path}"))
}

fn drop_from_store(data_dir: &Path, path: &str) -> Result<(), String> {
    let _g = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let key = norm(path);
    let mut items = load_store(data_dir);
    let before = items.len();
    items.retain(|w| norm(&w.path) != key);
    if items.len() != before {
        save_store(data_dir, &items)?;
    }
    Ok(())
}

// ---------- operations ----------

pub fn check(cwd: &Path) -> Result<WorktreeCheck, String> {
    let root = main_root(cwd)?;
    Ok(WorktreeCheck {
        repo_root: path_str(&root),
        base_branch: current_branch(&root)?,
    })
}

pub fn create(data_dir: &Path, cwd: &Path, hint: &str) -> Result<WorktreeInfo, String> {
    let root = main_root(cwd)?;
    let base_branch = current_branch(&root)?.ok_or_else(|| {
        "主工作区处于游离 HEAD（detached）状态，请先切换到一个分支再创建独立工作区".to_string()
    })?;
    let base_commit = git(&root, &["rev-parse", "--verify", "HEAD"])
        .map_err(|_| "仓库还没有任何提交，无法创建独立工作区".to_string())?;
    let mut slug = slugify(hint);
    if slug.is_empty() {
        slug = format!("session-{}", chrono::Local::now().format("%Y%m%d-%H%M"));
    }
    let slug = unique_slug(&root, &slug)?;
    ensure_excluded(&root)?;
    let dir = worktrees_dir(&root);
    fs::create_dir_all(&dir).map_err(|e| format!("创建工作区目录失败：{e}"))?;
    let wt = dir.join(&slug);
    let branch = format!("{BRANCH_PREFIX}{slug}");
    let wt_s = path_str(&wt);
    git(&root, &["worktree", "add", "-b", &branch, &wt_s, &base_commit])?;
    let info = WorktreeInfo {
        path: wt_s,
        branch,
        slug,
        repo_root: path_str(&root),
        base_branch,
        base_commit,
        created_at: chrono::Utc::now().timestamp_millis(),
    };
    let _g = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut items = load_store(data_dir);
    items.retain(|w| norm(&w.path) != norm(&info.path));
    items.push(info.clone());
    save_store(data_dir, &items)?;
    Ok(info)
}

pub fn list(data_dir: &Path) -> Result<Vec<WorktreeInfo>, String> {
    let _g = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let items = load_store(data_dir);
    let before = items.len();
    let kept: Vec<WorktreeInfo> = items.into_iter().filter(|w| Path::new(&w.path).is_dir()).collect();
    if kept.len() != before {
        save_store(data_dir, &kept)?;
    }
    Ok(kept)
}

/// The commit changes are measured against: merge-base of HEAD and the
/// current base-branch tip (equals `base_commit` until the base moves).
pub(crate) fn diff_base(info: &WorktreeInfo, wt: &Path) -> Result<String, String> {
    let tip = format!("refs/heads/{}", info.base_branch);
    match git(wt, &["merge-base", &tip, "HEAD"]) {
        Ok(mb) if !mb.is_empty() => Ok(mb),
        _ => Ok(info.base_commit.clone()),
    }
}

fn untracked(wt: &Path) -> Result<Vec<String>, String> {
    let out = git(wt, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    Ok(split_z(&out).into_iter().map(String::from).collect())
}

fn count_lines(p: &Path) -> u32 {
    match fs::read(p) {
        Ok(b) if !b.contains(&0) => {
            if b.is_empty() {
                0
            } else {
                let n = b.iter().filter(|&&c| c == b'\n').count();
                (n + usize::from(!b.ends_with(b"\n"))) as u32
            }
        }
        _ => 0,
    }
}

fn is_dirty(dir: &Path) -> Result<bool, String> {
    Ok(!git(dir, &["status", "--porcelain=v1", "-z"])?.is_empty())
}

pub fn status(data_dir: &Path, path: &str) -> Result<WorktreeStatus, String> {
    let info = find_info(data_dir, path)?;
    let wt = Path::new(&info.path);
    if !wt.is_dir() {
        return Err(format!("工作区目录不存在：{}", info.path));
    }
    let base = diff_base(&info, wt)?;
    // Working tree vs base covers committed + uncommitted tracked changes.
    let ns = git(wt, &["diff", "-M", "--name-status", "-z", &base])?;
    let mut files: Vec<ChangedFile> = Vec::new();
    let toks = split_z(&ns);
    let mut i = 0;
    while i < toks.len() {
        let code = toks[i];
        let letter = code.chars().next().unwrap_or('M');
        let two = matches!(letter, 'R' | 'C');
        let p = if two { toks.get(i + 2) } else { toks.get(i + 1) };
        let Some(p) = p else { break };
        let status = match letter {
            'A' | 'C' => "A",
            'D' => "D",
            'R' => "R",
            _ => "M",
        };
        files.push(ChangedFile {
            path: p.to_string(),
            status: status.into(),
            added: 0,
            deleted: 0,
        });
        i += if two { 3 } else { 2 };
    }
    // numstat -z: "a\td\tpath\0" or, for renames, "a\td\t\0old\0new\0".
    let num = git(wt, &["diff", "-M", "--numstat", "-z", &base])?;
    let toks = split_z(&num);
    let mut i = 0;
    while i < toks.len() {
        let mut parts = toks[i].splitn(3, '\t');
        let a = parts.next().unwrap_or("").parse().unwrap_or(0);
        let d = parts.next().unwrap_or("").parse().unwrap_or(0);
        let rest = parts.next().unwrap_or("");
        let (p, step) = if rest.is_empty() {
            (toks.get(i + 2).copied().unwrap_or(""), 3)
        } else {
            (rest, 1)
        };
        if let Some(f) = files.iter_mut().find(|f| f.path == p) {
            f.added = a;
            f.deleted = d;
        }
        i += step;
    }
    for p in untracked(wt)? {
        let added = count_lines(&wt.join(&p));
        files.push(ChangedFile {
            path: p,
            status: "A".into(),
            added,
            deleted: 0,
        });
    }
    let range = format!("{base}..HEAD");
    let ahead = git(wt, &["rev-list", "--count", &range])?.parse().unwrap_or(0);
    Ok(WorktreeStatus {
        files,
        ahead,
        dirty: is_dirty(wt)?,
    })
}

fn safe_rel(file: &str) -> Result<String, String> {
    let f = file.replace('\\', "/");
    if f.is_empty()
        || f.starts_with('/')
        || f.starts_with('-')
        || f.contains(':')
        || f.split('/').any(|seg| seg == "..")
    {
        return Err(format!("无效的文件路径：{file}"));
    }
    Ok(f)
}

/// Diff of an untracked file as a new file (`git diff --no-index` exits 1 on changes).
fn new_file_diff(wt: &Path, rel: &str) -> Result<String, String> {
    let o = git_raw(wt, &["diff", "--no-index", "--no-color", "--", "/dev/null", rel])?;
    if !o.ok && o.stdout.is_empty() {
        return Err(format!("git diff 失败：{}", o.stderr.trim()));
    }
    Ok(o.stdout)
}

pub fn diff(data_dir: &Path, path: &str, file: Option<&str>) -> Result<String, String> {
    let info = find_info(data_dir, path)?;
    let wt = Path::new(&info.path);
    if !wt.is_dir() {
        return Err(format!("工作区目录不存在：{}", info.path));
    }
    let base = diff_base(&info, wt)?;
    let untracked = untracked(wt)?;
    if let Some(f) = file {
        let rel = safe_rel(f)?;
        if untracked.contains(&rel) {
            return new_file_diff(wt, &rel);
        }
        return Ok(git_raw(wt, &["diff", "-M", "--no-color", &base, "--", &rel])?.stdout);
    }
    let mut out = git_raw(wt, &["diff", "-M", "--no-color", &base])?;
    if !out.ok {
        return Err(format!("git diff 失败：{}", out.stderr.trim()));
    }
    for u in &untracked {
        let d = new_file_diff(wt, u)?;
        if !out.stdout.is_empty() && !out.stdout.ends_with('\n') {
            out.stdout.push('\n');
        }
        out.stdout.push_str(&d);
    }
    Ok(out.stdout)
}

/// Tracked changes in the main worktree, ignoring `.memdock/`.
fn main_dirty(root: &Path) -> Result<Vec<String>, String> {
    let out = git(root, &["status", "--porcelain=v1", "-z", "--untracked-files=no"])?;
    Ok(split_z(&out)
        .into_iter()
        .map(|e| e.get(3..).unwrap_or(e).to_string())
        .filter(|p| !p.starts_with(".memdock/") && p != ".memdock")
        .collect())
}

fn conflict_files(root: &Path) -> Vec<String> {
    git(root, &["diff", "--name-only", "--diff-filter=U", "-z"])
        .map(|s| split_z(&s).into_iter().map(String::from).collect())
        .unwrap_or_default()
}

pub fn merge(
    data_dir: &Path,
    path: &str,
    mode: &str,
    message: &str,
    delete_after: bool,
) -> Result<MergeResult, String> {
    if !matches!(mode, "squash" | "merge" | "keep") {
        return Err(format!("未知的合并方式：{mode}"));
    }
    let info = find_info(data_dir, path)?;
    let wt = Path::new(&info.path);
    if !wt.is_dir() {
        return Err(format!("工作区目录不存在：{}", info.path));
    }
    let message = match message.trim() {
        "" => format!("Merge {}", info.branch),
        m => m.to_string(),
    };
    let mut res = MergeResult {
        merged: false,
        conflicts: vec![],
        auto_committed: false,
        commit: None,
    };
    // Commits what is still uncommitted in the worktree; done only once the merge can go ahead.
    let auto_commit = |res: &mut MergeResult| -> Result<(), String> {
        if is_dirty(wt)? {
            git(wt, &["add", "-A"])?;
            git(wt, &["commit", "-q", "--no-verify", "-m", &format!("MemDock: {message}")])?;
            res.auto_committed = true;
        }
        Ok(())
    };
    if mode == "keep" {
        auto_commit(&mut res)?;
        res.merged = true;
        if delete_after {
            // "keep" keeps the branch; only the checkout goes away.
            remove(data_dir, &info.path, false)?;
        }
        return Ok(res);
    }
    let root = PathBuf::from(&info.repo_root);
    let dirty = main_dirty(&root)?;
    if !dirty.is_empty() {
        let shown: Vec<&str> = dirty.iter().take(5).map(String::as_str).collect();
        return Err(format!(
            "主工作区有未提交的改动（{}{}），请先提交或暂存后再合并",
            shown.join("、"),
            if dirty.len() > 5 { " 等" } else { "" }
        ));
    }
    match current_branch(&root)? {
        Some(b) if b == info.base_branch => {}
        Some(b) => {
            return Err(format!(
                "主工作区当前在分支 {b}，请先切换回 {} 再合并",
                info.base_branch
            ))
        }
        None => {
            return Err(format!(
                "主工作区处于游离 HEAD（detached）状态，请先切换回 {} 再合并",
                info.base_branch
            ))
        }
    }
    auto_commit(&mut res)?;
    let head_before = git(&root, &["rev-parse", "HEAD"])?;
    if mode == "squash" {
        let o = git_raw(&root, &["merge", "--squash", &info.branch])?;
        if !o.ok {
            let conflicts = conflict_files(&root);
            // Safe: the main worktree was verified clean before the merge.
            let _ = git(&root, &["reset", "-q", "--hard", "HEAD"]);
            if conflicts.is_empty() {
                return Err(format!("合并失败：{}", o.stderr.trim()));
            }
            res.conflicts = conflicts;
            return Ok(res);
        }
        let staged = git_raw(&root, &["diff", "--cached", "--quiet"])?;
        if !staged.ok {
            git(&root, &["commit", "-q", "--no-verify", "-m", &message])?;
            res.commit = Some(git(&root, &["rev-parse", "HEAD"])?);
        }
    } else {
        let o = git_raw(&root, &["merge", "--no-ff", "--no-edit", "-m", &message, &info.branch])?;
        if !o.ok {
            let conflicts = conflict_files(&root);
            let _ = git_raw(&root, &["merge", "--abort"]);
            if conflicts.is_empty() {
                return Err(format!("合并失败：{}", o.stderr.trim()));
            }
            res.conflicts = conflicts;
            return Ok(res);
        }
        let head = git(&root, &["rev-parse", "HEAD"])?;
        if head != head_before {
            res.commit = Some(head);
        }
    }
    res.merged = true;
    if delete_after {
        remove(data_dir, &info.path, true)?;
    }
    Ok(res)
}

pub fn remove(data_dir: &Path, path: &str, delete_branch: bool) -> Result<(), String> {
    let info = find_info(data_dir, path)?;
    let root = PathBuf::from(&info.repo_root);
    let wt = PathBuf::from(&info.path);
    if wt.exists() {
        let o = git_raw(&root, &["worktree", "remove", "--force", &info.path])?;
        if !o.ok && wt.exists() {
            fs::remove_dir_all(&wt).map_err(|e| {
                format!("删除工作区目录失败：{e}（{}）", o.stderr.trim())
            })?;
        }
    }
    if root.is_dir() {
        let _ = git_raw(&root, &["worktree", "prune"]);
        if delete_branch && branch_exists(&root, &info.branch)? {
            git(&root, &["branch", "-D", &info.branch])?;
        }
    }
    drop_from_store(data_dir, &info.path)
}

// ---------- Tauri commands ----------

type St<'a> = State<'a, Arc<AppState>>;

#[tauri::command(async)]
pub fn worktree_check(cwd: String) -> Result<WorktreeCheck, String> {
    check(Path::new(&cwd))
}

#[tauri::command(async)]
pub fn worktree_create(st: St, cwd: String, hint: String) -> Result<WorktreeInfo, String> {
    create(&st.data_dir, Path::new(&cwd), &hint)
}

#[tauri::command(async)]
pub fn worktree_list(st: St) -> Result<Vec<WorktreeInfo>, String> {
    list(&st.data_dir)
}

#[tauri::command(async)]
pub fn worktree_status(st: St, path: String) -> Result<WorktreeStatus, String> {
    status(&st.data_dir, &path)
}

#[tauri::command(async)]
pub fn worktree_diff(st: St, path: String, file: Option<String>) -> Result<String, String> {
    diff(&st.data_dir, &path, file.as_deref())
}

#[tauri::command(async)]
pub fn worktree_merge(
    st: St,
    path: String,
    mode: String,
    message: String,
    delete_after: bool,
) -> Result<MergeResult, String> {
    merge(&st.data_dir, &path, &mode, &message, delete_after)
}

#[tauri::command(async)]
pub fn worktree_remove(st: St, path: String, delete_branch: bool) -> Result<(), String> {
    remove(&st.data_dir, &path, delete_branch)
}

#[cfg(test)]
#[path = "worktree_tests.rs"]
mod tests;
