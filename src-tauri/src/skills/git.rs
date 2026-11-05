use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

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

fn is_hex_sha(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn drain<R: Read + Send + 'static>(mut r: R) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        buf
    })
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
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GitError::NotInstalled
            } else {
                GitError::Failed {
                    stderr: e.to_string(),
                }
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
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(GitError::Timeout);
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                Err(e) => {
                    return Err(GitError::Failed {
                        stderr: e.to_string(),
                    });
                }
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
            Err(GitError::Failed {
                stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
            })
        }
    }

    fn clone_by_commit(&self, url: &str, sha: &str, dest: &Path) -> Result<(), GitError> {
        let _ = std::fs::remove_dir_all(dest);
        std::fs::create_dir_all(dest).map_err(|e| GitError::Failed {
            stderr: e.to_string(),
        })?;
        self.run(Some(dest), &["init", "-q"])?;
        self.run(Some(dest), &["remote", "add", "origin", url])?;
        self.run(Some(dest), &["fetch", "-q", "--depth", "1", "origin", sha])?;
        self.run(Some(dest), &["checkout", "-q", "FETCH_HEAD"])?;
        Ok(())
    }
}

impl GitRunner for SystemGit {
    fn available(&self) -> bool {
        self.run(None, &["--version"]).is_ok()
    }

    fn ls_remote_head(&self, url: &str, git_ref: Option<&str>) -> Result<String, GitError> {
        let Some(r) = git_ref else {
            return first_sha(&self.run(None, &["ls-remote", url, "HEAD"])?, "HEAD");
        };
        let peeled = format!("{r}^{{}}");
        let out = self.run(None, &["ls-remote", url, r, peeled.as_str()])?;
        if let Some(line) = out.lines().find(|l| l.ends_with("^{}")) {
            return first_sha(line, r);
        }
        match first_sha(&out, r) {
            Ok(s) => Ok(s),
            // A bare commit SHA is not advertised; it is its own identity.
            Err(_) if is_hex_sha(r) => Ok(r.to_string()),
            Err(e) => Err(e),
        }
    }

    fn shallow_clone(
        &self,
        url: &str,
        git_ref: Option<&str>,
        dest: &Path,
    ) -> Result<String, GitError> {
        let dest_s = dest.to_string_lossy().into_owned();
        let mut args = vec!["clone", "-q", "--depth", "1"];
        if let Some(r) = git_ref {
            args.extend(["--branch", r]);
        }
        args.extend([url, dest_s.as_str()]);
        if let Err(e) = self.run(None, &args) {
            match git_ref {
                Some(r) if is_hex_sha(r) && matches!(e, GitError::Failed { .. }) => {
                    self.clone_by_commit(url, r, dest)?;
                }
                _ => return Err(e),
            }
        }
        self.run(Some(dest), &["rev-parse", "HEAD"])
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

fn first_sha(out: &str, what: &str) -> Result<String, GitError> {
    out.lines()
        .next()
        .and_then(|l| l.split_whitespace().next())
        .map(str::to_string)
        .ok_or_else(|| GitError::Failed {
            stderr: format!("ref not found: {what}"),
        })
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
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

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
    fn shallow_clone_from_local_bare_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let (url, work) = make_remote(tmp.path());
        let first = git(&work, &["rev-parse", "HEAD"]);
        std::fs::write(work.join("two.txt"), "2").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "two"]);
        git(&work, &["push", "-q", "origin", "main"]);
        let second = git(&work, &["rev-parse", "HEAD"]);
        let g = SystemGit::new();

        let d1 = tmp.path().join("c1");
        assert_eq!(g.shallow_clone(&url, None, &d1).unwrap(), second);
        assert!(d1.join("two.txt").exists());
        assert_eq!(g.head_commit(&d1).unwrap(), second);
        assert_eq!(g.remote_url(&d1).unwrap(), url);
        assert!(g.repo_root(&d1).is_some());

        let d2 = tmp.path().join("c2");
        assert_eq!(g.shallow_clone(&url, Some("main"), &d2).unwrap(), second);

        // Commit SHA falls back to init + fetch + checkout.
        let d3 = tmp.path().join("c3");
        assert_eq!(g.shallow_clone(&url, Some(&first), &d3).unwrap(), first);
        assert!(!d3.join("two.txt").exists());
        assert_eq!(g.remote_url(&d3).unwrap(), url);

        let d4 = tmp.path().join("c4");
        assert!(matches!(
            g.shallow_clone(&url, Some("no-such-branch"), &d4),
            Err(GitError::Failed { .. })
        ));
    }

    #[test]
    fn non_repo_dir_has_no_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let g = SystemGit::new();
        assert!(g.remote_url(tmp.path()).is_none());
        assert!(g.head_commit(tmp.path()).is_none());
    }
}
