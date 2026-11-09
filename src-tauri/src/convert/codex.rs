//! Codex session files: `sessions/YYYY/MM/DD/rollout-<local time>-<uuid>.jsonl[.zst]`.
//!
//! Every line is `{timestamp, type, payload}`. The first line is
//! `session_meta` (id, cwd, originator, cli_version, source, model_provider);
//! the conversation lives in `response_item` lines (message / function_call /
//! function_call_output / custom_tool_call / local_shell_call / reasoning) and
//! is mirrored for the UI in `event_msg` lines (user_message / agent_message).
//! Codex lists threads from `state_<n>.sqlite` (`threads` table) and names
//! them from `session_index.jsonl`; `codex resume <id>` also finds rollouts by
//! the uuid in the file name.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{clean_cwd, now_rfc3339, Conversation, Item, Out};
use crate::extract::sessions::{codex_files_in, codex_id_from_path, for_each_json_prefix, is_injected_codex};
use crate::model::Tool;

/// Shell command line of a function call, when it runs a shell.
pub fn shell_command(name: &str, args: &Value) -> Option<String> {
    let from_argv = |argv: &[Value]| -> Option<String> {
        let parts: Vec<&str> = argv.iter().filter_map(Value::as_str).collect();
        if parts.is_empty() {
            return None;
        }
        let flag = parts.get(1).map(|f| f.to_ascii_lowercase());
        if parts.len() == 3 && matches!(flag.as_deref(), Some("-lc" | "-c" | "-command" | "/c")) {
            return Some(parts[2].to_string());
        }
        Some(parts.join(" "))
    };
    match name {
        "shell" | "container.exec" | "local_shell" => match args.get("command")? {
            Value::Array(a) => from_argv(a),
            Value::String(s) => Some(s.clone()),
            _ => None,
        },
        "shell_command" => args.get("command")?.as_str().map(String::from),
        "exec_command" => args
            .get("cmd")
            .or_else(|| args.get("command"))?
            .as_str()
            .map(String::from),
        _ => None,
    }
}

