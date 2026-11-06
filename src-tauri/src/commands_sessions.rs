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

/// Max preview messages returned.
const PREVIEW_LIMIT: usize = 400;
/// Max characters per preview message.
const PREVIEW_CHARS: usize = 1200;
/// Tool input / output kept per call in the preview timeline.
const TIMELINE_IO_CHARS: usize = 4000;

type CacheEntry = (u64, i64, SessionSummary);

pub struct SessionsState {
    pub app: Arc<AppState>,
    cache: Mutex<HashMap<PathBuf, CacheEntry>>,
    /// Serializes conversions (store load-modify-save).
    convert_lock: Mutex<()>,
}

impl SessionsState {
    pub fn new(app: Arc<AppState>) -> Self {
        Self { app, cache: Mutex::new(HashMap::new()), convert_lock: Mutex::new(()) }
    }
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
    let old = std::mem::take(&mut *lock(cache));
    for (tool, path) in files {
        let Some((len, mtime)) = file_key(&path) else { continue };
        let summary = match old.get(&path) {
            Some((l, m, s)) if *l == len && *m == mtime => s.clone(),
            _ => convert::summarize(&convert::parse(tool, &path), &path),
        };
        fresh.insert(path, (len, mtime, summary));
    }
    let store = load_store(data_dir);
    let mut out: Vec<SessionSummary> = fresh
        .values()
        .filter(|(_, _, s)| !s.first_prompt.is_empty())
        .map(|(_, _, s)| {
            let mut s = s.clone();
            let me = SessionRef { tool: s.tool, id: s.id.clone() };
            s.converted_from = store.source_of(&me).cloned();
            s.converted_to = store.targets_of(&me);
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
    /// The planned (converted) messages, capped.
    pub messages: Vec<PreviewMessage>,
    /// The original conversation, tool calls paired with results (capped).
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
    let messages = out
        .iter()
        .take(PREVIEW_LIMIT)
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
    timeline.truncate(PREVIEW_LIMIT);
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
    session_preview_impl(&ss.app.paths, &ss.app.data_dir, tool, &path)
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
