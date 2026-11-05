//! Prompt building (with secret redaction), the output JSON schema and
//! parsing/validation of the model output into memory-update suggestions.

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::sessions::{Role, Turn};
use crate::model::{MemoryKind, Tool};

/// Maximum characters of conversation excerpt in one prompt (newest kept).
pub const MAX_EXCERPT_CHARS: usize = 60_000;
/// Maximum characters of a single turn inside the excerpt.
const MAX_TURN_CHARS: usize = 6_000;
/// Maximum suggestions accepted from one call.
pub const MAX_SUGGESTIONS: usize = 20;
/// Maximum characters kept of an evidence quote.
const MAX_QUOTE_CHARS: usize = 500;

// ---------------------------------------------------------------- redaction

struct RedactRules {
    private_key: Regex,
    private_key_open: Regex,
    password: Regex,
    tokens: Vec<Regex>,
}

fn rules() -> &'static RedactRules {
    static R: OnceLock<RedactRules> = OnceLock::new();
    R.get_or_init(|| RedactRules {
        private_key: Regex::new(
            r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----",
        )
        .unwrap(),
        private_key_open: Regex::new(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*").unwrap(),
        password: Regex::new(r"(?i)((?:password|passwd|pwd)\s*[:=]\s*)\S+").unwrap(),
        tokens: [
            r"\bsk-[A-Za-z0-9_\-]{16,}",
            r"\bgh[pousr]_[A-Za-z0-9]{20,}",
            r"\bgithub_pat_[A-Za-z0-9_]{20,}",
            r"\bxox[bpas]-[A-Za-z0-9\-]{10,}",
            r"\bAKIA[0-9A-Z]{16}\b",
            r"(?i)\bbearer\s+[A-Za-z0-9._\-]{20,}",
        ]
        .iter()
        .map(|p| Regex::new(p).unwrap())
        .collect(),
    })
}

/// Replaces API keys, tokens, private keys and `password=...` values by
/// `[REDACTED]`. Token-like matches keep their first 4 characters.
pub fn redact(text: &str) -> String {
    let r = rules();
    let mut out = r.private_key.replace_all(text, "[REDACTED]").into_owned();
    out = r.private_key_open.replace_all(&out, "[REDACTED]").into_owned();
    out = r.password.replace_all(&out, "${1}[REDACTED]").into_owned();
    for re in &r.tokens {
        out = re
            .replace_all(&out, |c: &regex::Captures| {
                let head: String = c[0].chars().take(4).collect();
                format!("{head}…[REDACTED]")
            })
            .into_owned();
    }
    out
}

// ------------------------------------------------------------------ prompt

/// A memory file the model may propose changes for.
#[derive(Clone, Debug)]
pub struct MemoryTarget {
    pub file_id: String,
    pub path: String,
    pub tool: Tool,
    pub kind: MemoryKind,
    pub content: String,
    /// Hash of the file as read (`memory::io::read_memory`), stored as `base_hash`.
    pub hash: String,
}

fn tool_name(t: Tool) -> &'static str {
    match t {
        Tool::Claude => "claude",
        Tool::Codex => "codex",
    }
}

fn role_name(r: Role) -> &'static str {
    match r {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max).collect();
    t.push_str("…[truncated]");
    t
}

/// Formats the excerpt: chronological, newest turns kept within the cap.
fn excerpt(turns: &[&Turn]) -> String {
    let mut sorted: Vec<&Turn> = turns.to_vec();
    sorted.sort_by_key(|t| t.ts);
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0usize;
    for t in sorted.iter().rev() {
        let line = format!(
            "[{} {} {} {}] {}",
            tool_name(t.tool),
            t.session_id,
            t.ts,
            role_name(t.role),
            redact(&truncate_chars(&t.text, MAX_TURN_CHARS))
        );
        let n = line.chars().count() + 1;
        if used + n > MAX_EXCERPT_CHARS {
            break;
        }
        used += n;
        lines.push(line);
    }
    lines.reverse();
    lines.join("\n")
}

