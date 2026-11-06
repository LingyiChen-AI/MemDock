//! Preferences sync: `memdock_settings.json` at the root of the sync
//! repository holds the user's portable preferences (backend settings, the
//! portable part of the extraction config and an allow-listed set of
//! frontend `localStorage` keys). Local paths and secrets never go in.
//!
//! The memory sync engine never treats this file as a memory file (only
//! `global/` and `projects/` paths are memory); it only (re)writes it on
//! upload when the caller passes a snapshot.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::extract::store::ExtractConfig;
use crate::settings::Settings;

pub const PREFS_FILE: &str = "memdock_settings.json";
/// Highest file format this build understands.
pub const PREFS_VERSION: u32 = 1;

/// Frontend `localStorage` keys that are synced. Everything else (layout
/// widths, window / chat / terminal state, the theme mirror keys, hidden
/// projects keyed by local path) stays on this machine.
pub const FRONTEND_KEYS: [&str; 9] = [
    "memdock.chat.prefs",
    "memdock.chat.newChatDir",
    "memdock.chat.mode",
    "memdock.nav.collapsed",
    "memdock.diffLayout",
    "memdock.editorMode",
    "memdock.syncScroll",
    "memdock.showEmptyProjects",
    "memdock.showHiddenProjects",
];

/// Backend part of the file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct BackendPrefs {
    pub language: Option<String>,
    pub theme: Option<String>,
    pub accent: Option<String>,
    pub claude_quota_enabled: bool,
    pub codex_quota_enabled: bool,
    /// `ExtractConfig` without `projects` (project ids are local paths).
    pub extract: Option<Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct PrefsFile {
    pub version: u32,
    /// RFC 3339, UTC.
    pub updated_at: String,
    pub device: String,
    pub app_version: String,
    pub backend: BackendPrefs,
    pub frontend: BTreeMap<String, Value>,
}

/// Whether `s` looks like an absolute local path (`/x`, `~/x`, `C:\x`, `\\host`).
pub fn is_abs_path(s: &str) -> bool {
    let s = s.trim();
    let b = s.as_bytes();
    s.starts_with('/')
        || s.starts_with('~')
        || s.starts_with("\\\\")
        || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
}

/// Drops every string that is an absolute path (object entries and array
/// items). `None` when `v` itself is one.
fn strip_paths(v: Value) -> Option<Value> {
    match v {
        Value::String(s) if is_abs_path(&s) => None,
        Value::Array(a) => Some(Value::Array(a.into_iter().filter_map(strip_paths).collect())),
        Value::Object(o) => Some(Value::Object(
            o.into_iter()
                .filter_map(|(k, v)| strip_paths(v).map(|v| (k, v)))
                .collect(),
        )),
        other => Some(other),
    }
}

/// The synced subset of the frontend's `localStorage` values (key -> parsed
/// value): allow-listed keys only, with known local-path fields removed.
pub fn sanitize_frontend(input: &Value) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let Some(obj) = input.as_object() else { return out };
    for key in FRONTEND_KEYS {
        let Some(v) = obj.get(key) else { continue };
        let v = match (key, v.clone()) {
            ("memdock.chat.prefs", Value::Object(mut o)) => {
                o.remove("cwd");
                Value::Object(o)
            }
            ("memdock.chat.newChatDir", Value::Object(o)) => {
                let mut m = Map::new();
                if let Some(mode) = o.get("mode") {
                    m.insert("mode".into(), mode.clone());
                }
                Value::Object(m)
            }
            (_, v) => v,
        };
        if v.is_null() {
            continue;
        }
        if let Some(v) = strip_paths(v) {
            out.insert(key.to_string(), v);
        }
    }
    out
}

