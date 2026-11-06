//! Terminal mode: the real interactive Claude Code / Codex CLI in a pseudo
//! terminal (ConPTY on Windows), rendered by xterm.js on the chat page.
//!
//! `pty_open` spawns `claude [--resume id]` / `codex [resume id]` (or the plain
//! shell for `tool: "shell"`) in the conversation's folder. A reader thread forwards the output as Tauri events
//! `pty://data` {pty_id, data} (UTF-8 text, split multibyte sequences are
//! carried over to the next chunk) and, once the process ended and its output
//! is drained, `pty://exit` {pty_id, code}.
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};

use crate::extract::runner::{find_program, path_dirs};
use crate::live::{clean_effort, PermissionMode};

#[cfg(test)]
mod tests;

/// What runs in the terminal: an agent CLI or the user's plain shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PtyTool {
    Claude,
    Codex,
    Shell,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PtyOpenOptions {
    pub tool: PtyTool,
    pub cwd: String,
    /// Claude session id / Codex thread id; `None` starts a new conversation.
    /// `resume_id`, `permission`, `model` and `effort` are ignored for `shell`.
    #[serde(default)]
    pub resume_id: Option<String>,
    #[serde(default)]
    pub permission: PermissionMode,
    #[serde(default)]
    pub model: Option<String>,
    /// Thinking effort: Claude `--effort`, Codex `-c model_reasoning_effort=`.
    #[serde(default)]
    pub effort: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

/// The plain shell to start: program, arguments and the status bar label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellChoice {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub label: String,
}

/// Windows: `pwsh.exe` when `find` locates it, else `powershell.exe` (both `-NoLogo`).
/// macOS / Linux: `$SHELL -l`, else `/bin/zsh` when it exists, else `/bin/bash`.
pub fn shell_choice(
    windows: bool,
    find: impl Fn(&str) -> Option<PathBuf>,
    shell_env: Option<String>,
    exists: impl Fn(&Path) -> bool,
) -> ShellChoice {
    if windows {
        let program = find("pwsh")
            .or_else(|| find("powershell"))
            .unwrap_or_else(|| PathBuf::from("powershell.exe"));
        return ShellChoice { program, args: vec!["-NoLogo".into()], label: "PowerShell".into() };
    }
    let program = shell_env
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let zsh = PathBuf::from("/bin/zsh");
            if exists(&zsh) { zsh } else { PathBuf::from("/bin/bash") }
        });
    let label = program.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "shell".into());
    ShellChoice { program, args: vec!["-l".into()], label }
}

