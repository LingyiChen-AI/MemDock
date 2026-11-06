//! Reads Claude Code and Codex session logs into a unified list of turns.

use std::io::BufRead;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use walkdir::WalkDir;

use crate::model::Tool;
use crate::paths::AgentPaths;
use crate::projects::{normalize_path, open_jsonl};

/// Lines longer than this are skipped.
const MAX_LINE_BYTES: usize = 1024 * 1024;
/// Files larger than this are skipped.
const MAX_FILE_BYTES: u64 = 200 * 1024 * 1024;
/// Upper bound for `before` / `after` in `context_around`.
const MAX_CONTEXT: usize = 6;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Turn {
    pub tool: Tool,
    pub session_id: String,
    pub project_id: Option<String>,
    /// Unix milliseconds.
    pub ts: i64,
    pub role: Role,
    pub text: String,
}

/// Parses an RFC 3339 timestamp (`2026-11-05T10:00:00.000Z`, optional offset) to unix ms.
pub(crate) fn parse_ts(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !(b[10] == b'T' || b[10] == b' ') {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> { s.get(a..z)?.parse::<i64>().ok() };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, se) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let mut idx = 19;
    let mut millis = 0i64;
    if b.get(idx) == Some(&b'.') {
        let start = idx + 1;
        idx = start;
        while idx < b.len() && b[idx].is_ascii_digit() {
            idx += 1;
        }
        let frac = &s[start..idx];
        let three: String = frac.chars().chain("000".chars()).take(3).collect();
        millis = three.parse().ok()?;
    }
    let mut offset_min = 0i64;
    match b.get(idx) {
        Some(b'+') | Some(b'-') => {
            let sign = if b[idx] == b'+' { 1 } else { -1 };
            let oh: i64 = s.get(idx + 1..idx + 3)?.parse().ok()?;
            let om: i64 = s.get(idx + 4..idx + 6).and_then(|x| x.parse().ok()).unwrap_or(0);
            offset_min = sign * (oh * 60 + om);
        }
        _ => {}
    }
    // Days from civil (Howard Hinnant).
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + h * 3600 + mi * 60 + se - offset_min * 60;
    Some(secs * 1000 + millis)
}

/// Reads one line (without the newline) up to `MAX_LINE_BYTES`.
/// Returns `None` at EOF, `Some(None)` for an oversized (skipped) line.
fn next_line(r: &mut dyn BufRead) -> Option<Option<Vec<u8>>> {
    let mut buf: Vec<u8> = Vec::new();
    let mut too_long = false;
    let mut read_any = false;
    loop {
        let chunk = match r.fill_buf() {
            Ok(c) => c,
            Err(_) => return if read_any { Some(if too_long { None } else { Some(buf) }) } else { None },
        };
        if chunk.is_empty() {
            break;
        }
        read_any = true;
        let (take, found) = match chunk.iter().position(|&c| c == b'\n') {
            Some(p) => (p, true),
            None => (chunk.len(), false),
        };
        if !too_long {
            if buf.len() + take > MAX_LINE_BYTES {
                too_long = true;
                buf = Vec::new();
            } else {
                buf.extend_from_slice(&chunk[..take]);
            }
        }
        r.consume(take + usize::from(found));
        if found {
            break;
        }
    }
    if !read_any {
        return None;
    }
    Some(if too_long { None } else { Some(buf) })
}

/// Calls `f` for each parsed JSON line of `path`. Oversized lines and unparsable lines are skipped.
pub(crate) fn for_each_json<F: FnMut(Value)>(path: &Path, mut f: F) {
    let Ok(meta) = std::fs::metadata(path) else { return };
    if meta.len() > MAX_FILE_BYTES {
        eprintln!("extract: skipping oversized session file {}", path.display());
        return;
    }
    let Ok(mut reader) = open_jsonl(path) else { return };
    while let Some(line) = next_line(reader.as_mut()) {
        let Some(bytes) = line else { continue };
        if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
            f(v);
        }
    }
}

