//! Environment detection (Claude Code, Codex, Node.js, npm, git) and the
//! one-click npm install. Process execution is injected (`VersionProbe`,
//! `InstallExecutor`) so tests never run real programs.
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::extract::runner::find_program;
use crate::proc;

pub const TOOLS: [&str; 5] = ["claude", "codex", "node", "npm", "git"];
const VERSION_TIMEOUT: Duration = Duration::from_secs(2);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ToolStatus {
    pub name: String,
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    /// Only for claude / codex.
    pub logged_in: Option<bool>,
    /// Codex desktop app location (codex only), reported even without the CLI.
    pub desktop_app: Option<String>,
}

/// Runs `<program> --version` and returns its output.
pub trait VersionProbe: Send + Sync {
    fn output(&self, program: &Path) -> Option<String>;
}

pub struct SystemProbe;

impl VersionProbe for SystemProbe {
    fn output(&self, program: &Path) -> Option<String> {
        let mut cmd = Command::new(program);
        cmd.arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = proc::spawn_hidden(&mut cmd).ok()?;
        let out = proc::wait_with_timeout(child, VERSION_TIMEOUT, None).ok()?;
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        if s.trim().is_empty() {
            s = String::from_utf8_lossy(&out.stderr).into_owned();
        }
        Some(s)
    }
}

/// First `X.Y[.Z...]` token: `2.0.14 (Claude Code)` -> `2.0.14`,
/// `codex-cli 0.46.0` -> `0.46.0`, `v24.1.0` -> `24.1.0`.
pub fn parse_version(out: &str) -> Option<String> {
    for word in out.split_whitespace() {
        let w = word.trim_start_matches(|c: char| !c.is_ascii_digit());
        let w = w.trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
        let mut parts = w.split('.');
        let ok = parts.next().is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
            && parts.next().is_some_and(|p| p.chars().next().is_some_and(|c| c.is_ascii_digit()));
        if ok {
            return Some(w.to_string());
        }
    }
    None
}

/// Places where the Codex desktop app lives: an exact path, or a directory
/// containing an entry whose name starts with a prefix.
#[derive(Debug, Clone)]
pub enum DesktopCandidate {
    Exact(PathBuf),
    Prefix(PathBuf, String),
}

pub fn codex_desktop_candidates(home: &Path) -> Vec<DesktopCandidate> {
    use DesktopCandidate::*;
    let mut v = Vec::new();
    if cfg!(windows) {
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Local"));
        let roaming = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Roaming"));
        v.push(Exact(local.join("Programs").join("Codex")));
        v.push(Exact(roaming.join("Microsoft/Windows/Start Menu/Programs/Codex.lnk")));
        v.push(Prefix(local.join("Microsoft").join("WindowsApps"), "OpenAI.Codex".into()));
        v.push(Prefix(local.join("Packages"), "OpenAI.Codex".into()));
    } else if cfg!(target_os = "macos") {
        v.push(Exact(PathBuf::from("/Applications/Codex.app")));
        v.push(Exact(home.join("Applications").join("Codex.app")));
    }
    v
}