/// The shell of this machine.
fn local_shell() -> ShellChoice {
    let mut dirs = path_dirs();
    if cfg!(windows) {
        // Windows PowerShell 5.1 lives outside PATH-only installs sometimes.
        if let Some(root) = std::env::var_os("SystemRoot") {
            dirs.push(PathBuf::from(root).join("System32").join("WindowsPowerShell").join("v1.0"));
        }
    }
    shell_choice(
        cfg!(windows),
        |name| find_program(OsStr::new(name), &dirs),
        std::env::var("SHELL").ok(),
        |p| p.exists(),
    )
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PtyOpened {
    pub pty_id: String,
    /// What runs, for the status bar (`claude --resume 1234…`).
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PtyData {
    pub pty_id: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PtyExit {
    pub pty_id: String,
    pub code: Option<i64>,
}

fn clean(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// CLI arguments (without the program) for the interactive CLI.
///
/// | mode        | Claude Code                          | Codex                                        |
/// |-------------|--------------------------------------|----------------------------------------------|
/// | `ask`       | (default)                            | `-a on-request -s read-only`                 |
/// | `auto_edit` | `--permission-mode acceptEdits`      | `-a on-request -s workspace-write`           |
/// | `plan`      | `--permission-mode plan`             | `-a on-request -s read-only`                 |
/// | `bypass`    | `--dangerously-skip-permissions`     | `--dangerously-bypass-approvals-and-sandbox` |
pub fn build_args(opts: &PtyOpenOptions) -> Vec<String> {
    let mut a: Vec<String> = vec![];
    let s = |a: &mut Vec<String>, xs: &[&str]| a.extend(xs.iter().map(|x| x.to_string()));
    let effort = clean_effort(&opts.effort);
    match opts.tool {
        PtyTool::Shell => {}
        PtyTool::Claude => {
            if let Some(id) = clean(&opts.resume_id) {
                s(&mut a, &["--resume", id]);
            }
            if let Some(m) = clean(&opts.model) {
                s(&mut a, &["--model", m]);
            }
            if let Some(e) = effort {
                s(&mut a, &["--effort", e]);
            }
            match opts.permission {
                PermissionMode::Ask => {}
                PermissionMode::AutoEdit => s(&mut a, &["--permission-mode", "acceptEdits"]),
                PermissionMode::Plan => s(&mut a, &["--permission-mode", "plan"]),
                PermissionMode::Bypass => s(&mut a, &["--dangerously-skip-permissions"]),
            }
        }
        PtyTool::Codex => {
            if clean(&opts.resume_id).is_some() {
                a.push("resume".into());
            }
            match opts.permission {
                // Codex no longer supports "untrusted": a read-only sandbox makes every write ask.
                PermissionMode::Ask => s(&mut a, &["-a", "on-request", "-s", "read-only"]),
                PermissionMode::AutoEdit => s(&mut a, &["-a", "on-request", "-s", "workspace-write"]),
                PermissionMode::Plan => s(&mut a, &["-a", "on-request", "-s", "read-only"]),
                PermissionMode::Bypass => s(&mut a, &["--dangerously-bypass-approvals-and-sandbox"]),
            }
            if let Some(m) = clean(&opts.model) {
                s(&mut a, &["-m", m]);
            }
            if let Some(e) = effort {
                a.push("-c".into());
                a.push(format!("model_reasoning_effort={e}"));
            }
            if let Some(id) = clean(&opts.resume_id) {
                a.push(id.to_string());
            }
        }
    }
    a
}

fn program_name(tool: PtyTool) -> &'static str {
    match tool {
        PtyTool::Claude => "claude",
        PtyTool::Codex => "codex",
        PtyTool::Shell => "shell",
    }
}

/// `claude --resume 1234abcd…` for the status bar (ids shortened); the shell's
/// name (`PowerShell`, `zsh`) for a plain terminal.
pub fn display_command(opts: &PtyOpenOptions) -> String {
    if opts.tool == PtyTool::Shell {
        return local_shell().label;
    }
    let mut parts = vec![program_name(opts.tool).to_string()];
    let id = clean(&opts.resume_id);
    for a in build_args(opts) {
        if id == Some(a.as_str()) && a.chars().count() > 8 {
            parts.push(format!("{}…", a.chars().take(8).collect::<String>()));
        } else {
            parts.push(a);
        }
    }
    parts.join(" ")
}

fn is_batch(program: &Path) -> bool {
    program
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"))
}

/// Program + argv actually launched. npm `.cmd` / `.bat` shims cannot be
/// started directly by CreateProcess, so they run through `cmd.exe /d /c`.
pub fn launch_argv(program: &Path, args: &[String]) -> Vec<String> {
    let mut argv = vec![];
    if is_batch(program) {
        argv.extend(["cmd.exe".to_string(), "/d".into(), "/c".into()]);
    }
    argv.push(program.to_string_lossy().into_owned());
    argv.extend(args.iter().cloned());
    argv
}

/// Decodes a byte stream to UTF-8 text chunk by chunk: an incomplete sequence at
/// the end of a chunk waits for the next one, invalid bytes become U+FFFD.
#[derive(Debug, Default)]
pub struct Utf8Chunker {
    pending: Vec<u8>,
}

impl Utf8Chunker {
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut out = String::new();
        let mut rest: &[u8] = &self.pending;
        loop {
            match std::str::from_utf8(rest) {
                Ok(s) => {
                    out.push_str(s);
                    rest = &[];
                    break;
                }
                Err(e) => {
                    let (ok, tail) = rest.split_at(e.valid_up_to());
                    out.push_str(std::str::from_utf8(ok).unwrap_or_default());
                    match e.error_len() {
                        Some(n) => {
                            out.push('\u{FFFD}');
                            rest = &tail[n..];
                        }
                        None => {
                            rest = tail;
                            break;
                        }
                    }
                }
            }
        }
        self.pending = rest.to_vec();
        out
    }

    /// Whatever is left at end of stream.
    pub fn finish(&mut self) -> String {
        let s = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        s
    }
}

pub type Publish = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

struct Session {
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    pid: Option<u32>,
}

impl Session {
    fn kill(&self) {
        if let Some(pid) = self.pid {
            kill_pid_tree(pid);
        }
        let _ = self.killer.lock().unwrap().kill();
        self.writer.lock().unwrap().take();
        // Closing the pseudo console lets the reader see end of stream.
        self.master.lock().unwrap().take();
    }
}

fn kill_pid_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(0x0800_0000)
            .status();
    }
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// All open terminals.
pub struct PtyManager {
    sessions: Arc<Mutex<HashMap<String, Arc<Session>>>>,
    publish: Publish,
}

impl PtyManager {
    pub fn new(publish: Publish) -> Self {
        Self { sessions: Arc::new(Mutex::new(HashMap::new())), publish }
    }

    pub fn production(app: tauri::AppHandle) -> Self {
        Self::new(Arc::new(move |name: &str, payload: serde_json::Value| {
            use tauri::Emitter;
            let _ = app.emit(name, payload);
        }))
    }

    pub fn open(&self, opts: &PtyOpenOptions) -> Result<PtyOpened, String> {
        let dirs = path_dirs();
        if opts.tool == PtyTool::Shell {
            let sh = local_shell();
            return self
                .open_program(&sh.program, &sh.args, opts, &dirs)
                .map(|pty_id| PtyOpened { pty_id, command: sh.label });
        }
        let name = program_name(opts.tool);
        let program = find_program(OsStr::new(name), &dirs).ok_or_else(|| match opts.tool {
            PtyTool::Codex => "未找到 codex 命令，请先安装 Codex CLI（npm install -g @openai/codex）".to_string(),
            _ => "未找到 Claude Code 命令行（claude），请先安装并确认它在 PATH 中。".to_string(),
        })?;
        self.open_program(&program, &build_args(opts), opts, &dirs)
            .map(|pty_id| PtyOpened { pty_id, command: display_command(opts) })
    }

    /// Spawns `program args` in a new pseudo terminal; returns the pty id.
    pub fn open_program(&self, program: &Path, args: &[String], opts: &PtyOpenOptions, dirs: &[PathBuf]) -> Result<String, String> {
        if is_batch(program) && args.iter().any(|a| a.contains(['%', '"', '&', '|', '<', '>', '^'])) {
            return Err("参数包含命令行特殊字符，无法安全传给 .cmd 程序".into());
        }
        let size = PtySize { rows: opts.rows.max(2), cols: opts.cols.max(10), pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size).map_err(|e| format!("无法创建终端：{e}"))?;
        let argv = launch_argv(program, args);
        let mut cmd = CommandBuilder::new(&argv[0]);
        cmd.args(&argv[1..]);
        cmd.cwd(&opts.cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env_remove("CLAUDECODE");
        if cfg!(target_os = "macos")
            && let Ok(p) = std::env::join_paths(dirs)
        {
            cmd.env("PATH", p);
        }
        let mut child = pair.slave.spawn_command(cmd).map_err(|e| format!("启动 {} 失败：{e}", program.display()))?;
        // The child holds its own handle; ours would keep the terminal open.
        drop(pair.slave);
        let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let pid = child.process_id();
        let session = Arc::new(Session {
            master: Mutex::new(Some(pair.master)),
            writer: Mutex::new(Some(writer)),
            killer: Mutex::new(child.clone_killer()),
            pid,
        });
        let pty_id = uuid::Uuid::new_v4().to_string();
        self.sessions.lock().unwrap().insert(pty_id.clone(), session.clone());

        // Waiter: exit code, then close the pseudo console so the reader drains and ends.
        let (code_tx, code_rx) = mpsc::channel::<Option<i64>>();
        {
            let session = session.clone();
            thread::spawn(move || {
                let code = child.wait().ok().map(|s| i64::from(s.exit_code()));
                let _ = code_tx.send(code);
                // Give the last output a moment to arrive before closing (Windows
                // ConPTY only ends the output stream when the console is closed).
                thread::sleep(Duration::from_millis(300));
                session.writer.lock().unwrap().take();
                session.master.lock().unwrap().take();
            });
        }
        // Reader: forward output, then report the exit once.
        {
            let publish = self.publish.clone();
            let sessions = self.sessions.clone();
            let id = pty_id.clone();
            thread::spawn(move || {
                let mut reader = reader;
                let mut dec = Utf8Chunker::default();
                let mut buf = [0u8; 16 * 1024];
                let send = |data: String| {
                    if !data.is_empty() {
                        let payload = PtyData { pty_id: id.clone(), data };
                        publish("pty://data", serde_json::to_value(payload).unwrap_or_default());
                    }
                };
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => send(dec.push(&buf[..n])),
                    }
                }
                send(dec.finish());
                let code = code_rx.recv_timeout(Duration::from_secs(3)).ok().flatten();
                sessions.lock().unwrap().remove(&id);
                let payload = PtyExit { pty_id: id.clone(), code };
                publish("pty://exit", serde_json::to_value(payload).unwrap_or_default());
            });
        }
        Ok(pty_id)
    }

    fn get(&self, pty_id: &str) -> Result<Arc<Session>, String> {
        self.sessions.lock().unwrap().get(pty_id).cloned().ok_or_else(|| "终端已结束".to_string())
    }

    pub fn write(&self, pty_id: &str, data: &str) -> Result<(), String> {
        let s = self.get(pty_id)?;
        let mut w = s.writer.lock().unwrap();
        let w = w.as_mut().ok_or("终端已结束")?;
        w.write_all(data.as_bytes()).and_then(|_| w.flush()).map_err(|e| e.to_string())
    }

    pub fn resize(&self, pty_id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let s = self.get(pty_id)?;
        let m = s.master.lock().unwrap();
        let Some(m) = m.as_ref() else { return Ok(()) };
        m.resize(PtySize { rows: rows.max(2), cols: cols.max(10), pixel_width: 0, pixel_height: 0 })
            .map_err(|e| e.to_string())
    }

    /// Kills the process tree; the reader then reports `pty://exit`. Unknown ids are fine.
    pub fn close(&self, pty_id: &str) {
        let s = self.sessions.lock().unwrap().get(pty_id).cloned();
        if let Some(s) = s {
            s.kill();
        }
    }

    pub fn close_all(&self) {
        let all: Vec<_> = self.sessions.lock().unwrap().values().cloned().collect();
        for s in all {
            s.kill();
        }
    }

    pub fn is_open(&self, pty_id: &str) -> bool {
        self.sessions.lock().unwrap().contains_key(pty_id)
    }
}

#[tauri::command]
pub async fn pty_open(mgr: tauri::State<'_, Arc<PtyManager>>, opts: PtyOpenOptions) -> Result<PtyOpened, String> {
    let mgr = mgr.inner().clone();
    tauri::async_runtime::spawn_blocking(move || mgr.open(&opts)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn pty_write(mgr: tauri::State<'_, Arc<PtyManager>>, pty_id: String, data: String) -> Result<(), String> {
    mgr.write(&pty_id, &data)
}

#[tauri::command]
pub fn pty_resize(mgr: tauri::State<'_, Arc<PtyManager>>, pty_id: String, cols: u16, rows: u16) -> Result<(), String> {
    mgr.resize(&pty_id, cols, rows)
}

#[tauri::command]
pub async fn pty_close(mgr: tauri::State<'_, Arc<PtyManager>>, pty_id: String) -> Result<(), String> {
    let mgr = mgr.inner().clone();
    tauri::async_runtime::spawn_blocking(move || mgr.close(&pty_id)).await.map_err(|e| e.to_string())
}