/// Backend preferences of this machine.
pub fn backend_prefs(settings: &Settings, extract: Option<&ExtractConfig>) -> BackendPrefs {
    let extract = extract.and_then(|c| serde_json::to_value(c).ok()).map(|mut v| {
        if let Some(o) = v.as_object_mut() {
            o.remove("projects");
        }
        v
    });
    BackendPrefs {
        language: settings.language.clone(),
        theme: settings.theme.clone(),
        accent: settings.accent.clone(),
        claude_quota_enabled: settings.claude_quota_enabled,
        codex_quota_enabled: settings.codex_quota_enabled,
        extract,
    }
}

/// The file text: pretty-printed JSON with a stable key order and a trailing
/// newline. Fails when the content looks like it holds a secret.
pub fn build(
    backend: BackendPrefs,
    frontend: &Value,
    device: &str,
    updated_at: &str,
) -> Result<String, String> {
    let file = PrefsFile {
        version: PREFS_VERSION,
        updated_at: updated_at.to_string(),
        device: device.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        backend,
        frontend: sanitize_frontend(frontend),
    };
    let mut text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    text.push('\n');
    if !crate::secrets::scan(&text).is_empty() {
        return Err("secrets".into());
    }
    Ok(text)
}

/// Parses the file; rejects a missing / too new `version`.
pub fn parse(text: &str) -> Result<PrefsFile, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    let version = v
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| "missing version".to_string())?;
    if version > PREFS_VERSION as u64 {
        return Err(format!("too_new: version {version}"));
    }
    let mut f: PrefsFile = serde_json::from_value(v).map_err(|e| e.to_string())?;
    // Never hand a local path or an unknown key to the frontend.
    f.frontend = sanitize_frontend(&Value::Object(f.frontend.into_iter().collect()));
    Ok(f)
}

/// Same preferences, ignoring when / where / by which version they were written.
pub fn same_content(a: &str, b: &str) -> bool {
    let strip = |t: &str| -> Option<Value> {
        let mut v: Value = serde_json::from_str(t).ok()?;
        let o = v.as_object_mut()?;
        for k in ["updated_at", "device", "app_version"] {
            o.remove(k);
        }
        Some(v)
    };
    matches!((strip(a), strip(b)), (Some(x), Some(y)) if x == y)
}

