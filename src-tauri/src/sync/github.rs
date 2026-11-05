//! GitHub REST + Device Flow client (behind a trait so tests never touch the
//! network), the device-login polling loop and the MemDockHub bootstrap.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};

pub const CLIENT_ID: &str = match option_env!("MEMDOCK_GITHUB_CLIENT_ID") {
    Some(v) => v,
    None => "Ov23liN2y64SfCPmQRZG",
};

const MARKER_PATH: &str = "memdock.json";
const MARKER_BODY: &str = "{\"format\":1,\"app\":\"MemDock\"}\n";

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ApiError {
    Unauthorized,
    Network(String),
    Http { status: u16, message: String },
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Unauthorized => write!(f, "GitHub rejected the credentials (401)"),
            ApiError::Network(m) => write!(f, "network error: {m}"),
            ApiError::Http { status, message } => write!(f, "GitHub error {status}: {message}"),
        }
    }
}

impl std::error::Error for ApiError {}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_in: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub enum PollResult {
    Pending,
    /// GitHub may return a new minimum `interval`; `None` means "add 5 s".
    SlowDown(Option<u64>),
    Token(String),
    Expired,
    Denied,
    /// Any other OAuth error (device flow disabled, bad client id, ...).
    Failed(String),
}

impl fmt::Debug for PollResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PollResult::Pending => f.write_str("Pending"),
            PollResult::SlowDown(i) => write!(f, "SlowDown({i:?})"),
            PollResult::Token(_) => f.write_str("Token(***)"),
            PollResult::Expired => f.write_str("Expired"),
            PollResult::Denied => f.write_str("Denied"),
            PollResult::Failed(m) => write!(f, "Failed({m:?})"),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GhUser {
    pub login: String,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GhRepo {
    pub private: bool,
    pub default_branch: String,
    pub clone_url: String,
    pub description: Option<String>,
    /// `owner/name`; empty when unknown.
    pub full_name: String,
}

pub trait GitHubApi: Send + Sync {
    fn device_code(&self, client_id: &str) -> Result<DeviceCode, ApiError>;
    fn poll_token(&self, client_id: &str, device_code: &str) -> Result<PollResult, ApiError>;
    fn user(&self, token: &str) -> Result<GhUser, ApiError>;
    fn get_repo(&self, token: &str, owner: &str, name: &str) -> Result<Option<GhRepo>, ApiError>;
    fn create_repo(&self, token: &str, name: &str) -> Result<GhRepo, ApiError>;
    fn get_file(
        &self,
        token: &str,
        owner: &str,
        name: &str,
        path: &str,
    ) -> Result<Option<Vec<u8>>, ApiError>;
    fn put_file(
        &self,
        token: &str,
        owner: &str,
        name: &str,
        path: &str,
        bytes: &[u8],
        message: &str,
    ) -> Result<(), ApiError>;
    /// Names of the entries in the repository root (empty repo -> empty).
    fn list_root(&self, token: &str, owner: &str, name: &str) -> Result<Vec<String>, ApiError>;
}

// ---------------------------------------------------------------- HTTP impl

pub struct HttpGitHub {
    agent: ureq::Agent,
}

impl fmt::Debug for HttpGitHub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HttpGitHub { .. }")
    }
}

impl Default for HttpGitHub {
    fn default() -> Self {
        Self::new()
    }
}

struct Raw {
    status: u16,
    body: Vec<u8>,
}

impl HttpGitHub {
    pub fn new() -> Self {
        let proxy = crate::sync::net::resolve_proxy().and_then(|p| ureq::Proxy::new(&p).ok());
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .proxy(proxy)
            .build()
            .into();
        HttpGitHub { agent }
    }

    fn finish(
        res: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
        secret: Option<&str>,
    ) -> Result<Raw, ApiError> {
        let mut resp = res.map_err(|e| ApiError::Network(scrub(&e.to_string(), secret)))?;
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .read_to_vec()
            .map_err(|e| ApiError::Network(scrub(&e.to_string(), secret)))?;
        Ok(Raw { status, body })
    }