pub(crate) fn mtime_ms(path: &Path) -> Option<i64> {
    let m = std::fs::metadata(path).ok()?.modified().ok()?;
    let d = m.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(d.as_millis()).ok()
}

fn is_noise(text: &str, role: Role) -> bool {
    let t = text.trim_start();
    (role == Role::User && t.starts_with("[Request interrupted by user"))
        || t.starts_with("<command-")
        || t.starts_with("<local-command-")
        || t.starts_with("<system-reminder>")
        || t.starts_with("<task-notification>")
}

pub(crate) fn claude_files(paths: &AgentPaths) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(paths.claude_home.join("projects"))
        .min_depth(2)
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    files
}

fn claude_text(content: &Value, role: Role) -> Option<String> {
    let text = match content {
        Value::String(s) if is_noise(s, role) => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter(|i| i.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|i| i.get("text").and_then(Value::as_str))
            .filter(|t| !is_noise(t, role))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn read_claude_file(path: &Path, since_ms: i64, out: &mut Vec<Turn>) {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    for_each_json(path, |v| {
        let role = match v.get("type").and_then(Value::as_str) {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            _ => return,
        };
        if v.get("isMeta").and_then(Value::as_bool) == Some(true)
            || v.get("isCompactSummary").and_then(Value::as_bool) == Some(true)
            // Lines Claude Code wrote itself (background task notifications).
            || v.get("promptSource").and_then(Value::as_str) == Some("system")
        {
            return;
        }
        let Some(text) = v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| claude_text(c, role))
        else {
            return;
        };
        let Some(ts) = v.get("timestamp").and_then(Value::as_str).and_then(parse_ts) else {
            return;
        };
        if ts < since_ms {
            return;
        }
        let session_id = v
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .unwrap_or_else(|| stem.clone());
        let project_id = v
            .get("cwd")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(normalize_path);
        out.push(Turn { tool: Tool::Claude, session_id, project_id, ts, role, text });
    });
}

/// Reads Claude Code turns from `projects/<enc>/*.jsonl`, newer than `since_ms`.
pub fn read_claude_turns(paths: &AgentPaths, since_ms: i64) -> Vec<Turn> {
    let mut out = Vec::new();
    for f in claude_files(paths) {
        if mtime_ms(&f).is_some_and(|m| m < since_ms) {
            continue;
        }
        read_claude_file(&f, since_ms, &mut out);
    }
    out
}

pub(crate) fn codex_files(paths: &AgentPaths) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = ["sessions", "archived_sessions"]
        .iter()
        .flat_map(|d| WalkDir::new(paths.codex_home.join(d)))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            let n = e.file_name().to_string_lossy();
            n.starts_with("rollout-") && (n.ends_with(".jsonl") || n.ends_with(".jsonl.zst"))
        })
        .map(|e| e.into_path())
        .collect();
    files.sort();
    files
}

