//! Session list / preview / conversion commands (Claude Code <-> Codex).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::State;

use crate::commands::{lock, AppState, CommandError};
use crate::convert::store::{self, ConvertStore, Record, SessionRef};
use crate::convert::{self, claude, codex, Item, Out, PlanStats, SessionSummary};
use crate::extract::sessions::{claude_files, codex_files, mtime_ms};
use crate::model::Tool;
use crate::paths::AgentPaths;

/// Max preview messages / timeline items returned (the newest ones).
const PREVIEW_LIMIT: usize = 400;
/// Max characters per preview message.
const PREVIEW_CHARS: usize = 1200;
/// Tool input / output kept per call in the preview timeline.
const TIMELINE_IO_CHARS: usize = 4000;

type CacheEntry = (u64, i64, SessionSummary);

/// Parsed previews kept for quick re-opening (keyed by path, checked by size + mtime).
const PREVIEW_CACHE: usize = 6;
/// Worker threads for parsing changed session files.
const PARSE_THREADS: usize = 4;

pub struct SessionsState {
    pub app: Arc<AppState>,
    cache: Mutex<HashMap<PathBuf, CacheEntry>>,
    previews: Mutex<Vec<(PathBuf, u64, i64, SessionPreview)>>,
    /// Serializes conversions (store load-modify-save).
    convert_lock: Mutex<()>,
}

impl SessionsState {
    pub fn new(app: Arc<AppState>) -> Self {
        // Summaries from the last run: only files whose size / mtime changed are parsed again.
        let cache = load_index(&app.data_dir);
        Self { app, cache: Mutex::new(cache), previews: Mutex::new(Vec::new()), convert_lock: Mutex::new(()) }
    }
}

fn index_path(data_dir: &Path) -> PathBuf {
    data_dir.join("session-index.json")
}

#[derive(serde::Serialize, serde::Deserialize)]
struct IndexEntry {
    path: PathBuf,
    len: u64,
    mtime: i64,
    summary: SessionSummary,
}

fn load_index(data_dir: &Path) -> HashMap<PathBuf, CacheEntry> {
    let Ok(text) = std::fs::read_to_string(index_path(data_dir)) else { return HashMap::new() };
    serde_json::from_str::<Vec<IndexEntry>>(&text)
        .map(|v| v.into_iter().map(|e| (e.path, (e.len, e.mtime, e.summary))).collect())
        .unwrap_or_default()
}

