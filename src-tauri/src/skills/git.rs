use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// Marker written into a fully cloned directory; dirs without it are treated as absent.
pub const COMPLETE_MARKER: &str = ".memdock-complete";

/// Failure modes of a git invocation.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GitError {
    NotInstalled,
    Timeout,
    Failed { stderr: String },
}

/// Abstraction over the `git` executable so callers can be tested with a fake.
pub trait GitRunner: Send + Sync {
    fn available(&self) -> bool;
    fn ls_remote_head(&self, url: &str, git_ref: Option<&str>) -> Result<String, GitError>;
    fn shallow_clone(
        &self,
        url: &str,
        git_ref: Option<&str>,
        dest: &Path,
    ) -> Result<String, GitError>;
    fn remote_url(&self, dir: &Path) -> Option<String>;
    fn head_commit(&self, dir: &Path) -> Option<String>;
    fn repo_root(&self, dir: &Path) -> Option<PathBuf>;
}

/// Runs the real `git` executable.
pub struct SystemGit {
    program: PathBuf,
    pub timeout: Duration,
}

impl Default for SystemGit {
    fn default() -> Self {
        Self::with_program(PathBuf::from("git"))
    }
}

fn failed(msg: impl Into<String>) -> GitError {
    GitError::Failed { stderr: msg.into() }
}

fn is_full_sha(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Rejects arguments that could be read as options or break argument parsing.
fn check_arg(s: &str) -> Result<(), GitError> {
    if s.is_empty() || s.starts_with('-') || s.chars().any(|c| c == '\0' || c.is_whitespace()) {
        return Err(failed("invalid argument"));
    }
    Ok(())
}

const TOKEN_PREFIXES: [&str; 7] = [
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "github_pat_",
    "glpat-",
    "x-access-token",
];

/// Hides `scheme://user:pass@` credentials and token-looking words, truncating to 2000 chars.
pub fn redact(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || c == '/')
            .unwrap_or(tail.len());
        match tail[..end].rfind('@') {
            Some(at) => {
                out.push_str("***@");
                rest = &tail[at + 1..];
            }
            None => rest = tail,
        }
    }
    out.push_str(rest);
    let mut masked = String::new();
    for w in out.split_inclusive(char::is_whitespace) {
        let core = w.trim_end();
        let bare = core.trim_start_matches(|c: char| !c.is_alphanumeric());
        if TOKEN_PREFIXES.iter().any(|p| bare.starts_with(p)) {
            masked.push_str("***");
            masked.push_str(&w[core.len()..]);
        } else {
            masked.push_str(w);
        }
    }
    masked.chars().take(2000).collect()
}

fn drain<R: Read + Send + 'static>(mut r: R) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        buf
    })
}

/// Kills the child and its descendants (e.g. `git-remote-https`).
fn kill_tree(child: &mut Child) {
    let pid = child.id().to_string();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .status();
    }
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn tmp_sibling(dest: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "clone".into());
    dest.with_file_name(format!("{name}.tmp-{}-{nanos}", std::process::id()))
}

impl SystemGit {
    pub fn new() -> Self {
        Self::default()
    }

    /// Uses a specific git executable (tests point this at a missing program).
    pub fn with_program(program: PathBuf) -> Self {
        SystemGit {
            program,
            timeout: Duration::from_secs(60),
        }
    }

