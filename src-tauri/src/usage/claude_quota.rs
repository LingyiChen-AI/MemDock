//! Claude Code quota windows (opt-in, default off).
//!
//! Reads Claude Code's OAuth access token (Windows / Linux:
//! `~/.claude/.credentials.json`, macOS: keychain item
//! `Claude Code-credentials`) and calls the non-public endpoint
//! `GET https://api.anthropic.com/api/oauth/usage`. The token is only placed in
//! the request header: it never enters logs, errors or events.
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::{parse_ts, QuotaWindow, ToolQuota};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const TIMEOUT: Duration = Duration::from_secs(10);

/// `claudeAiOauth.accessToken` from the credentials JSON.
pub fn parse_credentials(text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(text).ok()?;
    v.get("claudeAiOauth")?
        .get("accessToken")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// The access token, or `None` when Claude Code is not logged in.
pub fn read_token(claude_home: &Path) -> Option<String> {
    if let Ok(text) = std::fs::read_to_string(claude_home.join(".credentials.json")) {
        if let Some(t) = parse_credentials(&text) {
            return Some(t);
        }
    }
    if cfg!(target_os = "macos") {
        return keychain_credentials().and_then(|t| parse_credentials(&t));
    }
    None
}

fn keychain_credentials() -> Option<String> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new("security");
    cmd.args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = crate::proc::spawn_hidden(&mut cmd).ok()?;
    let out = crate::proc::wait_with_timeout(child, Duration::from_secs(5), None).ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn window(v: Option<&Value>) -> Option<QuotaWindow> {
    let w = v.filter(|w| !w.is_null())?;
    let used = w.get("utilization").and_then(|x| x.as_f64())?;
    Some(QuotaWindow {
        used_percent: used,
        resets_at: w.get("resets_at").and_then(|x| x.as_str()).and_then(parse_ts),
        window_minutes: None,
    })
}

/// Maps the endpoint's JSON (`five_hour` / `seven_day` with `utilization`
/// in percent and RFC 3339 `resets_at`) to a quota.
pub fn parse_usage_response(v: &Value, now_ms: i64) -> ToolQuota {
    let mut q = ToolQuota::with_status("ok", None);
    q.five_hour = window(v.get("five_hour")).map(|w| QuotaWindow { window_minutes: Some(300), ..w });
    q.weekly = window(v.get("seven_day")).map(|w| QuotaWindow { window_minutes: Some(10080), ..w });
    q.updated_at = Some(now_ms);
    if q.five_hour.is_none() && q.weekly.is_none() {
        return ToolQuota::with_status("unavailable", Some("unexpected_response".into()));
    }
    q
}

/// Performs the network call. Errors are reduced to short codes.
pub fn fetch(claude_home: &Path, now_ms: i64) -> ToolQuota {
    let Some(token) = read_token(claude_home) else {
        return ToolQuota::with_status("unavailable", Some("not_logged_in".into()));
    };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let resp = agent
        .get(USAGE_URL)
        .header("Authorization", &format!("Bearer {token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Accept", "application/json")
        .call();
    drop(token);
    let mut resp = match resp {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(code)) => {
            return ToolQuota::with_status("unavailable", Some(format!("http_{code}")));
        }
        Err(_) => return ToolQuota::with_status("unavailable", Some("network".into())),
    };
    let Ok(text) = resp.body_mut().read_to_string() else {
        return ToolQuota::with_status("unavailable", Some("network".into()));
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => parse_usage_response(&v, now_ms),
        Err(_) => ToolQuota::with_status("unavailable", Some("unexpected_response".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_token_from_credentials_json() {
        assert_eq!(
            parse_credentials(r#"{"claudeAiOauth":{"accessToken":"tok","refreshToken":"r"}}"#).as_deref(),
            Some("tok")
        );
        assert_eq!(parse_credentials(r#"{"claudeAiOauth":{"accessToken":""}}"#), None);
        assert_eq!(parse_credentials("nope"), None);
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read_token(tmp.path()), None);
    }

    #[test]
    fn parses_usage_response() {
        let v: Value = serde_json::from_str(
            r#"{"five_hour":{"utilization":37.5,"resets_at":"2026-11-06T12:00:00Z"},"seven_day":{"utilization":12,"resets_at":null},"seven_day_opus":null}"#,
        )
        .unwrap();
        let q = parse_usage_response(&v, 5);
        assert_eq!(q.status, "ok");
        let f = q.five_hour.unwrap();
        assert_eq!(f.used_percent, 37.5);
        assert_eq!(f.resets_at, parse_ts("2026-11-06T12:00:00Z"));
        assert_eq!(q.weekly.unwrap().used_percent, 12.0);
        let bad = parse_usage_response(&serde_json::json!({"error": "x"}), 5);
        assert_eq!(bad.status, "unavailable");
    }
}