/// Whether `remote` (an `updated_at`) is later than `seen`; never seen = newer.
pub fn is_newer(remote: &str, seen: Option<&str>) -> bool {
    let Some(seen) = seen else { return true };
    match (
        chrono::DateTime::parse_from_rfc3339(remote),
        chrono::DateTime::parse_from_rfc3339(seen),
    ) {
        (Ok(r), Ok(s)) => r > s,
        _ => remote != seen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn frontend() -> Value {
        json!({
            "memdock.chat.prefs": {
                "agent": "codex",
                "permission": {"claude": "plan", "codex": "ask"},
                "model": {"claude": "opus", "codex": ""},
                "effort": {"claude": "high", "codex": ""},
                "args": {"claude": "--verbose", "codex": ""},
                "cwd": "C:\\codes\\secret-project"
            },
            "memdock.chat.newChatDir": {"mode": "fixed", "fixed": "D:/work"},
            "memdock.chat.mode": "terminal",
            "memdock.nav.collapsed": ["tools"],
            "memdock.diffLayout": "unified",
            "memdock.editorMode": "preview",
            "memdock.syncScroll": "0",
            "memdock.showEmptyProjects": "1",
            "memdock.hiddenProjects": ["c:/codes/a"],
            "memdock.treeWidth": "300",
            "memdock.theme": "light"
        })
    }

    #[test]
    fn sanitize_keeps_allowed_keys_and_drops_local_paths() {
        let out = sanitize_frontend(&frontend());
        let keys: Vec<&str> = out.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec![
                "memdock.chat.mode",
                "memdock.chat.newChatDir",
                "memdock.chat.prefs",
                "memdock.diffLayout",
                "memdock.editorMode",
                "memdock.nav.collapsed",
                "memdock.showEmptyProjects",
                "memdock.syncScroll",
            ]
        );
        assert!(out["memdock.chat.prefs"].get("cwd").is_none());
        assert_eq!(out["memdock.chat.prefs"]["model"]["claude"], "opus");
        assert_eq!(out["memdock.chat.newChatDir"], json!({"mode": "fixed"}));
        // a stray path anywhere is dropped
        let out = sanitize_frontend(&json!({"memdock.nav.collapsed": ["tools", "/home/me", "C:/x"]}));
        assert_eq!(out["memdock.nav.collapsed"], json!(["tools"]));
        let out = sanitize_frontend(&json!({"memdock.diffLayout": "~/x"}));
        assert!(out.is_empty());
    }

    #[test]
    fn build_is_stable_and_has_no_paths() {
        let settings = Settings {
            language: Some("zh-CN".into()),
            theme: Some("light".into()),
            accent: Some("blue".into()),
            manual_projects: vec!["C:/codes/private".into()],
            claude_quota_enabled: true,
            codex_quota_enabled: false,
        };
        let cfg = ExtractConfig {
            lookback_days: 14,
            custom_prompt: Some("only facts".into()),
            projects: crate::extract::store::ProjectFilter::Selected(vec!["c:/codes/a".into()]),
            ..ExtractConfig::default()
        };
        let a = build(backend_prefs(&settings, Some(&cfg)), &frontend(), "PC-1", "2026-11-07T10:00:00Z").unwrap();
        let b = build(backend_prefs(&settings, Some(&cfg)), &frontend(), "PC-1", "2026-11-07T10:00:00Z").unwrap();
        assert_eq!(a, b);
        assert!(a.ends_with("}\n"));
        assert!(!a.contains("codes"), "local paths leaked: {a}");
        assert!(!a.contains("manual_projects"));
        assert!(!a.contains("\"projects\""));
        assert!(a.find("\"version\"").unwrap() < a.find("\"updated_at\"").unwrap());
        let f = parse(&a).unwrap();
        assert_eq!(f.version, 1);
        assert_eq!(f.device, "PC-1");
        assert_eq!(f.backend.theme.as_deref(), Some("light"));
        assert!(f.backend.claude_quota_enabled);
        assert_eq!(f.backend.extract.as_ref().unwrap()["lookback_days"], 14);
        assert_eq!(f.frontend["memdock.chat.mode"], "terminal");
        // a different time / device is the same content
        let c = build(backend_prefs(&settings, Some(&cfg)), &frontend(), "PC-2", "2026-11-08T10:00:00Z").unwrap();
        assert!(same_content(&a, &c));
        let d = build(backend_prefs(&Settings::default(), None), &frontend(), "PC-2", "2026-11-08T10:00:00Z").unwrap();
        assert!(!same_content(&a, &d));
    }

    #[test]
    fn build_refuses_secrets() {
        let fe = json!({"memdock.chat.prefs": {"args": {"claude": "--api-key sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", "codex": ""}}});
        assert_eq!(
            build(BackendPrefs::default(), &fe, "PC", "2026-11-07T10:00:00Z"),
            Err("secrets".into())
        );
    }

    #[test]
    fn parse_checks_version() {
        assert!(parse("{}").unwrap_err().contains("missing version"));
        assert!(parse("{\"version\": 2}").unwrap_err().starts_with("too_new"));
        assert!(parse("not json").is_err());
        let f = parse("{\"version\":1,\"frontend\":{\"memdock.treeWidth\":\"1\",\"memdock.diffLayout\":\"split\"}}").unwrap();
        assert_eq!(f.frontend.len(), 1);
        assert_eq!(f.backend, BackendPrefs::default());
    }

    #[test]
    fn newer_compares_times() {
        assert!(is_newer("2026-11-07T10:00:00Z", None));
        assert!(is_newer("2026-11-07T10:00:01Z", Some("2026-11-07T10:00:00Z")));
        assert!(!is_newer("2026-11-07T10:00:00Z", Some("2026-11-07T18:00:00+08:00")));
    }
}
