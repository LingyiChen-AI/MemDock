//! GitHub access for imports: repository info and the recursive tree through
//! the REST API, file bytes through raw.githubusercontent.com (does not count
//! against the API rate limit). Uses the same proxy discovery as sync. The
//! token, when present, is only ever sent in a header.

use std::fmt;
use std::time::Duration;

use serde::Deserialize;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FetchError {
    /// 404 (also what GitHub answers for a private repository without access).
    NotFound,
    /// 401: the token was rejected.
    Unauthorized,
    /// 403/429 with the rate limit exhausted.
    RateLimited,
    /// A response body exceeded the allowed size.
    TooLarge,
    Network(String),
    Http { status: u16, message: String },
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::NotFound => f.write_str("not found on GitHub"),
            FetchError::Unauthorized => f.write_str("GitHub rejected the credentials (401)"),
            FetchError::RateLimited => f.write_str("GitHub API rate limit exceeded"),
            FetchError::TooLarge => f.write_str("response too large"),
            FetchError::Network(m) => write!(f, "network error: {m}"),
            FetchError::Http { status, message } => write!(f, "GitHub error {status}: {message}"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EntryKind {
    Blob,
    Tree,
    /// Submodules, symlinks are blobs with a special mode but we treat them as blobs.
    Other,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TreeEntry {
    /// `/`-separated path from the repository root.
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct RepoTree {
    pub entries: Vec<TreeEntry>,
    /// GitHub cut the listing short (very large repository).
    pub truncated: bool,
}

/// Everything the import needs from GitHub. `token` is optional: public
/// repositories work without one.
pub trait GhFetcher: Send + Sync {
    fn default_branch(&self, owner: &str, repo: &str, token: Option<&str>) -> Result<String, FetchError>;
    /// Commit sha of a branch / tag / sha; `Ok(None)` when the ref does not exist.
    fn commit_sha(&self, owner: &str, repo: &str, git_ref: &str, token: Option<&str>) -> Result<Option<String>, FetchError>;
    fn tree(&self, owner: &str, repo: &str, sha: &str, token: Option<&str>) -> Result<RepoTree, FetchError>;
    /// File bytes at `sha`; fails with `TooLarge` past `limit` bytes.
    fn raw(&self, owner: &str, repo: &str, sha: &str, path: &str, limit: u64, token: Option<&str>) -> Result<Vec<u8>, FetchError>;
}

/// Calls `f` with the token; when GitHub rejects it (401), retries once
/// without, so a stale login does not block public repositories.
pub fn with_fallback<T>(
    token: Option<&str>,
    f: impl Fn(Option<&str>) -> Result<T, FetchError>,
) -> Result<T, FetchError> {
    match f(token) {
        Err(FetchError::Unauthorized) if token.is_some() => f(None),
        r => r,
    }
}

/// Percent-encodes one path segment (RFC 3986 unreserved characters stay).
pub fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Encodes a `/`-separated path segment by segment.
pub fn encode_path(p: &str) -> String {
    p.split('/').map(encode_segment).collect::<Vec<_>>().join("/")
}

// ---------------------------------------------------------------- HTTP impl

/// API JSON responses are capped at this size (a recursive tree of a big repo
/// can be several MB).
const MAX_API_BODY: u64 = 16 * 1024 * 1024;

pub struct HttpFetcher {
    agent: ureq::Agent,
    api_base: String,
    raw_base: String,
}

impl Default for HttpFetcher {
    fn default() -> Self {
        Self::new()
    }
}

struct Raw {
    status: u16,
    remaining_zero: bool,
    body: Vec<u8>,
}

#[derive(Deserialize)]
struct RepoJson {
    default_branch: String,
}

#[derive(Deserialize)]
struct TreeJson {
    #[serde(default)]
    tree: Vec<TreeItemJson>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Deserialize)]
struct TreeItemJson {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    size: Option<u64>,
}

fn scrub(msg: &str, token: Option<&str>) -> String {
    match token {
        Some(t) if !t.is_empty() => msg.replace(t, "***"),
        _ => msg.to_string(),
    }
}

impl HttpFetcher {
    pub fn new() -> Self {
        let proxy = crate::sync::net::resolve_proxy().and_then(|p| ureq::Proxy::new(&p).ok());
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .proxy(proxy)
            .build()
            .into();
        HttpFetcher {
            agent,
            api_base: "https://api.github.com".into(),
            raw_base: "https://raw.githubusercontent.com".into(),
        }
    }

    fn get(&self, url: &str, accept: &str, token: Option<&str>, limit: u64) -> Result<Raw, FetchError> {
        let mut req = self
            .agent
            .get(url)
            .header("Accept", accept)
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "MemDock");
        if let Some(t) = token.filter(|t| !t.is_empty()) {
            req = req.header("Authorization", &format!("Bearer {t}"));
        }
        let mut resp = req
            .call()
            .map_err(|e| FetchError::Network(scrub(&e.to_string(), token)))?;
        let status = resp.status().as_u16();
        let remaining_zero = resp
            .headers()
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.trim() == "0");
        let body = match resp.body_mut().with_config().limit(limit).read_to_vec() {
            Ok(b) => b,
            Err(ureq::Error::BodyExceedsLimit(_)) => return Err(FetchError::TooLarge),
            Err(e) => {
                let s = e.to_string();
                if s.contains("limit") {
                    return Err(FetchError::TooLarge);
                }
                return Err(FetchError::Network(scrub(&s, token)));
            }
        };
        Ok(Raw { status, remaining_zero, body })
    }

    fn check(raw: Raw) -> Result<Vec<u8>, FetchError> {
        match raw.status {
            200..=299 => Ok(raw.body),
            401 => Err(FetchError::Unauthorized),
            404 => Err(FetchError::NotFound),
            429 => Err(FetchError::RateLimited),
            403 if raw.remaining_zero => Err(FetchError::RateLimited),
            status => {
                let message = serde_json::from_slice::<serde_json::Value>(&raw.body)
                    .ok()
                    .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string))
                    .unwrap_or_default();
                if status == 403 && message.to_ascii_lowercase().contains("rate limit") {
                    return Err(FetchError::RateLimited);
                }
                Err(FetchError::Http { status, message })
            }
        }
    }

    fn api(&self, path: &str, token: Option<&str>) -> Result<Vec<u8>, FetchError> {
        let url = format!("{}{}", self.api_base, path);
        Self::check(self.get(&url, "application/vnd.github+json", token, MAX_API_BODY)?)
    }
}

