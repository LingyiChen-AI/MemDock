//! Session conversion between Claude Code and Codex.
//!
//! Both formats are read into a small neutral model ([`Conversation`]), planned
//! into output units ([`Out`]: text messages and paired shell calls) and written
//! in the other tool's format as a brand-new session. Source files are never
//! modified. Tool calls that cannot be mapped are degraded into assistant text,
//! so the written file never contains an unpaired tool call.

pub mod claude;
pub mod codex;
pub mod store;

#[cfg(test)]
pub(crate) mod tests;

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::model::Tool;
use crate::projects::normalize_path;

/// Max characters of a degraded tool input summary.
pub const MAX_INPUT_CHARS: usize = 600;
/// Max characters of a tool output kept in the converted session.
pub const MAX_OUTPUT_CHARS: usize = 4000;

/// One event of a conversation in source order.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    User { ts: String, text: String },
    Assistant { ts: String, text: String },
    /// A tool call; `shell` is the command line when the call runs a shell command.
    Call { ts: String, id: String, name: String, input: Value, shell: Option<String> },
    Result { ts: String, id: String, output: String, is_error: bool },
}

/// A parsed session.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversation {
    pub tool: Tool,
    pub id: String,
    pub cwd: String,
    pub title: Option<String>,
    pub model: Option<String>,
    pub items: Vec<Item>,
}

/// An output unit: a text message or a shell call paired with its result.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    User { ts: String, text: String },
    Assistant { ts: String, text: String },
    Shell { ts: String, result_ts: String, command: String, output: String, is_error: bool },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct PlanStats {
    pub user_messages: usize,
    pub assistant_messages: usize,
    pub tool_calls: usize,
    /// Shell calls mapped to the target tool's shell tool.
    pub mapped_calls: usize,
    /// Calls (and orphan results) rewritten as assistant text.
    pub degraded_calls: usize,
}

/// Truncates to `max` characters, appending a marker with the dropped count.
pub fn truncate_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max).collect();
    format!("{kept}\n…[truncated {} chars]", count - max)
}

fn one_line(s: &str, max: usize) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        let mut t: String = flat.chars().take(max).collect();
        t.push('…');
        t
    }
}

fn degrade_call(name: &str, input: &Value, shell: Option<&str>) -> String {
    let args = match shell {
        Some(cmd) => cmd.to_string(),
        None => match input {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        },
    };
    let args = truncate_chars(&args, MAX_INPUT_CHARS);
    if args.is_empty() {
        format!("[Tool call: {name}]")
    } else {
        format!("[Tool call: {name}] {args}")
    }
}

fn degrade_result(output: &str, is_error: bool) -> String {
    let label = if is_error { "[Tool error]" } else { "[Tool result]" };
    let out = truncate_chars(output.trim_end(), MAX_OUTPUT_CHARS);
    if out.is_empty() {
        format!("{label} (empty)")
    } else {
        format!("{label}\n{out}")
    }
}

/// Plans the output: shell calls with a later result become `Shell` units (the
/// result moves right after the call); every other call and every unmatched
/// result becomes assistant text. Consecutive assistant texts are merged.
pub fn plan(items: &[Item]) -> (Vec<Out>, PlanStats) {
    let mut stats = PlanStats::default();
    let mut used = vec![false; items.len()];
    let mut raw: Vec<Out> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if used[i] {
            continue;
        }
        match item {
            Item::User { ts, text } => raw.push(Out::User { ts: ts.clone(), text: text.clone() }),
            Item::Assistant { ts, text } => {
                raw.push(Out::Assistant { ts: ts.clone(), text: text.clone() })
            }
            Item::Call { ts, id, name, input, shell } => {
                stats.tool_calls += 1;
                let result = items.iter().enumerate().skip(i + 1).find(|(j, it)| {
                    !used[*j] && matches!(it, Item::Result { id: rid, .. } if rid == id)
                });
                match (shell, result) {
                    (Some(cmd), Some((j, Item::Result { ts: rts, output, is_error, .. }))) => {
                        used[j] = true;
                        stats.mapped_calls += 1;
                        raw.push(Out::Shell {
                            ts: ts.clone(),
                            result_ts: rts.clone(),
                            command: cmd.clone(),
                            output: truncate_chars(output, MAX_OUTPUT_CHARS),
                            is_error: *is_error,
                        });
                    }
                    (_, result) => {
                        stats.degraded_calls += 1;
                        let mut text = degrade_call(name, input, shell.as_deref());
                        if let Some((j, Item::Result { output, is_error, .. })) = result {
                            used[j] = true;
                            text.push('\n');
                            text.push_str(&degrade_result(output, *is_error));
                        }
                        raw.push(Out::Assistant { ts: ts.clone(), text });
                    }
                }
            }
            Item::Result { ts, output, is_error, .. } => {
                stats.degraded_calls += 1;
                raw.push(Out::Assistant { ts: ts.clone(), text: degrade_result(output, *is_error) });
            }
        }
    }
    let mut out: Vec<Out> = Vec::with_capacity(raw.len());
    for o in raw {
        if let (Some(Out::Assistant { text: prev, .. }), Out::Assistant { text, .. }) = (out.last_mut(), &o) {
            prev.push_str("\n\n");
            prev.push_str(text);
            continue;
        }
        out.push(o);
    }
    for o in &out {
        match o {
            Out::User { .. } => stats.user_messages += 1,
            Out::Assistant { .. } => stats.assistant_messages += 1,
            Out::Shell { .. } => {}
        }
    }
    (out, stats)
}