pub fn find_codex_desktop(candidates: &[DesktopCandidate]) -> Option<PathBuf> {
    for c in candidates {
        match c {
            DesktopCandidate::Exact(p) if p.exists() => return Some(p.clone()),
            DesktopCandidate::Prefix(dir, prefix) => {
                if let Ok(rd) = std::fs::read_dir(dir) {
                    for e in rd.flatten() {
                        if e.file_name().to_string_lossy().starts_with(prefix.as_str()) {
                            return Some(e.path());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Inputs for `detect` (production values come from `EnvCtx::system`).
pub struct EnvCtx<'a> {
    pub dirs: Vec<PathBuf>,
    pub claude_home: PathBuf,
    pub codex_home: PathBuf,
    pub desktop: Vec<DesktopCandidate>,
    pub probe: &'a dyn VersionProbe,
    /// macOS keychain check for the Claude login.
    pub claude_keychain: &'a dyn Fn() -> bool,
}

pub fn claude_keychain_present() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    // No `-w`: only checks the item exists, the secret is never printed.
    let mut cmd = Command::new("security");
    cmd.args(["find-generic-password", "-s", "Claude Code-credentials"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let Ok(child) = proc::spawn_hidden(&mut cmd) else { return false };
    proc::wait_with_timeout(child, VERSION_TIMEOUT, None).is_ok_and(|o| o.status.success())
}

pub fn detect(ctx: &EnvCtx) -> Vec<ToolStatus> {
    TOOLS
        .iter()
        .map(|name| {
            let path = find_program(std::ffi::OsStr::new(name), &ctx.dirs);
            let version = path.as_deref().and_then(|p| ctx.probe.output(p)).and_then(|o| parse_version(&o));
            let logged_in = match *name {
                "claude" => Some(
                    ctx.claude_home.join(".credentials.json").is_file() || (ctx.claude_keychain)(),
                ),
                "codex" => Some(ctx.codex_home.join("auth.json").is_file()),
                _ => None,
            };
            let desktop_app = (*name == "codex")
                .then(|| find_codex_desktop(&ctx.desktop))
                .flatten()
                .map(|p| p.to_string_lossy().into_owned());
            ToolStatus {
                name: name.to_string(),
                installed: path.is_some(),
                path: path.map(|p| p.to_string_lossy().into_owned()),
                version,
                logged_in,
                desktop_app,
            }
        })
        .collect()
}

/// npm package for the installable CLIs.
pub fn npm_package(tool: &str) -> Option<&'static str> {
    match tool {
        "claude" => Some("@anthropic-ai/claude-code"),
        "codex" => Some("@openai/codex"),
        _ => None,
    }
}

/// Runs a program, reporting every output line (stdout and stderr).
pub trait InstallExecutor: Send + Sync {
    /// Returns the exit code, or an error message.
    fn run(&self, program: &Path, args: &[String], on_line: &mut dyn FnMut(String)) -> Result<i32, String>;
}

pub struct SystemExecutor {
    /// PATH for the child (npm needs node next to it).
    pub dirs: Vec<PathBuf>,
}

fn pump<R: std::io::Read + Send + 'static>(r: R, tx: mpsc::Sender<String>) {
    std::thread::spawn(move || {
        let mut br = std::io::BufReader::new(r);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match br.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf).trim_end().to_string();
                    if !line.is_empty() && tx.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });
}

impl InstallExecutor for SystemExecutor {
    fn run(&self, program: &Path, args: &[String], on_line: &mut dyn FnMut(String)) -> Result<i32, String> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Ok(p) = std::env::join_paths(&self.dirs) {
            cmd.env("PATH", p);
        }
        let mut child = proc::spawn_hidden(&mut cmd).map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::channel();
        if let Some(o) = child.stdout.take() {
            pump(o, tx.clone());
        }
        if let Some(e) = child.stderr.take() {
            pump(e, tx.clone());
        }
        drop(tx);
        let start = Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => on_line(line),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if start.elapsed() > INSTALL_TIMEOUT {
                proc::kill_tree(&mut child);
                return Err("timeout".into());
            }
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        Ok(status.code().unwrap_or(-1))
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallEvent {
    Log { tool: String, line: String },
    Done { tool: String, ok: bool, code: Option<i32>, error: Option<String> },
}

/// `npm install -g <package>`; emits log lines then `Done`.
pub fn run_install(tool: &str, npm: &Path, exec: &dyn InstallExecutor, emit: &dyn Fn(InstallEvent)) -> bool {
    let Some(pkg) = npm_package(tool) else {
        emit(InstallEvent::Done { tool: tool.into(), ok: false, code: None, error: Some("unsupported".into()) });
        return false;
    };
    let args: Vec<String> = ["install", "-g", pkg].iter().map(|s| s.to_string()).collect();
    emit(InstallEvent::Log { tool: tool.into(), line: format!("$ npm {}", args.join(" ")) });
    let mut on_line = |line: String| emit(InstallEvent::Log { tool: tool.into(), line });
    let res = exec.run(npm, &args, &mut on_line);
    let (ok, code, error) = match res {
        Ok(0) => (true, Some(0), None),
        Ok(c) => (false, Some(c), None),
        Err(e) => (false, None, Some(e)),
    };
    emit(InstallEvent::Done { tool: tool.into(), ok, code, error });
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeProbe;
    impl VersionProbe for FakeProbe {
        fn output(&self, program: &Path) -> Option<String> {
            let n = program.file_stem()?.to_string_lossy().into_owned();
            Some(match n.as_str() {
                "claude" => "2.0.14 (Claude Code)\n".into(),
                "node" => "v24.1.0\n".into(),
                _ => "garbage".into(),
            })
        }
    }

    fn touch_exe(dir: &Path, name: &str) {
        let f = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
        std::fs::write(dir.join(f), b"").unwrap();
    }

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("2.0.14 (Claude Code)").as_deref(), Some("2.0.14"));
        assert_eq!(parse_version("codex-cli 0.46.0").as_deref(), Some("0.46.0"));
        assert_eq!(parse_version("v24.1.0\r\n").as_deref(), Some("24.1.0"));
        assert_eq!(parse_version("git version 2.47.1.windows.1").as_deref(), Some("2.47.1.windows.1"));
        assert_eq!(parse_version("10.9.2").as_deref(), Some("10.9.2"));
        assert_eq!(parse_version("no version here 3"), None);
    }

    #[test]
    fn detects_tools_login_and_codex_desktop() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        touch_exe(&bin, "claude");
        touch_exe(&bin, "node");
        let ch = tmp.path().join(".claude");
        let xh = tmp.path().join(".codex");
        std::fs::create_dir_all(&ch).unwrap();
        std::fs::create_dir_all(&xh).unwrap();
        std::fs::write(ch.join(".credentials.json"), "{}").unwrap();
        let apps = tmp.path().join("WindowsApps");
        std::fs::create_dir_all(apps.join("OpenAI.Codex_2p2nqsd0c76g0")).unwrap();
        let no_keychain = || false;
        let ctx = EnvCtx {
            dirs: vec![bin],
            claude_home: ch,
            codex_home: xh,
            desktop: vec![
                DesktopCandidate::Exact(tmp.path().join("Programs").join("Codex")),
                DesktopCandidate::Prefix(apps.clone(), "OpenAI.Codex".into()),
            ],
            probe: &FakeProbe,
            claude_keychain: &no_keychain,
        };
        let r = detect(&ctx);
        let get = |n: &str| r.iter().find(|t| t.name == n).unwrap().clone();
        let c = get("claude");
        assert!(c.installed);
        assert_eq!(c.version.as_deref(), Some("2.0.14"));
        assert_eq!(c.logged_in, Some(true));
        let x = get("codex");
        assert!(!x.installed, "desktop app without CLI");
        assert_eq!(x.logged_in, Some(false));
        assert!(x.desktop_app.unwrap().contains("OpenAI.Codex_"));
        assert_eq!(get("node").version.as_deref(), Some("24.1.0"));
        assert!(!get("git").installed);
        assert_eq!(get("git").logged_in, None);
        assert!(find_codex_desktop(&[DesktopCandidate::Prefix(apps, "Nope".into())]).is_none());
    }

    struct FakeExec {
        code: Result<i32, String>,
        seen: Mutex<Vec<(PathBuf, Vec<String>)>>,
    }
    impl InstallExecutor for FakeExec {
        fn run(&self, program: &Path, args: &[String], on_line: &mut dyn FnMut(String)) -> Result<i32, String> {
            self.seen.lock().unwrap().push((program.to_path_buf(), args.to_vec()));
            on_line("added 3 packages".into());
            on_line("done".into());
            self.code.clone()
        }
    }

    #[test]
    fn install_emits_log_then_done() {
        let exec = FakeExec { code: Ok(0), seen: Mutex::new(vec![]) };
        let events = Mutex::new(Vec::new());
        let ok = run_install("codex", Path::new("npm"), &exec, &|e| events.lock().unwrap().push(e));
        assert!(ok);
        let ev = events.into_inner().unwrap();
        assert_eq!(ev.len(), 4);
        assert_eq!(ev[0], InstallEvent::Log { tool: "codex".into(), line: "$ npm install -g @openai/codex".into() });
        assert_eq!(ev[1], InstallEvent::Log { tool: "codex".into(), line: "added 3 packages".into() });
        assert_eq!(ev[3], InstallEvent::Done { tool: "codex".into(), ok: true, code: Some(0), error: None });
        assert_eq!(exec.seen.lock().unwrap()[0].1, vec!["install", "-g", "@openai/codex"]);

        let failing = FakeExec { code: Ok(1), seen: Mutex::new(vec![]) };
        let events = Mutex::new(Vec::new());
        assert!(!run_install("claude", Path::new("npm"), &failing, &|e| events.lock().unwrap().push(e)));
        let last = events.into_inner().unwrap().pop().unwrap();
        assert_eq!(last, InstallEvent::Done { tool: "claude".into(), ok: false, code: Some(1), error: None });

        let events = Mutex::new(Vec::new());
        assert!(!run_install("git", Path::new("npm"), &exec, &|e| events.lock().unwrap().push(e)));
        assert_eq!(exec.seen.lock().unwrap().len(), 1, "unsupported tool never runs");
    }
}
