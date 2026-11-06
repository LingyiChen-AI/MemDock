//! Codex live quota windows (opt-in, default off).
//!
//! Reads the ChatGPT login of Codex from `~/.codex/auth.json`
//! (`tokens.access_token`, `tokens.account_id`) and calls the non-public
//! endpoint `GET https://chatgpt.com/backend-api/wham/usage` (the same one the
//! Codex CLI and CodexBar use). The file is only read: the token is never
//! refreshed, written back, logged or put into an error. A 401/403 means the
//! login expired and the user has to sign in again in Codex.
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::{parse_ts, QuotaWindow, ToolQuota};

pub const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const TIMEOUT: Duration = Duration::from_secs(15);

/// Reason codes (also listed in `src/api/types.ts` as `QuotaReason`).
pub const NOT_LOGGED_IN: &str = "codex_not_logged_in";
pub const LOGIN_EXPIRED: &str = "codex_login_expired";
pub const UNEXPECTED: &str = "unexpected_response";
pub const NETWORK: &str = "network";

/// Credentials taken from `auth.json`.
#[derive(Debug, PartialEq)]
pub struct CodexAuth {
    pub access_token: String,
    pub account_id: Option<String>,
}

/// Minimal HTTP response for the injectable getter.
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// GET `url` with `headers`; `Err` is a transport failure (no status).
pub type HttpGet<'a> = &'a dyn Fn(&str, &[(&str, String)]) -> Result<HttpResponse, ()>;

fn str_at<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(|x| x.as_str()))
        .filter(|s| !s.trim().is_empty())
}

/// Token and account id from the `auth.json` text. Accepts the nested
/// `tokens` object and a few flat / camelCase variants.
pub fn parse_auth(text: &str) -> Option<CodexAuth> {
    let v: Value = serde_json::from_str(text).ok()?;
    let tokens = v.get("tokens").filter(|t| t.is_object()).unwrap_or(&v);
    let access_token = str_at(tokens, &["access_token", "accessToken"])?.to_string();
    let account_id = str_at(tokens, &["account_id", "accountId"])
        .or_else(|| str_at(&v, &["account_id", "accountId"]))
        .map(String::from);
    Some(CodexAuth { access_token, account_id })
}

pub fn read_auth(codex_home: &Path) -> Option<CodexAuth> {
    parse_auth(&std::fs::read_to_string(codex_home.join("auth.json")).ok()?)
}

fn num(v: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| {
        let x = v.get(*k)?;
        x.as_f64().or_else(|| x.as_str().and_then(|s| s.trim().parse().ok()))
    })
}

/// Epoch seconds, epoch ms or RFC 3339 → epoch ms.
fn time_ms(x: &Value) -> Option<i64> {
    if let Some(s) = x.as_str() {
        return parse_ts(s).or_else(|| s.trim().parse::<f64>().ok().map(epoch_ms));
    }
    x.as_f64().map(epoch_ms)
}

fn epoch_ms(n: f64) -> i64 {
    if n > 1e12 {
        n as i64
    } else {
        (n * 1000.0) as i64
    }
}

/// One window object; `None` when it carries no usage percentage.
fn window(w: Option<&Value>, now_ms: i64) -> Option<QuotaWindow> {
    let w = w.filter(|w| w.is_object())?;
    let mut used = num(w, &["used_percent", "usedPercent", "used_percentage", "percent_used", "utilization"])?;
    // Some payloads report a 0..1 fraction.
    if num(w, &["used_percent", "usedPercent", "used_percentage", "percent_used"]).is_none() && used <= 1.0 {
        used *= 100.0;
    }
    let window_minutes = num(w, &["limit_window_seconds", "window_seconds", "windowSeconds", "limitWindowSeconds"])
        .map(|s| (s / 60.0).round() as u64)
        .or_else(|| num(w, &["window_minutes", "windowMinutes", "limit_window_minutes"]).map(|m| m.round() as u64));
    let resets_at = ["reset_at", "resets_at", "resetAt", "resetsAt"]
        .iter()
        .find_map(|k| w.get(*k).and_then(time_ms))
        .or_else(|| {
            num(w, &["reset_after_seconds", "resets_in_seconds", "reset_in_seconds", "resetAfterSeconds"])
                .map(|s| now_ms + (s * 1000.0) as i64)
        });
    Some(QuotaWindow { used_percent: used, resets_at, window_minutes })
}

fn first<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|k| v.get(*k).filter(|x| !x.is_null()))
}

