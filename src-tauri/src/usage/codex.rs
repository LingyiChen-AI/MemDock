//! Codex rollout logs: `~/.codex/sessions/**/rollout-*.jsonl(.zst)` and
//! `~/.codex/archived_sessions`.
//!
//! `turn_context` lines carry the current model; each `event_msg` of type
//! `token_count` with a non-null `info` adds `info.last_token_usage`. Its
//! `input_tokens` include `cached_input_tokens`, so the cached part is moved
//! to `cache_read`. The newest `rate_limits` is the Codex quota snapshot.
use std::io::BufRead;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{parse_ts, u, RateSnapshot, RateWindow, UsageEvent};
use crate::model::Tool;

#[derive(Debug, Default)]
pub struct CodexParse {
    pub events: Vec<UsageEvent>,
    pub rate: Option<RateSnapshot>,
}

pub fn parse_reader(reader: impl BufRead) -> CodexParse {
    let mut out = CodexParse::default();
    let mut model = String::from("unknown");
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        let is_ctx = line.contains("\"turn_context\"");
        if !is_ctx && !line.contains("\"token_count\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(payload) = v.get("payload") else { continue };
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if kind == "turn_context" {
            if let Some(m) = payload.get("model").and_then(|m| m.as_str()) {
                model = m.to_string();
            }
            continue;
        }
        if kind != "event_msg" || payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
            continue;
        }
        let ts_str = v.get("timestamp").and_then(|t| t.as_str()).unwrap_or("");
        let Some(ts) = parse_ts(ts_str) else { continue };
        if let Some(rl) = payload.get("rate_limits").filter(|r| !r.is_null()) {
            let snap = parse_rate_limits(rl, ts);
            if out.rate.as_ref().is_none_or(|r| r.ts <= ts) {
                out.rate = Some(snap);
            }
        }
        let Some(info) = payload.get("info").filter(|i| !i.is_null()) else { continue };
        let Some(last) = info.get("last_token_usage") else { continue };
        let input_all = u(last, "input_tokens");
        let cached = u(last, "cached_input_tokens").min(input_all);
        let total = info
            .get("total_token_usage")
            .map(|t| u(t, "total_tokens"))
            .unwrap_or(0);
        out.events.push(UsageEvent {
            ts,
            tool: Tool::Codex,
            model: model.clone(),
            input: input_all - cached,
            output: u(last, "output_tokens"),
            cache_read: cached,
            cache_write: u(last, "cache_write_input_tokens"),
            cache_write_1h: 0,
            // Forked / resumed rollouts copy earlier lines verbatim.
            key: Some(format!("x:{ts_str}:{total}:{}", u(last, "total_tokens"))),
        });
    }
    out
}

fn parse_window(v: Option<&Value>, ts: i64) -> Option<RateWindow> {
    let w = v.filter(|w| !w.is_null())?;
    let used_percent = w.get("used_percent").and_then(|x| x.as_f64())?;
    let resets_at = w
        .get("resets_at")
        .and_then(|x| x.as_i64())
        .map(|s| s * 1000)
        .or_else(|| {
            w.get("resets_in_seconds")
                .and_then(|x| x.as_i64())
                .map(|s| ts + s * 1000)
        });
    Some(RateWindow {
        used_percent,
        window_minutes: w.get("window_minutes").and_then(|x| x.as_u64()),
        resets_at,
    })
}

pub fn parse_rate_limits(rl: &Value, ts: i64) -> RateSnapshot {
    RateSnapshot {
        ts,
        primary: parse_window(rl.get("primary"), ts),
        secondary: parse_window(rl.get("secondary"), ts),
    }
}

/// Rollout files under `sessions` and `archived_sessions`.
pub fn list_files(codex_home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for sub in ["sessions", "archived_sessions"] {
        for e in walkdir::WalkDir::new(codex_home.join(sub))
            .max_depth(6)
            .into_iter()
            .flatten()
        {
            if !e.file_type().is_file() {
                continue;
            }
            let name = e.file_name().to_string_lossy();
            if name.starts_with("rollout-") && (name.ends_with(".jsonl") || name.ends_with(".jsonl.zst")) {
                out.push(e.into_path());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"timestamp":"2026-10-27T14:30:41.913Z","type":"session_meta","payload":{"id":"s1"}}
{"timestamp":"2026-10-27T14:30:42.596Z","type":"turn_context","payload":{"model":"gpt-5-codex","cwd":"C:\\w"}}
{"timestamp":"2026-10-27T14:30:43.000Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":null}}
{"timestamp":"2026-10-27T14:30:45.881Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":33475},"last_token_usage":{"input_tokens":33351,"cached_input_tokens":24064,"cache_write_input_tokens":0,"output_tokens":124,"reasoning_output_tokens":20,"total_tokens":33475}},"rate_limits":{"primary":{"used_percent":46.0,"window_minutes":10080,"resets_at":1793599691},"secondary":null}}}
{"timestamp":"2026-10-27T14:31:00.000Z","type":"turn_context","payload":{"model":"gpt-6.1-sol"}}
{"timestamp":"2026-10-27T14:32:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":40000},"last_token_usage":{"input_tokens":6000,"cached_input_tokens":5000,"output_tokens":525,"total_tokens":6525}},"rate_limits":{"primary":{"used_percent":12.5,"window_minutes":300,"resets_in_seconds":600},"secondary":{"used_percent":47.0,"window_minutes":10080,"resets_at":1793599691}}}}
"#;

    #[test]
    fn parses_increments_models_and_latest_rate_limits() {
        let p = parse_reader(SAMPLE.as_bytes());
        assert_eq!(p.events.len(), 2, "null info is skipped");
        let a = &p.events[0];
        assert_eq!(a.model, "gpt-5-codex");
        assert_eq!((a.input, a.cache_read, a.output), (33351 - 24064, 24064, 124));
        assert_eq!(a.total_tokens(), 33475);
        let b = &p.events[1];
        assert_eq!(b.model, "gpt-6.1-sol");
        assert_eq!((b.input, b.cache_read, b.output), (1000, 5000, 525));
        assert_ne!(a.key, b.key);

        let r = p.rate.unwrap();
        let ts = parse_ts("2026-10-27T14:32:00.000Z").unwrap();
        assert_eq!(r.ts, ts);
        let pri = r.primary.unwrap();
        assert_eq!(pri.window_minutes, Some(300));
        assert_eq!(pri.resets_at, Some(ts + 600_000));
        assert_eq!(r.secondary.unwrap().resets_at, Some(1_793_599_691_000));
    }
}