/// The note appended to converted sessions.
pub fn conversion_note(from: Tool, source_id: &str) -> String {
    let name = match from {
        Tool::Claude => "Claude Code",
        Tool::Codex => "Codex",
    };
    format!(
        "[MemDock] This session was converted by MemDock from {name} session {source_id}. \
Shell commands were kept as shell calls; other tool calls were rewritten as text summaries and long outputs were truncated. \
(本会话由 MemDock 从 {name} 会话 {source_id} 转换而来，可在此继续工作。)"
    )
}

/// Current time as an RFC 3339 UTC string with milliseconds.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// Strips the Windows verbatim prefix (`\\?\`) Codex sometimes records.
pub fn clean_cwd(cwd: &str) -> String {
    cwd.strip_prefix(r"\\?\").unwrap_or(cwd).to_string()
}

/// A session row for the list.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SessionSummary {
    pub tool: Tool,
    pub id: String,
    pub path: String,
    pub cwd: String,
    pub project_id: String,
    pub project_name: String,
    pub title: Option<String>,
    pub first_prompt: String,
    pub message_count: usize,
    pub tool_calls: usize,
    /// Unix milliseconds of the first / last item (file mtime when absent).
    pub started_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub model: Option<String>,
    /// Set when this session was written by MemDock.
    pub converted_from: Option<store::SessionRef>,
    /// Sessions MemDock already wrote from this one.
    pub converted_to: Vec<store::SessionRef>,
}

fn item_ts(item: &Item) -> &str {
    match item {
        Item::User { ts, .. }
        | Item::Assistant { ts, .. }
        | Item::Call { ts, .. }
        | Item::Result { ts, .. } => ts,
    }
}

fn project_name(cwd: &str) -> String {
    let name = cwd.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("");
    if name.is_empty() { cwd.to_string() } else { name.to_string() }
}

/// Builds the list row for a parsed conversation (mapping fields left empty).
pub fn summarize(conv: &Conversation, path: &Path) -> SessionSummary {
    use crate::extract::sessions::parse_ts;
    let first_prompt = conv
        .items
        .iter()
        .find_map(|i| match i {
            Item::User { text, .. } => Some(one_line(text, 160)),
            _ => None,
        })
        .unwrap_or_default();
    let mut message_count = 0;
    let mut tool_calls = 0;
    for i in &conv.items {
        match i {
            Item::User { .. } | Item::Assistant { .. } => message_count += 1,
            Item::Call { .. } => tool_calls += 1,
            Item::Result { .. } => {}
        }
    }
    let started_at = conv.items.iter().find_map(|i| parse_ts(item_ts(i)));
    let updated_at = conv
        .items
        .iter()
        .rev()
        .find_map(|i| parse_ts(item_ts(i)))
        .or_else(|| crate::extract::sessions::mtime_ms(path));
    SessionSummary {
        tool: conv.tool,
        id: conv.id.clone(),
        path: path.to_string_lossy().into_owned(),
        cwd: conv.cwd.clone(),
        project_id: normalize_path(&conv.cwd),
        project_name: project_name(&conv.cwd),
        title: conv.title.clone(),
        first_prompt,
        message_count,
        tool_calls,
        started_at,
        updated_at,
        model: conv.model.clone(),
        converted_from: None,
        converted_to: Vec::new(),
    }
}

/// Parses a session file of `tool`.
pub fn parse(tool: Tool, path: &Path) -> Conversation {
    match tool {
        Tool::Claude => claude::parse(path),
        Tool::Codex => codex::parse(path),
    }
}

/// Result of a conversion.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Written {
    pub tool: Tool,
    pub session_id: String,
    pub path: PathBuf,
    pub cwd: String,
    /// Codex only: whether the thread was registered in Codex's state database.
    pub indexed: bool,
    pub warnings: Vec<String>,
    pub stats: PlanStats,
}

/// Windows PowerShell 5 has no `&&`; `;` works in PowerShell 5 and 7.
const SEP: &str = if cfg!(windows) { ";" } else { " &&" };

/// Shell command to resume the written session.
pub fn resume_command(tool: Tool, cwd: &str, session_id: &str) -> String {
    let cd = if cwd.is_empty() { String::new() } else { format!("cd \"{cwd}\"{SEP} ") };
    match tool {
        Tool::Claude => format!("{cd}claude --resume {session_id}"),
        Tool::Codex => format!("{cd}codex resume {session_id}"),
    }
}