/// Maps the endpoint JSON to a quota. Expected shape (names vary, all are
/// accepted): `{"plan_type", "rate_limit": {"primary_window": {"used_percent",
/// "limit_window_seconds", "reset_after_seconds" | "reset_at"},
/// "secondary_window": {...}}}`. Windows of up to 6 hours are the 5-hour
/// window, longer ones the weekly window; without a length, primary is the
/// 5-hour window and secondary the weekly one.
pub fn parse_usage_response(v: &Value, now_ms: i64) -> ToolQuota {
    let rl = first(v, &["rate_limit", "rate_limits", "rateLimit", "rateLimits"]).unwrap_or(v);
    let primary = window(first(rl, &["primary_window", "primary", "primaryWindow"]), now_ms);
    let secondary = window(first(rl, &["secondary_window", "secondary", "secondaryWindow"]), now_ms);
    let mut q = ToolQuota::with_status("ok", None);
    for (w, is_primary) in [(primary, true), (secondary, false)] {
        let Some(w) = w else { continue };
        let short = match w.window_minutes {
            Some(m) => m <= 360,
            None => is_primary,
        };
        let (want, other) = if short { (&mut q.five_hour, &mut q.weekly) } else { (&mut q.weekly, &mut q.five_hour) };
        if want.is_none() {
            *want = Some(w);
        } else if other.is_none() {
            *other = Some(w);
        }
    }
    if q.five_hour.is_none() && q.weekly.is_none() {
        return ToolQuota::with_status("unavailable", Some(UNEXPECTED.into()));
    }
    q.plan = str_at(v, &["plan_type", "planType", "plan"]).map(String::from);
    q.updated_at = Some(now_ms);
    q.source = Some("live".into());
    q
}

/// Performs the lookup through `get`; errors become short reason codes.
pub fn fetch_with(auth: Option<CodexAuth>, now_ms: i64, get: HttpGet) -> ToolQuota {
    let Some(auth) = auth else {
        return ToolQuota::with_status("unavailable", Some(NOT_LOGGED_IN.into()));
    };
    let mut headers: Vec<(&str, String)> = vec![
        ("Authorization", format!("Bearer {}", auth.access_token)),
        ("Accept", "application/json".into()),
        ("User-Agent", format!("MemDock/{}", env!("CARGO_PKG_VERSION"))),
    ];
    if let Some(id) = &auth.account_id {
        headers.push(("ChatGPT-Account-Id", id.clone()));
    }
    let res = get(USAGE_URL, &headers);
    drop(headers);
    drop(auth);
    let resp = match res {
        Ok(r) => r,
        Err(()) => return ToolQuota::with_status("unavailable", Some(NETWORK.into())),
    };
    match resp.status {
        200..=299 => {}
        401 | 403 => return ToolQuota::with_status("unavailable", Some(LOGIN_EXPIRED.into())),
        code => return ToolQuota::with_status("unavailable", Some(format!("http_{code}"))),
    }
    match serde_json::from_str::<Value>(&resp.body) {
        Ok(v) => parse_usage_response(&v, now_ms),
        Err(_) => ToolQuota::with_status("unavailable", Some(UNEXPECTED.into())),
    }
}

/// Real HTTP getter: system / git / env proxy, 15 s timeout.
fn ureq_get(url: &str, headers: &[(&str, String)]) -> Result<HttpResponse, ()> {
    let proxy = crate::sync::net::resolve_proxy().and_then(|p| ureq::Proxy::new(&p).ok());
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .proxy(proxy)
        .build()
        .into();
    let mut req = agent.get(url);
    for (k, v) in headers {
        req = req.header(*k, v.as_str());
    }
    let mut resp = req.call().map_err(|_| ())?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    Ok(HttpResponse { status, body })
}