fn save_index(data_dir: &Path, cache: &HashMap<PathBuf, CacheEntry>) {
    let entries: Vec<IndexEntry> = cache
        .iter()
        .map(|(p, (len, mtime, s))| IndexEntry { path: p.clone(), len: *len, mtime: *mtime, summary: s.clone() })
        .collect();
    if let Ok(json) = serde_json::to_vec(&entries) {
        let path = index_path(data_dir);
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Titles set in MemDock (`tool:id` -> title); they win over the tools' own names.
fn titles_path(data_dir: &Path) -> PathBuf {
    data_dir.join("session-titles.json")
}

fn load_titles(data_dir: &Path) -> HashMap<String, String> {
    std::fs::read_to_string(titles_path(data_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn title_key(tool: Tool, id: &str) -> String {
    format!("{}:{id}", match tool {
        Tool::Claude => "claude",
        Tool::Codex => "codex",
    })
}

/// Parses `jobs` on a few threads.
fn parse_parallel(jobs: Vec<(Tool, PathBuf, u64, i64)>) -> Vec<(PathBuf, CacheEntry)> {
    if jobs.len() <= 1 {
        return jobs
            .into_iter()
            .map(|(tool, path, len, mtime)| {
                let s = convert::summarize(&convert::parse(tool, &path), &path);
                (path, (len, mtime, s))
            })
            .collect();
    }
    let queue = Mutex::new(jobs);
    let out = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..PARSE_THREADS {
            scope.spawn(|| loop {
                let Some((tool, path, len, mtime)) = lock(&queue).pop() else { break };
                let s = convert::summarize(&convert::parse(tool, &path), &path);
                lock(&out).push((path, (len, mtime, s)));
            });
        }
    });
    out.into_inner().unwrap_or_default()
}

fn file_key(path: &Path) -> Option<(u64, i64)> {
    let len = std::fs::metadata(path).ok()?.len();
    Some((len, mtime_ms(path).unwrap_or(0)))
}

fn load_store(data_dir: &Path) -> ConvertStore {
    store::load(&store::store_path(data_dir)).unwrap_or_else(|e| {
        eprintln!("memdock: {e}");
        ConvertStore::default()
    })
}

/// Lists sessions of both tools, newest first. Parsed summaries are cached by
/// (path, size, mtime). Sessions without a user message are skipped.
pub fn list_sessions_impl(
    paths: &AgentPaths,
    data_dir: &Path,
    cache: &Mutex<HashMap<PathBuf, CacheEntry>>,
) -> Vec<SessionSummary> {
    let files: Vec<(Tool, PathBuf)> = claude_files(paths)
        .into_iter()
        .map(|p| (Tool::Claude, p))
        .chain(codex_files(paths).into_iter().map(|p| (Tool::Codex, p)))
        .collect();
    let mut fresh: HashMap<PathBuf, CacheEntry> = HashMap::new();
    let old = lock(cache).clone();
    let mut jobs = vec![];
    for (tool, path) in files {
        let Some((len, mtime)) = file_key(&path) else { continue };
        match old.get(&path) {
            Some((l, m, s)) if *l == len && *m == mtime => {
                fresh.insert(path, (len, mtime, s.clone()));
            }
            _ => jobs.push((tool, path, len, mtime)),
        }
    }
    let changed = !jobs.is_empty() || fresh.len() != old.len();
    fresh.extend(parse_parallel(jobs));
    if changed {
        save_index(data_dir, &fresh);
    }
    let store = load_store(data_dir);
    let codex_names = codex::read_session_names(&paths.codex_home);
    let titles = load_titles(data_dir);
    let mut out: Vec<SessionSummary> = fresh
        .values()
        .filter(|(_, _, s)| !s.first_prompt.is_empty())
        .map(|(_, _, s)| {
            let mut s = s.clone();
            let me = SessionRef { tool: s.tool, id: s.id.clone() };
            s.converted_from = store.source_of(&me).cloned();
            s.converted_to = store.targets_of(&me);
            if let Some(t) = titles.get(&title_key(s.tool, &s.id)) {
                s.title = Some(t.clone());
            } else if s.tool == Tool::Codex {
                if let Some(n) = codex_names.get(&s.id) {
                    s.title = Some(n.clone());
                }
            }
            s
        })
        .collect();
    *lock(cache) = fresh;
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.id.cmp(&b.id)));
    out
}

/// Canonical path of a session file of `tool`, refusing anything outside the
/// tool's session folders.
fn authorize(paths: &AgentPaths, tool: Tool, path: &str) -> Result<PathBuf, CommandError> {
    let p = std::fs::canonicalize(path).map_err(|_| CommandError::new("not_found", "session file not found"))?;
    let roots: Vec<PathBuf> = match tool {
        Tool::Claude => vec![paths.claude_home.join("projects")],
        Tool::Codex => vec![paths.codex_home.join("sessions"), paths.codex_home.join("archived_sessions")],
    };
    let ok = roots
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .any(|r| p.starts_with(&r));
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let named = match tool {
        Tool::Claude => name.ends_with(".jsonl"),
        Tool::Codex => name.starts_with("rollout-") && (name.ends_with(".jsonl") || name.ends_with(".jsonl.zst")),
    };
    if ok && named && p.is_file() {
        Ok(p)
    } else {
        Err(CommandError::new("forbidden", "not a session file"))
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewRole {
    User,
    Assistant,
    Shell,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PreviewMessage {
    pub role: PreviewRole,
    pub ts: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SessionPreview {
    pub summary: SessionSummary,
    /// The planned (converted) messages, capped to the newest ones.
    pub messages: Vec<PreviewMessage>,
    /// The original conversation, tool calls paired with results (capped to the newest ones).
    pub timeline: Vec<convert::timeline::TimelineItem>,
    pub counts: convert::timeline::TimelineCounts,
    pub timeline_total: usize,
    pub total_messages: usize,
    pub stats: PlanStats,
}

pub fn session_preview_impl(
    paths: &AgentPaths,
    data_dir: &Path,
    tool: Tool,
    path: &str,
) -> Result<SessionPreview, CommandError> {
    let p = authorize(paths, tool, path)?;
    let conv = convert::parse(tool, &p);
    let mut summary = convert::summarize(&conv, &p);
    let store = load_store(data_dir);
    let me = SessionRef { tool, id: summary.id.clone() };
    summary.converted_from = store.source_of(&me).cloned();
    summary.converted_to = store.targets_of(&me);
    let (out, stats) = convert::plan(&conv.items);
    // Long sessions keep the newest PREVIEW_LIMIT entries: the latest turns are what people open a session for.
    let messages = out[out.len().saturating_sub(PREVIEW_LIMIT)..]
        .iter()
        .map(|o| match o {
            Out::User { ts, text } => PreviewMessage {
                role: PreviewRole::User,
                ts: ts.clone(),
                text: convert::truncate_chars(text, PREVIEW_CHARS),
            },
            Out::Assistant { ts, text } => PreviewMessage {
                role: PreviewRole::Assistant,
                ts: ts.clone(),
                text: convert::truncate_chars(text, PREVIEW_CHARS),
            },
            Out::Shell { ts, command, .. } => PreviewMessage {
                role: PreviewRole::Shell,
                ts: ts.clone(),
                text: convert::truncate_chars(command, PREVIEW_CHARS),
            },
        })
        .collect();
    let (mut timeline, counts) = convert::timeline::build_with_images(&conv.items, &conv.images, PREVIEW_CHARS, TIMELINE_IO_CHARS);
    let timeline_total = timeline.len();
    timeline.drain(..timeline_total.saturating_sub(PREVIEW_LIMIT));
    Ok(SessionPreview { summary, messages, total_messages: out.len(), stats, timeline, counts, timeline_total })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConvertResult {
    pub tool: Tool,
    pub session_id: String,
    pub path: String,
    pub cwd: String,
    pub resume_command: String,
    /// Codex: registered in the state database (shows in history lists).
    pub indexed: bool,
    pub warnings: Vec<String>,
    pub stats: PlanStats,
}

/// Where a conversion of `tool` would be written (shown in the confirm dialog).
pub fn convert_target_dir_impl(paths: &AgentPaths, tool: Tool, path: &str) -> Result<String, CommandError> {
    let p = authorize(paths, tool, path)?;
    let conv = convert::parse(tool, &p);
    let dir = match tool {
        Tool::Claude => paths.codex_home.join("sessions"),
        Tool::Codex => claude::project_dir(paths, &conv.cwd),
    };
    Ok(dir.to_string_lossy().into_owned())
}

/// Converts the session at `path` (of `tool`) into the other tool's format.
/// `force` converts again even when an earlier conversion still exists.
pub fn convert_session_impl(
    paths: &AgentPaths,
    data_dir: &Path,
    lock_: &Mutex<()>,
    tool: Tool,
    path: &str,
    force: bool,
    include_note: bool,
) -> Result<ConvertResult, CommandError> {
    let _g = lock(lock_);
    let p = authorize(paths, tool, path)?;
    let conv = convert::parse(tool, &p);
    if !conv.items.iter().any(|i| matches!(i, Item::User { .. })) {
        return Err(CommandError::new("empty_session", "the session has no user message"));
    }
    if conv.cwd.is_empty() {
        return Err(CommandError::new("invalid", "the session does not record a working directory"));
    }
    let target = match tool {
        Tool::Claude => Tool::Codex,
        Tool::Codex => Tool::Claude,
    };
    let store_file = store::store_path(data_dir);
    let mut st = store::load(&store_file).map_err(|e| CommandError::new("io", e))?;
    let source = SessionRef { tool, id: conv.id.clone() };
    let earlier = st.existing(&source, target);
    if !force && !earlier.is_empty() {
        let mut e = CommandError::new(
            "already_converted",
            format!("already converted to {}", earlier.iter().map(|r| r.target.id.as_str()).collect::<Vec<_>>().join(", ")),
        );
        e.paths = Some(earlier.iter().map(|r| r.target_path.clone()).collect());
        return Err(e);
    }
    let (out, stats) = convert::plan(&conv.items);
    let note = include_note.then(|| convert::conversion_note(tool, &conv.id));
    let summary = convert::summarize(&conv, &p);
    let session_id = uuid::Uuid::new_v4().to_string();
    let mut warnings = Vec::new();
    let mut indexed = false;
    let written: PathBuf = match target {
        Tool::Claude => {
            let dir = claude::project_dir(paths, &conv.cwd);
            let lines = claude::render(&out, &session_id, &conv.cwd, note.as_deref());
            claude::write(&dir, &session_id, &lines)?
        }
        Tool::Codex => {
            let provider = codex::model_provider(&paths.codex_home);
            let started = conv
                .items
                .first()
                .map(|i| match i {
                    Item::User { ts, .. } | Item::Assistant { ts, .. } | Item::Call { ts, .. } | Item::Result { ts, .. } => ts.clone(),
                })
                .filter(|t| !t.is_empty())
                .unwrap_or_else(convert::now_rfc3339);
            let meta = codex::MetaInfo { session_id: &session_id, cwd: &conv.cwd, started_at: &started, model_provider: &provider };
            let lines = codex::render(&out, &meta, note.as_deref());
            let file = codex::rollout_path(&paths.codex_home, &session_id, chrono::Local::now());
            codex::write(&file, &lines)?;
            let title = conv.title.clone().unwrap_or_else(|| summary.first_prompt.chars().take(80).collect());
            if let Err(e) = codex::append_session_index(&paths.codex_home, &session_id, &title) {
                warnings.push(format!("session_index.jsonl: {e}"));
            }
            match codex::state_db_path(&paths.codex_home) {
                Some(db) => {
                    let now_ms = chrono::Utc::now().timestamp_millis();
                    let row = codex::ThreadRow {
                        id: &session_id,
                        rollout_path: &file,
                        cwd: &conv.cwd,
                        title: &title,
                        first_user_message: &summary.first_prompt,
                        model_provider: &provider,
                        model: None,
                        created_ms: summary.started_at.unwrap_or(now_ms),
                        updated_ms: now_ms,
                    };
                    match codex::register_thread(&db, &row) {
                        Ok(true) => indexed = true,
                        Ok(false) => warnings.push("state_db_schema".into()),
                        Err(e) => warnings.push(format!("state_db: {e}")),
                    }
                }
                None => warnings.push("state_db_missing".into()),
            }
            file
        }
    };
    st.records.push(Record {
        source,
        target: SessionRef { tool: target, id: session_id.clone() },
        target_path: written.to_string_lossy().into_owned(),
        cwd: conv.cwd.clone(),
        created_at: chrono::Utc::now().timestamp_millis(),
    });
    if let Err(e) = store::save(&store_file, &st) {
        warnings.push(format!("record: {e}"));
    }
    Ok(ConvertResult {
        tool: target,
        resume_command: convert::resume_command(target, &conv.cwd, &session_id),
        session_id,
        path: written.to_string_lossy().into_owned(),
        cwd: conv.cwd,
        indexed,
        warnings,
        stats,
    })
}

type Ss<'a> = State<'a, Arc<SessionsState>>;

#[tauri::command(async)]
pub fn list_sessions(ss: Ss) -> Vec<SessionSummary> {
    list_sessions_impl(&ss.app.paths, &ss.app.data_dir, &ss.cache)
}

#[tauri::command(async)]
pub fn session_preview(ss: Ss, tool: Tool, path: String) -> Result<SessionPreview, CommandError> {
    let key = std::fs::canonicalize(&path).ok().and_then(|p| file_key(&p).map(|k| (p, k)));
    if let Some((p, (len, mtime))) = &key {
        if let Some((_, _, _, pv)) = lock(&ss.previews).iter().find(|(q, l, m, _)| q == p && l == len && m == mtime) {
            return Ok(with_title(pv.clone(), &ss.app.data_dir));
        }
    }
    let pv = session_preview_impl(&ss.app.paths, &ss.app.data_dir, tool, &path)?;
    if let Some((p, (len, mtime))) = key {
        let mut cache = lock(&ss.previews);
        cache.retain(|(q, ..)| *q != p);
        cache.insert(0, (p, len, mtime, pv.clone()));
        cache.truncate(PREVIEW_CACHE);
    }
    Ok(with_title(pv, &ss.app.data_dir))
}

/// Applies a MemDock title to a (possibly cached) preview.
fn with_title(mut pv: SessionPreview, data_dir: &Path) -> SessionPreview {
    if let Some(t) = load_titles(data_dir).get(&title_key(pv.summary.tool, &pv.summary.id)) {
        pv.summary.title = Some(t.clone());
    }
    pv
}

/// Renames a session. The name is also written where each tool keeps it, so
/// Claude Code's `/resume` and Codex's history show it too: a `custom-title`
/// line in the Claude session file (what `/rename` writes); for Codex an entry
/// in `session_index.jsonl` and the thread title in the state database.
pub fn rename_session_impl(paths: &AgentPaths, data_dir: &Path, tool: Tool, path: &str, title: &str) -> Result<(), CommandError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(CommandError::new("empty_title", "名称不能为空"));
    }
    let title: String = title.chars().take(200).collect();
    let p = authorize(paths, tool, path)?;
    let id = convert::summarize(&convert::parse(tool, &p), &p).id;
    match tool {
        Tool::Claude => {
            use std::io::Write;
            let line = serde_json::json!({"type": "custom-title", "customTitle": title, "sessionId": id});
            let needs_nl = std::fs::read(&p).ok().and_then(|b| b.last().copied()).is_some_and(|b| b != b'\n');
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&p)
                .map_err(|e| CommandError::new("io", e.to_string()))?;
            f.write_all(format!("{}{line}\n", if needs_nl { "\n" } else { "" }).as_bytes())
                .map_err(|e| CommandError::new("io", e.to_string()))?;
        }
        Tool::Codex => {
            codex::append_session_index(&paths.codex_home, &id, &title).map_err(|e| CommandError::new("io", e.to_string()))?;
            if let Some(db) = codex::state_db_path(&paths.codex_home) {
                if let Err(e) = codex::set_thread_title(&db, &id, &title) {
                    eprintln!("memdock: codex thread title: {e}");
                }
            }
        }
    }
    let mut titles = load_titles(data_dir);
    titles.insert(title_key(tool, &id), title);
    std::fs::write(titles_path(data_dir), serde_json::to_vec_pretty(&titles).unwrap_or_default())
        .map_err(|e| CommandError::new("io", e.to_string()))?;
    Ok(())
}

#[tauri::command(async)]
pub fn rename_session(ss: Ss, tool: Tool, path: String, title: String) -> Result<(), CommandError> {
    rename_session_impl(&ss.app.paths, &ss.app.data_dir, tool, &path, &title)
}

#[tauri::command(async)]
pub fn convert_target_dir(ss: Ss, tool: Tool, path: String) -> Result<String, CommandError> {
    convert_target_dir_impl(&ss.app.paths, tool, &path)
}

#[tauri::command(async)]
pub fn convert_session(
    ss: Ss,
    tool: Tool,
    path: String,
    force: bool,
    include_note: Option<bool>,
) -> Result<ConvertResult, CommandError> {
    convert_session_impl(
        &ss.app.paths,
        &ss.app.data_dir,
        &ss.convert_lock,
        tool,
        &path,
        force,
        include_note.unwrap_or(true),
    )
}

#[cfg(test)]
#[path = "commands_sessions_tests.rs"]
mod tests;

/// How full a session's context window is, from its last recorded model request.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionContext {
    /// Tokens of the last request's prompt (what the model saw).
    pub used: u64,
    /// The model's context window.
    pub window: u64,
    pub model: Option<String>,
}

/// Bytes read from the end of a session file to find the last usage record.
const CONTEXT_TAIL: u64 = 4 * 1024 * 1024;

/// Claude 5-series models (and `[1m]` variants) have a 1M-token window; older ones 200K.
/// The bare aliases (`opus`, `sonnet`, `haiku`, `fable`, `default`) are the current 5-series models.
pub fn claude_context_window(model: &str) -> u64 {
    let m = model.trim().to_ascii_lowercase();
    if m.contains("[1m]") || matches!(m.as_str(), "opus" | "sonnet" | "haiku" | "fable" | "default") {
        return 1_000_000;
    }
    // claude-<family>-<major>[-<minor>...]
    let major = m
        .strip_prefix("claude-")
        .and_then(|rest| rest.split('-').nth(1))
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    if major >= 5 { 1_000_000 } else { 200_000 }
}

fn tail_lines(path: &Path) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return vec![] };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(CONTEXT_TAIL);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return vec![];
    }
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // probably cut in the middle
    }
    lines
}

