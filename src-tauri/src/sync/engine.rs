//! Three-way sync engine.
//!
//! `prepare` fetches the sync repository and compares, per file, the last
//! synced version (base), this machine's file (local) and the remote head
//! (remote). It never writes anything. `finish` validates the user's
//! resolutions, writes local files (under `write_lock`), builds one commit on
//! top of the remote head and pushes it, retrying once when the remote moved.
//!
//! The engine does not persist sync state: `SyncSummary` carries what the
//! caller must store (`last_sync_commit`, `synced_projects`).

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::memory::io::{write_memory, WriteError};
use crate::memory::snapshot::SnapshotStore;
use crate::model::MemoryIndex;
use crate::paths::AgentPaths;
use crate::projects::normalize_path;
use crate::sync::layout::{
    dir_key, expected_local_path, normalize_text, project_key, safe_key, Machine, SyncFile,
};
use crate::sync::manifest::{self, Manifest, ManifestSkill};
use crate::sync::repo::{Auth, CommitInfo, SyncGit, SyncGitError};

/// Author and committer of every sync commit.
pub const SYNC_AUTHOR: (&str, &str) = ("MemDock", "memdock@users.noreply.github.com");

const GLOBAL_PATHS: [&str; 2] = ["global/claude/CLAUDE.md", "global/codex/AGENTS.md"];
const SKILLS_JSON: &str = "skills.json";

/// Everything one sync run needs.
pub struct SyncCtx<'a> {
    pub git: &'a SyncGit,
    pub repo_dir: &'a Path,
    pub auth: Auth,
    /// Branch pushed to.
    pub branch: String,
    /// `SyncState.last_sync_commit`.
    pub base_commit: Option<String>,
    /// `layout::map_files(index, included_local)`.
    pub files: Vec<SyncFile>,
    /// Projects taking part on this machine (`sync_projects`).
    pub projects: Vec<(String, PathBuf)>,
    /// `SyncState.synced_projects`: the base is only valid for these projects.
    pub synced_projects: Vec<String>,
    pub paths: &'a AgentPaths,
    pub machine: &'a Machine,
    /// `manifest::local_entries(..)` of this machine; `None` when the skill
    /// lock could not be read (`lock_entries_for_sync` failed): `skills.json`
    /// is then left unchanged.
    pub local_skills: Option<Vec<ManifestSkill>>,
    pub snapshots: &'a SnapshotStore,
    pub write_lock: &'a Mutex<()>,
}

/// Projects of `index` that take part in sync, as `(key, directory)`; same key
/// rules and first-wins order as `layout::map_files`.
pub fn sync_projects(index: &MemoryIndex, included_local: &[String]) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for pm in &index.projects {
        let dir = PathBuf::from(&pm.project.path);
        let key = match project_key(&dir) {
            Some(k) => k,
            None => {
                let dk = dir_key(&dir);
                if !included_local.contains(&dk) {
                    continue;
                }
                dk
            }
        };
        let Some(key) = safe_key(&key) else { continue };
        if !out.iter().any(|(k, _)| *k == key) {
            out.push((key, dir));
        }
    }
    out
}

// ------------------------------------------------------------------ types

#[derive(Debug, Clone, PartialEq)]
pub enum SyncError {
    Git(SyncGitError),
    Io(String),
    /// `skills.json` in the repository could not be read: abort.
    Manifest(String),
    /// Conflicts without a valid `Text` resolution (or invalid resolutions).
    Unresolved { paths: Vec<String> },
    /// Content about to be pushed contains secrets; nothing was written.
    Secrets(Vec<FileSecretHit>),
    /// Local files changed after `prepare`; nothing was written.
    LocalChanged { paths: Vec<String> },
    /// The remote moved again (or the retry needs user input); nothing pushed.
    RemoteMoved,
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::Git(e) => write!(f, "git: {e:?}"),
            SyncError::Io(m) => write!(f, "{m}"),
            SyncError::Manifest(m) => write!(f, "skills.json: {m}"),
            SyncError::Unresolved { paths } => write!(f, "unresolved: {}", paths.join(", ")),
            SyncError::Secrets(h) => write!(f, "{} secret(s) found", h.len()),
            SyncError::LocalChanged { paths } => {
                write!(f, "changed during sync: {}", paths.join(", "))
            }
            SyncError::RemoteMoved => write!(f, "the remote changed during sync"),
        }
    }
}

