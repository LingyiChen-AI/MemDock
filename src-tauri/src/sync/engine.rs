//! Three-way sync engine.
//!
//! `prepare` fetches the sync repository and compares, per file, the last
//! synced version (base), this machine's file (local) and the remote head
//! (remote). It never writes anything. `finish` validates the user's
//! resolutions, writes local files (under `write_lock`), builds one commit on
//! top of the remote head and pushes it, retrying once when the remote moved.
//!
//! The engine does not persist sync state: `SyncSummary` (and, after local
//! files were written, `SyncError::base_to_persist`) carries what the caller
//! must store (`last_sync_commit`, `synced_projects`).
//!
//! Files are compared by git blob id (of the normalized text); blob contents
//! are only read when a file is pulled, merged or shown as a conflict.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
use crate::sync::repo::{blob_id, Auth, CommitInfo, SyncGit, SyncGitError, TreeBlob, TreeEntry};
use crate::sync::state::{SkipRecord, SyncedProject};

/// Author and committer of every sync commit.
pub const SYNC_AUTHOR: (&str, &str) = ("MemDock", "memdock@users.noreply.github.com");

const GLOBAL_PATHS: [&str; 2] = ["global/claude/CLAUDE.md", "global/codex/AGENTS.md"];
const SKILLS_JSON: &str = "skills.json";

/// Everything one sync run needs.
/// How a sync treats files that look like they hold secrets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecretPolicy {
    /// The user chose to sync them anyway.
    pub allow: bool,
    /// Repo paths left out of this sync (reported as skipped `secret`).
    pub exclude: BTreeSet<String>,
}

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
    /// `SyncState.synced_projects`: the base is only valid for these projects
    /// while their directory is unchanged.
    pub synced_projects: Vec<SyncedProject>,
    /// `SyncState.unsynced_paths`.
    pub unsynced_paths: BTreeSet<String>,
    /// `SyncState.held_projects`.
    pub held_projects: BTreeSet<String>,
    pub paths: &'a AgentPaths,
    pub machine: &'a Machine,
    /// `manifest::local_entries(..)` of this machine; `None` when the skill
    /// lock could not be read (`lock_entries_for_sync` failed): `skills.json`
    /// is then left unchanged.
    pub local_skills: Option<Vec<ManifestSkill>>,
    pub snapshots: &'a SnapshotStore,
    pub write_lock: &'a Mutex<()>,
    /// What to do about files that look like they hold secrets.
    pub secrets: SecretPolicy,
    /// Download only: bring remote changes to this machine, push nothing.
    pub download: bool,
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

/// `synced_projects` describing the projects of `ctx` right now.
fn current_synced(ctx: &SyncCtx) -> Vec<SyncedProject> {
    ctx.projects
        .iter()
        .map(|(k, d)| SyncedProject {
            key: k.clone(),
            dir: normalize_path(&d.to_string_lossy()),
        })
        .collect()
}

// ------------------------------------------------------------------ types

/// What the caller must store as sync state.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SyncBase {
    /// New `last_sync_commit`.
    pub commit: Option<String>,
    /// New `synced_projects`.
    pub synced_projects: Vec<SyncedProject>,
    /// New `unsynced_paths`.
    pub unsynced_paths: BTreeSet<String>,
    /// New `held_projects`.
    pub held_projects: BTreeSet<String>,
}