    /// Runs git with `args`, returning trimmed stdout.
    fn run<S: AsRef<OsStr>>(&self, cwd: Option<&Path>, args: &[S]) -> Result<String, GitError> {
        let mut cmd = Command::new(&self.program);
        cmd.args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(c) = cwd {
            cmd.current_dir(c);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GitError::NotInstalled
            } else {
                failed(redact(&e.to_string()))
            }
        })?;
        let out = child.stdout.take().map(drain);
        let err = child.stderr.take().map(drain);
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) => {
                    if start.elapsed() >= self.timeout {
                        kill_tree(&mut child);
                        return Err(GitError::Timeout);
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(failed(e.to_string())),
            }
        };
        let stdout = out
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default();
        let stderr = err
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default();
        if status.success() {
            Ok(String::from_utf8_lossy(&stdout).trim().to_string())
        } else {
            Err(failed(redact(String::from_utf8_lossy(&stderr).trim())))
        }
    }

    fn clone_into(&self, url: &str, git_ref: Option<&str>, tmp: &Path) -> Result<(), GitError> {
        let tmp_s = tmp.to_string_lossy().into_owned();
        let mut args = vec!["clone", "-q", "--depth", "1"];
        if let Some(r) = git_ref {
            args.extend(["--branch", r]);
        }
        args.extend(["--", url, tmp_s.as_str()]);
        match self.run(None, &args) {
            Ok(_) => Ok(()),
            Err(GitError::Failed { .. }) if git_ref.is_some_and(is_full_sha) => {
                self.clone_by_commit(url, git_ref.unwrap_or_default(), tmp)
            }
            Err(e) => Err(e),
        }
    }

    fn clone_by_commit(&self, url: &str, sha: &str, dir: &Path) -> Result<(), GitError> {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).map_err(|e| failed(e.to_string()))?;
        self.run(Some(dir), &["init", "-q"])?;
        self.run(Some(dir), &["remote", "add", "--", "origin", url])?;
        self.run(
            Some(dir),
            &["fetch", "-q", "--depth", "1", "--", "origin", sha],
        )?;
        self.run(Some(dir), &["checkout", "-q", "FETCH_HEAD"])?;
        Ok(())
    }
}

impl GitRunner for SystemGit {
    fn available(&self) -> bool {
        self.run(None, &["--version"]).is_ok()
    }

    fn ls_remote_head(&self, url: &str, git_ref: Option<&str>) -> Result<String, GitError> {
        check_arg(url)?;
        let Some(r) = git_ref else {
            let out = self.run(None, &["ls-remote", "--", url, "HEAD"])?;
            return pick(&out, &["HEAD"], "HEAD");
        };
        check_arg(r)?;
        let peeled = format!("{r}^{{}}");
        let out = self.run(None, &["ls-remote", "--", url, r, peeled.as_str()])?;
        let names = [
            format!("refs/heads/{r}"),
            format!("refs/tags/{r}^{{}}"),
            format!("refs/tags/{r}"),
            r.to_string(),
        ];
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        match pick(&out, &names, r) {
            Ok(s) => Ok(s),
            // A full commit SHA is not advertised; it is its own identity.
            Err(_) if is_full_sha(r) => Ok(r.to_string()),
            Err(e) => Err(e),
        }
    }

    fn shallow_clone(
        &self,
        url: &str,
        git_ref: Option<&str>,
        dest: &Path,
    ) -> Result<String, GitError> {
        check_arg(url)?;
        if let Some(r) = git_ref {
            check_arg(r)?;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| failed(e.to_string()))?;
        }
        let tmp = tmp_sibling(dest);
        let result = (|| {
            self.clone_into(url, git_ref, &tmp)?;
            let commit = self.run(Some(&tmp), &["rev-parse", "HEAD"])?;
            std::fs::write(tmp.join(COMPLETE_MARKER), &commit)
                .map_err(|e| failed(e.to_string()))?;
            if dest.exists() {
                std::fs::remove_dir_all(dest).map_err(|e| failed(e.to_string()))?;
            }
            std::fs::rename(&tmp, dest).map_err(|e| failed(e.to_string()))?;
            Ok(commit)
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&tmp);
        }
        result
    }

    fn remote_url(&self, dir: &Path) -> Option<String> {
        self.run(Some(dir), &["remote", "get-url", "origin"])
            .ok()
            .filter(|s| !s.is_empty())
    }

    fn head_commit(&self, dir: &Path) -> Option<String> {
        self.run(Some(dir), &["rev-parse", "HEAD"])
            .ok()
            .filter(|s| !s.is_empty())
    }

    fn repo_root(&self, dir: &Path) -> Option<PathBuf> {
        self.run(Some(dir), &["rev-parse", "--show-toplevel"])
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    }
}