fn instructions(project_name: &str, lang: &str) -> String {
    if lang == "en" {
        format!(
            "You maintain the persistent memory files (CLAUDE.md, AGENTS.md, auto-memory notes) of the project \"{project_name}\". \
Below are the current memory files (each with a file_id) and an excerpt of recent coding-agent conversations.\n\
\n\
Task: propose updates to the memory files.\n\
- Only extract durable knowledge: long-lived user preferences, conventions, project facts, and corrections of earlier mistakes or outdated statements.\n\
- Ignore one-off task details, temporary debugging state, and anything specific to a single task.\n\
- Never invent anything. Every suggestion must be backed by evidence quoted from the conversation (session_id and ts exactly as shown in the excerpt headers).\n\
- kind \"add\": a new entry; give new_text.\n\
- kind \"modify\": change existing text; old_text must quote the target file VERBATIM and appear exactly once there; give new_text.\n\
- kind \"obsolete\": remove outdated text; old_text must quote the target file VERBATIM and appear exactly once; omit new_text to delete it.\n\
- Do not suggest what the files already say. Never include secrets.\n\
- Return at most {MAX_SUGGESTIONS} suggestions. If there is nothing worth recording, return {{\"suggestions\":[]}}.\n\
- Output strict JSON only, matching the provided schema, with no extra text. You must not modify any file."
        )
    } else {
        format!(
            "你负责维护项目「{project_name}」的长期记忆文件（CLAUDE.md、AGENTS.md、自动记忆笔记）。\
下面是当前的记忆文件（每个带 file_id）和最近的编码助手对话摘录。\n\
\n\
任务：为记忆文件提出更新建议。\n\
- 只提炼长期有效的内容：用户的偏好、约定、项目事实，以及对先前错误或过时说法的纠正。\n\
- 忽略一次性任务细节、临时调试状态和只针对单次任务的内容。\n\
- 绝不编造。每条建议都必须有对话中的证据（session_id 和 ts 与摘录头部完全一致，quote 为原文片段）。\n\
- kind 为 \"add\"：新增条目，给出 new_text。\n\
- kind 为 \"modify\"：修改现有文字；old_text 必须逐字引用目标文件原文且在其中恰好出现一次；给出 new_text。\n\
- kind 为 \"obsolete\"：删除过时内容；old_text 必须逐字引用目标文件原文且恰好出现一次；省略 new_text 表示删除该段。\n\
- 文件里已有的内容不要重复建议；不要包含任何密钥或口令。\n\
- 每次最多返回 {MAX_SUGGESTIONS} 条建议。没有值得记录的内容时返回 {{\"suggestions\":[]}}。\n\
- 只输出严格的 JSON，必须符合给定的 schema，不要输出其他文字。不允许修改任何文件。"
        )
    }
}

/// Builds the full prompt: instructions, target files with ids, redacted excerpt.
pub fn build_prompt(project_name: &str, targets: &[MemoryTarget], turns: &[&Turn], lang: &str) -> String {
    let en = lang == "en";
    let mut p = instructions(project_name, lang);
    p.push_str(if en { "\n\n## Memory files\n" } else { "\n\n## 记忆文件\n" });
    if targets.is_empty() {
        p.push_str(if en { "(none)\n" } else { "（无）\n" });
    }
    for t in targets {
        p.push_str(&format!(
            "\n<memory_file file_id=\"{}\" path=\"{}\" tool=\"{}\">\n{}\n</memory_file>\n",
            t.file_id,
            t.path,
            tool_name(t.tool),
            redact(&t.content)
        ));
    }
    p.push_str(if en {
        "\n## Conversation excerpt (oldest to newest; header: [tool session_id ts role])\n"
    } else {
        "\n## 对话摘录（按时间从旧到新；头部格式：[tool session_id ts role]）\n"
    });
    p.push_str(&excerpt(turns));
    p.push('\n');
    p
}

/// JSON schema of the expected model output.
pub fn output_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["suggestions"],
        "properties": {
            "suggestions": {
                "type": "array",
                "maxItems": MAX_SUGGESTIONS,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["kind", "file_id", "reason", "evidence"],
                    "properties": {
                        "kind": { "type": "string", "enum": ["add", "modify", "obsolete"] },
                        "file_id": { "type": "string" },
                        "old_text": { "type": "string" },
                        "new_text": { "type": "string" },
                        "reason": { "type": "string" },
                        "evidence": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "required": ["session_id", "ts", "quote"],
                                "properties": {
                                    "session_id": { "type": "string" },
                                    "ts": { "type": "integer" },
                                    "quote": { "type": "string" }
                                }
                            }
                        }
                    }
                }
            }
        }
    })
}

// ------------------------------------------------------------- suggestions

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionKind {
    Add,
    Modify,
    Obsolete,
}

