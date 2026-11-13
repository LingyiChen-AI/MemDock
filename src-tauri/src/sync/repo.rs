//! Git layer for the sync repository: clone, fetch, read files at a commit,
//! build commits without touching the work tree, push, 3-way merge and log.
//!
//! The access token is only ever passed through environment variables
//! (`GIT_CONFIG_*`), never on the command line or in `.git/config`.
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use base64::Engine;
use serde::Serialize;
use sha1::{Digest, Sha1};

use crate::fsutil::tmp_sibling;
use crate::proc::WaitError;
use crate::skills::git::redact;

/// Access token for github.com; `None` for local `file://` remotes (tests).
pub type Auth = Option<String>;

/// Failure modes of a sync-repo git invocation.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncGitError {
    NotInstalled,
    Timeout,
    /// Push refused because the remote branch is ahead (non-fast-forward).
    Rejected,
    /// The remote refused the credentials (401 / 403 / authentication failed).
    Unauthorized,
    Failed { stderr: String },
}

fn failed(msg: impl AsRef<str>) -> SyncGitError {
    // Mask `Authorization: basic <b64>` headers (e.g. from GIT_CURL_VERBOSE output).
    let re = regex::Regex::new(r"(?i)(authorization:\s*basic\s+)\S+").expect("static regex");
    let masked = re.replace_all(msg.as_ref(), "${1}***");
    SyncGitError::Failed {
        stderr: redact(&masked),
    }
}

/// Rejects arguments that could be read as options or break argument parsing.
fn check_arg(s: &str) -> Result<(), SyncGitError> {
    if s.is_empty() || s.starts_with('-') || s.chars().any(|c| c == '\0' || c.is_whitespace()) {
        return Err(failed("invalid argument"));
    }
    Ok(())
}

/// Remote URL: may contain spaces (local paths) but no option-like start or control breaks.
fn check_url(s: &str) -> Result<(), SyncGitError> {
    if s.is_empty() || s.starts_with('-') || s.chars().any(|c| matches!(c, '\0' | '\n' | '\r')) {
        return Err(failed("invalid argument"));
    }
    Ok(())
}

/// Git config keys read from the user's real global config and forwarded as
/// `-c` so proxy / TLS settings keep working while everything else is isolated.
const FORWARDED_KEYS: [&str; 5] = [
    "http.proxy",
    "https.proxy",
    "http.https://github.com.proxy",
    "http.sslCAInfo",
    "http.sslBackend",
];

/// Turns `(key, value)` pairs into `-c key=value` args, skipping empty or
/// multi-line values.
fn forward_args(values: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for (k, v) in values {
        let v = v.trim();
        if v.is_empty() || v.contains(['\n', '\r', '\0']) {
            continue;
        }
        out.push("-c".to_string());
        out.push(format!("{k}={v}"));
    }
    out
}

/// Repository-relative path used as a tree entry.
fn check_repo_path(p: &str) -> Result<(), SyncGitError> {
    let bad = p.is_empty()
        || p.starts_with('/')
        || p.contains('\0')
        || p.contains('\\')
        || p.split('/').any(|c| c.is_empty() || c == "." || c == ".." || c == ".git");
    if bad {
        return Err(failed(format!("invalid repository path: {p}")));
    }
    Ok(())
}

fn classify(stderr: &str) -> SyncGitError {
    let low = stderr.to_lowercase();
    // `[remote rejected]` (push protection, rules, hooks) is a real failure, not a race.
    if !low.contains("[remote rejected]")
        && (low.contains("non-fast-forward")
            || low.contains("fetch first")
            || low.contains("[rejected]"))
    {
        return SyncGitError::Rejected;
    }
    // Only a refused credential counts. 403 (rate limits, repository rules, proxies) and
    // "no credentials given" (`terminal prompts disabled`) are ordinary failures: the token
    // may still be fine, and treating them as a logout throws a valid login away.
    if low.contains("authentication failed")
        || low.contains("invalid username or password")
        || low.contains("returned error: 401")
        || low.contains("error: 401")
        || low.contains("invalid credentials")
    {
        return SyncGitError::Unauthorized;
    }
    failed(stderr.trim())
}

#[cfg(windows)]
const NULL_DEVICE: &str = "NUL";
#[cfg(not(windows))]
const NULL_DEVICE: &str = "/dev/null";

/// Result of a 3-way text merge.
#[derive(Debug, Clone, PartialEq)]
pub struct Merge3 {
    pub text: String,
    pub conflicts: usize,
}

/// One commit of the sync repository.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommitInfo {
    pub id: String,
    pub message: String,
    /// Commit time in milliseconds since the epoch.
    pub time: i64,
    pub files: Vec<String>,
}

/// Runs the real `git` executable against the sync repository.
pub struct SyncGit {
    pub program: PathBuf,
    pub timeout: Duration,
    /// Proxy / TLS settings read once from the user's real global git config.
    forwarded: OnceLock<Vec<String>>,
    /// Number of git processes started through `exec` (diagnostics / tests).
    spawned: AtomicUsize,
}

/// One entry of a tree as `ls-tree` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// `100644`, `100755`, `120000` (symlink) or `160000` (gitlink).
    pub mode: String,
    /// `blob` or `commit` (gitlink).
    pub kind: String,
    pub sha: String,
}

impl TreeEntry {
    /// A plain or executable file (what a memory file can be compared with).
    pub fn is_regular(&self) -> bool {
        self.kind == "blob" && (self.mode == "100644" || self.mode == "100755")
    }

    fn valid(&self) -> bool {
        is_hex_id(&self.sha)
            && matches!(
                (self.mode.as_str(), self.kind.as_str()),
                ("100644" | "100755" | "120000", "blob") | ("160000", "commit")
            )
    }
}

/// One file of a tree to commit: an existing entry (mode kept) or new
/// content (written as `100644`).
#[derive(Debug, Clone, PartialEq)]
pub enum TreeBlob {
    Existing(TreeEntry),
    Data(Vec<u8>),
}