impl SyncBase {
    /// The base recorded in `ctx` (sync state as loaded).
    pub fn of(ctx: &SyncCtx) -> SyncBase {
        SyncBase {
            commit: ctx.base_commit.clone(),
            synced_projects: ctx.synced_projects.clone(),
            unsynced_paths: ctx.unsynced_paths.clone(),
            held_projects: ctx.held_projects.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SyncError {
    Git(SyncGitError),
    Io(String),
    /// `skills.json` in the repository could not be read: abort.
    Manifest(String),
    /// Conflicts without a valid resolution (or invalid resolutions).
    Unresolved { paths: Vec<String> },
    /// Content about to be pushed contains secrets; nothing was pushed.
    /// `base` is set when an earlier attempt already wrote local files.
    Secrets {
        hits: Vec<FileSecretHit>,
        base: Option<SyncBase>,
    },
    /// Local files changed after `prepare`; nothing was written.
    LocalChanged { paths: Vec<String> },
    /// The remote moved again (or the retry needs user input); nothing was
    /// pushed, but local files were written: persist `base`.
    RemoteMoved { base: SyncBase },
    /// Local files were written, then committing / pushing failed: persist
    /// `base` so the next sync does not replay this one as conflicts.
    /// `cause` is the git failure behind it, when there was one.
    PushFailed {
        message: String,
        cause: Option<SyncGitError>,
        base: SyncBase,
    },
}

impl SyncError {
    /// Sync state the caller must persist even though the sync failed.
    pub fn base_to_persist(&self) -> Option<&SyncBase> {
        match self {
            SyncError::Secrets { base, .. } => base.as_ref(),
            SyncError::RemoteMoved { base } | SyncError::PushFailed { base, .. } => Some(base),
            _ => None,
        }
    }
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::Git(e) => match e {
                SyncGitError::NotInstalled => write!(f, "git is not installed"),
                SyncGitError::Timeout => write!(f, "git timed out"),
                SyncGitError::Rejected => write!(f, "the remote rejected the push"),
                SyncGitError::Unauthorized => write!(f, "GitHub rejected the credentials"),
                SyncGitError::Failed { stderr } => write!(f, "git: {stderr}"),
            },
            SyncError::Io(m) => write!(f, "{m}"),
            SyncError::Manifest(m) => write!(f, "skills.json: {m}"),
            SyncError::Unresolved { paths } => write!(f, "unresolved: {}", paths.join(", ")),
            SyncError::Secrets { hits, .. } => write!(f, "{} secret(s) found", hits.len()),
            SyncError::LocalChanged { paths } => {
                write!(f, "changed during sync: {}", paths.join(", "))
            }
            SyncError::RemoteMoved { .. } => write!(f, "the remote changed during sync"),
            SyncError::PushFailed { message, .. } => write!(f, "push failed: {message}"),
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
    /// `not_utf8_local` / `not_utf8_remote` / `not_utf8_base` / `case_collision` /
    /// `duplicate_local` / `not_regular_file` / `secret`.
    pub reason: String,
}

/// A repository file a download could not place on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AbsentFile {
    pub repo_path: String,
    /// `projects/<key>/...` key (best effort for projects absent here).
    pub project_key: String,
    /// `no_project`: the project is not on this machine. `no_place`: the
    /// project is here but the file has nowhere to go yet (auto memory before
    /// the project's first Claude Code session).
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Action {
    TakeRemote,
    PushLocal,
    Merged(String),
    Conflict,
    RemoteDeleted,
    /// Every memory file of a synced project is gone locally.
    LocalAllDeleted,
}

#[derive(Debug, Clone)]
struct Entry {
    repo_path: String,
    local_path: PathBuf,
    key: Option<String>,
    /// sha256 of the raw local bytes at prepare time, "" when missing.
    local_hash: String,
    local: Option<String>,
    /// Blob ids of the normalized local text and of the remote file.
    l_sha: Option<String>,
    r_sha: Option<String>,
    /// Remote text, read only when needed (pull / merge / conflict).
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
    /// Files of synced projects whose local memory files are all gone. By
    /// default nothing happens to them; `DeleteRemote` / `KeepRemote` decide.
    pub local_all_deleted: Vec<String>,
    pub secret_hits: Vec<FileSecretHit>,
    pub missing_skills: Vec<ManifestSkill>,
    pub removed_elsewhere: Vec<ManifestSkill>,
    pub skipped: Vec<SkippedFile>,
    /// Built for a download (nothing is pushed).
    pub download: bool,
    /// Download: local changes left as they are (not uploaded).
    pub kept_local: Vec<String>,
    /// Download: repository files with no place on this machine.
    pub absent: Vec<AbsentFile>,
    #[serde(skip)]
    entries: Vec<Entry>,
    /// Download: paths whose base must stay what it was (blob id, `None` =
    /// absent), because this download did not bring them in line.
    #[serde(skip)]
    base_keep: BTreeMap<String, Option<String>>,
    /// `path -> blob id` of the remote head.
    #[serde(skip)]
    remote_tree: BTreeMap<String, TreeEntry>,
    /// New `skills.json` text; `None` keeps the remote file unchanged.
    #[serde(skip)]
    manifest_json: Option<String>,
    /// `unsynced_paths` once this plan is applied.
    #[serde(skip)]
    unsynced_after: BTreeSet<String>,
    /// `held_projects` once this plan is applied with no choices made.
    #[serde(skip)]
    held_after: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// Conflict: the resolved text.
    Text(String),
    /// Remote deleted: delete the local file too. Conflict with one side
    /// deleted: accept the deletion everywhere.
    DeleteLocal,
    /// Remote deleted: keep the local file and push it back (default).
    KeepLocal,
    /// Local all deleted: delete the file from the repository.
    DeleteRemote,
    /// Local all deleted: restore the remote file locally.
    KeepRemote,
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
    pub synced_projects: Vec<SyncedProject>,
    /// Store as `SyncState.unsynced_paths`.
    pub unsynced_paths: BTreeSet<String>,
    /// Store as `SyncState.held_projects`.
    pub held_projects: BTreeSet<String>,
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

#[path = "engine_plan.rs"]
mod plan;
pub use plan::prepare;
use plan::{base_tree, base_valid, build_plan, candidates, project_of, trusted_keys};

// ----------------------------------------------------------------- finish

/// What happens to one file: `None` = unchanged, `Some(None)` = delete,
/// `Some(Some(text))` = write.
struct Outcome<'p> {
    entry: &'p Entry,
    local: Option<Option<String>>,
    repo: Option<Option<String>>,
}

fn outcome_final<'p>(entry: &'p Entry, fin: Option<String>) -> Outcome<'p> {
    let f_sha = fin.as_deref().map(|t| blob_id(t.as_bytes()));
    Outcome {
        entry,
        local: (entry.l_sha != f_sha).then(|| fin.clone()),
        repo: (entry.r_sha != f_sha).then_some(fin),
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
        let o = match (&e.action, res) {
            (Action::TakeRemote, _) => Outcome {
                entry: e,
                local: Some(e.remote.clone()),
                repo: None,
            },
            (Action::PushLocal, _) => Outcome {
                entry: e,
                local: None,
                repo: Some(e.local.clone()),
            },
            (Action::Merged(t), _) => outcome_final(e, Some(t.clone())),
            (Action::Conflict, Some(Resolution::Text(t))) if !has_markers(t) => {
                outcome_final(e, Some(t.replace("\r\n", "\n")))
            }
            // One side deleted: accept the deletion everywhere.
            (Action::Conflict, Some(Resolution::DeleteLocal))
                if e.local.is_none() || e.r_sha.is_none() =>
            {
                outcome_final(e, None)
            }
            (Action::RemoteDeleted, None | Some(Resolution::KeepLocal)) => Outcome {
                entry: e,
                local: None,
                repo: Some(e.local.clone()),
            },
            (Action::RemoteDeleted, Some(Resolution::DeleteLocal)) => Outcome {
                entry: e,
                local: Some(None),
                repo: None,
            },
            (Action::LocalAllDeleted, None) => continue,
            (Action::LocalAllDeleted, Some(Resolution::DeleteRemote)) => Outcome {
                entry: e,
                local: None,
                repo: Some(None),
            },
            (Action::LocalAllDeleted, Some(Resolution::KeepRemote)) => Outcome {
                entry: e,
                local: Some(e.remote.clone()),
                repo: None,
            },
            _ => {
                unresolved.push(e.repo_path.clone());
                continue;
            }
        };
        out.push(o);
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

/// Attaches the base to persist to an error raised after local writes.
fn after_writes(e: SyncError, base: Option<SyncBase>) -> SyncError {
    let Some(base) = base else { return e };
    match e {
        SyncError::Secrets { hits, .. } => SyncError::Secrets {
            hits,
            base: Some(base),
        },
        SyncError::RemoteMoved { base } => SyncError::RemoteMoved { base },
        SyncError::PushFailed {
            message, cause, ..
        } => SyncError::PushFailed {
            message,
            cause,
            base,
        },
        other => SyncError::PushFailed {
            message: other.to_string(),
            cause: match &other {
                SyncError::Git(g) => Some(g.clone()),
                _ => None,
            },
            base,
        },
    }
}

/// `held_projects` after applying `resolutions`: a held project is released
/// once every one of its held files got `DeleteRemote` or `KeepRemote`.
fn held_after_choices(
    plan: &SyncPlan,
    resolutions: &BTreeMap<String, Resolution>,
) -> BTreeSet<String> {
    let mut held = plan.held_after.clone();
    let mut open: BTreeSet<&str> = BTreeSet::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for e in &plan.entries {
        if e.action != Action::LocalAllDeleted {
            continue;
        }
        let Some(k) = e.key.as_deref() else { continue };
        seen.insert(k);
        let decided = matches!(
            resolutions.get(&e.repo_path),
            Some(Resolution::DeleteRemote | Resolution::KeepRemote)
        );
        if !decided {
            open.insert(k);
        }
    }
    for k in seen.difference(&open) {
        held.remove(*k);
    }
    held
}

/// Resolutions for the retry plan: a choice carries over only to an entry of
/// the same kind with the same local and remote content. `None` when a choice
/// no longer applies or the new plan needs a decision the user never made.
fn carry_resolutions(
    old: &SyncPlan,
    new: &SyncPlan,
    resolutions: &BTreeMap<String, Resolution>,
) -> Option<BTreeMap<String, Resolution>> {
    let mut out = BTreeMap::new();
    for e in &new.entries {
        if !matches!(
            e.action,
            Action::Conflict | Action::RemoteDeleted | Action::LocalAllDeleted
        ) {
            continue;
        }
        match resolutions.get(&e.repo_path) {
            Some(r) => {
                let o = old.entries.iter().find(|o| o.repo_path == e.repo_path)?;
                let same = std::mem::discriminant(&o.action) == std::mem::discriminant(&e.action)
                    && o.r_sha == e.r_sha
                    && o.l_sha == e.l_sha;
                if !same {
                    return None;
                }
                out.insert(e.repo_path.clone(), r.clone());
            }
            // holding is the default and needs no decision
            None if e.action == Action::LocalAllDeleted => {}
            None => return None,
        }
    }
    Some(out)
}

/// Repo paths pulled / merged so far (a retry does not count a file twice).
#[derive(Default, Clone)]
struct Counts {
    pulled: BTreeSet<String>,
    merged: BTreeSet<String>,
}

/// Validates resolutions, writes local files, commits and pushes. Retries once
/// when the push is rejected because the remote moved.
pub fn finish(
    ctx: &SyncCtx,
    plan: &SyncPlan,
    resolutions: &BTreeMap<String, Resolution>,
) -> Result<SyncSummary, SyncError> {
    if plan.download {
        return finish_download(ctx, plan, resolutions);
    }
    run(ctx, plan, resolutions, true, None, Counts::default())
}

/// Download: writes the remote side to local files and pushes nothing.
///
/// The new base is the remote head, except for paths this download left
/// alone (local changes, files deleted remotely): those keep their old base,
/// so a later upload still sees them as local changes / remote deletions.
/// When such paths exist the base is a local-only commit (the remote tree
/// with those entries put back), kept alive by `refs/memdock/base`.
fn finish_download(
    ctx: &SyncCtx,
    plan: &SyncPlan,
    resolutions: &BTreeMap<String, Resolution>,
) -> Result<SyncSummary, SyncError> {
    let mut outs = outcomes(plan, resolutions)?;
    for o in &mut outs {
        o.repo = None;
    }
    {
        let _guard = ctx.write_lock.lock().unwrap_or_else(|p| p.into_inner());
        apply_local(ctx, &outs)?;
    }
    let mut counts = Counts::default();
    for o in &outs {
        match o.entry.action {
            Action::TakeRemote if o.local.is_some() => {
                counts.pulled.insert(o.entry.repo_path.clone());
            }
            Action::Merged(_) | Action::Conflict if o.local.is_some() => {
                counts.merged.insert(o.entry.repo_path.clone());
            }
            _ => {}
        }
    }
    let synced_projects = current_synced(ctx);
    let old = SyncBase::of(ctx);
    let fallback = |e: SyncError| after_writes(e, Some(old.clone()));
    let commit = match &plan.remote_commit {
        None => ctx.base_commit.clone(),
        Some(head) if plan.base_keep.is_empty() => Some(head.clone()),
        Some(head) => {
            let mut tree: BTreeMap<String, TreeBlob> = plan
                .remote_tree
                .iter()
                .map(|(p, e)| (p.clone(), TreeBlob::Existing(e.clone())))
                .collect();
            for (p, sha) in &plan.base_keep {
                match sha {
                    Some(sha) => {
                        tree.insert(
                            p.clone(),
                            TreeBlob::Existing(TreeEntry {
                                mode: "100644".into(),
                                kind: "blob".into(),
                                sha: sha.clone(),
                            }),
                        );
                    }
                    None => {
                        tree.remove(p);
                    }
                }
            }
            let c = ctx
                .git
                .commit_tree_blobs(ctx.repo_dir, Some(head), &tree, "memdock local base", SYNC_AUTHOR)
                .map_err(|e| fallback(e.into()))?;
            ctx.git
                .keep_ref(ctx.repo_dir, "base", &c)
                .map_err(|e| fallback(e.into()))?;
            Some(c)
        }
    };
    Ok(SyncSummary {
        commit,
        committed: false,
        pulled: counts.pulled.len(),
        pushed: 0,
        merged: counts.merged.len(),
        synced_projects,
        unsynced_paths: plan.unsynced_after.clone(),
        held_projects: plan.held_after.clone(),
    })
}

/// Persistent skip records after `plan` was applied (`now`: ms since epoch).
/// Paths skipped this time are (re)recorded; older records stay only while
/// the path is still not synced (still unsynced, or kept local by a download).
pub fn update_skip_records(
    plan: &SyncPlan,
    old: &BTreeMap<String, SkipRecord>,
    now: i64,
) -> BTreeMap<String, SkipRecord> {
    let mut out: BTreeMap<String, SkipRecord> = old
        .iter()
        .filter(|(p, _)| plan.unsynced_after.contains(*p) || plan.kept_local.contains(*p))
        .map(|(p, r)| (p.clone(), r.clone()))
        .collect();
    for s in &plan.skipped {
        out.insert(
            s.repo_path.clone(),
            SkipRecord {
                reason: s.reason.clone(),
                at: now,
            },
        );
    }
    out
}

fn run(
    ctx: &SyncCtx,
    plan: &SyncPlan,
    resolutions: &BTreeMap<String, Resolution>,
    may_retry: bool,
    written: Option<SyncBase>,
    mut counts: Counts,
) -> Result<SyncSummary, SyncError> {
    // 1. resolutions
    let outs = outcomes(plan, resolutions).map_err(|e| after_writes(e, written.clone()))?;

    // 2. secrets in everything that will be pushed
    let mut hits = Vec::new();
    for o in &outs {
        if let Some(Some(t)) = &o.repo {
            scan_into(&o.entry.repo_path, t, &mut hits);
        }
    }
    if let Some(json) = &plan.manifest_json {
        if Some(blob_id(json.as_bytes())) != plan.remote_tree.get(SKILLS_JSON).map(|e| e.sha.clone()) {
            scan_into(SKILLS_JSON, json, &mut hits);
        }
    }
    if !hits.is_empty() && !ctx.secrets.allow {
        return Err(SyncError::Secrets {
            hits,
            base: written,
        });
    }

    // 3. local writes, under the write lock only
    {
        let _guard = ctx.write_lock.lock().unwrap_or_else(|p| p.into_inner());
        apply_local(ctx, &outs).map_err(|e| after_writes(e, written.clone()))?;
    }
    // From here on local files equal the remote head plus this sync's changes:
    // the remote head is the base for every current project.
    let synced_projects = current_synced(ctx);
    let held_projects = held_after_choices(plan, resolutions);
    let written = Some(SyncBase {
        commit: plan.remote_commit.clone(),
        synced_projects: synced_projects.clone(),
        unsynced_paths: plan.unsynced_after.clone(),
        held_projects: held_projects.clone(),
    });
    for o in &outs {
        match o.entry.action {
            Action::TakeRemote | Action::RemoteDeleted | Action::LocalAllDeleted
                if o.local.is_some() =>
            {
                counts.pulled.insert(o.entry.repo_path.clone());
            }
            Action::Merged(_) | Action::Conflict => {
                counts.merged.insert(o.entry.repo_path.clone());
            }
            _ => {}
        }
    }

    // 4. full tree = remote tree + this machine's changes
    let mut tree: BTreeMap<String, TreeBlob> = plan
        .remote_tree
        .iter()
        .map(|(p, e)| (p.clone(), TreeBlob::Existing(e.clone())))
        .collect();
    let mut pushed = 0;
    for o in &outs {
        match &o.repo {
            None => {}
            Some(Some(t)) => {
                tree.insert(o.entry.repo_path.clone(), TreeBlob::Data(t.clone().into_bytes()));
                pushed += 1;
            }
            Some(None) => {
                if tree.remove(&o.entry.repo_path).is_some() {
                    pushed += 1;
                }
            }
        }
    }
    let mut manifest_changed = false;
    if let Some(json) = &plan.manifest_json {
        manifest_changed =
            Some(blob_id(json.as_bytes())) != plan.remote_tree.get(SKILLS_JSON).map(|e| e.sha.clone());
        if manifest_changed {
            tree.insert(SKILLS_JSON.to_string(), TreeBlob::Data(json.clone().into_bytes()));
        }
    }
    let summary = |commit: Option<String>, committed: bool, pushed: usize| SyncSummary {
        commit,
        committed,
        pulled: counts.pulled.len(),
        pushed,
        merged: counts.merged.len(),
        synced_projects: synced_projects.clone(),
        unsynced_paths: plan.unsynced_after.clone(),
        held_projects: held_projects.clone(),
    };
    if pushed == 0 && !manifest_changed {
        return Ok(summary(plan.remote_commit.clone(), false, 0));
    }

    let message = format!("sync from {}", ctx.machine.name);
    let commit = ctx
        .git
        .commit_tree_blobs(
            ctx.repo_dir,
            plan.remote_commit.as_deref(),
            &tree,
            &message,
            SYNC_AUTHOR,
        )
        .map_err(|e| after_writes(e.into(), written.clone()))?;

    // 5. push; on a race re-plan once against the head this sync was built on
    match ctx.git.push(ctx.repo_dir, &commit, &ctx.branch, &ctx.auth) {
        Ok(()) => Ok(summary(Some(commit), true, pushed)),
        Err(SyncGitError::Rejected) if may_retry => {
            let base = written.clone().expect("set after local writes");
            let again = ctx
                .git
                .fetch(ctx.repo_dir, &ctx.auth)
                .map_err(SyncError::from)
                .and_then(|r| build_plan(ctx, r, &base))
                .map_err(|e| after_writes(e, written.clone()))?;
            if !again.secret_hits.is_empty() && !ctx.secrets.allow {
                return Err(SyncError::Secrets {
                    hits: again.secret_hits,
                    base: written,
                });
            }
            // The user's choices carry over only where they still mean the
            // same thing; otherwise stop rather than drop a choice.
            let Some(carried) = carry_resolutions(plan, &again, resolutions) else {
                return Err(SyncError::RemoteMoved { base });
            };
            run(ctx, &again, &carried, false, written, counts)
        }
        Err(SyncGitError::Rejected) => Err(SyncError::RemoteMoved {
            base: written.expect("set after local writes"),
        }),
        Err(e) => Err(after_writes(e.into(), written)),
    }
}

// --------------------------------------------------------------- queries

/// Number of files that differ between this machine and the base commit
/// (no fetch). Never synced: number of existing local files.
pub fn pending_local(ctx: &SyncCtx) -> Result<usize, SyncError> {
    let Some(tree) = base_tree(ctx, ctx.base_commit.as_deref())? else {
        let mut n = 0;
        for f in &ctx.files {
            if read_opt(&f.local_path)?.is_some() {
                n += 1;
            }
        }
        return Ok(n);
    };
    let trusted = trusted_keys(ctx, &ctx.synced_projects);
    let base_paths: Vec<&String> = tree
        .keys()
        .filter(|p| base_valid(p, project_of(p, &ctx.projects), &trusted))
        .filter(|p| !ctx.unsynced_paths.contains(*p))
        .collect();
    let mut n = 0;
    for c in candidates(ctx, base_paths.into_iter(), std::iter::empty()).list {
        let l = read_opt(&c.local_path)?.map(|b| blob_id(normalize_text(&b).as_bytes()));
        let b = if base_valid(&c.repo_path, c.key.as_deref(), &trusted)
            && !ctx.unsynced_paths.contains(&c.repo_path)
        {
            tree.get(&c.repo_path).cloned()
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