impl SuggestionKind {
    fn as_str(self) -> &'static str {
        match self {
            SuggestionKind::Add => "add",
            SuggestionKind::Modify => "modify",
            SuggestionKind::Obsolete => "obsolete",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionStatus {
    Pending,
    Accepted,
    Rejected,
    Stale,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Evidence {
    pub tool: Tool,
    pub session_id: String,
    pub ts: i64,
    pub quote: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Suggestion {
    pub id: String,
    pub fingerprint: String,
    pub kind: SuggestionKind,
    pub file_id: String,
    pub path: String,
    pub tool_of_file: Tool,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
    pub reason: String,
    pub evidence: Vec<Evidence>,
    /// The CLI that ran the extraction.
    pub source_tool: Tool,
    pub created_at: i64,
    /// Hash of the target file when the suggestion was generated.
    pub base_hash: String,
    pub status: SuggestionStatus,
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `sha256(file_id + kind + normalize_ws(old_text) + normalize_ws(new_text))`, hex.
pub fn fingerprint(file_id: &str, kind: SuggestionKind, old_text: Option<&str>, new_text: Option<&str>) -> String {
    let s = format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        file_id,
        kind.as_str(),
        normalize_ws(old_text.unwrap_or("")),
        normalize_ws(new_text.unwrap_or(""))
    );
    Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(0))
        .unwrap_or(0)
}

/// Finds the JSON value in `raw`: plain, fenced, or the outermost braces.
fn parse_json_loose(raw: &str) -> Option<Value> {
    let s = raw.trim().trim_start_matches('\u{feff}');
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        return Some(v);
    }
    if let Some(start) = s.find("```") {
        let rest = &s[start + 3..];
        let rest = rest.split_once('\n').map(|x| x.1).unwrap_or(rest);
        if let Some(end) = rest.find("```") {
            if let Ok(v) = serde_json::from_str::<Value>(rest[..end].trim()) {
                return Some(v);
            }
        }
    }
    let (a, b) = (s.find('{')?, s.rfind('}')?);
    if a < b {
        return serde_json::from_str::<Value>(&s[a..=b]).ok();
    }
    None
}

/// Unwraps the CLI wrapper layers down to the object holding `suggestions`.
fn unwrap_output(raw: &str, warnings: &mut Vec<String>) -> Option<Value> {
    let mut cur = parse_json_loose(raw)?;
    for _ in 0..4 {
        if cur.get("suggestions").is_some() {
            return Some(cur);
        }
        if cur.get("is_error").and_then(Value::as_bool) == Some(true) {
            warnings.push("CLI reported an error result".into());
            return None;
        }
        if let Some(so) = cur.get("structured_output").filter(|v| v.is_object()) {
            cur = so.clone();
            continue;
        }
        if let Some(r) = cur.get("result").and_then(Value::as_str) {
            cur = parse_json_loose(r)?;
            continue;
        }
        return None;
    }
    None
}

fn str_field(item: &Value, key: &str) -> Option<String> {
    item.get(key).and_then(Value::as_str).map(String::from)
}

/// Parses and validates model output. Invalid suggestions are dropped with a warning.
pub fn parse_output(
    raw: &str,
    targets: &[MemoryTarget],
    source_tool: Tool,
    turns: &[&Turn],
) -> (Vec<Suggestion>, Vec<String>) {
    let mut warnings: Vec<String> = Vec::new();
    let mut out: Vec<Suggestion> = Vec::new();
    let Some(root) = unwrap_output(raw, &mut warnings) else {
        warnings.push("output is not valid JSON with a `suggestions` array".into());
        return (out, warnings);
    };
    let Some(items) = root.get("suggestions").and_then(Value::as_array) else {
        warnings.push("`suggestions` is not an array".into());
        return (out, warnings);
    };
    if items.len() > MAX_SUGGESTIONS {
        warnings.push(format!("{} suggestions returned, keeping the first {MAX_SUGGESTIONS}", items.len()));
    }
    let known_sessions: HashSet<&str> = turns.iter().map(|t| t.session_id.as_str()).collect();
    let created_at = now_ms();
    let mut seen: HashSet<String> = HashSet::new();

    for (i, item) in items.iter().take(MAX_SUGGESTIONS).enumerate() {
        let n = i + 1;
        let kind = match item.get("kind").and_then(Value::as_str) {
            Some("add") => SuggestionKind::Add,
            Some("modify") => SuggestionKind::Modify,
            Some("obsolete") => SuggestionKind::Obsolete,
            _ => {
                warnings.push(format!("#{n}: unknown kind"));
                continue;
            }
        };
        let file_id = str_field(item, "file_id").unwrap_or_default();
        let Some(target) = targets.iter().find(|t| t.file_id == file_id) else {
            warnings.push(format!("#{n}: unknown file_id `{file_id}`"));
            continue;
        };
        if target.kind == MemoryKind::CodexDb {
            warnings.push(format!("#{n}: the Codex memory database is read-only"));
            continue;
        }
        let old_text = str_field(item, "old_text").filter(|s| !s.trim().is_empty());
        let new_text = str_field(item, "new_text").filter(|s| !s.trim().is_empty());
        let (old_text, new_text) = match kind {
            SuggestionKind::Add => {
                if new_text.is_none() {
                    warnings.push(format!("#{n}: add without new_text"));
                    continue;
                }
                (None, new_text)
            }
            SuggestionKind::Modify | SuggestionKind::Obsolete => {
                let Some(old) = old_text else {
                    warnings.push(format!("#{n}: {} without old_text", kind.as_str()));
                    continue;
                };
                let count = target.content.matches(old.as_str()).count();
                if count != 1 {
                    warnings.push(format!("#{n}: old_text found {count} times in the target (need exactly 1)"));
                    continue;
                }
                if kind == SuggestionKind::Modify && new_text.is_none() {
                    warnings.push(format!("#{n}: modify without new_text"));
                    continue;
                }
                (Some(old), new_text)
            }
        };
        let reason = str_field(item, "reason").map(|s| s.trim().to_string()).unwrap_or_default();
        if reason.is_empty() {
            warnings.push(format!("#{n}: missing reason"));
            continue;
        }
        let mut evidence: Vec<Evidence> = Vec::new();
        for e in item.get("evidence").and_then(Value::as_array).into_iter().flatten() {
            let sid = str_field(e, "session_id").unwrap_or_default();
            let Some(turn) = turns.iter().find(|t| t.session_id == sid && known_sessions.contains(sid.as_str()))
            else {
                warnings.push(format!("#{n}: dropped evidence with unknown session `{sid}`"));
                continue;
            };
            let ts = e.get("ts").and_then(Value::as_i64).unwrap_or(turn.ts);
            let quote = truncate_chars(str_field(e, "quote").unwrap_or_default().trim(), MAX_QUOTE_CHARS);
            evidence.push(Evidence { tool: turn.tool, session_id: sid, ts, quote });
        }
        if evidence.is_empty() {
            warnings.push(format!("#{n}: no valid evidence"));
            continue;
        }
        let fp = fingerprint(&file_id, kind, old_text.as_deref(), new_text.as_deref());
        if !seen.insert(fp.clone()) {
            warnings.push(format!("#{n}: duplicate suggestion"));
            continue;
        }
        out.push(Suggestion {
            id: uuid::Uuid::new_v4().to_string(),
            fingerprint: fp,
            kind,
            file_id,
            path: target.path.clone(),
            tool_of_file: target.tool,
            old_text,
            new_text,
            reason,
            evidence,
            source_tool,
            created_at,
            base_hash: target.hash.clone(),
            status: SuggestionStatus::Pending,
        });
    }
    (out, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(sid: &str, ts: i64, role: Role, text: &str) -> Turn {
        Turn { tool: Tool::Claude, session_id: sid.into(), project_id: None, ts, role, text: text.into() }
    }

    fn target(id: &str, kind: MemoryKind, content: &str) -> MemoryTarget {
        MemoryTarget {
            file_id: id.into(),
            path: format!("/p/{id}.md"),
            tool: Tool::Claude,
            kind,
            content: content.into(),
            hash: "h1".into(),
        }
    }

    fn fixture() -> (Vec<MemoryTarget>, Vec<Turn>) {
        (
            vec![target("f1", MemoryKind::ClaudeInstructions, "# Rules\nUse tabs.\nUse tabs and more.\nPrefer Rust.\n")],
            vec![turn("s1", 100, Role::User, "always run tests"), turn("s1", 200, Role::Assistant, "ok")],
        )
    }

    #[test]
    fn prompt_redacts_secrets() {
        let key = "sk-ant-api03-abcdefghijklmnop1234";
        let t = turn(
            "s1",
            1,
            Role::User,
            &format!(
                "key {key} and ghp_abcdefghijklmnopqrstuvwxyz0123 and AKIAABCDEFGHIJKLMNOP password: hunter2 \
                 xoxb-1234567890-abcdef -----BEGIN RSA PRIVATE KEY-----\nMIIBVQ\n-----END RSA PRIVATE KEY----- end"
            ),
        );
        let tg = target("f1", MemoryKind::ClaudeInstructions, "token github_pat_abcdefghijklmnopqrstuv1234 here");
        let p = build_prompt("proj", &[tg], &[&t], "en");
        for s in [
            "abcdefghijklmnop1234",
            "ghp_abcdefghijklmnopqrstuvwxyz0123",
            "AKIAABCDEFGHIJKLMNOP",
            "hunter2",
            "1234567890-abcdef",
            "MIIBVQ",
            "github_pat_abcdefghijklmnopqrstuv1234",
        ] {
            assert!(!p.contains(s), "leaked {s}");
        }
        assert!(p.contains("sk-a…[REDACTED]"));
        assert!(p.contains("password: [REDACTED]"));
        assert!(p.contains(" end"));
    }

    #[test]
    fn prompt_truncates_oldest_first() {
        let turns: Vec<Turn> = (0..30)
            .map(|i| turn("s1", i, Role::User, &format!("MARK{i:02} {}", "x".repeat(3000))))
            .collect();
        let refs: Vec<&Turn> = turns.iter().collect();
        let p = build_prompt("proj", &[], &refs, "zh");
        assert!(p.contains("MARK29"));
        assert!(!p.contains("MARK00"));
        let ex = p.split("[claude").skip(1).count();
        assert!(ex < 30 && ex >= 15, "kept {ex}");
        // chronological order
        assert!(p.find("MARK28").unwrap() < p.find("MARK29").unwrap());
        assert!(p.chars().count() < MAX_EXCERPT_CHARS + 5_000);
    }

    #[test]
    fn prompt_includes_targets_with_ids() {
        let (targets, turns) = fixture();
        let refs: Vec<&Turn> = turns.iter().collect();
        for (lang, needle) in [("zh", "只提炼长期有效"), ("en", "durable")] {
            let p = build_prompt("MyProj", &targets, &refs, lang);
            assert!(p.contains("file_id=\"f1\""));
            assert!(p.contains("Use tabs and more."));
            assert!(p.contains("MyProj"));
            assert!(p.contains(needle));
            assert!(p.contains("20"));
            assert!(p.contains("{\"suggestions\":[]}"));
            assert!(p.contains("[claude s1 100 user] always run tests"));
        }
    }

    const BODY: &str = r#"{"suggestions":[{"kind":"modify","file_id":"f1","old_text":"Prefer Rust.","new_text":"Prefer Rust 2024.","reason":"user said so","evidence":[{"session_id":"s1","ts":100,"quote":"always"}]}]}"#;

    #[test]
    fn parse_accepts_fenced_and_wrapped_json() {
        let (targets, turns) = fixture();
        let refs: Vec<&Turn> = turns.iter().collect();
        let fenced = format!("Here you go:\n```json\n{BODY}\n```\n");
        let wrapped_result = json!({"type":"result","is_error":false,"result": BODY}).to_string();
        let wrapped_fenced = json!({"type":"result","result": format!("```json\n{BODY}\n```")}).to_string();
        let wrapped_struct = json!({
            "type":"result","result":"done",
            "structured_output": serde_json::from_str::<Value>(BODY).unwrap()
        })
        .to_string();
        for raw in [BODY.to_string(), fenced, wrapped_result, wrapped_fenced, wrapped_struct] {
            let (s, w) = parse_output(&raw, &targets, Tool::Codex, &refs);
            assert_eq!(s.len(), 1, "{raw} -> {w:?}");
            let s = &s[0];
            assert_eq!(s.kind, SuggestionKind::Modify);
            assert_eq!(s.path, "/p/f1.md");
            assert_eq!(s.source_tool, Tool::Codex);
            assert_eq!(s.base_hash, "h1");
            assert_eq!(s.status, SuggestionStatus::Pending);
            assert_eq!(s.id.len(), 36);
            assert_eq!(s.evidence[0].tool, Tool::Claude);
        }
        let (s, w) = parse_output(r#"{"suggestions":[]}"#, &targets, Tool::Claude, &refs);
        assert!(s.is_empty() && w.is_empty());
        let (s, w) = parse_output("not json", &targets, Tool::Claude, &refs);
        assert!(s.is_empty() && !w.is_empty());
    }

    fn one(kind: &str, file_id: &str, old: Option<&str>, new: Option<&str>) -> String {
        let mut o = json!({"kind":kind,"file_id":file_id,"reason":"r",
            "evidence":[{"session_id":"s1","ts":100,"quote":"q"}]});
        if let Some(x) = old {
            o["old_text"] = json!(x);
        }
        if let Some(x) = new {
            o["new_text"] = json!(x);
        }
        json!({"suggestions":[o]}).to_string()
    }

    #[test]
    fn parse_rejects_invalid_targets() {
        let (mut targets, turns) = fixture();
        targets.push(target("db", MemoryKind::CodexDb, "Use tabs."));
        let refs: Vec<&Turn> = turns.iter().collect();
        let cases = [
            one("add", "nope", None, Some("x")),
            one("modify", "f1", Some("absent text"), Some("x")),
            one("modify", "f1", Some("Use tabs"), Some("x")), // appears twice
            one("obsolete", "f1", Some("Use tabs"), None),
            one("modify", "db", Some("Use tabs."), Some("x")),
            one("add", "db", None, Some("x")),
            one("add", "f1", None, Some("   ")),
            one("modify", "f1", None, Some("x")),
            one("modify", "f1", Some("Prefer Rust."), None),
        ];
        for raw in cases {
            let (s, w) = parse_output(&raw, &targets, Tool::Claude, &refs);
            assert!(s.is_empty(), "{raw}");
            assert_eq!(w.len(), 1, "{raw} -> {w:?}");
        }
        // valid ones
        let ok = [
            one("add", "f1", None, Some("New rule")),
            one("obsolete", "f1", Some("Prefer Rust."), None),
            one("obsolete", "f1", Some("# Rules"), Some("# Rules v2")),
        ];
        for raw in ok {
            let (s, w) = parse_output(&raw, &targets, Tool::Claude, &refs);
            assert_eq!(s.len(), 1, "{raw} -> {w:?}");
        }
    }

    #[test]
    fn parse_drops_unknown_evidence() {
        let (targets, turns) = fixture();
        let refs: Vec<&Turn> = turns.iter().collect();
        let raw = json!({"suggestions":[
            {"kind":"add","file_id":"f1","new_text":"A","reason":"r","evidence":[
                {"session_id":"ghost","ts":1,"quote":"q"},{"session_id":"s1","ts":200,"quote":"real"}]},
            {"kind":"add","file_id":"f1","new_text":"B","reason":"r","evidence":[
                {"session_id":"ghost","ts":1,"quote":"q"}]},
            {"kind":"add","file_id":"f1","new_text":"C","reason":"r","evidence":[]}
        ]})
        .to_string();
        let (s, w) = parse_output(&raw, &targets, Tool::Claude, &refs);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].new_text.as_deref(), Some("A"));
        assert_eq!(s[0].evidence.len(), 1);
        assert_eq!(s[0].evidence[0].session_id, "s1");
        assert_eq!(s[0].evidence[0].ts, 200);
        assert_eq!(w.len(), 4);
    }

    #[test]
    fn caps_at_twenty_suggestions() {
        let (targets, turns) = fixture();
        let refs: Vec<&Turn> = turns.iter().collect();
        let items: Vec<Value> = (0..25)
            .map(|i| json!({"kind":"add","file_id":"f1","new_text":format!("rule {i}"),"reason":"r",
                "evidence":[{"session_id":"s1","ts":100,"quote":"q"}]}))
            .collect();
        let (s, w) = parse_output(&json!({"suggestions":items}).to_string(), &targets, Tool::Claude, &refs);
        assert_eq!(s.len(), 20);
        assert!(!w.is_empty());
    }

    #[test]
    fn fingerprint_ignores_whitespace() {
        let a = fingerprint("f", SuggestionKind::Modify, Some("a  b\n c"), Some("x y"));
        let b = fingerprint("f", SuggestionKind::Modify, Some(" a b c "), Some("x\ty"));
        assert_eq!(a, b);
        assert_ne!(a, fingerprint("f", SuggestionKind::Obsolete, Some("a b c"), Some("x y")));
        assert_ne!(a, fingerprint("g", SuggestionKind::Modify, Some("a b c"), Some("x y")));
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn schema_shape_and_enums_snake_case() {
        let s = output_schema();
        assert_eq!(s["properties"]["suggestions"]["items"]["properties"]["kind"]["enum"][2], "obsolete");
        assert_eq!(serde_json::to_value(SuggestionStatus::Stale).unwrap(), "stale");
        assert_eq!(serde_json::to_value(SuggestionKind::Add).unwrap(), "add");
    }
}