/// Git blob id of `data`, as `hash-object` would compute it.
pub fn blob_id(data: &[u8]) -> String {
    blob_sha(data)
}

fn is_hex_id(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Git blob id of `data` (SHA-1 of `blob <len>\0` + bytes).
fn blob_sha(data: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(format!("blob {}\0", data.len()).as_bytes());
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

impl Default for SyncGit {
    fn default() -> Self {
        Self::with_program(PathBuf::from("git"))
    }
}

impl SyncGit {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_program(program: PathBuf) -> Self {
        SyncGit {
            program,
            timeout: Duration::from_secs(60),
            forwarded: OnceLock::new(),
            spawned: AtomicUsize::new(0),
        }
    }

    /// Git processes started so far by this instance (excluding the one-off
    /// global config probe).
    pub fn spawn_count(&self) -> usize {
        self.spawned.load(Ordering::Relaxed)
    }

    /// `-c key=value` args for the user's proxy / TLS settings (cached).
    fn forwarded_args(&self) -> &[String] {
        self.forwarded.get_or_init(|| {
            let mut vals = Vec::new();
            for key in FORWARDED_KEYS {
                let mut cmd = Command::new(&self.program);
                cmd.args(["config", "--global", "--get", key])
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null());
                let Ok(child) = crate::proc::spawn_hidden(&mut cmd) else {
                    break;
                };
                if let Ok(out) =
                    crate::proc::wait_with_timeout(child, Duration::from_secs(5), None)
                {
                    if out.status.success() {
                        vals.push((
                            key.to_string(),
                            String::from_utf8_lossy(&out.stdout).trim().to_string(),
                        ));
                    }
                }
            }
            let has_proxy = vals.iter().any(|(k, v)| k.ends_with("proxy") && !v.trim().is_empty());
            if !has_proxy {
                if let Some(p) = crate::sync::net::resolve_proxy_for_git() {
                    vals.push(("http.proxy".to_string(), p));
                }
            }
            forward_args(&vals)
        })
    }

    /// Spawns git with the hardened base configuration and returns the raw
    /// output regardless of the exit status.
    fn exec(
        &self,
        cwd: Option<&Path>,
        args: &[&OsStr],
        auth: &Auth,
        env: &[(&str, &OsStr)],
        stdin: Option<Vec<u8>>,
    ) -> Result<Output, SyncGitError> {
        let mut cmd = Command::new(&self.program);
        cmd.args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.quotepath=false",
            "-c",
        ])
        .arg(format!("core.hooksPath={NULL_DEVICE}"))
        // Empty value clears the helper list so GCM never sees (or erases) the
        // user's own stored credentials.
        .args(["-c", "credential.helper="])
        .args(self.forwarded_args())
        .args(args)
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .env_remove("GIT_TRACE")
        .env_remove("GIT_TRACE_CURL")
        .env_remove("GIT_TRACE_PACKET")
        .env_remove("GIT_CURL_VERBOSE")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_NAMESPACE")
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        if let Some(token) = auth {
            let b64 = base64::engine::general_purpose::STANDARD
                .encode(format!("x-access-token:{token}"));
            cmd.env("GIT_CONFIG_COUNT", "1")
                .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraheader")
                .env("GIT_CONFIG_VALUE_0", format!("AUTHORIZATION: basic {b64}"));
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        if let Some(c) = cwd {
            cmd.current_dir(c);
        }
        self.spawned.fetch_add(1, Ordering::Relaxed);
        let mut child = crate::proc::spawn_hidden(&mut cmd).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                SyncGitError::NotInstalled
            } else {
                failed(e.to_string())
            }
        })?;
        if let Some(data) = stdin {
            if let Some(mut pipe) = child.stdin.take() {
                std::thread::spawn(move || {
                    let _ = pipe.write_all(&data);
                });
            }
        }
        crate::proc::wait_with_timeout(child, self.timeout, None).map_err(|e| match e {
            WaitError::Timeout | WaitError::Cancelled => SyncGitError::Timeout,
            WaitError::Io(m) => failed(m),
        })
    }

    /// Runs a read-only git command, returning the output whatever the status.
    fn probe(&self, repo: &Path, args: &[&str]) -> Result<Output, SyncGitError> {
        let os: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        self.exec(Some(repo), &os, &None, &[], None)
    }

    /// Runs git expecting success; returns raw stdout.
    fn run_bytes(
        &self,
        cwd: Option<&Path>,
        args: &[&str],
        auth: &Auth,
        env: &[(&str, &OsStr)],
        stdin: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, SyncGitError> {
        let os: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        let out = self.exec(cwd, &os, auth, env, stdin)?;
        if out.status.success() {
            Ok(out.stdout)
        } else {
            Err(classify(&String::from_utf8_lossy(&out.stderr)))
        }
    }

    fn run(&self, cwd: &Path, args: &[&str], auth: &Auth) -> Result<String, SyncGitError> {
        let b = self.run_bytes(Some(cwd), args, auth, &[], None)?;
        Ok(String::from_utf8_lossy(&b).trim().to_string())
    }

    /// Clones `url` into `dest` (via a temporary sibling directory). Empty
    /// remote repositories are fine.
    pub fn clone(&self, url: &str, dest: &Path, auth: &Auth) -> Result<(), SyncGitError> {
        check_url(url)?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| failed(e.to_string()))?;
        }
        let tmp = tmp_sibling(dest);
        let result = (|| {
            let tmp_s = tmp.to_string_lossy().into_owned();
            self.run_bytes(None, &["clone", "-q", "--", url, &tmp_s], auth, &[], None)?;
            if dest.exists() {
                // Move the old clone aside first so a failed swap can be undone.
                let old = tmp_sibling(dest);
                std::fs::rename(dest, &old).map_err(|e| failed(e.to_string()))?;
                if let Err(e) = std::fs::rename(&tmp, dest) {
                    let _ = std::fs::rename(&old, dest);
                    return Err(failed(e.to_string()));
                }
                let _ = std::fs::remove_dir_all(&old);
                Ok(())
            } else {
                std::fs::rename(&tmp, dest).map_err(|e| failed(e.to_string()))
            }
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&tmp);
        }
        result
    }

    /// Fetches `origin`; returns the commit of `origin/<default branch>`, or
    /// `None` while the remote branch does not exist yet.
    pub fn fetch(&self, repo: &Path, auth: &Auth) -> Result<Option<String>, SyncGitError> {
        self.run(repo, &["fetch", "-q", "--prune", "--", "origin"], auth)?;
        // Learn the remote's default branch (no-op / harmless failure on an empty remote).
        let _ = self.run(repo, &["remote", "set-head", "origin", "-a"], auth);
        let branch = self.default_branch(repo)?;
        let spec = format!("refs/remotes/origin/{branch}^{{commit}}");
        let out = self.probe(repo, &["rev-parse", "--verify", "-q", spec.as_str()])?;
        let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
        Ok((out.status.success() && !id.is_empty()).then_some(id))
    }

    /// Default branch from `refs/remotes/origin/HEAD`, `main` when unknown.
    pub fn default_branch(&self, repo: &Path) -> Result<String, SyncGitError> {
        let out = self.probe(repo, &["symbolic-ref", "-q", "--short", "refs/remotes/origin/HEAD"])?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let branch = if out.status.success() {
            s.strip_prefix("origin/").unwrap_or(&s).to_string()
        } else {
            String::new()
        };
        Ok(if branch.is_empty() { "main".into() } else { branch })
    }

    /// File content at `commit`, `None` when the path does not exist there.
    pub fn read_at(
        &self,
        repo: &Path,
        commit: &str,
        path: &str,
    ) -> Result<Option<Vec<u8>>, SyncGitError> {
        check_arg(commit)?;
        check_repo_path(path)?;
        let commit_spec = format!("{commit}^{{commit}}");
        let c = self.probe(repo, &["rev-parse", "--verify", "-q", &commit_spec])?;
        if !c.status.success() {
            return Err(failed(format!("unknown commit: {commit}")));
        }
        let spec = format!("{commit}:{path}");
        let probe =self.probe(repo, &["rev-parse", "--verify", "-q", &spec])?;
        if !probe.status.success() {
            let err = String::from_utf8_lossy(&probe.stderr);
            if err.trim().is_empty() {
                return Ok(None);
            }
            return Err(classify(&err));
        }
        let bytes = self.run_bytes(Some(repo), &["cat-file", "blob", &spec], &None, &[], None)?;
        Ok(Some(bytes))
    }

    /// All file paths of the tree at `commit` (UTF-8, `-z` parsed).
    pub fn list_at(&self, repo: &Path, commit: &str) -> Result<Vec<String>, SyncGitError> {
        check_arg(commit)?;
        let out = self.run_bytes(
            Some(repo),
            &["ls-tree", "-r", "-z", "--name-only", commit, "--"],
            &None,
            &[],
            None,
        )?;
        Ok(out
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect())
    }

    /// Whether `commit` names a commit object in this clone. Other git
    /// failures are errors.
    pub fn has_commit(&self, repo: &Path, commit: &str) -> Result<bool, SyncGitError> {
        check_arg(commit)?;
        let spec = format!("{commit}^{{commit}}");
        let out = self.probe(repo, &["rev-parse", "--verify", "-q", &spec])?;
        if out.status.success() {
            return Ok(true);
        }
        let err = String::from_utf8_lossy(&out.stderr);
        if err.trim().is_empty() {
            Ok(false)
        } else {
            Err(classify(&err))
        }
    }

    /// Points `refs/memdock/<name>` at `commit`, so a local-only commit stays
    /// reachable (never pruned by `gc`). Never pushed.
    pub fn keep_ref(&self, repo: &Path, name: &str, commit: &str) -> Result<(), SyncGitError> {
        check_arg(name)?;
        check_arg(commit)?;
        let r = format!("refs/memdock/{name}");
        self.run_bytes(Some(repo), &["update-ref", &r, commit], &None, &[], None)?;
        Ok(())
    }

    /// Every entry (files, symlinks, gitlinks) at `commit`, one `ls-tree`.
    pub fn tree_entries(
        &self,
        repo: &Path,
        commit: &str,
    ) -> Result<BTreeMap<String, TreeEntry>, SyncGitError> {
        check_arg(commit)?;
        let out = self.run_bytes(Some(repo), &["ls-tree", "-r", "-z", commit, "--"], &None, &[], None)?;
        let mut map = BTreeMap::new();
        for rec in out.split(|b| *b == 0).filter(|s| !s.is_empty()) {
            let rec = String::from_utf8_lossy(rec);
            if let Some((meta, path)) = rec.split_once('\t') {
                let mut it = meta.split(' ');
                if let (Some(mode), Some(kind), Some(sha)) = (it.next(), it.next(), it.next()) {
                    map.insert(
                        path.to_string(),
                        TreeEntry {
                            mode: mode.to_string(),
                            kind: kind.to_string(),
                            sha: sha.to_string(),
                        },
                    );
                }
            }
        }
        Ok(map)
    }

    /// Contents of the given blobs through one `git cat-file --batch` process.
    pub fn read_blobs(
        &self,
        repo: &Path,
        shas: &[String],
    ) -> Result<HashMap<String, Vec<u8>>, SyncGitError> {
        let mut want: Vec<&str> = Vec::new();
        for s in shas {
            if !is_hex_id(s) {
                return Err(failed(format!("invalid object id: {s}")));
            }
            if !want.contains(&s.as_str()) {
                want.push(s);
            }
        }
        let mut map = HashMap::new();
        if want.is_empty() {
            return Ok(map);
        }
        let mut input = want.join("\n").into_bytes();
        input.push(b'\n');
        let out = self.run_bytes(Some(repo), &["cat-file", "--batch"], &None, &[], Some(input))?;
        let mut i = 0;
        for _ in 0..want.len() {
            let nl = out[i..]
                .iter()
                .position(|b| *b == b'\n')
                .ok_or_else(|| failed("truncated cat-file output"))?;
            let header = String::from_utf8_lossy(&out[i..i + nl]).into_owned();
            i += nl + 1;
            let mut it = header.split(' ');
            let (Some(sha), Some(kind), Some(size)) = (it.next(), it.next(), it.next()) else {
                return Err(failed(format!("object not found: {header}")));
            };
            if kind != "blob" {
                return Err(failed(format!("not a blob: {header}")));
            }
            let size: usize = size.trim().parse().map_err(|_| failed("bad cat-file size"))?;
            if out.len() < i + size + 1 {
                return Err(failed("truncated cat-file output"));
            }
            map.insert(sha.to_string(), out[i..i + size].to_vec());
            i += size + 1;
        }
        Ok(map)
    }

    /// Builds a commit whose tree is exactly `files`, without touching the
    /// work tree or any ref. Returns the new commit id.
    pub fn commit_tree(
        &self,
        repo: &Path,
        parent: Option<&str>,
        files: &BTreeMap<String, Vec<u8>>,
        message: &str,
        author: (&str, &str),
    ) -> Result<String, SyncGitError> {
        let blobs: BTreeMap<String, TreeBlob> = files
            .iter()
            .map(|(k, v)| (k.clone(), TreeBlob::Data(v.clone())))
            .collect();
        self.commit_tree_blobs(repo, parent, &blobs, message, author)
    }

    /// `commit_tree` where files may name existing blobs. New contents are
    /// written with one `hash-object --stdin-paths` call.
    pub fn commit_tree_blobs(
        &self,
        repo: &Path,
        parent: Option<&str>,
        files: &BTreeMap<String, TreeBlob>,
        message: &str,
        author: (&str, &str),
    ) -> Result<String, SyncGitError> {
        if let Some(p) = parent {
            check_arg(p)?;
        }
        for (p, b) in files {
            check_repo_path(p)?;
            if let TreeBlob::Existing(e) = b {
                if !e.valid() {
                    return Err(failed(format!("invalid tree entry: {} {} {}", e.mode, e.kind, e.sha)));
                }
            }
        }
        let tmp_dir = std::env::temp_dir().join(format!("memdock-idx-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).map_err(|e| failed(e.to_string()))?;
        let index = tmp_dir.join("index");
        let result = self.commit_tree_inner(repo, parent, files, message, author, &index);
        let _ = std::fs::remove_dir_all(&tmp_dir);
        result
    }

    fn commit_tree_inner(
        &self,
        repo: &Path,
        parent: Option<&str>,
        files: &BTreeMap<String, TreeBlob>,
        message: &str,
        author: (&str, &str),
        index: &Path,
    ) -> Result<String, SyncGitError> {
        let idx_env: [(&str, &OsStr); 1] = [("GIT_INDEX_FILE", index.as_os_str())];
        // Blobs already present in the parent commit are reused, not re-hashed.
        let mut known: BTreeMap<String, String> = BTreeMap::new();
        if let Some(p) = parent {
            let out = self.run_bytes(Some(repo), &["ls-tree", "-r", "-z", p, "--"], &None, &[], None)?;
            for rec in out.split(|b| *b == 0).filter(|s| !s.is_empty()) {
                let rec = String::from_utf8_lossy(rec);
                if let Some((meta, path)) = rec.split_once('\t') {
                    let mut it = meta.split(' ');
                    if let (Some("100644"), Some("blob"), Some(sha)) = (it.next(), it.next(), it.next()) {
                        known.insert(path.to_string(), sha.to_string());
                    }
                }
            }
        }
        // Resolve every entry to a blob id; new contents are written to temp
        // files and hashed into the object store in one process.
        let tmp_dir = index.parent().unwrap_or(index);
        // path -> (mode, object id)
        let mut shas: BTreeMap<&str, (String, String)> = BTreeMap::new();
        let mut pending: Vec<(&str, String)> = Vec::new();
        let mut stdin_paths: Vec<u8> = Vec::new();
        for (n, (path, blob)) in files.iter().enumerate() {
            match blob {
                TreeBlob::Existing(e) => {
                    shas.insert(path, (e.mode.clone(), e.sha.clone()));
                }
                TreeBlob::Data(content) => {
                    let want = blob_sha(content);
                    if known.get(path) == Some(&want) {
                        shas.insert(path, ("100644".to_string(), want));
                    } else {
                        let f = tmp_dir.join(format!("blob-{n}"));
                        std::fs::write(&f, content).map_err(|e| failed(e.to_string()))?;
                        let fs_path = f.to_string_lossy().into_owned();
                        if fs_path.contains(['\n', '\r']) {
                            return Err(failed("invalid temp path"));
                        }
                        stdin_paths.extend_from_slice(fs_path.as_bytes());
                        stdin_paths.push(b'\n');
                        pending.push((path, want));
                    }
                }
            }
        }
        if !pending.is_empty() {
            let out = self.run_bytes(
                Some(repo),
                &["hash-object", "-w", "--no-filters", "--stdin-paths"],
                &None,
                &[],
                Some(stdin_paths),
            )?;
            let text = String::from_utf8_lossy(&out);
            let got: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            if got.len() != pending.len() {
                return Err(failed("hash-object returned an unexpected number of ids"));
            }
            for ((path, want), sha) in pending.into_iter().zip(got) {
                if sha != want {
                    return Err(failed("hash-object id mismatch"));
                }
                shas.insert(path, ("100644".to_string(), sha.to_string()));
            }
        }
        let mut info: Vec<u8> = Vec::new();
        for (path, (mode, sha)) in &shas {
            info.extend_from_slice(format!("{mode} {sha}\t{path}").as_bytes());
            info.push(0);
        }
        self.run_bytes(
            Some(repo),
            &["update-index", "-z", "--add", "--index-info"],
            &None,
            &idx_env,
            Some(info),
        )?;
        let tree = self.run_bytes(Some(repo), &["write-tree"], &None, &idx_env, None)?;
        let tree = String::from_utf8_lossy(&tree).trim().to_string();
        let (name, email) = author;
        let ident: [(&str, &OsStr); 4] = [
            ("GIT_AUTHOR_NAME", OsStr::new(name)),
            ("GIT_AUTHOR_EMAIL", OsStr::new(email)),
            ("GIT_COMMITTER_NAME", OsStr::new(name)),
            ("GIT_COMMITTER_EMAIL", OsStr::new(email)),
        ];
        let mut args = vec!["commit-tree", tree.as_str()];
        if let Some(p) = parent {
            args.extend(["-p", p]);
        }
        args.extend(["-m", message]);
        let id = self.run_bytes(Some(repo), &args, &None, &ident, None)?;
        Ok(String::from_utf8_lossy(&id).trim().to_string())
    }

    /// Pushes `commit` to `refs/heads/<branch>` of `origin` (no force).
    pub fn push(
        &self,
        repo: &Path,
        commit: &str,
        branch: &str,
        auth: &Auth,
    ) -> Result<(), SyncGitError> {
        check_arg(commit)?;
        check_arg(branch)?;
        let refspec = format!("{commit}:refs/heads/{branch}");
        self.run_bytes(
            Some(repo),
            &["push", "-q", "--", "origin", &refspec],
            auth,
            &[],
            None,
        )?;
        Ok(())
    }

    /// 3-way text merge with diff3 conflict markers (`local` / `base` / `remote`).
    pub fn merge3(&self, base: &str, ours: &str, theirs: &str) -> Result<Merge3, SyncGitError> {
        let dir = std::env::temp_dir().join(format!("memdock-merge-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).map_err(|e| failed(e.to_string()))?;
        let result = (|| {
            let (b, o, t) = (dir.join("base"), dir.join("ours"), dir.join("theirs"));
            for (p, s) in [(&b, base), (&o, ours), (&t, theirs)] {
                std::fs::write(p, s).map_err(|e| failed(e.to_string()))?;
            }
            let args: Vec<&OsStr> = [
                OsStr::new("merge-file"),
                OsStr::new("-p"),
                OsStr::new("--diff3"),
                OsStr::new("-L"),
                OsStr::new("local"),
                OsStr::new("-L"),
                OsStr::new("base"),
                OsStr::new("-L"),
                OsStr::new("remote"),
                o.as_os_str(),
                b.as_os_str(),
                t.as_os_str(),
            ]
            .to_vec();
            let out = self.exec(Some(&dir), &args, &None, &[], None)?;
            match out.status.code() {
                // 0 = clean; 1..=127 = number of conflicts; anything else is an error.
                Some(c) if (0..=127).contains(&c) => Ok(Merge3 {
                    text: String::from_utf8_lossy(&out.stdout).into_owned(),
                    conflicts: c as usize,
                }),
                _ => Err(failed(String::from_utf8_lossy(&out.stderr).trim())),
            }
        })();
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    /// Latest `limit` commits reachable from `branch_ref` (empty when the ref
    /// does not exist), newest first, with the files each commit touched.
    pub fn log(
        &self,
        repo: &Path,
        branch_ref: &str,
        limit: usize,
    ) -> Result<Vec<CommitInfo>, SyncGitError> {
        check_arg(branch_ref)?;
        let probe_spec = format!("{branch_ref}^{{commit}}");
        let probe = self.probe(repo, &["rev-parse", "--verify", "-q", probe_spec.as_str()])?;
        if !probe.status.success() {
            return Ok(Vec::new());
        }
        let n = limit.to_string();
        let out = self.run_bytes(
            Some(repo),
            &[
                "log",
                "-z",
                "-n",
                &n,
                "--format=%H%x01%ct%x01%B",
                branch_ref,
                "--",
            ],
            &None,
            &[],
            None,
        )?;
        let text = String::from_utf8_lossy(&out).into_owned();
        let mut list = Vec::new();
        for rec in text.split('\0').filter(|r| !r.trim().is_empty()) {
            let mut it = rec.trim_start_matches('\n').splitn(3, '\u{1}');
            let (Some(id), Some(ct), Some(msg)) = (it.next(), it.next(), it.next()) else {
                continue;
            };
            let files = self.run_bytes(
                Some(repo),
                &["diff-tree", "-r", "-z", "--root", "--no-commit-id", "--name-only", id],
                &None,
                &[],
                None,
            )?;
            list.push(CommitInfo {
                id: id.to_string(),
                message: msg.trim_end().to_string(),
                time: ct.trim().parse::<i64>().unwrap_or(0) * 1000,
                files: files
                    .split(|b| *b == 0)
                    .filter(|s| !s.is_empty())
                    .map(|s| String::from_utf8_lossy(s).into_owned())
                    .collect(),
            });
        }
        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::git::testutil::git;

    const AUTHOR: (&str, &str) = ("MemDock", "memdock@users.noreply.github.com");

    fn empty_bare(root: &Path) -> String {
        let bare = root.join("remote.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare", "--initial-branch=main"]);
        format!("file:///{}", bare.to_string_lossy().replace('\\', "/"))
    }

    fn files(pairs: &[(&str, &str)]) -> BTreeMap<String, Vec<u8>> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_bytes().to_vec()))
            .collect()
    }

    fn fake_program(dir: &Path, win: &str, unix: &str) -> PathBuf {
        #[cfg(windows)]
        {
            let _ = unix;
            let p = dir.join("fakegit.cmd");
            std::fs::write(&p, win).unwrap();
            p
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = win;
            let p = dir.join("fakegit.sh");
            std::fs::write(&p, unix).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        }
    }

    #[test]
    fn clone_empty_and_push_first_commit() {
        let t = tempfile::tempdir().unwrap();
        let url = empty_bare(t.path());
        let g = SyncGit::new();
        let a = t.path().join("a");
        g.clone(&url, &a, &None).unwrap();
        assert_eq!(g.fetch(&a, &None).unwrap(), None);
        assert_eq!(g.default_branch(&a).unwrap(), "main");
        let m = files(&[("memdock.json", "{\"format\":1,\"app\":\"MemDock\"}")]);
        let c = g.commit_tree(&a, None, &m, "init", AUTHOR).unwrap();
        // Work tree untouched.
        assert!(!a.join("memdock.json").exists());
        g.push(&a, &c, "main", &None).unwrap();
        let b = t.path().join("b");
        g.clone(&url, &b, &None).unwrap();
        let head = g.fetch(&b, &None).unwrap().unwrap();
        assert_eq!(head, c);
        assert_eq!(
            g.read_at(&b, &head, "memdock.json").unwrap().unwrap(),
            m["memdock.json"]
        );
        assert_eq!(g.fetch(&a, &None).unwrap().as_deref(), Some(c.as_str()));
        let l = g.log(&b, "origin/main", 10).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].message, "init");
        assert_eq!(l[0].files, vec!["memdock.json"]);
        assert!(l[0].time > 1_600_000_000_000);
        let author = git(&b, &["log", "-1", "--format=%an <%ae>", "origin/main"]);
        assert_eq!(author, "MemDock <memdock@users.noreply.github.com>");
    }

    #[test]
    fn push_non_fast_forward_is_rejected() {
        let t = tempfile::tempdir().unwrap();
        let url = empty_bare(t.path());
        let g = SyncGit::new();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        g.clone(&url, &a, &None).unwrap();
        g.clone(&url, &b, &None).unwrap();
        let c1 = g
            .commit_tree(&a, None, &files(&[("x.md", "1")]), "a", AUTHOR)
            .unwrap();
        g.push(&a, &c1, "main", &None).unwrap();
        let c2 = g
            .commit_tree(&b, None, &files(&[("y.md", "2")]), "b", AUTHOR)
            .unwrap();
        assert_eq!(g.push(&b, &c2, "main", &None), Err(SyncGitError::Rejected));
    }

    #[test]
    fn read_list_and_exact_tree_at_commit() {
        let t = tempfile::tempdir().unwrap();
        let url = empty_bare(t.path());
        let g = SyncGit::new();
        let a = t.path().join("a");
        g.clone(&url, &a, &None).unwrap();
        let first = files(&[
            ("global/claude/CLAUDE.md", "a\nb\n"),
            ("projects/我的 项目/claude/CLAUDE.md", "中文内容\n"),
            ("skills.json", "{}"),
        ]);
        let c1 = g.commit_tree(&a, None, &first, "one", AUTHOR).unwrap();
        // Second tree drops two files and changes one: absent files must be absent.
        let second = files(&[("skills.json", "{\"v\":2}")]);
        let c2 = g.commit_tree(&a, Some(&c1), &second, "two", AUTHOR).unwrap();
        let mut l1 = g.list_at(&a, &c1).unwrap();
        l1.sort();
        assert_eq!(
            l1,
            vec![
                "global/claude/CLAUDE.md",
                "projects/我的 项目/claude/CLAUDE.md",
                "skills.json"
            ]
        );
        assert_eq!(g.list_at(&a, &c2).unwrap(), vec!["skills.json"]);
        assert_eq!(
            g.read_at(&a, &c1, "projects/我的 项目/claude/CLAUDE.md")
                .unwrap()
                .unwrap(),
            "中文内容\n".as_bytes()
        );
        assert_eq!(g.read_at(&a, &c2, "global/claude/CLAUDE.md").unwrap(), None);
        assert_eq!(
            g.read_at(&a, &c2, "skills.json").unwrap().unwrap(),
            b"{\"v\":2}"
        );
        // Empty tree is representable.
        let c3 = g
            .commit_tree(&a, Some(&c2), &BTreeMap::new(), "empty", AUTHOR)
            .unwrap();
        assert!(g.list_at(&a, &c3).unwrap().is_empty());
        // Binary-safe and LF/CRLF preserved byte for byte.
        let raw = BTreeMap::from([("r.bin".to_string(), vec![0u8, 13, 10, 255])]);
        let c4 = g.commit_tree(&a, None, &raw, "bin", AUTHOR).unwrap();
        assert_eq!(g.read_at(&a, &c4, "r.bin").unwrap().unwrap(), vec![0u8, 13, 10, 255]);
        // Bad paths and options are refused.
        assert!(g.commit_tree(&a, None, &files(&[("../x", "")]), "m", AUTHOR).is_err());
        assert!(g.commit_tree(&a, Some("--x"), &files(&[]), "m", AUTHOR).is_err());
        assert!(g.read_at(&a, "-x", "a").is_err());
        assert!(g.push(&a, &c1, "--force", &None).is_err());
    }

    #[test]
    fn tree_entries_read_blobs_and_commit_by_sha() {
        let t = tempfile::tempdir().unwrap();
        let url = empty_bare(t.path());
        let g = SyncGit::new();
        let a = t.path().join("a");
        g.clone(&url, &a, &None).unwrap();
        let bin = vec![0u8, b'\n', 13, 10, 255, b'\n'];
        let mut m = files(&[("a.md", "A\n"), ("dir/b.md", "")]);
        m.insert("c.bin".into(), bin.clone());
        let c = g.commit_tree(&a, None, &m, "one", AUTHOR).unwrap();
        assert!(g.has_commit(&a, &c).unwrap());
        assert!(!g.has_commit(&a, &"0".repeat(40)).unwrap());
        let e = g.tree_entries(&a, &c).unwrap();
        assert_eq!(e.len(), 3);
        assert_eq!(e["a.md"].sha, blob_id(b"A\n"));
        assert_eq!((e["a.md"].mode.as_str(), e["a.md"].kind.as_str()), ("100644", "blob"));
        let before = g.spawn_count();
        let shas: Vec<String> = e.values().map(|x| x.sha.clone()).collect();
        let blobs = g.read_blobs(&a, &shas).unwrap();
        assert_eq!(g.spawn_count(), before + 1);
        assert_eq!(blobs[&e["c.bin"].sha], bin);
        assert_eq!(blobs[&e["dir/b.md"].sha], b"");
        assert_eq!(blobs[&e["a.md"].sha], b"A\n");
        assert!(g.read_blobs(&a, &["zz".to_string()]).is_err());
        assert!(g.read_blobs(&a, &["1".repeat(40)]).is_err());
        // existing blob by id + new content
        let mut tb: BTreeMap<String, TreeBlob> = BTreeMap::new();
        // executable and gitlink entries keep their mode
        let exe = TreeEntry { mode: "100755".into(), ..e["c.bin"].clone() };
        tb.insert("kept.bin".into(), TreeBlob::Existing(exe.clone()));
        let link = TreeEntry { mode: "160000".into(), kind: "commit".into(), sha: c.clone() };
        tb.insert("vendor/sub".into(), TreeBlob::Existing(link.clone()));
        tb.insert("n1.md".into(), TreeBlob::Data(b"n1".to_vec()));
        tb.insert("n2.md".into(), TreeBlob::Data(b"n2".to_vec()));
        let c2 = g.commit_tree_blobs(&a, Some(&c), &tb, "two", AUTHOR).unwrap();
        let e2 = g.tree_entries(&a, &c2).unwrap();
        assert_eq!(e2.keys().collect::<Vec<_>>(), vec!["kept.bin", "n1.md", "n2.md", "vendor/sub"]);
        assert_eq!(e2["kept.bin"], exe);
        assert_eq!(e2["vendor/sub"], link);
        assert!(!e2["vendor/sub"].is_regular() && e2["kept.bin"].is_regular());
        let bad = TreeEntry { mode: "160000".into(), kind: "blob".into(), sha: c.clone() };
        let bad_tree = BTreeMap::from([("x".to_string(), TreeBlob::Existing(bad))]);
        assert!(g.commit_tree_blobs(&a, None, &bad_tree, "bad", AUTHOR).is_err());
        assert_eq!(g.read_at(&a, &c2, "n2.md").unwrap().unwrap(), b"n2");
        assert_eq!(g.read_at(&a, &c2, "kept.bin").unwrap().unwrap(), bin);
    }

    #[test]
    fn merge3_clean_and_conflict() {
        let g = SyncGit::new();
        let base = "l1\nl2\nl3\nl4\nl5\nl6\nl7\n";
        let ours = "L1\nl2\nl3\nl4\nl5\nl6\nl7\n";
        let theirs = "l1\nl2\nl3\nl4\nl5\nl6\nL7\n";
        let m = g.merge3(base, ours, theirs).unwrap();
        assert_eq!(m.conflicts, 0);
        assert!(m.text.contains("L1") && m.text.contains("L7"));
        let theirs2 = "X1\nl2\nl3\nl4\nl5\nl6\nl7\n";
        let m = g.merge3(base, ours, theirs2).unwrap();
        assert!(m.conflicts > 0);
        assert!(m.text.contains("<<<<<<< local"), "{}", m.text);
        assert!(m.text.contains("||||||| base"), "{}", m.text);
        assert!(m.text.contains(">>>>>>> remote"), "{}", m.text);
    }

    #[test]
    fn token_never_in_args() {
        let t = tempfile::tempdir().unwrap();
        let prog = fake_program(
            t.path(),
            "@echo off\r\necho %* >> \"%~dp0args.txt\"\r\nset >> \"%~dp0env.txt\"\r\nexit /b 0\r\n",
            "#!/bin/sh\necho \"$@\" >> \"$(dirname \"$0\")/args.txt\"\nenv >> \"$(dirname \"$0\")/env.txt\"\nexit 0\n",
        );
        let repo = t.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let g = SyncGit::with_program(prog);
        let token = "ghp_SECRETTOKEN123";
        let b64 = base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"));
        assert_eq!(g.fetch(&repo, &Some(token.to_string())).unwrap(), None);
        let args = std::fs::read_to_string(t.path().join("args.txt")).unwrap();
        let env = std::fs::read_to_string(t.path().join("env.txt")).unwrap();
        assert!(args.contains("fetch"), "{args}");
        assert!(!args.contains(token) && !args.contains(&b64), "{args}");
        assert!(env.contains("GIT_CONFIG_VALUE_0=AUTHORIZATION: basic "), "{env}");
        assert!(env.contains(&b64));
        assert!(env.contains("GIT_CONFIG_COUNT=1"));
        assert!(env.contains("GIT_CONFIG_KEY_0=http.https://github.com/.extraheader"));
        assert!(env.contains("GIT_TERMINAL_PROMPT=0"));
        assert!(!repo.join(".git").exists());
    }

    #[test]
    fn error_text_is_redacted() {
        let t = tempfile::tempdir().unwrap();
        let prog = fake_program(
            t.path(),
            "@echo off\r\necho fatal: unable to access https://user:pw@github.com/o/r ghp_abcdef123456 1>&2\r\nexit /b 1\r\n",
            "#!/bin/sh\necho 'fatal: unable to access https://user:pw@github.com/o/r ghp_abcdef123456' >&2\nexit 1\n",
        );
        let g = SyncGit::with_program(prog);
        let Err(SyncGitError::Failed { stderr }) = g.fetch(t.path(), &None) else {
            panic!("expected Failed");
        };
        assert!(!stderr.contains("pw@") && !stderr.contains("abcdef"), "{stderr}");
        assert!(stderr.contains("***"));
    }

    #[test]
    fn classifies_errors() {
        assert_eq!(
            classify("remote: Invalid username or password.\nfatal: Authentication failed for 'x'"),
            SyncGitError::Unauthorized
        );
        assert_eq!(
            classify("fatal: unable to access: The requested URL returned error: 401"),
            SyncGitError::Unauthorized
        );
        // 403 and a missing credential keep the login.
        assert!(matches!(
            classify("fatal: unable to access: The requested URL returned error: 403"),
            SyncGitError::Failed { .. }
        ));
        assert!(matches!(
            classify("fatal: could not read Username for 'https://github.com': terminal prompts disabled"),
            SyncGitError::Failed { .. }
        ));
        assert_eq!(
            classify(" ! [rejected]        a -> main (fetch first)"),
            SyncGitError::Rejected
        );
        assert!(matches!(classify("boom"), SyncGitError::Failed { .. }));
    }

    #[test]
    fn not_installed_and_timeout() {
        let t = tempfile::tempdir().unwrap();
        let g = SyncGit::with_program(PathBuf::from("definitely-not-a-git-program"));
        assert_eq!(g.fetch(t.path(), &None), Err(SyncGitError::NotInstalled));
        let prog = fake_program(
            t.path(),
            "@ping -n 30 127.0.0.1 >nul\r\n",
            "#!/bin/sh\nsleep 30\n",
        );
        let mut g = SyncGit::with_program(prog);
        g.timeout = Duration::from_millis(200);
        assert_eq!(g.fetch(t.path(), &None), Err(SyncGitError::Timeout));
    }

    #[test]
    fn failed_clone_leaves_no_partial_directory() {
        let t = tempfile::tempdir().unwrap();
        let dest = t.path().join("c");
        let g = SyncGit::new();
        assert!(g.clone("file:///does/not/exist", &dest, &None).is_err());
        assert!(g.clone("--evil", &dest, &None).is_err());
        assert_eq!(std::fs::read_dir(t.path()).unwrap().count(), 0);
    }

    #[test]
    fn classify_remote_rejected_is_failure_not_retry() {
        let s = " ! [remote rejected] a -> main (push declined due to repository rule violations)\nerror: failed to push some refs to 'x'";
        assert!(matches!(classify(s), SyncGitError::Failed { .. }));
        assert_eq!(
            classify(" ! [rejected] a -> main (non-fast-forward)\nerror: failed to push some refs"),
            SyncGitError::Rejected
        );
    }

    #[test]
    fn forward_args_skips_empty_and_multiline() {
        let v = vec![
            ("http.proxy".to_string(), " http://p:1 \n".to_string()),
            ("https.proxy".to_string(), "".to_string()),
            ("http.sslCAInfo".to_string(), "a\nb".to_string()),
        ];
        assert_eq!(forward_args(&v), vec!["-c", "http.proxy=http://p:1"]);
    }

    #[test]
    fn user_config_is_isolated_but_proxy_forwarded() {
        let t = tempfile::tempdir().unwrap();
        let prog = fake_program(
            t.path(),
            "@echo off\r\necho %* >> \"%~dp0args.txt\"\r\nset >> \"%~dp0env.txt\"\r\necho http://proxy.test:8080\r\nexit /b 0\r\n",
            "#!/bin/sh\necho \"$@\" >> \"$(dirname \"$0\")/args.txt\"\nenv >> \"$(dirname \"$0\")/env.txt\"\necho http://proxy.test:8080\nexit 0\n",
        );
        let repo = t.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let g = SyncGit::with_program(prog);
        let _ = g.fetch(&repo, &None);
        let args = std::fs::read_to_string(t.path().join("args.txt")).unwrap();
        let env = std::fs::read_to_string(t.path().join("env.txt")).unwrap();
        assert!(args.contains("credential.helper="), "{args}");
        assert!(args.contains("http.proxy=http://proxy.test:8080"), "{args}");
        assert!(env.contains("GIT_CONFIG_GLOBAL="), "{env}");
        assert!(env.contains("GIT_SSH_COMMAND=ssh -oBatchMode=yes"), "{env}");
    }

    #[test]
    fn failed_masks_authorization_header() {
        let SyncGitError::Failed { stderr } =
            failed("> Authorization: basic eC1hY2Nlc3MtdG9rZW46c2VjcmV0\n< HTTP/1.1 401")
        else {
            panic!()
        };
        assert!(!stderr.contains("eC1hY2Nl"), "{stderr}");
        assert!(stderr.contains("401"));
    }

    #[test]
    fn read_at_bad_commit_is_error_and_clone_replaces_existing() {
        let t = tempfile::tempdir().unwrap();
        let url = empty_bare(t.path());
        let g = SyncGit::new();
        let a = t.path().join("a");
        g.clone(&url, &a, &None).unwrap();
        let zeros = "0".repeat(40);
        assert!(g.read_at(&a, &zeros, "x.md").is_err());
        let c = g.commit_tree(&a, None, &files(&[("x.md", "1")]), "m", AUTHOR).unwrap();
        // Reusing an unchanged blob from the parent still yields the exact tree.
        let c2 = g
            .commit_tree(&a, Some(&c), &files(&[("x.md", "1"), ("y.md", "2")]), "m2", AUTHOR)
            .unwrap();
        let mut l = g.list_at(&a, &c2).unwrap();
        l.sort();
        assert_eq!(l, vec!["x.md", "y.md"]);
        // Re-clone over an existing destination leaves exactly one directory.
        g.clone(&url, &a, &None).unwrap();
        assert_eq!(std::fs::read_dir(t.path()).unwrap().count(), 2);
        // URLs may contain spaces but not option-like starts.
        assert!(check_url("file:///a b/c.git").is_ok());
        assert!(check_url("--upload-pack=x").is_err());
    }
}