/// Reads `auth.json` under `codex_home` and queries the endpoint.
pub fn fetch(codex_home: &Path, now_ms: i64) -> ToolQuota {
    fetch_with(read_auth(codex_home), now_ms, &ureq_get)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn reads_auth_json_variants() {
        let a = parse_auth(
            r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token":"i","access_token":"tok","refresh_token":"r","account_id":"acc"},"last_refresh":"2026-11-01T00:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(a, CodexAuth { access_token: "tok".into(), account_id: Some("acc".into()) });
        let b = parse_auth(r#"{"tokens":{"accessToken":"t2"}}"#).unwrap();
        assert_eq!(b.account_id, None);
        assert_eq!(parse_auth(r#"{"OPENAI_API_KEY":"sk-x","tokens":null}"#), None);
        assert_eq!(parse_auth(r#"{"tokens":{"access_token":""}}"#), None);
        assert_eq!(parse_auth("not json"), None);
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read_auth(tmp.path()), None);
    }

    #[test]
    fn parses_codexbar_style_response() {
        let v: Value = serde_json::from_str(
            r#"{"plan_type":"plus","rate_limit":{"allowed":true,"limit_reached":false,
                "primary_window":{"used_percent":23,"limit_window_seconds":18000,"reset_after_seconds":3600,"reset_at":1793600000},
                "secondary_window":{"used_percent":61.5,"limit_window_seconds":604800,"reset_after_seconds":86400}},
                "credits":{"has_credits":false,"unlimited":false,"balance":"0"}}"#,
        )
        .unwrap();
        let q = parse_usage_response(&v, 1_000);
        assert_eq!(q.status, "ok");
        assert_eq!(q.source.as_deref(), Some("live"));
        assert_eq!(q.plan.as_deref(), Some("plus"));
        assert_eq!(q.updated_at, Some(1_000));
        let f = q.five_hour.unwrap();
        assert_eq!((f.used_percent, f.window_minutes, f.resets_at), (23.0, Some(300), Some(1_793_600_000_000)));
        let w = q.weekly.unwrap();
        assert_eq!((w.used_percent, w.window_minutes, w.resets_at), (61.5, Some(10080), Some(1_000 + 86_400_000)));
    }

    #[test]
    fn parses_alternative_names_and_order() {
        // Weekly window listed first, camelCase names, ISO reset time, minutes.
        let v = serde_json::json!({
            "rateLimits": {
                "primary": {"usedPercent": 40, "windowMinutes": 10080, "resetsAt": "2026-11-10T00:00:00Z"},
                "secondary": {"utilization": 0.5, "window_minutes": 300, "resets_in_seconds": 60}
            }
        });
        let q = parse_usage_response(&v, 0);
        assert_eq!(q.weekly.as_ref().unwrap().used_percent, 40.0);
        assert_eq!(q.weekly.as_ref().unwrap().resets_at, parse_ts("2026-11-10T00:00:00Z"));
        assert_eq!(q.five_hour.as_ref().unwrap().used_percent, 50.0);
        assert_eq!(q.five_hour.as_ref().unwrap().resets_at, Some(60_000));
        // No window length: primary is the 5-hour window.
        let v = serde_json::json!({"rate_limit": {"primary_window": {"used_percent": 5}, "secondary_window": null}});
        let q = parse_usage_response(&v, 0);
        assert_eq!(q.five_hour.unwrap().used_percent, 5.0);
        assert!(q.weekly.is_none());
    }

    #[test]
    fn unknown_shape_is_an_error_not_zero() {
        for v in [serde_json::json!({}), serde_json::json!({"rate_limit": {"primary_window": {"foo": 1}}}), serde_json::json!({"detail": "x"})] {
            let q = parse_usage_response(&v, 0);
            assert_eq!(q.status, "unavailable");
            assert_eq!(q.reason.as_deref(), Some(UNEXPECTED));
        }
    }

    #[test]
    fn fetch_sends_headers_and_maps_status() {
        let auth = || Some(CodexAuth { access_token: "secret".into(), account_id: Some("acc".into()) });
        let seen: RefCell<Vec<(String, String)>> = RefCell::new(vec![]);
        let ok = |url: &str, h: &[(&str, String)]| {
            assert_eq!(url, USAGE_URL);
            *seen.borrow_mut() = h.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
            Ok(HttpResponse { status: 200, body: r#"{"rate_limit":{"primary_window":{"used_percent":10,"limit_window_seconds":18000}}}"#.into() })
        };
        let q = fetch_with(auth(), 7, &ok);
        assert_eq!(q.status, "ok");
        let h = seen.borrow();
        assert!(h.contains(&("Authorization".into(), "Bearer secret".into())));
        assert!(h.contains(&("ChatGPT-Account-Id".into(), "acc".into())));
        assert!(h.iter().any(|(k, v)| k == "User-Agent" && v.starts_with("MemDock/")));

        let status = |code: u16| move |_: &str, _: &[(&str, String)]| Ok(HttpResponse { status: code, body: "{}".into() });
        let q = fetch_with(auth(), 0, &status(401));
        assert_eq!(q.reason.as_deref(), Some(LOGIN_EXPIRED));
        assert_eq!(fetch_with(auth(), 0, &status(403)).reason.as_deref(), Some(LOGIN_EXPIRED));
        assert_eq!(fetch_with(auth(), 0, &status(500)).reason.as_deref(), Some("http_500"));
        let down = |_: &str, _: &[(&str, String)]| Err(());
        assert_eq!(fetch_with(auth(), 0, &down).reason.as_deref(), Some(NETWORK));
        let garbage = |_: &str, _: &[(&str, String)]| Ok(HttpResponse { status: 200, body: "<html>".into() });
        assert_eq!(fetch_with(auth(), 0, &garbage).reason.as_deref(), Some(UNEXPECTED));
        let never = |_: &str, _: &[(&str, String)]| -> Result<HttpResponse, ()> { panic!("no request without a login") };
        assert_eq!(fetch_with(None, 0, &never).reason.as_deref(), Some(NOT_LOGGED_IN));
        // Errors never contain the token.
        assert!(!format!("{:?}", fetch_with(auth(), 0, &status(401))).contains("secret"));
    }
}
