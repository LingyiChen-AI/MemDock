//! Display timeline of a parsed session: the original conversation with each
//! tool call paired to its result and classified for the session viewer.

use serde::Serialize;
use serde_json::Value;

use super::{truncate_chars, Item};

/// Coarse tool category for icons and filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Shell,
    Read,
    Edit,
    Write,
    Search,
    Web,
    Agent,
    Todo,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineItem {
    User { ts: String, text: String },
    Assistant { ts: String, text: String },
    Tool {
        ts: String,
        name: String,
        kind: ToolKind,
        /// One line: the command, file path, pattern, URL...
        summary: String,
        /// Pretty-printed input, truncated.
        input: String,
        /// `None` when no result was recorded.
        output: Option<String>,
        is_error: bool,
        /// Converting keeps it as a real shell call (otherwise it becomes text).
        mapped: bool,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TimelineCounts {
    pub user: usize,
    pub assistant: usize,
    pub tools: usize,
    pub shell: usize,
    pub files: usize,
    pub errors: usize,
}

pub fn classify(name: &str) -> ToolKind {
    let n = name.to_ascii_lowercase();
    match n.as_str() {
        "bash" | "shell" | "shell_command" | "exec_command" | "local_shell_call" | "local_shell" | "powershell" | "exec" => ToolKind::Shell,
        "read" | "view" | "notebookread" | "read_file" => ToolKind::Read,
        "edit" | "multiedit" | "apply_patch" | "str_replace" | "str_replace_editor" | "notebookedit" => ToolKind::Edit,
        "write" | "create_file" | "write_file" => ToolKind::Write,
        "grep" | "glob" | "ls" | "search" | "find" | "list_dir" => ToolKind::Search,
        "webfetch" | "websearch" | "web_search" | "fetch" | "browse" => ToolKind::Web,
        "task" | "agent" | "spawn_agent" => ToolKind::Agent,
        "todowrite" | "update_plan" | "todo" => ToolKind::Todo,
        _ if n.starts_with("mcp__") => ToolKind::Other,
        _ => ToolKind::Other,
    }
}

/// The most telling single line of a tool input.
pub fn summarize_input(input: &Value, shell: Option<&str>) -> String {
    if let Some(cmd) = shell {
        return one_line(cmd);
    }
    let obj = match input {
        Value::Object(o) => o,
        Value::String(s) => return one_line(s),
        _ => return String::new(),
    };
    for key in ["command", "cmd", "file_path", "path", "notebook_path", "pattern", "url", "query", "description", "prompt"] {
        match obj.get(key) {
            Some(Value::String(s)) if !s.is_empty() => return one_line(s),
            Some(Value::Array(a)) if !a.is_empty() => {
                let parts: Vec<String> = a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
                if !parts.is_empty() {
                    return one_line(&parts.join(" "));
                }
            }
            _ => {}
        }
    }
    obj.values().find_map(|v| v.as_str().map(one_line)).unwrap_or_default()
}

fn one_line(s: &str) -> String {
    let first = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    truncate_chars(first, 200)
}

fn pretty(input: &Value, max: usize) -> String {
    let s = match input {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => serde_json::to_string_pretty(v).unwrap_or_default(),
    };
    truncate_chars(&s, max)
}

/// Builds the timeline (calls paired with results by id) and its counts.
pub fn build(items: &[Item], text_max: usize, io_max: usize) -> (Vec<TimelineItem>, TimelineCounts) {
    let mut out: Vec<TimelineItem> = Vec::new();
    let mut counts = TimelineCounts::default();
    // call id -> index in `out`
    let mut open: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for it in items {
        match it {
            Item::User { ts, text } => {
                if text.trim().is_empty() {
                    continue;
                }
                counts.user += 1;
                out.push(TimelineItem::User { ts: ts.clone(), text: truncate_chars(text.trim(), text_max) });
            }
            Item::Assistant { ts, text } => {
                if text.trim().is_empty() {
                    continue;
                }
                counts.assistant += 1;
                out.push(TimelineItem::Assistant { ts: ts.clone(), text: truncate_chars(text.trim(), text_max) });
            }
            Item::Call { ts, id, name, input, shell } => {
                let kind = if shell.is_some() { ToolKind::Shell } else { classify(name) };
                counts.tools += 1;
                match kind {
                    ToolKind::Shell => counts.shell += 1,
                    ToolKind::Read | ToolKind::Edit | ToolKind::Write => counts.files += 1,
                    _ => {}
                }
                open.insert(id.clone(), out.len());
                out.push(TimelineItem::Tool {
                    ts: ts.clone(),
                    name: name.clone(),
                    kind,
                    summary: summarize_input(input, shell.as_deref()),
                    input: shell.clone().map(|s| truncate_chars(&s, io_max)).unwrap_or_else(|| pretty(input, io_max)),
                    output: None,
                    is_error: false,
                    mapped: false,
                });
            }
            Item::Result { id, output, is_error, .. } => {
                if let Some(&i) = open.get(id) {
                    if let TimelineItem::Tool { output: o, is_error: e, kind, mapped, .. } = &mut out[i] {
                        *o = Some(truncate_chars(output, io_max));
                        *e = *is_error;
                        // Only shell calls with a result survive conversion as calls.
                        *mapped = *kind == ToolKind::Shell;
                        if *is_error {
                            counts.errors += 1;
                        }
                    }
                    open.remove(id);
                }
            }
        }
    }
    (out, counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pairs_calls_with_results_and_counts_kinds() {
        let items = vec![
            Item::User { ts: "1".into(), text: "hi".into() },
            Item::Call { ts: "2".into(), id: "a".into(), name: "Bash".into(), input: json!({"command": "ls -la\nmore"}), shell: Some("ls -la".into()) },
            Item::Call { ts: "3".into(), id: "b".into(), name: "Read".into(), input: json!({"file_path": "C:/x/y.rs"}), shell: None },
            Item::Result { ts: "4".into(), id: "a".into(), output: "ok".into(), is_error: false },
            Item::Result { ts: "5".into(), id: "b".into(), output: "boom".into(), is_error: true },
            Item::Assistant { ts: "6".into(), text: "done".into() },
            Item::Call { ts: "7".into(), id: "c".into(), name: "mcp__x__y".into(), input: json!({"q": "z"}), shell: None },
        ];
        let (tl, c) = build(&items, 100, 100);
        assert_eq!(tl.len(), 5);
        assert_eq!(c, TimelineCounts { user: 1, assistant: 1, tools: 3, shell: 1, files: 1, errors: 1 });
        match &tl[1] {
            TimelineItem::Tool { kind, summary, output, mapped, .. } => {
                assert_eq!(*kind, ToolKind::Shell);
                assert_eq!(summary, "ls -la");
                assert_eq!(output.as_deref(), Some("ok"));
                assert!(*mapped);
            }
            other => panic!("{other:?}"),
        }
        match &tl[2] {
            TimelineItem::Tool { kind, summary, is_error, mapped, .. } => {
                assert_eq!(*kind, ToolKind::Read);
                assert_eq!(summary, "C:/x/y.rs");
                assert!(*is_error);
                assert!(!*mapped);
            }
            other => panic!("{other:?}"),
        }
        match &tl[4] {
            TimelineItem::Tool { kind, output, summary, .. } => {
                assert_eq!(*kind, ToolKind::Other);
                assert!(output.is_none());
                assert_eq!(summary, "z");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn skips_blank_messages() {
        let items = vec![
            Item::User { ts: "1".into(), text: "  

 ".into() },
            Item::Assistant { ts: "2".into(), text: "
".into() },
            Item::User { ts: "3".into(), text: "
 hi 
".into() },
        ];
        let (tl, c) = build(&items, 100, 100);
        assert_eq!(tl, vec![TimelineItem::User { ts: "3".into(), text: "hi".into() }]);
        assert_eq!(c.user, 1);
        assert_eq!(c.assistant, 0);
    }

    #[test]
    fn classifies_codex_and_claude_names() {
        assert_eq!(classify("apply_patch"), ToolKind::Edit);
        assert_eq!(classify("exec_command"), ToolKind::Shell);
        assert_eq!(classify("WebFetch"), ToolKind::Web);
        assert_eq!(classify("Task"), ToolKind::Agent);
        assert_eq!(classify("TodoWrite"), ToolKind::Todo);
        assert_eq!(classify("Glob"), ToolKind::Search);
    }
}