fn codex_text(content: &Value) -> Option<String> {
    let items = content.as_array()?;
    let text = items
        .iter()
        .filter(|i| {
            matches!(
                i.get("type").and_then(Value::as_str),
                Some("input_text") | Some("output_text")
            )
        })
        .filter_map(|i| i.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Injected user context such as `<environment_context>` starts with an angle-bracket tag.
pub(crate) fn is_injected_codex(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("<environment_context>")
        || t.starts_with("<user_instructions>")
        || t.starts_with("<permissions")
        || t.starts_with("<turn_aborted")
        || t.starts_with("<user_action")
        || t.starts_with("<collaboration_mode")
        || t.starts_with("<world_state")
        || t.starts_with("<skills_instructions")
        || t.starts_with("# AGENTS.md instructions for")
}

/// Session id from a rollout file name: the trailing uuid, else the last dash segment.
pub(crate) fn codex_id_from_path(path: &Path) -> String {
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = name
        .trim_end_matches(".zst")
        .trim_end_matches(".jsonl")
        .to_string();
    let is_uuid = |s: &str| {
        let parts: Vec<&str> = s.split('-').collect();
        parts.len() == 5
            && parts.iter().zip([8usize, 4, 4, 4, 12]).all(|(p, n)| {
                p.len() == n && p.chars().all(|c| c.is_ascii_hexdigit())
            })
    };
    if stem.len() >= 36 && stem.is_char_boundary(stem.len() - 36) && is_uuid(&stem[stem.len() - 36..]) {
        return stem[stem.len() - 36..].to_string();
    }
    stem.strip_prefix("rollout-").unwrap_or(&stem).to_string()
}

fn read_codex_file(path: &Path, since_ms: i64, out: &mut Vec<Turn>) {
    let mut session_id = codex_id_from_path(path);
    let mut project_id: Option<String> = None;
    let mut seen_meta = false;
    for_each_json(path, |v| {
        let kind = v.get("type").and_then(Value::as_str);
        if kind == Some("session_meta") && !seen_meta {
            seen_meta = true;
            let p = v.get("payload");
            if let Some(id) = p
                .and_then(|p| p.get("id").or_else(|| p.get("session_id")))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                session_id = id.to_string();
            }
            project_id = p
                .and_then(|p| p.get("cwd"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(normalize_path);
            return;
        }
        if kind != Some("response_item") {
            return;
        }
        let Some(p) = v.get("payload") else { return };
        if p.get("type").and_then(Value::as_str) != Some("message") {
            return;
        }
        let role = match p.get("role").and_then(Value::as_str) {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            _ => return,
        };
        let Some(text) = p.get("content").and_then(codex_text) else { return };
        if role == Role::User && is_injected_codex(&text) {
            return;
        }
        let Some(ts) = v.get("timestamp").and_then(Value::as_str).and_then(parse_ts) else {
            return;
        };
        if ts < since_ms {
            return;
        }
        out.push(Turn {
            tool: Tool::Codex,
            session_id: session_id.clone(),
            project_id: project_id.clone(),
            ts,
            role,
            text,
        });
    });
}

/// Reads Codex turns from `sessions/**` and `archived_sessions/**`, newer than `since_ms`.
pub fn read_codex_turns(paths: &AgentPaths, since_ms: i64) -> Vec<Turn> {
    let mut out = Vec::new();
    for f in codex_files(paths) {
        if mtime_ms(&f).is_some_and(|m| m < since_ms) {
            continue;
        }
        read_codex_file(&f, since_ms, &mut out);
    }
    out
}

pub fn turns_for_project<'a>(turns: &'a [Turn], project_id: &str) -> Vec<&'a Turn> {
    turns
        .iter()
        .filter(|t| t.project_id.as_deref() == Some(project_id))
        .collect()
}

/// Returns up to `before` turns before and `after` turns after the turn at `ts`
/// (the nearest turn when there is no exact match), including that turn. Each side is capped at 6.
pub fn context_around(
    paths: &AgentPaths,
    tool: Tool,
    session_id: &str,
    ts: i64,
    before: usize,
    after: usize,
) -> Vec<Turn> {
    if session_id.is_empty() {
        return Vec::new();
    }
    let (before, after) = (before.min(MAX_CONTEXT), after.min(MAX_CONTEXT));
    let mut turns = Vec::new();
    match tool {
        Tool::Claude => {
            for f in claude_files(paths) {
                if f.file_stem().is_some_and(|s| s.to_string_lossy() == session_id) {
                    read_claude_file(&f, i64::MIN, &mut turns);
                }
            }
        }
        Tool::Codex => {
            for f in codex_files(paths) {
                if f.file_name().is_some_and(|n| n.to_string_lossy().contains(session_id)) {
                    read_codex_file(&f, i64::MIN, &mut turns);
                }
            }
        }
    }
    turns.retain(|t| t.session_id == session_id);
    turns.sort_by_key(|t| t.ts);
    let Some(center) = turns
        .iter()
        .enumerate()
        .min_by_key(|(_, t)| (t.ts - ts).abs())
        .map(|(i, _)| i)
    else {
        return Vec::new();
    };
    let start = center.saturating_sub(before);
    let end = (center + after + 1).min(turns.len());
    turns[start..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime};

    fn setup() -> (tempfile::TempDir, AgentPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = AgentPaths::from_home(dir.path());
        (dir, paths)
    }

    fn write_claude(paths: &AgentPaths, enc: &str, session: &str, lines: &[String]) -> PathBuf {
        let d = paths.claude_home.join("projects").join(enc);
        fs::create_dir_all(&d).unwrap();
        let f = d.join(format!("{session}.jsonl"));
        fs::write(&f, lines.join("\n") + "\n").unwrap();
        f
    }

    fn cl(kind: &str, content: &str, ts: &str, extra: &str) -> String {
        format!(
            r#"{{"type":"{kind}","message":{{"role":"{kind}","content":{content}}},"timestamp":"{ts}","cwd":"C:\\codes\\x","sessionId":"s1"{extra}}}"#
        )
    }

    fn write_codex(paths: &AgentPaths, rel: &str, lines: &[String], zst: bool) -> PathBuf {
        let f = paths.codex_home.join(rel);
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        let body = (lines.join("\n") + "\n").into_bytes();
        if zst {
            fs::write(&f, zstd::encode_all(&body[..], 3).unwrap()).unwrap();
        } else {
            fs::write(&f, body).unwrap();
        }
        f
    }

    fn meta() -> String {
        r#"{"timestamp":"2026-11-05T10:00:00.000Z","type":"session_meta","payload":{"id":"c1","session_id":"c1","cwd":"C:\\codes\\y"}}"#.into()
    }

    fn cx(role: &str, ty: &str, text: &str, ts: &str) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"response_item","payload":{{"type":"message","role":"{role}","content":[{{"type":"{ty}","text":"{text}"}}]}}}}"#
        )
    }

    #[test]
    fn parse_ts_handles_z_and_offset() {
        assert_eq!(parse_ts("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_ts("2026-11-05T10:00:00.250Z"), Some(1_793_872_800_250));
        assert_eq!(parse_ts("2026-11-05T18:00:00+08:00"), Some(1_793_872_800_000));
        assert_eq!(parse_ts("garbage"), None);
    }

    #[test]
    fn claude_text_and_array_content() {
        let (_d, paths) = setup();
        write_claude(
            &paths,
            "C--codes-x",
            "s1",
            &[
                cl("user", r#""plain question""#, "2026-11-05T10:00:00.000Z", ""),
                cl(
                    "assistant",
                    r#"[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"answer"},{"type":"tool_use","name":"x"}]"#,
                    "2026-11-05T10:00:01.000Z",
                    "",
                ),
                r#"{"type":"summary","summary":"x"}"#.into(),
            ],
        );
        let t = read_claude_turns(&paths, 0);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].role, Role::User);
        assert_eq!(t[0].text, "plain question");
        assert_eq!(t[0].tool, Tool::Claude);
        assert_eq!(t[0].session_id, "s1");
        assert_eq!(t[0].project_id.as_deref(), Some(normalize_path("C:\\codes\\x").as_str()));
        assert_eq!(t[0].ts, 1_793_872_800_000);
        assert_eq!(t[1].role, Role::Assistant);
        assert_eq!(t[1].text, "answer");
    }

    #[test]
    fn claude_skips_tool_and_meta_lines() {
        let (_d, paths) = setup();
        write_claude(
            &paths,
            "C--codes-x",
            "s1",
            &[
                cl("user", r#"[{"type":"tool_result","content":"out"}]"#, "2026-11-05T10:00:00.000Z", ""),
                cl("user", r#""meta text""#, "2026-11-05T10:00:01.000Z", r#","isMeta":true"#),
                cl("user", r#""<command-name>/clear</command-name>""#, "2026-11-05T10:00:02.000Z", ""),
                cl("user", r#""<local-command-stdout>x</local-command-stdout>""#, "2026-11-05T10:00:03.000Z", ""),
                cl("user", r#""<system-reminder>r</system-reminder>""#, "2026-11-05T10:00:04.000Z", ""),
                cl("assistant", r#"[{"type":"tool_use","name":"Bash"}]"#, "2026-11-05T10:00:05.000Z", ""),
                cl("user", r#""kept""#, "2026-11-05T10:00:06.000Z", ""),
            ],
        );
        let t = read_claude_turns(&paths, 0);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].text, "kept");
    }

    #[test]
    fn codex_zst_messages_user_assistant_only() {
        let (_d, paths) = setup();
        write_codex(
            &paths,
            "sessions/2026/11/05/rollout-a-c1.jsonl.zst",
            &[
                meta(),
                cx("developer", "input_text", "dev rules", "2026-11-05T10:00:01.000Z"),
                cx("user", "input_text", "hello codex", "2026-11-05T10:00:02.000Z"),
                cx("assistant", "output_text", "hi there", "2026-11-05T10:00:03.000Z"),
                r#"{"timestamp":"2026-11-05T10:00:04.000Z","type":"event_msg","payload":{"type":"agent_message"}}"#.into(),
                r#"{"timestamp":"2026-11-05T10:00:05.000Z","type":"response_item","payload":{"type":"function_call","name":"x"}}"#.into(),
            ],
            true,
        );
        write_codex(
            &paths,
            "archived_sessions/rollout-b-c2.jsonl",
            &[meta().replace("c1", "c2"), cx("user", "input_text", "archived q", "2026-11-05T11:00:00.000Z")],
            false,
        );
        let mut t = read_codex_turns(&paths, 0);
        t.sort_by_key(|x| x.ts);
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].text, "hello codex");
        assert_eq!(t[0].tool, Tool::Codex);
        assert_eq!(t[0].session_id, "c1");
        assert_eq!(t[0].project_id.as_deref(), Some(normalize_path("C:\\codes\\y").as_str()));
        assert_eq!(t[1].role, Role::Assistant);
        assert_eq!(t[2].session_id, "c2");
    }

    #[test]
    fn codex_skips_environment_context() {
        let (_d, paths) = setup();
        write_codex(
            &paths,
            "sessions/2026/11/05/rollout-a-c1.jsonl",
            &[
                meta(),
                cx("user", "input_text", "<environment_context>cwd</environment_context>", "2026-11-05T10:00:01.000Z"),
                cx("user", "input_text", "<user_instructions>x</user_instructions>", "2026-11-05T10:00:02.000Z"),
                cx("user", "input_text", "<permissions instructions>x", "2026-11-05T10:00:03.000Z"),
                cx("user", "input_text", "real", "2026-11-05T10:00:04.000Z"),
            ],
            false,
        );
        let t = read_codex_turns(&paths, 0);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].text, "real");
    }

    #[test]
    fn since_filter_by_ts() {
        let (_d, paths) = setup();
        write_claude(
            &paths,
            "C--codes-x",
            "s1",
            &[
                cl("user", r#""old""#, "2026-11-05T10:00:00.000Z", ""),
                cl("user", r#""new""#, "2026-11-05T12:00:00.000Z", ""),
            ],
        );
        let since = parse_ts("2026-11-05T11:00:00Z").unwrap();
        let t = read_claude_turns(&paths, since);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].text, "new");

        // A file whose mtime predates `since` is skipped entirely.
        let f = write_claude(&paths, "C--codes-z", "s9", &[cl("user", r#""stale""#, "2026-11-05T12:00:00.000Z", "")]);
        let old = SystemTime::now() - Duration::from_secs(10 * 86400);
        fs::File::options().write(true).open(&f).unwrap().set_modified(old).unwrap();
        let now = mtime_ms(&f).unwrap() + 1000;
        assert!(read_claude_turns(&paths, now).iter().all(|t| t.text != "stale"));
    }

    #[test]
    fn context_around_returns_neighbors() {
        let (_d, paths) = setup();
        let lines: Vec<String> = (0..10)
            .map(|i| cl("user", &format!("\"m{i}\""), &format!("2026-11-05T10:00:{:02}.000Z", i), ""))
            .collect();
        write_claude(&paths, "C--codes-x", "s1", &lines);
        let center = parse_ts("2026-11-05T10:00:05Z").unwrap();
        let c = context_around(&paths, Tool::Claude, "s1", center, 2, 1);
        let texts: Vec<&str> = c.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, vec!["m3", "m4", "m5", "m6"]);
        let c = context_around(&paths, Tool::Claude, "s1", center, 99, 99);
        assert_eq!(c.len(), 5 + 1 + 4); // only 5 before and 4 after exist
        assert!(context_around(&paths, Tool::Claude, "nope", center, 2, 2).is_empty());

        write_codex(
            &paths,
            "sessions/2026/11/05/rollout-a-c1.jsonl",
            &[meta(), cx("user", "input_text", "q", "2026-11-05T10:00:02.000Z"), cx("assistant", "output_text", "a", "2026-11-05T10:00:03.000Z")],
            false,
        );
        let c = context_around(&paths, Tool::Codex, "c1", parse_ts("2026-11-05T10:00:03Z").unwrap(), 1, 1);
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn oversized_line_skipped() {
        let (_d, paths) = setup();
        let big = "x".repeat(MAX_LINE_BYTES + 10);
        write_claude(
            &paths,
            "C--codes-x",
            "s1",
            &[
                cl("user", &format!("\"{big}\""), "2026-11-05T10:00:00.000Z", ""),
                cl("user", r#""after big""#, "2026-11-05T10:00:01.000Z", ""),
            ],
        );
        let t = read_claude_turns(&paths, 0);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].text, "after big");
    }

    #[test]
    fn claude_noise_filtered_per_block() {
        let (_d, paths) = setup();
        write_claude(
            &paths,
            "C--codes-x",
            "s1",
            &[
                cl(
                    "user",
                    r#"[{"type":"text","text":"<system-reminder>r1</system-reminder>"},{"type":"text","text":"real text"},{"type":"text","text":"<system-reminder>r2</system-reminder>"}]"#,
                    "2026-11-05T10:00:00.000Z",
                    "",
                ),
                cl("user", r#""summary of the past""#, "2026-11-05T10:00:01.000Z", r#","isCompactSummary":true"#),
                cl("user", r#""[Request interrupted by user for tool use]""#, "2026-11-05T10:00:02.000Z", ""),
                cl(
                    "user",
                    r#"[{"type":"text","text":"[Request interrupted by user]"},{"type":"text","text":"after interrupt"}]"#,
                    "2026-11-05T10:00:03.000Z",
                    "",
                ),
            ],
        );
        let t = read_claude_turns(&paths, 0);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].text, "real text");
        assert_eq!(t[1].text, "after interrupt");
    }

    #[test]
    fn codex_fallback_session_id_is_full_stem() {
        assert_eq!(codex_id_from_path(Path::new("rollout-2026-11-05T10-00-00-abc.jsonl.zst")), "2026-11-05T10-00-00-abc");
    }

    #[test]
    fn codex_skips_agents_md_and_derives_session_id_from_filename() {
        let (_d, paths) = setup();
        let uuid = "019a5b3c-1234-7abc-8def-0123456789ab";
        write_codex(
            &paths,
            &format!("sessions/2026/11/05/rollout-2026-11-05T10-00-00-{uuid}.jsonl"),
            &[
                cx("user", "input_text", "# AGENTS.md instructions for C:\\\\x", "2026-11-05T10:00:01.000Z"),
                cx("user", "input_text", "real q", "2026-11-05T10:00:02.000Z"),
            ],
            false,
        );
        let t = read_codex_turns(&paths, 0);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].text, "real q");
        assert_eq!(t[0].session_id, uuid);
    }

    #[test]
    fn turns_for_project_filters() {
        let (_d, paths) = setup();
        write_claude(&paths, "C--codes-x", "s1", &[cl("user", r#""a""#, "2026-11-05T10:00:00.000Z", "")]);
        let t = read_claude_turns(&paths, 0);
        let id = normalize_path("C:\\codes\\x");
        assert_eq!(turns_for_project(&t, &id).len(), 1);
        assert!(turns_for_project(&t, "other").is_empty());
    }
}