impl From<SyncGitError> for SyncError {
    fn from(e: SyncGitError) -> Self {
        SyncError::Git(e)
    }
}

impl From<io::Error> for SyncError {
    fn from(e: io::Error) -> Self {
        SyncError::Io(e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileSecretHit {
    pub repo_path: String,
    pub line: usize,
    pub rule: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConflictFile {
    pub repo_path: String,
    pub local_path: String,
    pub local: Option<String>,
    pub remote: Option<String>,
    pub base: Option<String>,
    /// diff3 text with `local` / `base` / `remote` markers.
    pub merged: String,
    pub conflicts: usize,
}

/// A file left out of this sync (never lossy-converted into the repo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedFile {
    pub repo_path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Action {
    TakeRemote,
    PushLocal,
    Merged(String),
    Conflict,
    RemoteDeleted,
}

#[derive(Debug, Clone)]
struct Entry {
    repo_path: String,
    local_path: PathBuf,
    /// sha256 of the raw local bytes at prepare time, "" when missing.
    local_hash: String,
    local: Option<String>,
    remote: Option<String>,
    action: Action,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncPlan {
    pub id: String,
    pub remote_commit: Option<String>,
    pub take_remote: Vec<String>,
    pub push_local: Vec<String>,
    pub merged: Vec<String>,
    pub conflicts: Vec<ConflictFile>,
    pub remote_deleted: Vec<String>,
    pub secret_hits: Vec<FileSecretHit>,
    pub missing_skills: Vec<ManifestSkill>,
    pub removed_elsewhere: Vec<ManifestSkill>,
    pub skipped: Vec<SkippedFile>,
    #[serde(skip)]
    entries: Vec<Entry>,
    #[serde(skip)]
    remote_tree: BTreeMap<String, Vec<u8>>,
    /// New `skills.json` text; `None` keeps the remote file unchanged.
    #[serde(skip)]
    manifest_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    Text(String),
    DeleteLocal,
    KeepLocal,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SyncSummary {
    /// The commit local files now match: the pushed commit, or the remote head
    /// when nothing had to be committed. Store as `last_sync_commit`.
    pub commit: Option<String>,
    /// Whether a new commit was pushed.
    pub committed: bool,
    pub pulled: usize,
    pub pushed: usize,
    pub merged: usize,
    /// Store as `SyncState.synced_projects`.
    pub synced_projects: Vec<String>,
}

// ---------------------------------------------------------------- helpers

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn read_opt(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Strict text decoding: UTF-8 (BOM stripped, CRLF -> LF); UTF-16 or invalid
/// UTF-8 is `None`.
fn decode_text(bytes: &[u8]) -> Option<String> {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return None;
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let s = std::str::from_utf8(body).ok()?;
    Some(s.replace("\r\n", "\n"))
}

fn has_markers(text: &str) -> bool {
    text.lines()
        .any(|l| l.starts_with("<<<<<<<") || l.starts_with(">>>>>>>"))
}

fn scan_into(repo_path: &str, text: &str, out: &mut Vec<FileSecretHit>) {
    for h in crate::secrets::scan(text) {
        out.push(FileSecretHit {
            repo_path: repo_path.to_string(),
            line: h.line,
            rule: h.rule.to_string(),
        });
    }
}

/// Project key of a `projects/<key>/...` path among the projects present here
/// (longest match, as `expected_local_path`).
fn project_of<'p>(repo_path: &str, projects: &'p [(String, PathBuf)]) -> Option<&'p str> {
    let rest = repo_path.strip_prefix("projects/")?;
    projects
        .iter()
        .filter(|(k, _)| rest.strip_prefix(k.as_str()).is_some_and(|t| t.starts_with('/')))
        .max_by_key(|(k, _)| k.len())
        .map(|(k, _)| k.as_str())
}

/// Whether the base commit is meaningful for `repo_path` on this machine.
fn base_valid(ctx: &SyncCtx, repo_path: &str, key: Option<&str>) -> bool {
    if GLOBAL_PATHS.contains(&repo_path) {
        return true;
    }
    let key = key.or_else(|| project_of(repo_path, &ctx.projects));
    key.is_some_and(|k| ctx.synced_projects.iter().any(|s| s == k))
}

/// Paths of the base commit, `None` without a usable base (no previous sync,
/// or the commit is unknown to this clone).
fn base_paths(ctx: &SyncCtx) -> Option<(String, Vec<String>)> {
    let b = ctx.base_commit.as_ref()?;
    ctx.git.list_at(ctx.repo_dir, b).ok().map(|l| (b.clone(), l))
}

/// One repo path to compare and the local file it maps to.
struct Candidate {
    repo_path: String,
    local_path: PathBuf,
    key: Option<String>,
}

/// `union(local files, base paths, remote paths)` restricted to files that map
/// to this machine. Repo files of projects absent here are not included.
fn candidates(ctx: &SyncCtx, base: &[String], remote: &[String]) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    let mut seen_repo: HashSet<String> = HashSet::new();
    let mut seen_local: HashSet<String> = HashSet::new();
    for f in &ctx.files {
        if seen_repo.insert(f.repo_path.clone()) {
            seen_local.insert(normalize_path(&f.local_path.to_string_lossy()).to_lowercase());
            out.push(Candidate {
                repo_path: f.repo_path.clone(),
                local_path: f.local_path.clone(),
                key: f.project_key.clone(),
            });
        }
    }
    for p in base.iter().chain(remote.iter()) {
        if seen_repo.contains(p) {
            continue;
        }
        let Some(local) = expected_local_path(p, &ctx.projects, ctx.paths) else {
            continue;
        };
        seen_repo.insert(p.clone());
        // Two repo paths must never write the same local file.
        if !seen_local.insert(normalize_path(&local.to_string_lossy()).to_lowercase()) {
            continue;
        }
        out.push(Candidate {
            repo_path: p.clone(),
            local_path: local,
            key: project_of(p, &ctx.projects).map(String::from),
        });
    }
    out
}

// ---------------------------------------------------------------- prepare

/// Fetches and builds the plan. Writes nothing locally or to the repository.
pub fn prepare(ctx: &SyncCtx) -> Result<SyncPlan, SyncError> {
    let remote = ctx.git.fetch(ctx.repo_dir, &ctx.auth)?;
    build_plan(ctx, remote)
}

fn build_plan(ctx: &SyncCtx, remote: Option<String>) -> Result<SyncPlan, SyncError> {
    let git = ctx.git;
    let repo = ctx.repo_dir;

    let mut remote_tree: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    if let Some(r) = &remote {
        for p in git.list_at(repo, r)? {
            if let Some(bytes) = git.read_at(repo, r, &p)? {
                remote_tree.insert(p, bytes);
            }
        }
    }
    let remote_paths: Vec<String> = remote_tree.keys().cloned().collect();
    let base = base_paths(ctx);
    let base_list: Vec<String> = match &base {
        Some((_, l)) => l
            .iter()
            .filter(|p| base_valid(ctx, p, None))
            .cloned()
            .collect(),
        None => Vec::new(),
    };

    let mut plan = SyncPlan {
        id: uuid::Uuid::new_v4().to_string(),
        remote_commit: remote.clone(),
        take_remote: Vec::new(),
        push_local: Vec::new(),
        merged: Vec::new(),
        conflicts: Vec::new(),
        remote_deleted: Vec::new(),
        secret_hits: Vec::new(),
        missing_skills: Vec::new(),
        removed_elsewhere: Vec::new(),
        skipped: Vec::new(),
        entries: Vec::new(),
        remote_tree: BTreeMap::new(),
        manifest_json: None,
    };

    for c in candidates(ctx, &base_list, &remote_paths) {
        let skip = |plan: &mut SyncPlan, reason: &str| {
            plan.skipped.push(SkippedFile {
                repo_path: c.repo_path.clone(),
                reason: reason.to_string(),
            })
        };
        let lbytes = read_opt(&c.local_path)?;
        let local_hash = lbytes.as_deref().map(sha256_hex).unwrap_or_default();
        let l = match &lbytes {
            None => None,
            Some(b) => match decode_text(b) {
                Some(t) => Some(t),
                None => {
                    skip(&mut plan, "local file is not UTF-8 text");
                    continue;
                }
            },
        };
        let r = match remote_tree.get(&c.repo_path) {
            None => None,
            Some(b) => match decode_text(b) {
                Some(t) => Some(t),
                None => {
                    skip(&mut plan, "remote file is not UTF-8 text");
                    continue;
                }
            },
        };
        let b = match &base {
            Some((bc, _)) if base_valid(ctx, &c.repo_path, c.key.as_deref()) => {
                match git.read_at(repo, bc, &c.repo_path)? {
                    None => None,
                    Some(bytes) => match decode_text(&bytes) {
                        Some(t) => Some(t),
                        None => {
                            skip(&mut plan, "base file is not UTF-8 text");
                            continue;
                        }
                    },
                }
            }
            _ => None,
        };

        if l == r {
            continue;
        }
        let rp = c.repo_path.clone();
        let action = if l == b {
            if r.is_none() {
                plan.remote_deleted.push(rp.clone());
                if let Some(t) = &l {
                    scan_into(&rp, t, &mut plan.secret_hits);
                }
                Action::RemoteDeleted
            } else {
                plan.take_remote.push(rp.clone());
                Action::TakeRemote
            }
        } else if r == b {
            plan.push_local.push(rp.clone());
            if let Some(t) = &l {
                scan_into(&rp, t, &mut plan.secret_hits);
            }
            Action::PushLocal
        } else {
            let m = git.merge3(
                b.as_deref().unwrap_or(""),
                l.as_deref().unwrap_or(""),
                r.as_deref().unwrap_or(""),
            )?;
            let clean = b.is_some() && l.is_some() && r.is_some() && m.conflicts == 0;
            if clean {
                plan.merged.push(rp.clone());
                scan_into(&rp, &m.text, &mut plan.secret_hits);
                Action::Merged(m.text)
            } else {
                plan.conflicts.push(ConflictFile {
                    repo_path: rp.clone(),
                    local_path: c.local_path.to_string_lossy().into_owned(),
                    local: l.clone(),
                    remote: r.clone(),
                    base: b.clone(),
                    merged: m.text,
                    conflicts: m.conflicts.max(1),
                });
                Action::Conflict
            }
        };
        plan.entries.push(Entry {
            repo_path: rp,
            local_path: c.local_path,
            local_hash,
            local: l,
            remote: r,
            action,
        });
    }

    if let Some(local) = &ctx.local_skills {
        let remote_m = match remote_tree.get(SKILLS_JSON) {
            None => Manifest::default(),
            Some(bytes) => {
                let text = decode_text(bytes)
                    .ok_or_else(|| SyncError::Manifest("not UTF-8 text".into()))?;
                Manifest::from_json(&text).map_err(SyncError::Manifest)?
            }
        };
        let id = ctx.machine.id.as_str();
        let merged = manifest::merge(&remote_m, local, id);
        plan.missing_skills = manifest::missing_here(&merged, local, id);
        plan.removed_elsewhere = manifest::removed_elsewhere(&merged, local, id);
        plan.manifest_json = Some(
            merged
                .to_json()
                .map_err(|e| SyncError::Manifest(e.to_string()))?,
        );
    }
    plan.remote_tree = remote_tree;
    Ok(plan)
}

// ----------------------------------------------------------------- finish

/// What happens to one file: `None` = unchanged, `Some(None)` = delete,
/// `Some(Some(text))` = write.
struct Outcome<'p> {
    entry: &'p Entry,
    local: Option<Option<String>>,
    repo: Option<Option<String>>,
}

fn outcome_for_text<'p>(entry: &'p Entry, text: &str) -> Outcome<'p> {
    let t = Some(text.to_string());
    Outcome {
        entry,
        local: (entry.local != t).then(|| t.clone()),
        repo: (entry.remote != t).then_some(t),
    }
}

fn outcomes<'p>(
    plan: &'p SyncPlan,
    resolutions: &BTreeMap<String, Resolution>,
) -> Result<Vec<Outcome<'p>>, SyncError> {
    let mut out = Vec::new();
    let mut unresolved = Vec::new();
    for e in &plan.entries {
        let res = resolutions.get(&e.repo_path);
        match &e.action {
            Action::TakeRemote => out.push(Outcome {
                entry: e,
                local: Some(e.remote.clone()),
                repo: None,
            }),
            Action::PushLocal => out.push(Outcome {
                entry: e,
                local: None,
                repo: Some(e.local.clone()),
            }),
            Action::Merged(t) => out.push(outcome_for_text(e, t)),
            Action::Conflict => match res {
                Some(Resolution::Text(t)) if !has_markers(t) => {
                    out.push(outcome_for_text(e, &t.replace("\r\n", "\n")))
                }
                _ => unresolved.push(e.repo_path.clone()),
            },
            Action::RemoteDeleted => match res {
                None | Some(Resolution::KeepLocal) => out.push(Outcome {
                    entry: e,
                    local: None,
                    repo: Some(e.local.clone()),
                }),
                Some(Resolution::DeleteLocal) => out.push(Outcome {
                    entry: e,
                    local: Some(None),
                    repo: None,
                }),
                Some(Resolution::Text(_)) => unresolved.push(e.repo_path.clone()),
            },
        }
    }
    if !unresolved.is_empty() {
        return Err(SyncError::Unresolved { paths: unresolved });
    }
    Ok(out)
}

