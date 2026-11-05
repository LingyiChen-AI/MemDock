//! Proxy discovery shared by the REST client and git: environment, the user's
//! global git config, then the OS system proxy. Credentials in a proxy URL are
//! never logged.

use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

const ENV_KEYS: [&str; 6] = [
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "HTTP_PROXY",
    "http_proxy",
];

/// Adds `http://` when the value has no scheme.
pub(crate) fn normalize_proxy(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s.contains(['\n', '\r', '\0', ' ']) {
        return None;
    }
    if s.contains("://") {
        Some(s.to_string())
    } else {
        Some(format!("http://{s}"))
    }
}

/// First non-empty proxy variable in priority order.
pub(crate) fn proxy_from_env(get: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    ENV_KEYS
        .iter()
        .filter_map(|k| get(k))
        .find_map(|v| normalize_proxy(&v))
}

/// Parses the Windows `ProxyServer` value: `host:port` or
/// `http=h:p;https=h:p;socks=h:p`. Prefers https, then http, then socks5.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn parse_windows_proxy(enabled: bool, server: &str) -> Option<String> {
    if !enabled {
        return None;
    }
    let server = server.trim();
    if server.is_empty() {
        return None;
    }
    if !server.contains('=') {
        return normalize_proxy(server);
    }
    let find = |name: &str| {
        server.split(';').find_map(|part| {
            let (k, v) = part.split_once('=')?;
            (k.trim().eq_ignore_ascii_case(name) && !v.trim().is_empty())
                .then(|| v.trim().to_string())
        })
    };
    if let Some(v) = find("https").or_else(|| find("http")) {
        return normalize_proxy(&v);
    }
    let socks = find("socks")?;
    if socks.contains("://") {
        normalize_proxy(&socks)
    } else {
        Some(format!("socks5://{socks}"))
    }
}

/// Parses `scutil --proxy` output (HTTPS, then HTTP, then SOCKS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn parse_scutil(out: &str) -> Option<String> {
    let get = |key: &str| -> Option<String> {
        out.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == key).then(|| v.trim().to_string())
        })
    };
    for (prefix, scheme) in [("HTTPS", "http"), ("HTTP", "http"), ("SOCKS", "socks5")] {
        if get(&format!("{prefix}Enable")).as_deref() != Some("1") {
            continue;
        }
        let (Some(host), Some(port)) = (
            get(&format!("{prefix}Proxy")),
            get(&format!("{prefix}Port")),
        ) else {
            continue;
        };
        if !host.is_empty() && port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() {
            return Some(format!("{scheme}://{host}:{port}"));
        }
    }
    None
}

#[cfg(windows)]
fn system_proxy() -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
        .ok()?;
    let enable: u32 = key.get_value("ProxyEnable").ok()?;
    let server: String = key.get_value("ProxyServer").ok()?;
    parse_windows_proxy(enable == 1, &server)
}