/// Last main-thread assistant usage of a Claude Code session (jsonl lines).
pub fn claude_context(lines: &[String]) -> Option<SessionContext> {
    // A compaction after the last request: the context is now its `post_tokens`.
    let mut compacted: Option<u64> = None;
    for l in lines.iter().rev() {
        if compacted.is_none() && l.contains("\"compact_boundary\"") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(l) {
                compacted = v.pointer("/compactMetadata/postTokens").or_else(|| v.pointer("/compact_metadata/post_tokens")).and_then(|x| x.as_u64());
                if compacted.is_some() {
                    continue;
                }
            }
        }
        if !l.contains("\"usage\"") || !l.contains("\"assistant\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") || v.get("isSidechain").and_then(|s| s.as_bool()) == Some(true) {
            continue;
        }
        let msg = v.get("message")?;
        let model = msg.get("model").and_then(|m| m.as_str()).filter(|m| !m.starts_with('<')).map(String::from);
        let u = msg.get("usage")?;
        let n = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        let used = n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens");
        if used == 0 {
            continue;
        }
        let window = claude_context_window(model.as_deref().unwrap_or(""));
        return Some(SessionContext { used: compacted.unwrap_or(used), window, model });
    }
    None
}

/// Last `token_count` event of a Codex rollout.
pub fn codex_context(lines: &[String]) -> Option<SessionContext> {
    let model = lines.iter().rev().find_map(|l| {
        if !l.contains("\"turn_context\"") {
            return None;
        }
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        v.pointer("/payload/model").and_then(|m| m.as_str()).map(String::from)
    });
    for l in lines.iter().rev() {
        if !l.contains("\"token_count\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
        let Some(info) = v.pointer("/payload/info").filter(|i| i.is_object()) else { continue };
        let window = info.get("model_context_window").and_then(|w| w.as_u64()).unwrap_or(0);
        let last = info.get("last_token_usage");
        let used = last.and_then(|x| x.get("total_tokens")).and_then(|x| x.as_u64()).unwrap_or(0);
        if window == 0 || used == 0 {
            continue;
        }
        return Some(SessionContext { used, window, model });
    }
    None
}

pub fn session_context_impl(paths: &AgentPaths, tool: Tool, path: &str) -> Result<Option<SessionContext>, CommandError> {
    let p = authorize(paths, tool, path)?;
    if p.extension().and_then(|e| e.to_str()) == Some("zst") {
        return Ok(None);
    }
    let lines = tail_lines(&p);
    Ok(match tool {
        Tool::Claude => claude_context(&lines),
        Tool::Codex => codex_context(&lines),
    })
}

#[tauri::command(async)]
pub fn session_context(ss: Ss, tool: Tool, path: String) -> Result<Option<SessionContext>, CommandError> {
    session_context_impl(&ss.app.paths, tool, &path)
}

/// Like [`session_context`], for a conversation known only by its Claude session id / Codex
/// thread id (a live chat whose file isn't on the list yet). The list is refreshed once on a miss.
#[tauri::command(async)]
pub fn session_context_by_id(ss: Ss, tool: Tool, id: String) -> Result<Option<SessionContext>, CommandError> {
    let find = || {
        lock(&ss.cache)
            .values()
            .find(|(_, _, s)| s.tool == tool && s.id == id)
            .map(|(_, _, s)| s.path.clone())
    };
    let path = match find() {
        Some(p) => Some(p),
        None => {
            list_sessions_impl(&ss.app.paths, &ss.app.data_dir, &ss.cache);
            find()
        }
    };
    match path {
        Some(p) => session_context_impl(&ss.app.paths, tool, &p),
        None => Ok(None),
    }
}