fn local_changed(path: &Path) -> SyncError {
    SyncError::LocalChanged {
        paths: vec![path.to_string_lossy().into_owned()],
    }
}

/// Verifies every involved local file and applies local writes / deletes.
/// Caller holds `write_lock`.
fn apply_local(ctx: &SyncCtx, outs: &[Outcome]) -> Result<(), SyncError> {
    let mut changed = Vec::new();
    for o in outs {
        let now = read_opt(&o.entry.local_path)?
            .as_deref()
            .map(sha256_hex)
            .unwrap_or_default();
        if now != o.entry.local_hash {
            changed.push(o.entry.local_path.to_string_lossy().into_owned());
        }
    }
    if !changed.is_empty() {
        return Err(SyncError::LocalChanged { paths: changed });
    }
    for o in outs {
        let e = o.entry;
        match &o.local {
            None => {}
            Some(Some(text)) => {
                let text = if e.local_hash.is_empty() && cfg!(windows) {
                    // New file: platform default (CRLF on Windows, no BOM).
                    text.replace('\n', "\r\n")
                } else {
                    text.clone()
                };
                write_memory(&e.local_path, &text, &e.local_hash, ctx.snapshots).map_err(
                    |err| match err {
                        WriteError::DiskChanged { .. } => local_changed(&e.local_path),
                        WriteError::Io(io) => SyncError::Io(io.to_string()),
                    },
                )?;
            }
            Some(None) => {
                let Some(bytes) = read_opt(&e.local_path)? else {
                    continue;
                };
                if sha256_hex(&bytes) != e.local_hash {
                    return Err(local_changed(&e.local_path));
                }
                ctx.snapshots.save(&e.local_path, &bytes)?;
                fs::remove_file(&e.local_path)?;
            }
        }
    }
    Ok(())
}

