//! Tauri commands for "Import from GitHub".
//!
//! Flow: `gh_import_scan` (link -> commit -> tree -> importable items, kept as
//! a short-lived session) -> `gh_import_preview` (one item's text) ->
//! `gh_import_memories` (memory files become Pending review suggestions that
//! append the file to the chosen target; nothing is written here) and / or
//! `gh_import_skill` (download one skill into a staging folder and run the
//! normal folder install).
//!
//! Abuse limits: memory files up to 256 KB, skills up to 300 files / 10 MB,
//! at most 300 listed items and 50 MB downloaded per session.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::commands::{lock, AppState, CommandError};
use crate::commands_extract::ExtractState;
use crate::commands_skills::{install_skill_impl, register_skill_source_impl, InstallFromDto, InstallTargetDto};
use crate::extract::prompt::{fingerprint, Suggestion, SuggestionKind, SuggestionOrigin, SuggestionStatus};
use crate::ghimport::fetch::{with_fallback, FetchError, GhFetcher, HttpFetcher, TreeEntry};
use crate::ghimport::link::{parse_link, LinkKind};
use crate::ghimport::scan::{classify, skill_files, ImportItem, ItemKind, MAX_MEMORY_BYTES, MAX_SKILL_BYTES};
use crate::memory::io::read_memory;
use crate::model::Tool;
use crate::projects::normalize_path;
use crate::skills::model::SkillEntry;

/// Sessions older than this are dropped.
const SESSION_TTL: Duration = Duration::from_secs(30 * 60);
/// At most this many scans are kept.
const MAX_SESSIONS: usize = 8;
/// Download budget per session (previews, imports and skill installs).
const MAX_SESSION_BYTES: u64 = 50 * 1024 * 1024;
/// How many `(ref, path)` splits of a tree / blob link are tried.
const MAX_REF_TRIES: usize = 8;

struct Session {
    created: Instant,
    owner: String,
    repo: String,
    git_ref: String,
    commit: String,
    entries: Vec<TreeEntry>,
    items: Vec<ImportItem>,
    downloaded: u64,
    /// Decoded memory texts already downloaded (by path).
    texts: HashMap<String, String>,
}

/// Import runtime managed by Tauri next to `AppState`.
pub struct GhImportState {
    app: Arc<AppState>,
    fetcher: Arc<dyn GhFetcher>,
    sessions: Mutex<HashMap<String, Session>>,
}

impl GhImportState {
    pub fn new(app: Arc<AppState>, fetcher: Arc<dyn GhFetcher>) -> Self {
        GhImportState { app, fetcher, sessions: Mutex::new(HashMap::new()) }
    }

    /// Production state: the real HTTP client, built on first use.
    pub fn production(app: Arc<AppState>) -> Self {
        Self::new(app, Arc::new(LazyFetcher::default()))
    }
}

/// Builds the HTTP client (proxy discovery may run git) on first use.
#[derive(Default)]
struct LazyFetcher(std::sync::OnceLock<HttpFetcher>);

impl LazyFetcher {
    fn get(&self) -> &HttpFetcher {
        self.0.get_or_init(HttpFetcher::new)
    }
}

impl GhFetcher for LazyFetcher {
    fn default_branch(&self, o: &str, r: &str, t: Option<&str>) -> Result<String, FetchError> {
        self.get().default_branch(o, r, t)
    }
    fn commit_sha(&self, o: &str, r: &str, g: &str, t: Option<&str>) -> Result<Option<String>, FetchError> {
        self.get().commit_sha(o, r, g, t)
    }
    fn tree(&self, o: &str, r: &str, s: &str, t: Option<&str>) -> Result<crate::ghimport::fetch::RepoTree, FetchError> {
        self.get().tree(o, r, s, t)
    }
    fn raw(&self, o: &str, r: &str, s: &str, p: &str, l: u64, t: Option<&str>) -> Result<Vec<u8>, FetchError> {
        self.get().raw(o, r, s, p, l, t)
    }
}

