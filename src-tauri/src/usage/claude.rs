//! Claude Code session logs: `~/.claude/projects/**/*.jsonl`.
//!
//! Every assistant API response is logged as one line per content block, all
//! carrying the same `message.id` / `requestId` and usage; the last line of a
//! message wins within a file, and the key deduplicates across files.
use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{parse_ts, u, UsageEvent};
use crate::model::Tool;

/// Parses one session log.
pub fn parse_reader(reader: impl BufRead) -> Vec<UsageEvent> {
    let mut order: Vec<String> = Vec::new();
    let mut by_key: HashMap<String, UsageEvent> = HashMap::new();
    let mut anon: Vec<UsageEvent> = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        // Cheap pre-filter: most lines are user turns / tool results.
        if !line.contains("\"usage\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(ev) = parse_line(&v) else { continue };
        match ev.key.clone() {
            Some(k) => {
                if !by_key.contains_key(&k) {
                    order.push(k.clone());
                }
                by_key.insert(k, ev);
            }
            None => anon.push(ev),
        }
    }
    let mut out: Vec<UsageEvent> = order.into_iter().filter_map(|k| by_key.remove(&k)).collect();
    out.extend(anon);
    out
}

/// One `type == "assistant"` line with `message.usage`; synthetic messages skipped.
pub fn parse_line(v: &Value) -> Option<UsageEvent> {
    if v.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let msg = v.get("message")?;
    let usage = msg.get("usage")?;
    let model = msg.get("model").and_then(|m| m.as_str()).unwrap_or("unknown");
    if model == "<synthetic>" {
        return None;
    }
    let ts = v.get("timestamp").and_then(|t| t.as_str()).and_then(parse_ts)?;
    let cache_write = u(usage, "cache_creation_input_tokens");
    let cache_write_1h = usage
        .get("cache_creation")
        .map(|c| u(c, "ephemeral_1h_input_tokens"))
        .unwrap_or(0)
        .min(cache_write);
    let ev = UsageEvent {
        ts,
        tool: Tool::Claude,
        model: model.to_string(),
        input: u(usage, "input_tokens"),
        output: u(usage, "output_tokens"),
        cache_read: u(usage, "cache_read_input_tokens"),
        cache_write,
        cache_write_1h,
        key: None,
    };
    let id = msg.get("id").and_then(|x| x.as_str());
    let req = v.get("requestId").and_then(|x| x.as_str());
    let key = match (id, req) {
        (Some(i), Some(r)) => Some(format!("c:{i}:{r}")),
        (Some(i), None) => Some(format!("c:{i}")),
        _ => None,
    };
    Some(UsageEvent { key, ..ev })
}

/// All session log files under `claude_home/projects` (subagent logs included).
pub fn list_files(claude_home: &Path) -> Vec<PathBuf> {
    let root = claude_home.join("projects");
    walkdir::WalkDir::new(root)
        .max_depth(6)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"type":"user","message":{"role":"user","content":"hi"},"timestamp":"2026-11-05T10:00:00.000Z"}
{"type":"assistant","requestId":"req_1","timestamp":"2026-11-05T10:00:01.000Z","message":{"model":"claude-opus-5-5","id":"msg_1","usage":{"input_tokens":2,"cache_creation_input_tokens":100,"cache_read_input_tokens":500,"output_tokens":10,"cache_creation":{"ephemeral_1h_input_tokens":100,"ephemeral_5m_input_tokens":0}}}}
{"type":"assistant","requestId":"req_1","timestamp":"2026-11-05T10:00:02.000Z","message":{"model":"claude-opus-5-5","id":"msg_1","usage":{"input_tokens":2,"cache_creation_input_tokens":100,"cache_read_input_tokens":500,"output_tokens":40,"cache_creation":{"ephemeral_1h_input_tokens":100,"ephemeral_5m_input_tokens":0}}}}
{"type":"assistant","timestamp":"2026-11-05T10:00:03.000Z","message":{"model":"<synthetic>","id":"msg_x","usage":{"input_tokens":0,"output_tokens":0}}}
not json with "usage"
{"type":"assistant","requestId":"req_2","timestamp":"2026-11-05T11:00:00.000Z","message":{"model":"claude-haiku-4-5-20251001","id":"msg_2","usage":{"input_tokens":7,"cache_creation_input_tokens":3,"cache_read_input_tokens":0,"output_tokens":5}}}
"#;

    #[test]
    fn parses_dedups_and_skips_synthetic() {
        let evs = parse_reader(SAMPLE.as_bytes());
        assert_eq!(evs.len(), 2);
        let a = &evs[0];
        assert_eq!(a.model, "claude-opus-5-5");
        assert_eq!(a.output, 40, "last line of a message wins");
        assert_eq!((a.input, a.cache_read, a.cache_write, a.cache_write_1h), (2, 500, 100, 100));
        assert_eq!(a.key.as_deref(), Some("c:msg_1:req_1"));
        assert_eq!(a.ts, parse_ts("2026-11-05T10:00:02.000Z").unwrap());
        let b = &evs[1];
        assert_eq!(b.model, "claude-haiku-4-5-20251001");
        assert_eq!((b.input, b.output, b.cache_write, b.cache_write_1h), (7, 5, 3, 0));
        assert_eq!(b.total_tokens(), 15);
    }
}