fn output_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|p| match p.get("type").and_then(Value::as_str) {
                Some("input_text") | Some("output_text") | Some("text") => {
                    p.get("text").and_then(Value::as_str).unwrap_or("").to_string()
                }
                Some("input_image") => "[image]".to_string(),
                _ => String::new(),
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(o) => o.get("content").map(output_text).unwrap_or_default(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn message_text(content: &Value) -> String {
    let Some(items) = content.as_array() else { return output_text(content) };
    items
        .iter()
        .filter_map(|i| match i.get("type").and_then(Value::as_str) {
            Some("input_text") | Some("output_text") => i.get("text").and_then(Value::as_str).map(String::from),
            Some("input_image") => Some("[image]".to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parses a Codex rollout. Developer/system messages, injected context,
/// reasoning and event lines are dropped.
pub fn parse(path: &Path) -> Conversation {
    parse_prefix(path, None, None, &mut Vec::new())
}

fn parse_prefix(
    path: &Path,
    bytes: Option<u64>,
    lines: Option<u64>,
    ancestors: &mut Vec<PathBuf>,
) -> Conversation {
    let mut conv = Conversation {
        tool: Tool::Codex,
        id: codex_id_from_path(path),
        cwd: String::new(),
        title: None,
        model: None,
        items: Vec::new(),
        images: Vec::new(),
    };
    let mut seen_meta = false;
    let mut last_ts = String::new();
    ancestors.push(path.to_owned());
    for_each_json_prefix(path, bytes, lines, |v| {
        let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
        let ts = v
            .get("timestamp")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| last_ts.clone());
        last_ts = ts.clone();
        let Some(p) = v.get("payload") else { return };
        match kind {
            "session_meta" if !seen_meta => {
                seen_meta = true;
                // Paginated Codex forks store only a snapshot pointer plus their own new turns.
                // Legacy forks inline their history and have no history_base.
                if let Some(base) = p.get("history_base") {
                    if let Some(parent) = read_fork_base(path, base, ancestors) {
                        conv.items = parent.items;
                        conv.images = parent.images;
                        conv.model = parent.model;
                    }
                }
                if let Some(id) = p.get("id").or_else(|| p.get("session_id")).and_then(Value::as_str) {
                    if !id.is_empty() {
                        conv.id = id.to_string();
                    }
                }
                if let Some(c) = p.get("cwd").and_then(Value::as_str) {
                    conv.cwd = clean_cwd(c);
                }
            }
            "turn_context" => {
                if let Some(m) = p.get("model").and_then(Value::as_str) {
                    conv.model = Some(m.to_string());
                }
            }
            "compacted" => {
                if let Some(m) = p.get("message").and_then(Value::as_str).filter(|m| !m.trim().is_empty()) {
                    conv.items.push(Item::User {
                        ts,
                        text: format!("[Summary of the earlier conversation]\n{m}"),
                    });
                }
            }
            "response_item" => parse_response_item(p, ts, &mut conv.items, &mut conv.images),
            _ => {}
        }
    });
    ancestors.pop();
    conv
}

fn read_fork_base(path: &Path, base: &Value, ancestors: &mut Vec<PathBuf>) -> Option<Conversation> {
    // Resolve only within this Codex home. Bound traversal for malformed or cyclic rollouts.
    if ancestors.len() >= 64 {
        return None;
    }
    let id = base.get("thread_id")?.as_str()?;
    let bytes = base.get("end_byte_offset").and_then(Value::as_u64);
    let lines = base.get("end_ordinal_exclusive").and_then(Value::as_u64);
    if bytes.is_none() && lines.is_none() {
        return None;
    }
    let home = path.ancestors()
        .find(|part| part.file_name().is_some_and(|name| name == "sessions" || name == "archived_sessions"))?
        .parent()?;
    let parent = codex_files_in(home).into_iter().rev()
        .find(|candidate| codex_id_from_path(candidate) == id && !ancestors.contains(candidate))?;
    // A byte cursor counts raw JSONL records, including any line too large to parse.
    Some(parse_prefix(&parent, bytes, if bytes.is_some() { None } else { lines }, ancestors))
}

fn parse_response_item(p: &Value, ts: String, items: &mut Vec<Item>, images: &mut Vec<(usize, String)>) {
    let str_field = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match p.get("type").and_then(Value::as_str) {
        Some("message") => {
            let text = message_text(p.get("content").unwrap_or(&Value::Null));
            if text.trim().is_empty() {
                return;
            }
            match p.get("role").and_then(Value::as_str) {
                Some("user") if !is_injected_codex(&text) => {
                    items.push(Item::User { ts, text });
                    let at = items.len() - 1;
                    for part in p.get("content").and_then(Value::as_array).into_iter().flatten() {
                        if part.get("type").and_then(Value::as_str) == Some("input_image") {
                            if let Some(url) = part.get("image_url").and_then(Value::as_str).filter(|u| u.starts_with("data:image/")) {
                                images.push((at, url.to_string()));
                            }
                        }
                    }
                }
                Some("assistant") => items.push(Item::Assistant { ts, text }),
                _ => {}
            }
        }
        Some("function_call") => {
            let name = str_field("name");
            let raw = str_field("arguments");
            let input = serde_json::from_str::<Value>(&raw).unwrap_or(Value::String(raw));
            let shell = shell_command(&name, &input);
            items.push(Item::Call { ts, id: str_field("call_id"), name, input, shell });
        }
        Some("local_shell_call") => {
            let action = p.get("action").cloned().unwrap_or(Value::Null);
            let shell = shell_command("local_shell", &action);
            let id = p.get("call_id").or_else(|| p.get("id")).and_then(Value::as_str).unwrap_or("").to_string();
            items.push(Item::Call { ts, id, name: "local_shell".into(), input: action, shell });
        }
        Some("custom_tool_call") => {
            items.push(Item::Call {
                ts,
                id: str_field("call_id"),
                name: str_field("name"),
                input: Value::String(str_field("input")),
                shell: None,
            });
        }
        Some("function_call_output") | Some("custom_tool_call_output") => {
            let out = p.get("output").unwrap_or(&Value::Null);
            let is_error = out.get("success").and_then(Value::as_bool) == Some(false);
            items.push(Item::Result { ts, id: str_field("call_id"), output: output_text(out), is_error });
        }
        // reasoning (often encrypted), web_search_call, additional_tools … are dropped.
        _ => {}
    }
}

fn line(ts: &str, kind: &str, payload: Value) -> String {
    json!({"timestamp": ts, "type": kind, "payload": payload}).to_string()
}

/// Metadata for the written rollout.
pub struct MetaInfo<'a> {
    pub session_id: &'a str,
    pub cwd: &'a str,
    pub started_at: &'a str,
    pub model_provider: &'a str,
}

/// Renders planned output as rollout lines: session_meta, then each message as
/// a response_item plus its event_msg mirror, and each shell call as a
/// `shell` function_call immediately followed by its output.
pub fn render(out: &[Out], meta: &MetaInfo, note: Option<&str>) -> Vec<String> {
    let mut lines = vec![line(
        meta.started_at,
        "session_meta",
        json!({
            "id": meta.session_id,
            "timestamp": meta.started_at,
            "cwd": meta.cwd,
            "originator": "memdock",
            "cli_version": "0.0.0",
            "instructions": null,
            "source": "vscode", // Codex Desktop writes "vscode"; a "cli" thread did not show in its list
            "model_provider": meta.model_provider,
        }),
    )];
    let user = |lines: &mut Vec<String>, ts: &str, text: &str| {
        lines.push(line(
            ts,
            "response_item",
            json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": text}]}),
        ));
        lines.push(line(ts, "event_msg", json!({"type": "user_message", "message": text, "images": []})));
    };
    let assistant = |lines: &mut Vec<String>, ts: &str, text: &str| {
        lines.push(line(
            ts,
            "response_item",
            json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": text}]}),
        ));
        lines.push(line(ts, "event_msg", json!({"type": "agent_message", "message": text})));
    };
    if !matches!(out.first(), Some(Out::User { .. })) {
        user(&mut lines, meta.started_at, "(Continuing a conversation converted by MemDock.)");
    }
    for o in out {
        match o {
            Out::User { ts, text } => user(&mut lines, ts, text),
            Out::Assistant { ts, text } => assistant(&mut lines, ts, text),
            Out::Shell { ts, result_ts, command, output, is_error } => {
                let call_id = format!("call_memdock_{}", uuid::Uuid::new_v4().simple());
                let args = json!({"command": ["bash", "-lc", command], "workdir": meta.cwd}).to_string();
                lines.push(line(
                    ts,
                    "response_item",
                    json!({"type": "function_call", "name": "shell", "arguments": args, "call_id": call_id}),
                ));
                let output = if *is_error { format!("[error]\n{output}") } else { output.clone() };
                lines.push(line(
                    result_ts,
                    "response_item",
                    json!({"type": "function_call_output", "call_id": call_id, "output": output}),
                ));
            }
        }
    }
    if let Some(n) = note {
        assistant(&mut lines, &now_rfc3339(), n);
    }
    lines
}

/// `sessions/YYYY/MM/DD/rollout-YYYY-MM-DDTHH-MM-SS-<id>.jsonl` for local time `now`.
pub fn rollout_path(codex_home: &Path, session_id: &str, now: chrono::DateTime<chrono::Local>) -> PathBuf {
    codex_home
        .join("sessions")
        .join(now.format("%Y").to_string())
        .join(now.format("%m").to_string())
        .join(now.format("%d").to_string())
        .join(format!("rollout-{}-{session_id}.jsonl", now.format("%Y-%m-%dT%H-%M-%S")))
}

/// Writes the rollout (never overwrites).
pub fn write(path: &Path, lines: &[String]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut f = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut body = lines.join("\n");
    body.push('\n');
    if let Err(e) = f.write_all(body.as_bytes()).and_then(|_| f.sync_all()) {
        drop(f);
        let _ = fs::remove_file(path);
        return Err(e);
    }
    Ok(())
}

/// Top-level `model_provider` from `config.toml`, defaulting to `openai`.
pub fn model_provider(codex_home: &Path) -> String {
    let Ok(text) = fs::read_to_string(codex_home.join("config.toml")) else {
        return "openai".into();
    };
    for l in text.lines() {
        let l = l.trim();
        if l.starts_with('[') {
            break;
        }
        if let Some(rest) = l.strip_prefix("model_provider") {
            if let Some(v) = rest.trim_start().strip_prefix('=') {
                let v = v.split('#').next().unwrap_or("").trim().trim_matches('"').trim_matches('\'');
                if !v.is_empty() {
                    return v.to_string();
                }
            }
        }
    }
    "openai".into()
}

/// Thread row data for the state database.
pub struct ThreadRow<'a> {
    pub id: &'a str,
    pub rollout_path: &'a Path,
    pub cwd: &'a str,
    pub title: &'a str,
    pub first_user_message: &'a str,
    pub model_provider: &'a str,
    pub model: Option<&'a str>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

/// The newest `state_<n>.sqlite` in `codex_home`.
pub fn state_db_path(codex_home: &Path) -> Option<PathBuf> {
    let rd = fs::read_dir(codex_home).ok()?;
    rd.filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n: u32 = name.strip_prefix("state_")?.strip_suffix(".sqlite")?.parse().ok()?;
            Some((n, e.path()))
        })
        .max_by_key(|(n, _)| *n)
        .map(|(_, p)| p)
}

/// Registers the thread in Codex's state database so history lists show it.
/// Only columns that exist are written; when a required column is unknown the
/// row is not written. Returns whether a row was inserted.
pub fn register_thread(db: &Path, row: &ThreadRow) -> Result<bool, String> {
    use rusqlite::{types::Value as Sql, Connection, OpenFlags};
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(3)).map_err(|e| e.to_string())?;
    // (name, notnull, has_default)
    let mut cols: Vec<(String, bool, bool)> = Vec::new();
    {
        let mut stmt = conn.prepare("PRAGMA table_info(threads)").map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(3)? != 0,
                    r.get::<_, Option<String>>(4)?.is_some(),
                ))
            })
            .map_err(|e| e.to_string())?;
        for r in rows {
            cols.push(r.map_err(|e| e.to_string())?);
        }
    }
    if cols.is_empty() {
        return Ok(false);
    }
    let secs = |ms: i64| ms.div_euclid(1000);
    let path = row.rollout_path.to_string_lossy().into_owned();
    let mut values: Vec<(&str, Sql)> = vec![
        ("id", Sql::Text(row.id.into())),
        ("rollout_path", Sql::Text(path)),
        ("created_at", Sql::Integer(secs(row.created_ms))),
        ("updated_at", Sql::Integer(secs(row.updated_ms))),
        ("created_at_ms", Sql::Integer(row.created_ms)),
        ("updated_at_ms", Sql::Integer(row.updated_ms)),
        ("recency_at", Sql::Integer(secs(row.updated_ms))),
        ("recency_at_ms", Sql::Integer(row.updated_ms)),
        ("source", Sql::Text("vscode".into())),
        ("model_provider", Sql::Text(row.model_provider.into())),
        ("cwd", Sql::Text(row.cwd.into())),
        ("title", Sql::Text(row.title.into())),
        ("first_user_message", Sql::Text(row.first_user_message.into())),
        ("preview", Sql::Text(row.first_user_message.into())),
        ("sandbox_policy", Sql::Text(r#"{"type":"workspace-write"}"#.into())),
        ("approval_mode", Sql::Text("on-request".into())),
        ("has_user_event", Sql::Integer(1)),
        ("archived", Sql::Integer(0)),
        ("cli_version", Sql::Text("0.0.0".into())),
        ("thread_source", Sql::Text("user".into())),
        ("history_mode", Sql::Text("legacy".into())),
        ("originator", Sql::Text("memdock".into())),
    ];
    if let Some(m) = row.model {
        values.push(("model", Sql::Text(m.into())));
    }
    values.retain(|(k, _)| cols.iter().any(|(c, _, _)| c == k));
    let missing_required = cols
        .iter()
        .any(|(c, notnull, has_default)| *notnull && !*has_default && !values.iter().any(|(k, _)| k == c));
    if missing_required {
        return Ok(false);
    }
    let names: Vec<&str> = values.iter().map(|(k, _)| *k).collect();
    let marks = vec!["?"; names.len()].join(", ");
    let sql = format!("INSERT OR IGNORE INTO threads ({}) VALUES ({marks})", names.join(", "));
    let params: Vec<Sql> = values.into_iter().map(|(_, v)| v).collect();
    let n = conn
        .execute(&sql, rusqlite::params_from_iter(params))
        .map_err(|e| e.to_string())?;
    Ok(n > 0)
}

/// Thread names from `session_index.jsonl` (the last entry of an id wins).
pub fn read_session_names(codex_home: &Path) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let Ok(text) = fs::read_to_string(codex_home.join("session_index.jsonl")) else { return out };
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let (Some(id), Some(name)) = (v.get("id").and_then(Value::as_str), v.get("thread_name").and_then(Value::as_str)) else {
            continue;
        };
        if name.trim().is_empty() {
            out.remove(id);
        } else {
            out.insert(id.to_string(), name.trim().to_string());
        }
    }
    out
}

/// Sets the thread title in the state database (best effort; `false` when no row).
pub fn set_thread_title(db: &Path, id: &str, title: &str) -> Result<bool, String> {
    use rusqlite::{Connection, OpenFlags};
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(3)).map_err(|e| e.to_string())?;
    let n = conn
        .execute("UPDATE threads SET title = ?1 WHERE id = ?2", rusqlite::params![title, id])
        .map_err(|e| e.to_string())?;
    Ok(n > 0)
}

/// Appends `{id, thread_name, updated_at}` to `session_index.jsonl`.
pub fn append_session_index(codex_home: &Path, id: &str, name: &str) -> std::io::Result<()> {
    let path = codex_home.join("session_index.jsonl");
    let mut prefix = String::new();
    if let Ok(existing) = fs::read(&path) {
        if existing.last().is_some_and(|b| *b != b'\n') {
            prefix.push('\n');
        }
    }
    let mut f = fs::OpenOptions::new().create(true).append(true).open(&path)?;
    let entry = json!({"id": id, "thread_name": name, "updated_at": now_rfc3339()});
    f.write_all(format!("{prefix}{entry}\n").as_bytes())
}
