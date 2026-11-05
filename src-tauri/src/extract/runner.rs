//! Runners that send a prompt to the user's local `claude` / `codex` CLI and
//! return the raw model output. The CLIs run read-only, in an empty temp
//! directory, with the prompt on stdin.
//!
//! Windows `.cmd` wrappers (npm-installed `claude.cmd`): `std::process::Command`
//! runs them through `cmd.exe` with its own batch-safe argument escaping, so we
//! spawn the wrapper directly (no manual `cmd /C`). Arguments it cannot escape
//! safely make `spawn` fail with `InvalidInput`, reported as `Failed`.
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::extract::prompt::redact;
use crate::model::Tool;
use crate::proc::{self, WaitError};

const STDERR_LIMIT: usize = 2000;

/// Why a CLI run produced no output.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunError {
    NotInstalled,
    NotLoggedIn(String),
    Timeout,
    Cancelled,
    Failed { code: Option<i32>, stderr: String },
}

/// Abstraction over a local coding-agent CLI so extraction can be tested with fakes.
pub trait CliRunner: Send + Sync {
    fn tool(&self) -> Tool;
    fn available(&self) -> bool;
    /// Sends `prompt` (stdin) and returns the raw model output (JSON text).
    fn run(
        &self,
        prompt: &str,
        schema: &Value,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<String, RunError>;
}

fn clean(s: &str) -> String {
    redact(s.trim()).chars().take(STDERR_LIMIT).collect()
}

fn failed(code: Option<i32>, stderr: &str) -> RunError {
    RunError::Failed {
        code,
        stderr: clean(stderr),
    }
}

fn looks_not_logged_in(s: &str) -> bool {
    let l = s.to_lowercase();
    ["login", "please run /login", "invalid api key", "not logged in"]
        .iter()
        .any(|p| l.contains(p))
}

/// Command-line arguments (after the program) for each tool. For Claude `schema`
/// is the schema JSON text; for Codex it is the schema file path and `out_file`
/// is the last-message file path.
pub fn args_for(tool: Tool, schema: &str, out_file: &str) -> Vec<String> {
    let v: Vec<&str> = match tool {
        Tool::Claude => vec![
            "-p",
            "--output-format",
            "json",
            "--json-schema",
            schema,
            "--tools",
            "",
            "--no-session-persistence",
            "--bare",
        ],
        Tool::Codex => vec![
            "exec",
            "--sandbox",
            "read-only",
            "--skip-git-repo-check",
            "--output-schema",
            schema,
            "--output-last-message",
            out_file,
            "-",
        ],
    };
    v.into_iter().map(String::from).collect()
}

/// Temp directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<Self, RunError> {
        let p = std::env::temp_dir().join(format!("memdock-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).map_err(|e| failed(None, &e.to_string()))?;
        Ok(TempDir(p))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Spawns `program` in `cwd`, feeds `stdin_text`, waits with timeout / cancel.
pub(crate) fn exec(
    program: &Path,
    args: &[String],
    cwd: &Path,
    stdin_text: &str,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<Output, RunError> {
    if cancel.load(Ordering::SeqCst) {
        return Err(RunError::Cancelled);
    }
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = proc::spawn_hidden(&mut cmd).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            RunError::NotInstalled
        } else {
            failed(None, &e.to_string())
        }
    })?;
    if let Some(mut stdin) = child.stdin.take() {
        let data = stdin_text.as_bytes().to_vec();
        // Separate thread: a program that never reads stdin must not block us.
        thread::spawn(move || {
            let _ = stdin.write_all(&data);
        });
    }
    proc::wait_with_timeout(child, timeout, Some(cancel)).map_err(|e| match e {
        WaitError::Timeout => RunError::Timeout,
        WaitError::Cancelled => RunError::Cancelled,
        WaitError::Io(m) => failed(None, &m),
    })
}

/// True when `program` is an existing file, or (bare name) found on PATH.
fn program_exists(program: &Path) -> bool {
    if program.components().count() > 1 || program.is_absolute() {
        return program.is_file();
    }
    let dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    find_program(program.as_os_str(), &dirs).is_some()
}

/// Finds executable `name` in `dirs`. Windows prefers `.exe`, then npm `.cmd` / `.bat`
/// wrappers (the extension-less file there is a shell script we cannot run).
pub fn find_program(name: &OsStr, dirs: &[PathBuf]) -> Option<PathBuf> {
    #[cfg(windows)]
    let exts: &[&str] = &["exe", "cmd", "bat"];
    #[cfg(not(windows))]
    let exts: &[&str] = &[""];
    for d in dirs {
        for e in exts {
            let mut f = name.to_os_string();
            if !e.is_empty() {
                f.push(".");
                f.push(e);
            }
            let p = d.join(f);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Runs `claude -p` and extracts `structured_output` (preferred) or `result`.
pub struct ClaudeCli {
    program: PathBuf,
}

impl Default for ClaudeCli {
    fn default() -> Self {
        Self::with_program(PathBuf::from("claude"))
    }
}

impl ClaudeCli {
    pub fn with_program(program: PathBuf) -> Self {
        ClaudeCli { program }
    }
}

impl CliRunner for ClaudeCli {
    fn tool(&self) -> Tool {
        Tool::Claude
    }

    fn available(&self) -> bool {
        program_exists(&self.program)
    }

    fn run(
        &self,
        prompt: &str,
        schema: &Value,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<String, RunError> {
        let tmp = TempDir::new()?;
        let args = args_for(Tool::Claude, &schema.to_string(), "");
        let out = exec(&self.program, &args, &tmp.0, prompt, timeout, cancel)?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        let code = out.status.code();
        let parsed: Option<Value> = serde_json::from_str(stdout.trim()).ok();
        if let Some(v) = &parsed {
            let result_text = v.get("result").and_then(Value::as_str).unwrap_or("");
            if v.get("is_error").and_then(Value::as_bool) == Some(true) {
                let msg = format!("{result_text}\n{stderr}");
                return Err(if looks_not_logged_in(&msg) {
                    RunError::NotLoggedIn(clean(&msg))
                } else {
                    failed(code, &msg)
                });
            }
            if out.status.success() {
                match v.get("structured_output") {
                    Some(s) if !s.is_null() => return Ok(s.to_string()),
                    _ => {
                        if let Some(r) = v.get("result").and_then(Value::as_str) {
                            return Ok(r.to_string());
                        }
                    }
                }
            }
        }
        if !out.status.success() {
            let msg = format!("{stderr}\n{stdout}");
            return Err(if looks_not_logged_in(&msg) {
                RunError::NotLoggedIn(clean(&msg))
            } else {
                failed(code, &stderr)
            });
        }
        Err(failed(code, &format!("unexpected output: {stdout}\n{stderr}")))
    }
}

/// Runs `codex exec`; the answer is read from `--output-last-message`.
pub struct CodexCli {
    program: PathBuf,
}

impl Default for CodexCli {
    fn default() -> Self {
        Self::with_program(PathBuf::from("codex"))
    }
}

impl CodexCli {
    pub fn with_program(program: PathBuf) -> Self {
        CodexCli { program }
    }
}

impl CliRunner for CodexCli {
    fn tool(&self) -> Tool {
        Tool::Codex
    }

    fn available(&self) -> bool {
        program_exists(&self.program)
    }

    fn run(
        &self,
        prompt: &str,
        schema: &Value,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<String, RunError> {
        let tmp = TempDir::new()?;
        // cwd stays empty; schema / output files live beside it.
        let cwd = tmp.0.join("cwd");
        let io = tmp.0.join("io");
        for d in [&cwd, &io] {
            std::fs::create_dir_all(d).map_err(|e| failed(None, &e.to_string()))?;
        }
        let schema_file = io.join("schema.json");
        let out_file = io.join("out.txt");
        std::fs::write(&schema_file, schema.to_string())
            .map_err(|e| failed(None, &e.to_string()))?;
        let args = args_for(
            Tool::Codex,
            &schema_file.to_string_lossy(),
            &out_file.to_string_lossy(),
        );
        let out = exec(&self.program, &args, &cwd, prompt, timeout, cancel)?;
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        let code = out.status.code();
        if !out.status.success() {
            return Err(if looks_not_logged_in(&stderr) {
                RunError::NotLoggedIn(clean(&stderr))
            } else {
                failed(code, &stderr)
            });
        }
        match std::fs::read_to_string(&out_file) {
            Ok(s) if !s.trim().is_empty() => Ok(s),
            _ => Err(failed(code, &format!("no output file written\n{stderr}"))),
        }
    }
}

/// Builds runners for each CLI found in `dirs` (PATH entries).
pub fn detect_runners(dirs: &[PathBuf]) -> Vec<Arc<dyn CliRunner>> {
    let mut v: Vec<Arc<dyn CliRunner>> = Vec::new();
    if let Some(p) = find_program(OsStr::new("claude"), dirs) {
        v.push(Arc::new(ClaudeCli::with_program(p)));
    }
    if let Some(p) = find_program(OsStr::new("codex"), dirs) {
        v.push(Arc::new(CodexCli::with_program(p)));
    }
    v
}

/// `detect_runners` over the process `PATH`.
pub fn detect_runners_from_env() -> Vec<Arc<dyn CliRunner>> {
    let dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    detect_runners(&dirs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Instant;

    /// Writes a fake program script (.cmd on Windows, sh on Unix).
    fn script(dir: &Path, name: &str, win: &str, unix: &str) -> PathBuf {
        #[cfg(windows)]
        {
            let _ = unix;
            let p = dir.join(format!("{name}.cmd"));
            std::fs::write(&p, format!("@echo off\r\n{win}\r\n")).unwrap();
            p
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = win;
            let p = dir.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{unix}\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        }
    }

    fn no_cancel() -> AtomicBool {
        AtomicBool::new(false)
    }

    const SECS5: Duration = Duration::from_secs(5);

    #[test]
    fn claude_args_disable_tools_and_persistence() {
        let a = args_for(Tool::Claude, "{\"type\":\"object\"}", "");
        assert_eq!(a[0], "-p");
        assert!(a.windows(2).any(|w| w == ["--output-format", "json"]));
        assert!(a.windows(2).any(|w| w == ["--json-schema", "{\"type\":\"object\"}"]));
        assert!(a.windows(2).any(|w| w == ["--tools", ""]));
        assert!(a.contains(&"--no-session-persistence".to_string()));
        assert!(a.contains(&"--bare".to_string()));
    }

    #[test]
    fn codex_args_read_only_sandbox() {
        let a = args_for(Tool::Codex, "S.json", "O.txt");
        assert_eq!(&a[..2], ["exec", "--sandbox"]);
        assert!(a.windows(2).any(|w| w == ["--sandbox", "read-only"]));
        assert!(a.contains(&"--skip-git-repo-check".to_string()));
        assert!(a.windows(2).any(|w| w == ["--output-schema", "S.json"]));
        assert!(a.windows(2).any(|w| w == ["--output-last-message", "O.txt"]));
        assert_eq!(a.last().unwrap(), "-");
    }

    #[test]
    fn fake_program_output_returned() {
        let t = tempfile::tempdir().unwrap();
        // Echoes stdin to stdout.
        let echo = script(t.path(), "echo-stdin", "findstr \"^\"", "cat");
        let out = exec(&echo, &[], t.path(), "hello prompt", SECS5, &no_cancel()).unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello prompt");

        // Claude JSON: structured_output wins over result.
        let s = script(
            t.path(),
            "claude-structured",
            "echo {\"is_error\":false,\"result\":\"plain\",\"structured_output\":{\"a\":1}}",
            "echo '{\"is_error\":false,\"result\":\"plain\",\"structured_output\":{\"a\":1}}'",
        );
        let r = ClaudeCli::with_program(s)
            .run("p", &json!({}), SECS5, &no_cancel())
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&r).unwrap(), json!({"a":1}));
        let s = script(
            t.path(),
            "claude-result",
            "echo {\"is_error\":false,\"result\":\"just text\"}",
            "echo '{\"is_error\":false,\"result\":\"just text\"}'",
        );
        let r = ClaudeCli::with_program(s)
            .run("p", &json!({}), SECS5, &no_cancel())
            .unwrap();
        assert_eq!(r, "just text");
    }

    #[test]
    fn codex_reads_output_file_in_empty_cwd() {
        let t = tempfile::tempdir().unwrap();
        // Args: exec --sandbox read-only --skip-git-repo-check --output-schema S --output-last-message O -
        // Windows batch: %7 is the O path; the cwd must contain no files.
        let s = script(
            t.path(),
            "codex-fake",
            "dir /b | findstr \"^\" >nul && (echo cwd not empty 1>&2 & exit /b 3)\r\nfindstr \"^\" > %8",
            "[ -z \"$(ls -A)\" ] || { echo cwd not empty >&2; exit 3; }\ncat > \"$7\"",
        );
        let r = CodexCli::with_program(s)
            .run("{\"x\":2}", &json!({"type":"object"}), SECS5, &no_cancel())
            .unwrap();
        assert_eq!(r.trim(), "{\"x\":2}");
    }

    #[test]
    fn schema_survives_argument_quoting() {
        let t = tempfile::tempdir().unwrap();
        let echo_args = script(t.path(), "argv", "echo %*", "echo \"$@\"");
        let schema = json!({"type":"object","properties":{"k":{"type":"string"}}}).to_string();
        let args = args_for(Tool::Claude, &schema, "");
        let out = exec(&echo_args, &args, t.path(), "", SECS5, &no_cancel()).unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        // After the shell / batch layer the JSON must still be the same text.
        let normalized = text.replace("\"\"", "\"");
        assert!(
            text.contains(&schema) || normalized.contains(&schema),
            "argv was: {text}"
        );
    }

    #[test]
    fn timeout_kills_process_tree() {
        let t = tempfile::tempdir().unwrap();
        let slow = script(t.path(), "slow", "ping -n 30 127.0.0.1 >nul", "sleep 30");
        let start = Instant::now();
        let r = exec(&slow, &[], t.path(), "", Duration::from_millis(300), &no_cancel());
        assert_eq!(r.unwrap_err(), RunError::Timeout);
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn cancel_flag_stops_run() {
        let t = tempfile::tempdir().unwrap();
        let slow = script(t.path(), "slow2", "ping -n 30 127.0.0.1 >nul", "sleep 30");
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            c2.store(true, Ordering::SeqCst);
        });
        let start = Instant::now();
        let r = ClaudeCli::with_program(slow).run("p", &json!({}), Duration::from_secs(30), &cancel);
        assert_eq!(r.unwrap_err(), RunError::Cancelled);
        assert!(start.elapsed() < Duration::from_secs(10));
        // Already-cancelled flag short-circuits before spawning.
        let r = exec(Path::new("nope"), &[], t.path(), "", SECS5, &cancel);
        assert_eq!(r.unwrap_err(), RunError::Cancelled);
    }

    #[test]
    fn not_logged_in_detected() {
        let t = tempfile::tempdir().unwrap();
        let s = script(
            t.path(),
            "login-json",
            "echo {\"is_error\":true,\"result\":\"Please run /login\"}",
            "echo '{\"is_error\":true,\"result\":\"Please run /login\"}'",
        );
        let e = ClaudeCli::with_program(s)
            .run("p", &json!({}), SECS5, &no_cancel())
            .unwrap_err();
        assert!(matches!(e, RunError::NotLoggedIn(_)), "{e:?}");
        let s = script(
            t.path(),
            "login-stderr",
            "echo Invalid API key 1>&2\r\nexit /b 1",
            "echo 'Invalid API key' >&2; exit 1",
        );
        let e = ClaudeCli::with_program(s)
            .run("p", &json!({}), SECS5, &no_cancel())
            .unwrap_err();
        assert!(matches!(e, RunError::NotLoggedIn(_)), "{e:?}");
        let e = CodexCli::with_program(script(
            t.path(),
            "codex-login",
            "echo You are not logged in 1>&2\r\nexit /b 1",
            "echo 'You are not logged in' >&2; exit 1",
        ))
        .run("p", &json!({}), SECS5, &no_cancel())
        .unwrap_err();
        assert!(matches!(e, RunError::NotLoggedIn(_)), "{e:?}");
    }

    #[test]
    fn failure_stderr_is_redacted_and_truncated() {
        let t = tempfile::tempdir().unwrap();
        let s = script(
            t.path(),
            "boom",
            "echo password=hunter2 1>&2\r\nexit /b 7",
            "echo 'password=hunter2' >&2; exit 7",
        );
        let e = ClaudeCli::with_program(s)
            .run("p", &json!({}), SECS5, &no_cancel())
            .unwrap_err();
        match e {
            RunError::Failed { code, stderr } => {
                assert_eq!(code, Some(7));
                assert!(!stderr.contains("hunter2"), "{stderr}");
                assert!(stderr.chars().count() <= 2000);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(failed(None, &"x".repeat(5000)).clone(), RunError::Failed {
            code: None,
            stderr: "x".repeat(2000)
        });
    }

    #[test]
    fn missing_program_not_installed() {
        let c = ClaudeCli::with_program(PathBuf::from("definitely-not-a-claude-program"));
        assert!(!c.available());
        let e = c.run("p", &json!({}), SECS5, &no_cancel()).unwrap_err();
        assert_eq!(e, RunError::NotInstalled);
        let x = CodexCli::with_program(PathBuf::from("definitely-not-a-codex-program"));
        assert_eq!(
            x.run("p", &json!({}), SECS5, &no_cancel()).unwrap_err(),
            RunError::NotInstalled
        );
    }

    #[test]
    fn detect_runners_finds_fake_executables_on_path() {
        let t = tempfile::tempdir().unwrap();
        assert!(detect_runners(&[t.path().to_path_buf()]).is_empty());
        script(t.path(), "claude", "echo x", "echo x");
        let found = detect_runners(&[t.path().to_path_buf()]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].tool(), Tool::Claude);
        assert!(found[0].available());
        script(t.path(), "codex", "echo x", "echo x");
        let found = detect_runners(&[t.path().to_path_buf()]);
        let tools: Vec<Tool> = found.iter().map(|r| r.tool()).collect();
        assert_eq!(tools, vec![Tool::Claude, Tool::Codex]);
    }
}