    fn get(&self, url: &str, token: &str, accept: &str) -> Result<Raw, ApiError> {
        let res = self
            .agent
            .get(url)
            .header("Accept", accept)
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "MemDock")
            .header("Authorization", &format!("Bearer {token}"))
            .call();
        Self::finish(res, Some(token))
    }

    fn send(&self, method: &str, url: &str, token: &str, body: &Value) -> Result<Raw, ApiError> {
        let auth = format!("Bearer {token}");
        let res = if method == "PUT" {
            self.agent.put(url).header("Authorization", &auth)
        } else {
            self.agent.post(url).header("Authorization", &auth)
        }
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "MemDock")
        .send_json(body);
        Self::finish(res, Some(token))
    }

    fn device_post(&self, url: &str, form: &[(&str, &str)]) -> Result<Raw, ApiError> {
        let res = self
            .agent
            .post(url)
            .header("Accept", "application/json")
            .header("User-Agent", "MemDock")
            .send_form(form.iter().copied());
        Self::finish(res, None)
    }
}

/// Removes a secret from a message (defense in depth; the token is only ever
/// sent in a header, never in URLs or bodies).
fn scrub(msg: &str, secret: Option<&str>) -> String {
    match secret {
        Some(s) if !s.is_empty() => msg.replace(s, "***"),
        _ => msg.to_string(),
    }
}

fn error_from(raw: &Raw, secret: Option<&str>) -> ApiError {
    if raw.status == 401 {
        return ApiError::Unauthorized;
    }
    let message = serde_json::from_slice::<Value>(&raw.body)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string))
        .unwrap_or_else(|| format!("HTTP {}", raw.status));
    ApiError::Http {
        status: raw.status,
        message: scrub(&message, secret),
    }
}

fn is_ok(status: u16) -> bool {
    (200..300).contains(&status)
}

fn parse_json(raw: &Raw) -> Result<Value, ApiError> {
    serde_json::from_slice(&raw.body)
        .map_err(|_| ApiError::Network("unexpected response from GitHub".into()))
}