// ------------------------------------------------------------------ DTOs

#[derive(Serialize, Debug, Clone)]
pub struct ScanResultDto {
    pub session_id: String,
    /// `owner/repo`.
    pub repo: String,
    pub repo_url: String,
    pub git_ref: String,
    pub commit: String,
    /// Subdirectory (tree link) or file (blob link) the scan was limited to.
    pub scope: String,
    pub items: Vec<ImportItem>,
    /// More candidates than could be listed.
    pub truncated: bool,
    /// GitHub cut the repository listing short.
    pub tree_truncated: bool,
    /// A GitHub token was available for the scan.
    pub authenticated: bool,
}

#[derive(Serialize, Debug, Clone)]
pub struct PreviewFile {
    /// Relative to the skill directory.
    pub path: String,
    pub size: u64,
}

#[derive(Serialize, Debug, Clone)]
pub struct PreviewDto {
    /// File text (memory) or SKILL.md text (skill).
    pub text: String,
    /// Not UTF-8 text: `text` is empty.
    pub binary: bool,
    /// Skill files (empty for memories).
    pub files: Vec<PreviewFile>,
}

/// Where an imported memory goes.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImportTarget {
    GlobalClaude,
    GlobalAgents,
    ProjectClaude { project_id: String },
    ProjectAgents { project_id: String },
}