/// Validates resolutions, writes local files, commits and pushes. Retries once
/// when the push is rejected because the remote moved.
pub fn finish(
    ctx: &SyncCtx,
    plan: &SyncPlan,
    resolutions: &BTreeMap<String, Resolution>,
) -> Result<SyncSummary, SyncError> {
    finish_inner(ctx, plan, resolutions, true)
}

fn finish_inner(
    ctx: &SyncCtx,
    plan: &SyncPlan,
    resolutions: &BTreeMap<String, Resolution>,
    may_retry: bool,
) -> Result<SyncSummary, SyncError> {
    // 1. resolutions
    let outs = outcomes(plan, resolutions)?;

    // 2. secrets in everything that will be pushed
    let mut hits = Vec::new();
    for o in &outs {
        if let Some(Some(t)) = &o.repo {
            scan_into(&o.entry.repo_path, t, &mut hits);
        }
    }
    if !hits.is_empty() {
        return Err(SyncError::Secrets(hits));
    }

    // 3. local writes, under the write lock only
    {
        let _guard = ctx.write_lock.lock().unwrap_or_else(|p| p.into_inner());
        apply_local(ctx, &outs)?;
    }

    // 4. full tree = remote tree + this machine's changes
    let mut tree = plan.remote_tree.clone();
    let mut pushed = 0;
    for o in &outs {
        match &o.repo {
            None => {}
            Some(Some(t)) => {
                tree.insert(o.entry.repo_path.clone(), t.clone().into_bytes());
                pushed += 1;
            }
            Some(None) => {
                if tree.remove(&o.entry.repo_path).is_some() {
                    pushed += 1;
                }
            }
        }
    }
    if let Some(json) = &plan.manifest_json {
        tree.insert(SKILLS_JSON.to_string(), json.clone().into_bytes());
    }
    let pulled = outs
        .iter()
        .filter(|o| matches!(o.entry.action, Action::TakeRemote | Action::RemoteDeleted))
        .filter(|o| o.local.is_some())
        .count();
    let merged = outs
        .iter()
        .filter(|o| matches!(o.entry.action, Action::Merged(_) | Action::Conflict))
        .count();
    let synced_projects: Vec<String> = ctx.projects.iter().map(|(k, _)| k.clone()).collect();

    let unchanged = tree == plan.remote_tree && (plan.remote_commit.is_some() || tree.is_empty());
    if unchanged {
        return Ok(SyncSummary {
            commit: plan.remote_commit.clone(),
            committed: false,
            pulled,
            pushed: 0,
            merged,
            synced_projects,
        });
    }

    let message = format!("sync from {}", ctx.machine.name);
    let commit = ctx.git.commit_tree(
        ctx.repo_dir,
        plan.remote_commit.as_deref(),
        &tree,
        &message,
        SYNC_AUTHOR,
    )?;

    // 5. push; on a race re-plan once
    match ctx.git.push(ctx.repo_dir, &commit, &ctx.branch, &ctx.auth) {
        Ok(()) => Ok(SyncSummary {
            commit: Some(commit),
            committed: true,
            pulled,
            pushed,
            merged,
            synced_projects,
        }),
        Err(SyncGitError::Rejected) if may_retry => {
            let remote = ctx.git.fetch(ctx.repo_dir, &ctx.auth)?;
            let again = build_plan(ctx, remote)?;
            if !again.conflicts.is_empty()
                || !again.remote_deleted.is_empty()
                || !again.secret_hits.is_empty()
            {
                return Err(SyncError::RemoteMoved);
            }
            let mut s = finish_inner(ctx, &again, &BTreeMap::new(), false)?;
            s.pulled += pulled;
            s.merged += merged;
            Ok(s)
        }
        Err(SyncGitError::Rejected) => Err(SyncError::RemoteMoved),
        Err(e) => Err(e.into()),
    }
}