/// Picks the sha of the first of `names` (by priority) present as an exact ref in ls-remote output.
fn pick(out: &str, names: &[&str], what: &str) -> Result<String, GitError> {
    let lines: Vec<(&str, &str)> = out
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?, it.next()?))
        })
        .collect();
    for n in names {
        if let Some((sha, _)) = lines.iter().find(|(_, name)| name == n) {
            return Ok((*sha).to_string());
        }
    }
    Err(failed(format!("ref not found: {what}")))
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    /// Runs git with fixed identity; panics on failure.
    pub fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Creates a bare remote plus a work clone with one commit on `main`.
    /// Returns `(file:// url, work dir)`.
    pub fn make_remote(root: &Path) -> (String, PathBuf) {
        let bare = root.join("remote.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare", "--initial-branch=main"]);
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        git(&work, &["init", "-q", "--initial-branch=main"]);
        std::fs::write(work.join("README.md"), "hi").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "one"]);
        let url = format!("file:///{}", bare.to_string_lossy().replace('\\', "/"));
        git(&work, &["remote", "add", "origin", &url]);
        git(&work, &["push", "-q", "origin", "main"]);
        (url, work)
    }

    pub fn commit_file(work: &Path, name: &str) -> String {
        std::fs::write(work.join(name), name).unwrap();
        git(work, &["add", "."]);
        git(work, &["commit", "-q", "-m", name]);
        git(work, &["rev-parse", "HEAD"])
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    fn entries(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn not_installed_when_git_missing() {
        let g = SystemGit::with_program(PathBuf::from("definitely-not-a-git-program"));
        assert!(!g.available());
        assert_eq!(
            g.ls_remote_head("file:///nope", None),
            Err(GitError::NotInstalled)
        );
    }

    #[test]
    fn available_with_real_git() {
        assert!(SystemGit::new().available());
    }

    #[test]
    fn ls_remote_head_from_local_bare_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let (url, work) = make_remote(tmp.path());
        let head = git(&work, &["rev-parse", "HEAD"]);
        let g = SystemGit::new();
        assert_eq!(g.ls_remote_head(&url, None).unwrap(), head);
        assert_eq!(g.ls_remote_head(&url, Some("main")).unwrap(), head);
        // Annotated tag resolves to the peeled commit, not the tag object.
        git(&work, &["tag", "-a", "v1", "-m", "v1"]);
        git(&work, &["push", "-q", "origin", "v1"]);
        assert_eq!(g.ls_remote_head(&url, Some("v1")).unwrap(), head);
        assert!(matches!(
            g.ls_remote_head(&url, Some("nope")),
            Err(GitError::Failed { .. })
        ));
    }

    #[test]
    fn ls_remote_requires_exact_ref_with_priority() {
        let tmp = tempfile::tempdir().unwrap();
        let (url, work) = make_remote(tmp.path());
        let main = git(&work, &["rev-parse", "HEAD"]);
        // feature/main tail-matches "main" and sorts before refs/heads/main.
        git(&work, &["checkout", "-q", "-b", "feature/main"]);
        let feat = commit_file(&work, "f.txt");
        git(&work, &["push", "-q", "origin", "feature/main"]);
        git(&work, &["checkout", "-q", "main"]);
        // Same name for a tag and a branch: branch wins.
        git(&work, &["checkout", "-q", "-b", "dup"]);
        let dup_branch = commit_file(&work, "d.txt");
        git(&work, &["push", "-q", "origin", "dup"]);
        git(&work, &["checkout", "-q", "main"]);
        git(&work, &["tag", "dup"]);
        git(&work, &["push", "-q", "origin", "refs/tags/dup"]);
        // A tag only reachable via a longer name must not match.
        git(&work, &["tag", "x/only-tag"]);
        git(&work, &["push", "-q", "origin", "x/only-tag"]);
        let g = SystemGit::new();
        assert_ne!(main, feat);
        assert_eq!(g.ls_remote_head(&url, Some("main")).unwrap(), main);
        assert_eq!(g.ls_remote_head(&url, Some("feature/main")).unwrap(), feat);
        assert_eq!(g.ls_remote_head(&url, Some("dup")).unwrap(), dup_branch);
        assert!(g.ls_remote_head(&url, Some("only-tag")).is_err());
    }

    #[test]
    fn rejects_option_injection() {
        let g = SystemGit::new();
        let bad = GitError::Failed {
            stderr: "invalid argument".into(),
        };
        assert_eq!(g.ls_remote_head("--upload-pack=x", None), Err(bad.clone()));
        assert_eq!(
            g.ls_remote_head("file:///x", Some("--upload-pack=x")),
            Err(bad.clone())
        );
        assert_eq!(g.ls_remote_head("file:///x", Some("a b")), Err(bad.clone()));
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("d");
        assert_eq!(
            g.shallow_clone("file:///x", Some("-x"), &dest),
            Err(bad.clone())
        );
        assert_eq!(g.shallow_clone("--help", None, &dest), Err(bad.clone()));
        assert_eq!(g.shallow_clone("file:///x", Some("a\nb"), &dest), Err(bad));
        assert!(entries(tmp.path()).is_empty());
    }

    #[test]
    fn shallow_clone_from_local_bare_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let (url, work) = make_remote(tmp.path());
        let first = git(&work, &["rev-parse", "HEAD"]);
        let second = commit_file(&work, "two.txt");
        git(&work, &["push", "-q", "origin", "main"]);
        let g = SystemGit::new();

        let d1 = tmp.path().join("c1");
        assert_eq!(g.shallow_clone(&url, None, &d1).unwrap(), second);
        assert!(d1.join("two.txt").exists());
        assert!(d1.join(COMPLETE_MARKER).exists());
        assert_eq!(g.head_commit(&d1).unwrap(), second);
        assert_eq!(g.remote_url(&d1).unwrap(), url);
        assert!(g.repo_root(&d1).is_some());

        let d2 = tmp.path().join("c2");
        assert_eq!(g.shallow_clone(&url, Some("main"), &d2).unwrap(), second);

        // Full commit SHA falls back to init + fetch + checkout.
        let d3 = tmp.path().join("c3");
        assert_eq!(g.shallow_clone(&url, Some(&first), &d3).unwrap(), first);
        assert!(!d3.join("two.txt").exists());
        assert_eq!(g.remote_url(&d3).unwrap(), url);
        assert!(d3.join(COMPLETE_MARKER).exists());

        // Re-cloning over an existing dest replaces it.
        assert_eq!(g.shallow_clone(&url, None, &d3).unwrap(), second);
    }

    #[test]
    fn failed_clone_leaves_no_partial_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let (url, _work) = make_remote(tmp.path());
        let before = entries(tmp.path());
        let g = SystemGit::new();
        let dest = tmp.path().join("c4");
        assert!(matches!(
            g.shallow_clone(&url, Some("no-such-branch"), &dest),
            Err(GitError::Failed { .. })
        ));
        let zeros = "0".repeat(40);
        assert!(matches!(
            g.shallow_clone(&url, Some(&zeros), &dest),
            Err(GitError::Failed { .. })
        ));
        assert!(matches!(
            g.shallow_clone("file:///does/not/exist", None, &dest),
            Err(GitError::Failed { .. })
        ));
        assert_eq!(entries(tmp.path()), before);
    }

    #[test]
    fn timeout_kills_and_cleans_up() {
        let tmp = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let prog = {
            let p = tmp.path().join("slow.cmd");
            std::fs::write(&p, "@ping -n 30 127.0.0.1 >nul\r\n").unwrap();
            p
        };
        #[cfg(unix)]
        let prog = {
            use std::os::unix::fs::PermissionsExt;
            let p = tmp.path().join("slow.sh");
            std::fs::write(&p, "#!/bin/sh\nsleep 30\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        let mut g = SystemGit::with_program(prog);
        g.timeout = Duration::from_millis(200);
        let start = Instant::now();
        assert_eq!(g.ls_remote_head("file:///x", None), Err(GitError::Timeout));
        let dest = tmp.path().join("clone");
        assert_eq!(
            g.shallow_clone("file:///x", None, &dest),
            Err(GitError::Timeout)
        );
        assert!(start.elapsed() < Duration::from_secs(15));
        assert!(!dest.exists());
        assert_eq!(entries(tmp.path()).len(), 1, "only the script remains");
    }

    #[test]
    fn redacts_credentials_and_truncates() {
        let s = redact("fatal: unable to access 'https://user:pw@github.com/o/r/': bad");
        assert!(!s.contains("pw") && !s.contains("user:"), "{s}");
        assert!(s.contains("https://***@github.com/o/r/"));
        let s = redact("auth failed ghp_abcdef123456 for you\nand github_pat_XYZ");
        assert!(!s.contains("abcdef") && !s.contains("XYZ"), "{s}");
        assert!(s.contains("for you"));
        assert_eq!(redact(&"x".repeat(5000)).chars().count(), 2000);
    }

    #[test]
    fn non_repo_dir_has_no_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let g = SystemGit::new();
        assert!(g.remote_url(tmp.path()).is_none());
        assert!(g.head_commit(tmp.path()).is_none());
    }
}