#[derive(Deserialize, Debug, Clone)]
pub struct MemoryPick {
    pub item_id: String,
    pub target: ImportTarget,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct SkippedItem {
    pub item_id: String,
    /// `binary` | `empty` | `too_large`.
    pub reason: String,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct ImportMemoriesResult {
    /// New Pending suggestions.
    pub added: u32,
    /// Picks that matched an existing suggestion.
    pub duplicates: u32,
    pub skipped: Vec<SkippedItem>,
}

// ---------------------------------------------------------------- helpers

fn fetch_err(e: FetchError) -> CommandError {
    match e {
        FetchError::NotFound => CommandError::new(
            "import_not_found",
            "repository, branch or path not found (private repositories need a GitHub login)",
        ),
        FetchError::Unauthorized => CommandError::new("unauthorized", e.to_string()),
        FetchError::RateLimited => CommandError::new("import_rate_limited", e.to_string()),
        FetchError::TooLarge => CommandError::new("import_too_large", e.to_string()),
        FetchError::Network(m) => CommandError::new("network", m),
        FetchError::Http { .. } => CommandError::new("github_error", e.to_string()),
    }
}

fn expired() -> CommandError {
    CommandError::new("import_expired", "the scan expired; scan the link again")
}

fn invalid(m: impl Into<String>) -> CommandError {
    CommandError::new("invalid", m)
}

fn prune(sessions: &mut HashMap<String, Session>) {
    sessions.retain(|_, s| s.created.elapsed() < SESSION_TTL);
    while sessions.len() >= MAX_SESSIONS {
        let Some(oldest) = sessions.iter().min_by_key(|(_, s)| s.created).map(|(k, _)| k.clone()) else {
            break;
        };
        sessions.remove(&oldest);
    }
}

/// Decodes downloaded bytes as text: BOM dropped, CRLF -> LF. `None` = binary.
fn decode_text(bytes: &[u8]) -> Option<String> {
    let b = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    if b.contains(&0) {
        return None;
    }
    std::str::from_utf8(b).ok().map(|s| s.replace("\r\n", "\n"))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(0))
        .unwrap_or(0)
}

/// Download coordinates of a session (copied out so the lock is not held during IO).
#[derive(Clone)]
struct Coords {
    owner: String,
    repo: String,
    git_ref: String,
    commit: String,
}

fn with_session<T>(
    gs: &GhImportState,
    id: &str,
    f: impl FnOnce(&mut Session) -> Result<T, CommandError>,
) -> Result<T, CommandError> {
    let mut sessions = lock(&gs.sessions);
    let s = sessions.get_mut(id).filter(|s| s.created.elapsed() < SESSION_TTL).ok_or_else(expired)?;
    f(s)
}

/// Charges `n` bytes to the session budget.
fn charge(gs: &GhImportState, id: &str, n: u64) -> Result<(), CommandError> {
    with_session(gs, id, |s| {
        if s.downloaded + n > MAX_SESSION_BYTES {
            return Err(CommandError::new("import_too_large", "download limit for this import reached"));
        }
        s.downloaded += n;
        Ok(())
    })
}

fn download(
    gs: &GhImportState,
    id: &str,
    c: &Coords,
    path: &str,
    limit: u64,
    token: Option<&str>,
) -> Result<Vec<u8>, CommandError> {
    let bytes = with_fallback(token, |t| gs.fetcher.raw(&c.owner, &c.repo, &c.commit, path, limit, t))
        .map_err(fetch_err)?;
    charge(gs, id, bytes.len() as u64)?;
    Ok(bytes)
}

/// Memory text of `path` (cached per session). `Ok(None)`: not text.
fn memory_text(
    gs: &GhImportState,
    id: &str,
    path: &str,
    token: Option<&str>,
) -> Result<Option<String>, CommandError> {
    let (cached, coords) = with_session(gs, id, |s| {
        Ok((
            s.texts.get(path).cloned(),
            Coords { owner: s.owner.clone(), repo: s.repo.clone(), git_ref: s.git_ref.clone(), commit: s.commit.clone() },
        ))
    })?;
    if let Some(t) = cached {
        return Ok(Some(t));
    }
    let bytes = download(gs, id, &coords, path, MAX_MEMORY_BYTES, token)?;
    let text = decode_text(&bytes);
    if let Some(t) = &text {
        with_session(gs, id, |s| {
            s.texts.insert(path.to_string(), t.clone());
            Ok(())
        })?;
    }
    Ok(text)
}

fn find_item(gs: &GhImportState, id: &str, item_id: &str) -> Result<(ImportItem, Coords), CommandError> {
    with_session(gs, id, |s| {
        let item = s
            .items
            .iter()
            .find(|i| i.id == item_id)
            .cloned()
            .ok_or_else(|| invalid(format!("unknown item: {item_id}")))?;
        Ok((item, Coords { owner: s.owner.clone(), repo: s.repo.clone(), git_ref: s.git_ref.clone(), commit: s.commit.clone() }))
    })
}

/// Path below the skill directory, `/`-separated.
fn rel_to(dir: &str, path: &str) -> String {
    if dir.is_empty() { path.to_string() } else { path[dir.len() + 1..].to_string() }
}

/// Rejects anything that could escape the staging folder.
fn safe_rel(rel: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in rel.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.contains(['\\', ':', '\0'])
            || part.ends_with(['.', ' '])
        {
            return None;
        }
        out.push(part);
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// A folder name the skill installer accepts as a fallback name.
fn folder_name(name: &str, repo: &str) -> String {
    let base = if name.is_empty() { repo } else { name };
    let mut s: String = base
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' })
        .collect();
    s = s.trim_start_matches(|c: char| !c.is_ascii_alphanumeric()).trim_end_matches('.').to_string();
    s.truncate(64);
    if s.is_empty() { "skill".into() } else { s }
}

/// Removes a directory tree when dropped.
struct Staging(PathBuf);

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ------------------------------------------------------------------ impls

pub fn scan_impl(gs: &GhImportState, input: &str, token: Option<&str>) -> Result<ScanResultDto, CommandError> {
    let link = parse_link(input).map_err(|m| CommandError::new("import_bad_url", m))?;
    let f = &*gs.fetcher;
    let (o, r) = (link.owner.as_str(), link.repo.as_str());
    let (git_ref, rest, commit) = match link.kind {
        LinkKind::Repo => {
            let branch = with_fallback(token, |t| f.default_branch(o, r, t)).map_err(fetch_err)?;
            let sha = with_fallback(token, |t| f.commit_sha(o, r, &branch, t))
                .map_err(fetch_err)?
                .ok_or_else(|| fetch_err(FetchError::NotFound))?;
            (branch, String::new(), sha)
        }
        LinkKind::Tree | LinkKind::Blob => {
            let mut found = None;
            for (cand, path) in link.ref_candidates(MAX_REF_TRIES) {
                if link.kind == LinkKind::Blob && path.is_empty() {
                    continue;
                }
                if let Some(sha) = with_fallback(token, |t| f.commit_sha(o, r, &cand, t)).map_err(fetch_err)? {
                    found = Some((cand, path, sha));
                    break;
                }
            }
            found.ok_or_else(|| fetch_err(FetchError::NotFound))?
        }
    };
    let tree = with_fallback(token, |t| f.tree(o, r, &commit, t)).map_err(fetch_err)?;
    let classified = match link.kind {
        LinkKind::Blob => {
            if !tree.entries.iter().any(|e| e.path == rest) {
                return Err(fetch_err(FetchError::NotFound));
            }
            classify(&tree.entries, "", Some(&rest))
        }
        _ => {
            if !rest.is_empty() && !tree.entries.iter().any(|e| e.path == rest) {
                return Err(fetch_err(FetchError::NotFound));
            }
            classify(&tree.entries, &rest, None)
        }
    };
    let session_id = uuid::Uuid::new_v4().to_string();
    let dto = ScanResultDto {
        session_id: session_id.clone(),
        repo: link.full_name(),
        repo_url: link.repo_url(),
        git_ref: git_ref.clone(),
        commit: commit.clone(),
        scope: rest,
        items: classified.items.clone(),
        truncated: classified.truncated,
        tree_truncated: tree.truncated,
        authenticated: token.is_some(),
    };
    let mut sessions = lock(&gs.sessions);
    prune(&mut sessions);
    sessions.insert(
        session_id,
        Session {
            created: Instant::now(),
            owner: link.owner,
            repo: link.repo,
            git_ref,
            commit,
            entries: tree.entries,
            items: classified.items,
            downloaded: 0,
            texts: HashMap::new(),
        },
    );
    Ok(dto)
}

pub fn preview_impl(
    gs: &GhImportState,
    session_id: &str,
    item_id: &str,
    token: Option<&str>,
) -> Result<PreviewDto, CommandError> {
    let (item, _) = find_item(gs, session_id, item_id)?;
    if item.too_large && matches!(item.kind, ItemKind::Memory { .. }) {
        return Err(CommandError::new("import_too_large", "file is too large to import"));
    }
    match item.kind {
        ItemKind::Memory { .. } => {
            let text = memory_text(gs, session_id, &item.path, token)?;
            Ok(PreviewDto { binary: text.is_none(), text: text.unwrap_or_default(), files: vec![] })
        }
        ItemKind::Skill { .. } => {
            let files: Vec<PreviewFile> = with_session(gs, session_id, |s| {
                Ok(skill_files(&item.path, &s.entries)
                    .into_iter()
                    .map(|e| PreviewFile { path: rel_to(&item.path, &e.path), size: e.size })
                    .collect())
            })?;
            let skill_md = files
                .iter()
                .find(|f| f.path.eq_ignore_ascii_case("SKILL.md"))
                .map(|f| if item.path.is_empty() { f.path.clone() } else { format!("{}/{}", item.path, f.path) })
                .ok_or_else(|| invalid("SKILL.md not found"))?;
            let text = memory_text(gs, session_id, &skill_md, token)?;
            Ok(PreviewDto { binary: text.is_none(), text: text.unwrap_or_default(), files })
        }
    }
}

/// Absolute target path and the tool whose file it is.
fn resolve_target(app: &AppState, t: &ImportTarget) -> Result<(PathBuf, Tool), CommandError> {
    let project_root = |pid: &str| -> Result<PathBuf, CommandError> {
        let idx = app.snapshot_index();
        let p = idx
            .projects
            .iter()
            .find(|pm| pm.project.id == pid)
            .map(|pm| PathBuf::from(&pm.project.path))
            .ok_or_else(|| invalid(format!("unknown project: {pid}")))?;
        if !p.is_dir() {
            return Err(invalid("project directory not found"));
        }
        Ok(p)
    };
    Ok(match t {
        ImportTarget::GlobalClaude => (app.paths.claude_home.join("CLAUDE.md"), Tool::Claude),
        ImportTarget::GlobalAgents => (app.paths.codex_home.join("AGENTS.md"), Tool::Codex),
        ImportTarget::ProjectClaude { project_id } => (project_root(project_id)?.join("CLAUDE.md"), Tool::Claude),
        ImportTarget::ProjectAgents { project_id } => (project_root(project_id)?.join("AGENTS.md"), Tool::Codex),
    })
}

pub fn import_memories_impl(
    gs: &GhImportState,
    es: &ExtractState,
    session_id: &str,
    picks: &[MemoryPick],
    token: Option<&str>,
) -> Result<ImportMemoriesResult, CommandError> {
    if picks.is_empty() {
        return Err(invalid("nothing selected"));
    }
    // Validate everything before downloading anything.
    let mut plan = Vec::with_capacity(picks.len());
    for p in picks {
        let (item, coords) = find_item(gs, session_id, &p.item_id)?;
        if !matches!(item.kind, ItemKind::Memory { .. }) {
            return Err(invalid(format!("not a memory file: {}", p.item_id)));
        }
        let (path, tool) = resolve_target(&gs.app, &p.target)?;
        plan.push((item, coords, path, tool));
    }
    let mut skipped = Vec::new();
    let mut out = Vec::new();
    for (item, c, path, tool) in plan {
        if item.too_large {
            skipped.push(SkippedItem { item_id: item.id, reason: "too_large".into() });
            continue;
        }
        let Some(text) = memory_text(gs, session_id, &item.path, token)? else {
            skipped.push(SkippedItem { item_id: item.id, reason: "binary".into() });
            continue;
        };
        let text = text.trim_end().to_string();
        if text.trim().is_empty() {
            skipped.push(SkippedItem { item_id: item.id, reason: "empty".into() });
            continue;
        }
        let path_str = path.to_string_lossy().into_owned();
        let file_id = normalize_path(&path_str);
        let base_hash = read_memory(&path).map(|f| f.hash).unwrap_or_default();
        out.push(Suggestion {
            id: uuid::Uuid::new_v4().to_string(),
            fingerprint: fingerprint(&file_id, SuggestionKind::Add, None, Some(&text)),
            kind: SuggestionKind::Add,
            file_id,
            path: path_str,
            tool_of_file: tool,
            old_text: None,
            new_text: Some(text),
            reason: String::new(),
            evidence: vec![],
            source_tool: tool,
            created_at: now_ms(),
            base_hash,
            status: SuggestionStatus::Pending,
            origin: Some(SuggestionOrigin::Github {
                repo: format!("{}/{}", c.owner, c.repo),
                path: item.path.clone(),
                git_ref: c.git_ref.clone(),
                commit: c.commit.clone(),
                url: format!(
                    "https://github.com/{}/{}/blob/{}/{}",
                    c.owner,
                    c.repo,
                    c.commit,
                    crate::ghimport::fetch::encode_path(&item.path)
                ),
            }),
            run_id: None,
        });
    }
    let total = out.len() as u32;
    let added = if out.is_empty() { 0 } else { es.add_external_suggestions(out)? };
    Ok(ImportMemoriesResult { added, duplicates: total - added, skipped })
}

pub fn import_skill_impl(
    gs: &GhImportState,
    session_id: &str,
    item_id: &str,
    targets: Vec<InstallTargetDto>,
    overwrite: bool,
    token: Option<&str>,
) -> Result<Vec<SkillEntry>, CommandError> {
    let (item, c) = find_item(gs, session_id, item_id)?;
    if !matches!(item.kind, ItemKind::Skill { .. }) {
        return Err(invalid(format!("not a skill: {item_id}")));
    }
    if item.too_large {
        return Err(CommandError::new("import_too_large", "skill is too large to import"));
    }
    if targets.is_empty() {
        return Err(invalid("no install target"));
    }
    let files: Vec<TreeEntry> = with_session(gs, session_id, |s| {
        Ok(skill_files(&item.path, &s.entries).into_iter().cloned().collect())
    })?;
    let root = gs.app.data_dir.join("skill-tmp").join(format!("gh-{}", uuid::Uuid::new_v4()));
    let staging = Staging(root);
    let dir = staging.0.join(folder_name(&item.name, &c.repo));
    std::fs::create_dir_all(&dir)?;
    let mut total = 0u64;
    for e in &files {
        let rel = rel_to(&item.path, &e.path);
        let rel_path = safe_rel(&rel).ok_or_else(|| invalid(format!("unsafe file name in skill: {rel}")))?;
        let bytes = download(gs, session_id, &c, &e.path, MAX_SKILL_BYTES, token)?;
        total += bytes.len() as u64;
        if total > MAX_SKILL_BYTES {
            return Err(CommandError::new("import_too_large", "skill is too large to import"));
        }
        let dest = dir.join(rel_path);
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&dest, bytes)?;
    }
    let from = InstallFromDto::Folder { path: dir.to_string_lossy().into_owned() };
    let installed = install_skill_impl(&gs.app, from, targets, overwrite)?;
    // Remember where it came from so updates can be checked later (best effort).
    let url = format!("https://github.com/{}/{}", c.owner, c.repo);
    let mut out = Vec::with_capacity(installed.len());
    for entry in installed {
        match register_skill_source_impl(&gs.app, &entry.id, &url, &item.path) {
            Ok(e) => out.push(e),
            Err(_) => out.push(entry),
        }
    }
    drop(staging);
    Ok(out)
}

// --------------------------------------------------------------- commands

type Gs<'a> = State<'a, Arc<GhImportState>>;
type Es<'a> = State<'a, Arc<ExtractState>>;

fn token_of(gs: &GhImportState) -> Option<String> {
    gs.app.sync.import_token()
}

#[tauri::command(async)]
pub fn gh_import_scan(gs: Gs, input: String) -> Result<ScanResultDto, CommandError> {
    let token = token_of(&gs);
    scan_impl(&gs, &input, token.as_deref())
}

#[tauri::command(async)]
pub fn gh_import_preview(gs: Gs, session_id: String, item_id: String) -> Result<PreviewDto, CommandError> {
    let token = token_of(&gs);
    preview_impl(&gs, &session_id, &item_id, token.as_deref())
}

#[tauri::command(async)]
pub fn gh_import_memories(
    gs: Gs,
    es: Es,
    session_id: String,
    picks: Vec<MemoryPick>,
) -> Result<ImportMemoriesResult, CommandError> {
    let token = token_of(&gs);
    import_memories_impl(&gs, &es, &session_id, &picks, token.as_deref())
}

#[tauri::command(async)]
pub fn gh_import_skill(
    gs: Gs,
    app: AppHandle,
    session_id: String,
    item_id: String,
    targets: Vec<InstallTargetDto>,
    overwrite: bool,
) -> Result<Vec<SkillEntry>, CommandError> {
    use tauri::Emitter;
    let token = token_of(&gs);
    let r = import_skill_impl(&gs, &session_id, &item_id, targets, overwrite, token.as_deref());
    if r.is_ok() {
        let _ = app.emit("skills-changed", crate::commands_skills::SkillsChangedPayload { paths: vec![] });
    }
    r
}

#[cfg(test)]
#[path = "commands_ghimport_tests.rs"]
mod tests;