fn parse_repo(v: &Value) -> Result<GhRepo, ApiError> {
    let bad = || ApiError::Network("unexpected repository response from GitHub".into());
    Ok(GhRepo {
        private: v.get("private").and_then(Value::as_bool).ok_or_else(bad)?,
        default_branch: v
            .get("default_branch")
            .and_then(Value::as_str)
            .unwrap_or("main")
            .to_string(),
        clone_url: v
            .get("clone_url")
            .and_then(Value::as_str)
            .ok_or_else(bad)?
            .to_string(),
        description: v
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        full_name: v
            .get("full_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

/// Percent-encodes one URL path segment.
fn enc(seg: &str) -> String {
    let mut out = String::new();
    for b in seg.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Percent-encodes each segment of a `/`-separated path.
fn enc_path(path: &str) -> String {
    path.split('/').map(enc).collect::<Vec<_>>().join("/")
}

fn repo_url(owner: &str, name: &str) -> String {
    format!("https://api.github.com/repos/{}/{}", enc(owner), enc(name))
}

#[derive(Deserialize)]
struct DeviceCodeResp {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    interval: Option<u64>,
    #[serde(default)]
    expires_in: Option<u64>,
}

impl GitHubApi for HttpGitHub {
    fn device_code(&self, client_id: &str) -> Result<DeviceCode, ApiError> {
        let raw = self.device_post(
            "https://github.com/login/device/code",
            &[("client_id", client_id), ("scope", "repo")],
        )?;
        if !is_ok(raw.status) {
            return Err(error_from(&raw, None));
        }
        let r: DeviceCodeResp = serde_json::from_slice(&raw.body)
            .map_err(|_| ApiError::Network("unexpected device-code response".into()))?;
        Ok(DeviceCode {
            device_code: r.device_code,
            user_code: r.user_code,
            verification_uri: r.verification_uri,
            interval: r.interval.unwrap_or(5),
            expires_in: r.expires_in.unwrap_or(900),
        })
    }

    fn poll_token(&self, client_id: &str, device_code: &str) -> Result<PollResult, ApiError> {
        let raw = self.device_post(
            "https://github.com/login/oauth/access_token",
            &[
                ("client_id", client_id),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ],
        )?;
        if !is_ok(raw.status) {
            return Err(error_from(&raw, None));
        }
        let v = parse_json(&raw)?;
        Ok(map_poll(&v))
    }

    fn user(&self, token: &str) -> Result<GhUser, ApiError> {
        let raw = self.get(
            "https://api.github.com/user",
            token,
            "application/vnd.github+json",
        )?;
        if !is_ok(raw.status) {
            return Err(error_from(&raw, Some(token)));
        }
        let v = parse_json(&raw)?;
        let login = v
            .get("login")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::Network("unexpected user response".into()))?;
        Ok(GhUser {
            login: login.to_string(),
        })
    }

    fn get_repo(&self, token: &str, owner: &str, name: &str) -> Result<Option<GhRepo>, ApiError> {
        let raw = self.get(
            &repo_url(owner, name),
            token,
            "application/vnd.github+json",
        )?;
        if raw.status == 404 {
            return Ok(None);
        }
        if !is_ok(raw.status) {
            return Err(error_from(&raw, Some(token)));
        }
        parse_repo(&parse_json(&raw)?).map(Some)
    }

    fn create_repo(&self, token: &str, name: &str) -> Result<GhRepo, ApiError> {
        let raw = self.send(
            "POST",
            "https://api.github.com/user/repos",
            token,
            &json!({
                "name": name,
                "private": true,
                "auto_init": true,
                "description": "MemDock memory sync",
            }),
        )?;
        if !is_ok(raw.status) {
            return Err(error_from(&raw, Some(token)));
        }
        parse_repo(&parse_json(&raw)?)
    }

    fn get_file(
        &self,
        token: &str,
        owner: &str,
        name: &str,
        path: &str,
    ) -> Result<Option<Vec<u8>>, ApiError> {
        let raw = self.get(
            &format!("{}/contents/{}", repo_url(owner, name), enc_path(path)),
            token,
            "application/vnd.github.raw+json",
        )?;
        if raw.status == 404 {
            return Ok(None);
        }
        if !is_ok(raw.status) {
            return Err(error_from(&raw, Some(token)));
        }
        Ok(Some(raw.body))
    }

    fn put_file(
        &self,
        token: &str,
        owner: &str,
        name: &str,
        path: &str,
        bytes: &[u8],
        message: &str,
    ) -> Result<(), ApiError> {
        let url = format!("{}/contents/{}", repo_url(owner, name), enc_path(path));
        // An existing file needs its blob sha to be replaced.
        let existing = self.get(&url, token, "application/vnd.github+json")?;
        let mut body = json!({
            "message": message,
            "content": base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        if is_ok(existing.status) {
            if let Some(sha) = parse_json(&existing)?.get("sha").and_then(Value::as_str) {
                body["sha"] = Value::String(sha.to_string());
            }
        } else if existing.status != 404 {
            return Err(error_from(&existing, Some(token)));
        }
        let raw = self.send("PUT", &url, token, &body)?;
        if !is_ok(raw.status) {
            return Err(error_from(&raw, Some(token)));
        }
        Ok(())
    }

    fn list_root(&self, token: &str, owner: &str, name: &str) -> Result<Vec<String>, ApiError> {
        let raw = self.get(
            &format!("{}/contents", repo_url(owner, name)),
            token,
            "application/vnd.github+json",
        )?;
        // 404 / 409: empty repository.
        if raw.status == 404 || raw.status == 409 {
            return Ok(Vec::new());
        }
        if !is_ok(raw.status) {
            return Err(error_from(&raw, Some(token)));
        }
        let v = parse_json(&raw)?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|e| e.get("name").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }
}

fn map_poll(v: &Value) -> PollResult {
    if let Some(t) = v.get("access_token").and_then(Value::as_str) {
        return PollResult::Token(t.to_string());
    }
    match v.get("error").and_then(Value::as_str) {
        Some("slow_down") => PollResult::SlowDown(v.get("interval").and_then(Value::as_u64)),
        Some("expired_token") => PollResult::Expired,
        Some("access_denied") => PollResult::Denied,
        Some("authorization_pending") => PollResult::Pending,
        Some(other) => {
            let desc = v
                .get("error_description")
                .and_then(Value::as_str)
                .filter(|d| !d.is_empty())
                .unwrap_or(other);
            PollResult::Failed(desc.to_string())
        }
        None => PollResult::Failed("unexpected response from GitHub".to_string()),
    }
}

// ------------------------------------------------------------- device login

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LoginError {
    Expired,
    Denied,
    Cancelled,
    Failed(String),
    Api(ApiError),
}

impl fmt::Display for LoginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoginError::Expired => write!(f, "the login code expired"),
            LoginError::Denied => write!(f, "authorization was denied"),
            LoginError::Cancelled => write!(f, "login cancelled"),
            LoginError::Failed(m) => write!(f, "login failed: {m}"),
            LoginError::Api(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LoginError {}

impl From<ApiError> for LoginError {
    fn from(e: ApiError) -> Self {
        LoginError::Api(e)
    }
}

/// Polls until the user authorizes. Polls first, then sleeps `interval`;
/// `SlowDown` raises the interval (GitHub's value, else +5 s). The deadline is
/// the sum of slept durations vs `expires_in`.
pub fn device_login(
    api: &dyn GitHubApi,
    client_id: &str,
    code: &DeviceCode,
    cancel: &AtomicBool,
    sleep: &dyn Fn(Duration),
) -> Result<String, LoginError> {
    let mut interval = code.interval;
    let mut elapsed: u64 = 0;
    let started = Instant::now();
    let mut net_errors = 0u32;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(LoginError::Cancelled);
        }
        match api.poll_token(client_id, &code.device_code) {
            Ok(PollResult::Token(t)) => return Ok(t),
            Ok(PollResult::Expired) => return Err(LoginError::Expired),
            Ok(PollResult::Denied) => return Err(LoginError::Denied),
            Ok(PollResult::Failed(m)) => return Err(LoginError::Failed(m)),
            Ok(PollResult::Pending) => net_errors = 0,
            Ok(PollResult::SlowDown(new)) => {
                net_errors = 0;
                interval = match new {
                    Some(n) if n > interval => n,
                    _ => interval + 5,
                };
            }
            // Tolerate a few consecutive transient network errors.
            Err(ApiError::Network(_)) if net_errors < 3 => net_errors += 1,
            Err(e) => return Err(e.into()),
        }
        sleep(Duration::from_secs(interval));
        elapsed += interval;
        if cancel.load(Ordering::SeqCst) {
            return Err(LoginError::Cancelled);
        }
        // Summed sleeps (deterministic in tests) or real time, whichever is larger.
        if elapsed.max(started.elapsed().as_secs()) >= code.expires_in {
            return Err(LoginError::Expired);
        }
    }
}

// ------------------------------------------------------------- repo bootstrap

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RepoOutcome {
    Ready(GhRepo),
    Created(GhRepo),
    NotOurs,
    PublicNeedsAck(GhRepo),
}

fn is_marker(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .map(|v| {
            v.get("app").and_then(Value::as_str) == Some("MemDock")
                && v.get("format").and_then(Value::as_u64).is_some_and(|f| f >= 1)
        })
        .unwrap_or(false)
}

const PUT_ATTEMPTS: u32 = 3;
const CREATED_DESCRIPTION: &str = "MemDock memory sync";

fn put_marker(
    api: &dyn GitHubApi,
    token: &str,
    owner: &str,
    name: &str,
    sleep: &dyn Fn(Duration),
) -> Result<(), ApiError> {
    let mut last = None;
    for attempt in 0..PUT_ATTEMPTS {
        if attempt > 0 {
            // Right after auto_init the contents API can briefly 404.
            sleep(Duration::from_secs(u64::from(attempt)));
        }
        match api.put_file(
            token,
            owner,
            name,
            MARKER_PATH,
            MARKER_BODY.as_bytes(),
            "Initialize MemDock sync repository",
        ) {
            Ok(()) => return Ok(()),
            Err(ApiError::Unauthorized) => return Err(ApiError::Unauthorized),
            Err(e) => last = Some(e),
        }
    }
    Err(last.expect("at least one attempt"))
}

pub fn ensure_repo(
    api: &dyn GitHubApi,
    token: &str,
    owner: &str,
    name: &str,
    public_ack: bool,
) -> Result<RepoOutcome, ApiError> {
    ensure_repo_with(api, token, owner, name, public_ack, &std::thread::sleep)
}

pub fn ensure_repo_with(
    api: &dyn GitHubApi,
    token: &str,
    owner: &str,
    name: &str,
    public_ack: bool,
    sleep: &dyn Fn(Duration),
) -> Result<RepoOutcome, ApiError> {
    let Some(repo) = api.get_repo(token, owner, name)? else {
        let repo = api.create_repo(token, name)?;
        // Use the owner GitHub reports for the new repository.
        let real_owner = repo
            .full_name
            .split_once('/')
            .map(|(o, _)| o.to_string())
            .filter(|o| !o.is_empty())
            .unwrap_or_else(|| owner.to_string());
        let real_name = repo
            .full_name
            .split_once('/')
            .map(|(_, n)| n.to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| name.to_string());
        put_marker(api, token, &real_owner, &real_name, sleep)?;
        return Ok(RepoOutcome::Created(repo));
    };
    match api.get_file(token, owner, name, MARKER_PATH)? {
        Some(b) if is_marker(&b) => {
            if !repo.private && !public_ack {
                Ok(RepoOutcome::PublicNeedsAck(repo))
            } else {
                Ok(RepoOutcome::Ready(repo))
            }
        }
        _ => {
            // A repo we created whose marker write failed: private, our
            // description and nothing but the auto_init README.
            let unfinished = repo.private
                && repo.description.as_deref() == Some(CREATED_DESCRIPTION)
                && api
                    .list_root(token, owner, name)?
                    .iter()
                    .all(|n| n == "README.md");
            if unfinished {
                put_marker(api, token, owner, name, sleep)?;
                Ok(RepoOutcome::Ready(repo))
            } else {
                Ok(RepoOutcome::NotOurs)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeGitHub {
        polls: Mutex<VecDeque<Result<PollResult, ApiError>>>,
        repo: Mutex<Option<GhRepo>>,
        marker: Mutex<Option<Vec<u8>>>,
        calls: Mutex<Vec<String>>,
        puts: Mutex<Vec<(String, Vec<u8>)>>,
        put_failures: Mutex<u32>,
        put_owners: Mutex<Vec<String>>,
    }

    fn repo(private: bool) -> GhRepo {
        GhRepo {
            private,
            default_branch: "main".into(),
            clone_url: "https://github.com/o/MemDockHub.git".into(),
            description: Some("MemDock memory sync".into()),
            full_name: "real/MemDockHub".into(),
        }
    }

    impl FakeGitHub {
        fn with_polls(p: Vec<PollResult>) -> Self {
            let f = FakeGitHub::default();
            *f.polls.lock().unwrap() = p.into_iter().map(Ok).collect();
            f
        }
        fn with_repo(r: Option<GhRepo>, marker: Option<&str>) -> Self {
            let f = FakeGitHub::default();
            *f.repo.lock().unwrap() = r;
            *f.marker.lock().unwrap() = marker.map(|m| m.as_bytes().to_vec());
            f
        }
        fn log(&self, s: &str) {
            self.calls.lock().unwrap().push(s.to_string());
        }
    }

    impl GitHubApi for FakeGitHub {
        fn device_code(&self, _: &str) -> Result<DeviceCode, ApiError> {
            unreachable!()
        }
        fn poll_token(&self, _: &str, _: &str) -> Result<PollResult, ApiError> {
            self.log("poll");
            self.polls.lock().unwrap().pop_front().expect("script exhausted")
        }
        fn user(&self, _: &str) -> Result<GhUser, ApiError> {
            unreachable!()
        }
        fn get_repo(&self, _: &str, _: &str, _: &str) -> Result<Option<GhRepo>, ApiError> {
            self.log("get_repo");
            Ok(self.repo.lock().unwrap().clone())
        }
        fn create_repo(&self, _: &str, _: &str) -> Result<GhRepo, ApiError> {
            self.log("create_repo");
            let r = repo(true);
            *self.repo.lock().unwrap() = Some(r.clone());
            Ok(r)
        }
        fn get_file(&self, _: &str, _: &str, _: &str, p: &str) -> Result<Option<Vec<u8>>, ApiError> {
            self.log(&format!("get_file {p}"));
            Ok(self.marker.lock().unwrap().clone())
        }
        fn put_file(
            &self,
            _: &str,
            o: &str,
            _: &str,
            p: &str,
            b: &[u8],
            _: &str,
        ) -> Result<(), ApiError> {
            self.log(&format!("put_file {p}"));
            self.put_owners.lock().unwrap().push(o.to_string());
            {
                let mut f = self.put_failures.lock().unwrap();
                if *f > 0 {
                    *f -= 1;
                    return Err(ApiError::Http {
                        status: 404,
                        message: "Not Found".into(),
                    });
                }
            }
            self.puts.lock().unwrap().push((p.to_string(), b.to_vec()));
            *self.marker.lock().unwrap() = Some(b.to_vec());
            Ok(())
        }
        fn list_root(&self, _: &str, _: &str, _: &str) -> Result<Vec<String>, ApiError> {
            self.log("list_root");
            Ok(if self.marker.lock().unwrap().is_some() {
                vec!["README.md".into(), "memdock.json".into()]
            } else {
                vec!["README.md".into()]
            })
        }
    }

    fn code(interval: u64, expires_in: u64) -> DeviceCode {
        DeviceCode {
            device_code: "dc".into(),
            user_code: "UC".into(),
            verification_uri: "https://github.com/login/device".into(),
            interval,
            expires_in,
        }
    }

    const MARKER: &str = r#"{"format":1,"app":"MemDock"}"#;

    #[test]
    fn device_login_handles_pending_slowdown_then_token() {
        let api = FakeGitHub::with_polls(vec![
            PollResult::Pending,
            PollResult::SlowDown(None),
            PollResult::Token("tok".into()),
        ]);
        let slept = RefCell::new(Vec::new());
        let cancel = AtomicBool::new(false);
        let sleep = |d: Duration| slept.borrow_mut().push(d.as_secs());
        let t = device_login(&api, "cid", &code(5, 900), &cancel, &sleep).unwrap();
        assert_eq!(t, "tok");
        assert_eq!(*slept.borrow(), vec![5, 10]);
    }

    #[test]
    fn device_login_fails_immediately_on_other_oauth_errors() {
        let api = FakeGitHub::with_polls(vec![
            PollResult::Failed("Device Flow must be explicitly enabled".into()),
            PollResult::Token("never".into()),
        ]);
        let cancel = AtomicBool::new(false);
        let slept = RefCell::new(0u32);
        let sleep = |_: Duration| *slept.borrow_mut() += 1;
        let r = device_login(&api, "c", &code(5, 900), &cancel, &sleep);
        assert!(matches!(r, Err(LoginError::Failed(ref m)) if m.contains("Device Flow")));
        assert_eq!(api.calls.lock().unwrap().len(), 1);
        assert_eq!(*slept.borrow(), 0);
    }

    #[test]
    fn device_login_tolerates_three_network_errors() {
        let net = || Err(ApiError::Network("down".into()));
        let mk = |n: usize| {
            let f = FakeGitHub::default();
            let mut q: VecDeque<Result<PollResult, ApiError>> = (0..n).map(|_| net()).collect();
            q.push_back(Ok(PollResult::Token("t".into())));
            *f.polls.lock().unwrap() = q;
            f
        };
        let cancel = AtomicBool::new(false);
        let noop = |_: Duration| {};
        assert_eq!(
            device_login(&mk(3), "c", &code(5, 900), &cancel, &noop),
            Ok("t".to_string())
        );
        assert!(matches!(
            device_login(&mk(4), "c", &code(5, 900), &cancel, &noop),
            Err(LoginError::Api(ApiError::Network(_)))
        ));
    }

    #[test]
    fn debug_hides_token_and_urls_are_encoded() {
        assert_eq!(format!("{:?}", PollResult::Token("gho_x".into())), "Token(***)");
        assert_eq!(enc("a b/c"), "a%20b%2Fc");
        assert_eq!(enc_path("dir/a b.json"), "dir/a%20b.json");
    }

    #[test]
    fn device_login_honors_returned_interval() {
        let api = FakeGitHub::with_polls(vec![
            PollResult::SlowDown(Some(20)),
            PollResult::Token("t".into()),
        ]);
        let slept = RefCell::new(Vec::new());
        let cancel = AtomicBool::new(false);
        let sleep = |d: Duration| slept.borrow_mut().push(d.as_secs());
        device_login(&api, "cid", &code(5, 900), &cancel, &sleep).unwrap();
        assert_eq!(*slept.borrow(), vec![20]);
    }

    #[test]
    fn device_login_expired_and_cancel() {
        let cancel = AtomicBool::new(false);
        let noop = |_: Duration| {};

        // Server says expired.
        let api = FakeGitHub::with_polls(vec![PollResult::Expired]);
        assert_eq!(
            device_login(&api, "c", &code(5, 900), &cancel, &noop),
            Err(LoginError::Expired)
        );
        // Denied.
        let api = FakeGitHub::with_polls(vec![PollResult::Denied]);
        assert_eq!(
            device_login(&api, "c", &code(5, 900), &cancel, &noop),
            Err(LoginError::Denied)
        );
        // Deadline by summed sleeps: 5 + 5 >= 10 -> Expired after 2 polls.
        let api = FakeGitHub::with_polls(vec![PollResult::Pending, PollResult::Pending]);
        assert_eq!(
            device_login(&api, "c", &code(5, 10), &cancel, &noop),
            Err(LoginError::Expired)
        );
        assert_eq!(api.calls.lock().unwrap().len(), 2);
        // Cancel before first poll: no poll at all.
        let api = FakeGitHub::with_polls(vec![]);
        let pre = AtomicBool::new(true);
        assert_eq!(
            device_login(&api, "c", &code(5, 900), &pre, &noop),
            Err(LoginError::Cancelled)
        );
        assert!(api.calls.lock().unwrap().is_empty());
        // Cancel during sleep: no further poll.
        let api = FakeGitHub::with_polls(vec![PollResult::Pending]);
        let c2 = AtomicBool::new(false);
        let sleep = |_: Duration| c2.store(true, Ordering::SeqCst);
        assert_eq!(
            device_login(&api, "c", &code(5, 900), &c2, &sleep),
            Err(LoginError::Cancelled)
        );
        assert_eq!(api.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn ensure_repo_creates_private_with_marker() {
        let api = FakeGitHub::with_repo(None, None);
        let out = ensure_repo(&api, "t", "o", "MemDockHub", false).unwrap();
        assert!(matches!(out, RepoOutcome::Created(ref r) if r.private));
        let puts = api.puts.lock().unwrap();
        assert_eq!(puts.len(), 1);
        assert_eq!(puts[0].0, "memdock.json");
        assert!(is_marker(&puts[0].1));
        assert_eq!(
            api.calls.lock().unwrap().as_slice(),
            ["get_repo", "create_repo", "put_file memdock.json"]
        );
    }

    #[test]
    fn ensure_repo_retries_marker_write_and_uses_returned_owner() {
        let api = FakeGitHub::with_repo(None, None);
        *api.put_failures.lock().unwrap() = 1;
        let slept = RefCell::new(0u32);
        let sleep = |_: Duration| *slept.borrow_mut() += 1;
        let out = ensure_repo_with(&api, "t", "o", "MemDockHub", false, &sleep).unwrap();
        assert!(matches!(out, RepoOutcome::Created(_)));
        assert_eq!(*slept.borrow(), 1);
        assert_eq!(api.puts.lock().unwrap().len(), 1);
        assert!(api.put_owners.lock().unwrap().iter().all(|o| o == "real"));
    }

    #[test]
    fn ensure_repo_recovers_unfinished_repo_after_failed_marker() {
        let api = FakeGitHub::with_repo(None, None);
        *api.put_failures.lock().unwrap() = 3;
        let sleep = |_: Duration| {};
        let err = ensure_repo_with(&api, "t", "o", "MemDockHub", false, &sleep);
        assert!(matches!(err, Err(ApiError::Http { status: 404, .. })));
        assert_eq!(api.put_owners.lock().unwrap().len(), 3);
        // Second run: repo exists, no marker, ours-unfinished -> marker written.
        let out = ensure_repo_with(&api, "t", "o", "MemDockHub", false, &sleep).unwrap();
        assert!(matches!(out, RepoOutcome::Ready(_)));
        assert_eq!(api.puts.lock().unwrap().len(), 1);
    }

    #[test]
    fn ensure_repo_unfinished_rule_is_strict() {
        let sleep = |_: Duration| {};
        // Public, other description, or extra files: NotOurs and no writes.
        let mut other = repo(true);
        other.description = Some("something".into());
        for r in [repo(false), other] {
            let api = FakeGitHub::with_repo(Some(r), None);
            let out = ensure_repo_with(&api, "t", "o", "MemDockHub", true, &sleep).unwrap();
            assert_eq!(out, RepoOutcome::NotOurs);
            assert!(api.puts.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn ensure_repo_attaches_existing() {
        let api = FakeGitHub::with_repo(Some(repo(true)), Some(MARKER));
        let out = ensure_repo(&api, "t", "o", "MemDockHub", false).unwrap();
        assert_eq!(out, RepoOutcome::Ready(repo(true)));
        assert!(api.puts.lock().unwrap().is_empty());
    }

    #[test]
    fn ensure_repo_not_ours_writes_nothing() {
        for marker in [None, Some("garbage"), Some(r#"{"format":1,"app":"Other"}"#)] {
            let api = FakeGitHub::with_repo(Some(repo(false)), marker);
            let out = ensure_repo(&api, "t", "o", "MemDockHub", true).unwrap();
            assert_eq!(out, RepoOutcome::NotOurs);
            assert!(api.puts.lock().unwrap().is_empty());
            assert!(!api.calls.lock().unwrap().iter().any(|c| c == "create_repo"));
        }
    }

    #[test]
    fn ensure_repo_public_needs_ack() {
        let api = FakeGitHub::with_repo(Some(repo(false)), Some(MARKER));
        assert_eq!(
            ensure_repo(&api, "t", "o", "MemDockHub", false).unwrap(),
            RepoOutcome::PublicNeedsAck(repo(false))
        );
        assert_eq!(
            ensure_repo(&api, "t", "o", "MemDockHub", true).unwrap(),
            RepoOutcome::Ready(repo(false))
        );
    }

    #[test]
    fn poll_response_mapping() {
        assert_eq!(map_poll(&json!({"access_token":"x"})), PollResult::Token("x".into()));
        assert_eq!(map_poll(&json!({"error":"authorization_pending"})), PollResult::Pending);
        assert_eq!(map_poll(&json!({"error":"slow_down"})), PollResult::SlowDown(None));
        assert_eq!(
            map_poll(&json!({"error":"slow_down","interval":15})),
            PollResult::SlowDown(Some(15))
        );
        assert_eq!(map_poll(&json!({"error":"expired_token"})), PollResult::Expired);
        assert_eq!(
            map_poll(&json!({"error":"device_flow_disabled","error_description":"off"})),
            PollResult::Failed("off".into())
        );
        assert_eq!(
            map_poll(&json!({"error":"incorrect_client_credentials"})),
            PollResult::Failed("incorrect_client_credentials".into())
        );
        assert!(matches!(map_poll(&json!({})), PollResult::Failed(_)));
        assert_eq!(map_poll(&json!({"error":"access_denied"})), PollResult::Denied);
    }

    #[test]
    fn errors_never_contain_token() {
        let token = "gho_SECRET_TOKEN_123";
        let raw = Raw {
            status: 403,
            body: format!(r#"{{"message":"bad credentials {token}"}}"#).into_bytes(),
        };
        let e = error_from(&raw, Some(token));
        assert!(!format!("{e}").contains(token));
        assert!(!format!("{e:?}").contains(token));
        assert_eq!(error_from(&Raw { status: 401, body: vec![] }, Some(token)), ApiError::Unauthorized);
        let net = ApiError::Network(scrub(&format!("failed with {token}"), Some(token)));
        assert!(!format!("{net} {net:?}").contains(token));
        // The client itself holds no token.
        let http = HttpGitHub::new();
        assert!(!format!("{http:?}").contains(token));
    }
}