fn bad_json(e: serde_json::Error) -> FetchError {
    FetchError::Network(format!("unexpected GitHub response: {e}"))
}

impl GhFetcher for HttpFetcher {
    fn default_branch(&self, owner: &str, repo: &str, token: Option<&str>) -> Result<String, FetchError> {
        let body = self.api(&format!("/repos/{}/{}", encode_segment(owner), encode_segment(repo)), token)?;
        Ok(serde_json::from_slice::<RepoJson>(&body).map_err(bad_json)?.default_branch)
    }

    fn commit_sha(&self, owner: &str, repo: &str, git_ref: &str, token: Option<&str>) -> Result<Option<String>, FetchError> {
        let url = format!(
            "{}/repos/{}/{}/commits/{}",
            self.api_base,
            encode_segment(owner),
            encode_segment(repo),
            encode_path(git_ref)
        );
        let raw = self.get(&url, "application/vnd.github.sha", token, 4096)?;
        // 422: "No commit found for SHA" (unknown ref).
        if raw.status == 404 || raw.status == 422 {
            return Ok(None);
        }
        let body = Self::check(raw)?;
        let sha = String::from_utf8_lossy(&body).trim().to_string();
        if sha.len() >= 7 && sha.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(Some(sha))
        } else {
            Err(FetchError::Network("unexpected GitHub response for the commit".into()))
        }
    }

    fn tree(&self, owner: &str, repo: &str, sha: &str, token: Option<&str>) -> Result<RepoTree, FetchError> {
        let body = self.api(
            &format!(
                "/repos/{}/{}/git/trees/{}?recursive=1",
                encode_segment(owner),
                encode_segment(repo),
                encode_segment(sha)
            ),
            token,
        )?;
        let t: TreeJson = serde_json::from_slice(&body).map_err(bad_json)?;
        Ok(RepoTree {
            truncated: t.truncated,
            entries: t
                .tree
                .into_iter()
                .map(|i| TreeEntry {
                    kind: match i.kind.as_str() {
                        "blob" => EntryKind::Blob,
                        "tree" => EntryKind::Tree,
                        _ => EntryKind::Other,
                    },
                    path: i.path,
                    size: i.size.unwrap_or(0),
                })
                .collect(),
        })
    }

    fn raw(&self, owner: &str, repo: &str, sha: &str, path: &str, limit: u64, token: Option<&str>) -> Result<Vec<u8>, FetchError> {
        let url = format!(
            "{}/{}/{}/{}/{}",
            self.raw_base,
            encode_segment(owner),
            encode_segment(repo),
            encode_segment(sha),
            encode_path(path)
        );
        Self::check(self.get(&url, "*/*", token, limit)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn encodes_paths() {
        assert_eq!(encode_path("a b/CLAUDE.md"), "a%20b/CLAUDE.md");
        assert_eq!(encode_segment("feature/x"), "feature%2Fx");
        assert_eq!(encode_path("中/x"), "%E4%B8%AD/x");
    }

    #[test]
    fn fallback_retries_without_token_on_401() {
        let calls = AtomicU32::new(0);
        let r = with_fallback(Some("tok"), |t| {
            calls.fetch_add(1, Ordering::SeqCst);
            if t.is_some() { Err(FetchError::Unauthorized) } else { Ok(1) }
        });
        assert_eq!(r, Ok(1));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        // No token: no retry.
        let r: Result<u8, _> = with_fallback(None, |_| Err(FetchError::Unauthorized));
        assert_eq!(r, Err(FetchError::Unauthorized));
        // Other errors: no retry.
        calls.store(0, Ordering::SeqCst);
        let r: Result<u8, _> = with_fallback(Some("t"), |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(FetchError::NotFound)
        });
        assert_eq!(r, Err(FetchError::NotFound));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn status_mapping() {
        let raw = |status, remaining_zero, body: &str| Raw { status, remaining_zero, body: body.as_bytes().to_vec() };
        assert_eq!(HttpFetcher::check(raw(200, false, "x")), Ok(b"x".to_vec()));
        assert_eq!(HttpFetcher::check(raw(404, false, "")), Err(FetchError::NotFound));
        assert_eq!(HttpFetcher::check(raw(401, false, "")), Err(FetchError::Unauthorized));
        assert_eq!(HttpFetcher::check(raw(403, true, "")), Err(FetchError::RateLimited));
        assert_eq!(
            HttpFetcher::check(raw(403, false, r#"{"message":"API rate limit exceeded for 1.2.3.4"}"#)),
            Err(FetchError::RateLimited)
        );
        assert_eq!(
            HttpFetcher::check(raw(500, false, r#"{"message":"boom"}"#)),
            Err(FetchError::Http { status: 500, message: "boom".into() })
        );
    }
}