// --------------------------------------------------------------- queries

/// Number of files that differ between this machine and the base commit
/// (no fetch). Never synced: number of existing local files.
pub fn pending_local(ctx: &SyncCtx) -> Result<usize, SyncError> {
    let Some((bc, list)) = base_paths(ctx) else {
        let mut n = 0;
        for f in &ctx.files {
            if read_opt(&f.local_path)?.is_some() {
                n += 1;
            }
        }
        return Ok(n);
    };
    let base_list: Vec<String> = list
        .into_iter()
        .filter(|p| base_valid(ctx, p, None))
        .collect();
    let mut n = 0;
    for c in candidates(ctx, &base_list, &[]) {
        let l = read_opt(&c.local_path)?.map(|b| normalize_text(&b));
        let b = if base_valid(ctx, &c.repo_path, c.key.as_deref()) {
            ctx.git
                .read_at(ctx.repo_dir, &bc, &c.repo_path)?
                .map(|b| normalize_text(&b))
        } else {
            None
        };
        if l != b {
            n += 1;
        }
    }
    Ok(n)
}

/// Latest commits of the remote branch (as last fetched), newest first.
pub fn history(ctx: &SyncCtx, limit: usize) -> Result<Vec<CommitInfo>, SyncError> {
    Ok(ctx
        .git
        .log(ctx.repo_dir, &format!("origin/{}", ctx.branch), limit)?)
}

/// Normalized text of `repo_path` at `commit`, `None` when absent.
pub fn file_at(ctx: &SyncCtx, commit: &str, repo_path: &str) -> Result<Option<String>, SyncError> {
    Ok(ctx
        .git
        .read_at(ctx.repo_dir, commit, repo_path)?
        .map(|b| normalize_text(&b)))
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
