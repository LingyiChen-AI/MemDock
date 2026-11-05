//! Local token usage statistics for Claude Code and Codex, read from their
//! session logs (read-only), priced with the bundled public price table.
pub mod aggregate;
pub mod cache;
pub mod claude;
pub mod claude_quota;
pub mod codex;
pub mod pricing;

use serde::{Deserialize, Serialize};

use crate::model::Tool;

/// One billed model request.
///
/// Token buckets do not overlap: `input` is uncached input, `cache_read` is
/// input served from the cache, `cache_write` is input written to the cache
/// (`cache_write_1h` is the part of it written with the 1-hour TTL).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct UsageEvent {
    /// Epoch milliseconds (UTC).
    #[serde(rename = "t")]
    pub ts: i64,
    #[serde(rename = "k")]
    pub tool: Tool,
    #[serde(rename = "m")]
    pub model: String,
    #[serde(rename = "i", default)]
    pub input: u64,
    #[serde(rename = "o", default)]
    pub output: u64,
    #[serde(rename = "r", default)]
    pub cache_read: u64,
    #[serde(rename = "w", default)]
    pub cache_write: u64,
    #[serde(rename = "h", default)]
    pub cache_write_1h: u64,
    /// Deduplication key (same message logged on several lines / files).
    #[serde(rename = "d", default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

impl UsageEvent {
    pub fn total_tokens(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

/// One Codex rate-limit window as logged in `token_count` events.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RateWindow {
    pub used_percent: f64,
    pub window_minutes: Option<u64>,
    /// Epoch milliseconds when the window resets.
    pub resets_at: Option<i64>,
}

/// The newest Codex `rate_limits` snapshot of a file.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RateSnapshot {
    /// Epoch milliseconds of the log line.
    pub ts: i64,
    pub primary: Option<RateWindow>,
    pub secondary: Option<RateWindow>,
}

/// Parses an RFC 3339 timestamp (`2026-10-27T14:30:41.913Z`) to epoch ms.
pub fn parse_ts(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

fn u(v: &serde_json::Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

/// One quota window for the UI.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct QuotaWindow {
    pub used_percent: f64,
    /// Epoch ms.
    pub resets_at: Option<i64>,
    pub window_minutes: Option<u64>,
}

/// Quota of one tool. `status`: "ok" | "disabled" | "unavailable" | "no_data".
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ToolQuota {
    pub status: String,
    pub five_hour: Option<QuotaWindow>,
    pub weekly: Option<QuotaWindow>,
    /// Epoch ms the data was observed.
    pub updated_at: Option<i64>,
    /// Short machine-readable reason for `unavailable` (never contains credentials).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ToolQuota {
    pub fn with_status(status: &str, reason: Option<String>) -> Self {
        ToolQuota { status: status.into(), five_hour: None, weekly: None, updated_at: None, reason }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct QuotaDto {
    pub claude: ToolQuota,
    pub codex: ToolQuota,
}

/// Codex quota from the newest log snapshot. Windows of up to 6 hours are the
/// session window, longer ones the weekly window; a window whose reset time has
/// passed is shown as 0% used.
pub fn codex_quota(snap: Option<&RateSnapshot>, now_ms: i64) -> ToolQuota {
    let Some(s) = snap else { return ToolQuota::with_status("no_data", None) };
    let mut q = ToolQuota::with_status("ok", None);
    q.updated_at = Some(s.ts);
    for w in [&s.primary, &s.secondary].into_iter().flatten() {
        let expired = w.resets_at.is_some_and(|r| r <= now_ms);
        let win = QuotaWindow {
            used_percent: if expired { 0.0 } else { w.used_percent },
            resets_at: if expired { None } else { w.resets_at },
            window_minutes: w.window_minutes,
        };
        if w.window_minutes.is_some_and(|m| m <= 360) {
            q.five_hour = Some(win);
        } else {
            q.weekly = Some(win);
        }
    }
    q
}

#[cfg(test)]
mod quota_tests {
    use super::*;

    #[test]
    fn codex_windows_are_classified_and_expire() {
        let snap = RateSnapshot {
            ts: 1_000,
            primary: Some(RateWindow { used_percent: 30.0, window_minutes: Some(300), resets_at: Some(5_000) }),
            secondary: Some(RateWindow { used_percent: 60.0, window_minutes: Some(10080), resets_at: Some(50_000) }),
        };
        let q = codex_quota(Some(&snap), 2_000);
        assert_eq!(q.status, "ok");
        assert_eq!(q.five_hour.as_ref().unwrap().used_percent, 30.0);
        assert_eq!(q.weekly.as_ref().unwrap().used_percent, 60.0);
        let q = codex_quota(Some(&snap), 10_000);
        assert_eq!(q.five_hour.as_ref().unwrap().used_percent, 0.0);
        assert_eq!(q.five_hour.as_ref().unwrap().resets_at, None);
        assert_eq!(codex_quota(None, 0).status, "no_data");
    }
}