#[cfg(target_os = "macos")]
fn system_proxy() -> Option<String> {
    let mut cmd = Command::new("scutil");
    cmd.arg("--proxy")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = crate::proc::spawn_hidden(&mut cmd).ok()?;
    let out = crate::proc::wait_with_timeout(child, Duration::from_secs(2), None).ok()?;
    parse_scutil(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn system_proxy() -> Option<String> {
    None
}

/// The user's global git `https.proxy`, then `http.proxy`.
fn git_global_proxy(program: &std::path::Path) -> Option<String> {
    for key in ["https.proxy", "http.proxy"] {
        let mut cmd = Command::new(program);
        cmd.args(["config", "--global", "--get", key])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let Ok(child) = crate::proc::spawn_hidden(&mut cmd) else {
            return None;
        };
        if let Ok(out) = crate::proc::wait_with_timeout(child, Duration::from_secs(5), None) {
            if out.status.success() {
                if let Some(p) = normalize_proxy(&String::from_utf8_lossy(&out.stdout)) {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Pure resolution order, injectable for tests.
pub(crate) fn resolve_with(
    env: &dyn Fn(&str) -> Option<String>,
    git: &dyn Fn() -> Option<String>,
    system: &dyn Fn() -> Option<String>,
) -> Option<String> {
    proxy_from_env(env).or_else(git).or_else(system)
}

fn env_var(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

/// Proxy for the REST client: env, then git global config, then OS (cached).
pub fn resolve_proxy() -> Option<String> {
    static CACHE: OnceLock<Option<String>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            resolve_with(
                &env_var,
                &|| git_global_proxy(std::path::Path::new("git")),
                &system_proxy,
            )
        })
        .clone()
}

/// Proxy for git when its own global config has none: env, then OS (cached).
pub fn resolve_proxy_for_git() -> Option<String> {
    static CACHE: OnceLock<Option<String>> = OnceLock::new();
    CACHE
        .get_or_init(|| resolve_with(&env_var, &|| None, &system_proxy))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_priority_and_normalize() {
        let get = |m: &'static [(&'static str, &'static str)]| {
            move |k: &str| m.iter().find(|(a, _)| *a == k).map(|(_, v)| v.to_string())
        };
        let g = get(&[("http_proxy", "h:1"), ("HTTPS_PROXY", "http://s:2")]);
        assert_eq!(proxy_from_env(&g).as_deref(), Some("http://s:2"));
        let g = get(&[("http_proxy", "h:1"), ("ALL_PROXY", "socks5://a:3")]);
        assert_eq!(proxy_from_env(&g).as_deref(), Some("socks5://a:3"));
        let g = get(&[("http_proxy", "h:1")]);
        assert_eq!(proxy_from_env(&g).as_deref(), Some("http://h:1"));
        let g = get(&[("HTTPS_PROXY", "  ")]);
        assert_eq!(proxy_from_env(&g), None);
    }

    #[test]
    fn windows_proxy_forms() {
        assert_eq!(parse_windows_proxy(false, "h:1"), None);
        assert_eq!(parse_windows_proxy(true, ""), None);
        assert_eq!(parse_windows_proxy(true, "h:1").as_deref(), Some("http://h:1"));
        assert_eq!(
            parse_windows_proxy(true, "http=a:1;https=b:2;socks=c:3").as_deref(),
            Some("http://b:2")
        );
        assert_eq!(
            parse_windows_proxy(true, "http=a:1;socks=c:3").as_deref(),
            Some("http://a:1")
        );
        assert_eq!(
            parse_windows_proxy(true, "socks=c:3").as_deref(),
            Some("socks5://c:3")
        );
        assert_eq!(parse_windows_proxy(true, "ftp=x:9"), None);
    }

    #[test]
    fn scutil_parsing() {
        let out = "<dictionary> {\n  HTTPEnable : 1\n  HTTPPort : 8080\n  HTTPProxy : 10.0.0.1\n  HTTPSEnable : 0\n  HTTPSPort : 8443\n  HTTPSProxy : 10.0.0.2\n}";
        assert_eq!(parse_scutil(out).as_deref(), Some("http://10.0.0.1:8080"));
        let out = "HTTPSEnable : 1\nHTTPSPort : 8443\nHTTPSProxy : p.example\nHTTPEnable : 1\nHTTPPort : 1\nHTTPProxy : q";
        assert_eq!(parse_scutil(out).as_deref(), Some("http://p.example:8443"));
        let out = "SOCKSEnable : 1\nSOCKSPort : 1080\nSOCKSProxy : s.example";
        assert_eq!(parse_scutil(out).as_deref(), Some("socks5://s.example:1080"));
        assert_eq!(parse_scutil("HTTPEnable : 0"), None);
    }

    #[test]
    fn resolution_order() {
        let none = |_: &str| None;
        let envp = |k: &str| (k == "HTTPS_PROXY").then(|| "e:1".to_string());
        let git = || Some("http://g:2".to_string());
        let sys = || Some("http://s:3".to_string());
        assert_eq!(resolve_with(&envp, &git, &sys).as_deref(), Some("http://e:1"));
        assert_eq!(resolve_with(&none, &git, &sys).as_deref(), Some("http://g:2"));
        assert_eq!(resolve_with(&none, &|| None, &sys).as_deref(), Some("http://s:3"));
        assert_eq!(resolve_with(&none, &|| None, &|| None), None);
    }
}
