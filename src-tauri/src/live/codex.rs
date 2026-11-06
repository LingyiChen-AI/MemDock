//! Codex driver for live chat. See the module docs of `live` for the protocol.
//!
//! `codex app-server` speaks JSON-RPC 2.0 (without the `jsonrpc` field) as
//! newline-delimited JSON over stdio. All protocol state lives in the pure
//! [`mapper::Mapper`]; this file only does the process IO.
mod mapper;
#[cfg(test)]
mod tests;

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{Decision, Driver, Emit, LiveEvent, StartOptions};
use crate::extract::runner::{find_program, path_dirs};
use crate::proc;
use mapper::Mapper;

const STDERR_TAIL: usize = 4000;

struct CodexDriver {
    mapper: Arc<Mutex<Mapper>>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    child: Arc<Mutex<Child>>,
}

fn write_all(stdin: &Mutex<Option<ChildStdin>>, msgs: &[Value]) -> Result<(), String> {
    if msgs.is_empty() {
        return Ok(());
    }
    let mut guard = stdin.lock().unwrap();
    let w = guard.as_mut().ok_or("Codex 进程已结束")?;
    for m in msgs {
        let mut line = serde_json::to_string(m).map_err(|e| e.to_string())?;
        line.push('\n');
        w.write_all(line.as_bytes()).map_err(|e| format!("写入 Codex 失败：{e}"))?;
    }
    w.flush().map_err(|e| format!("写入 Codex 失败：{e}"))
}

/// Codex takes images as local files: writes a data URI to `%TEMP%/memdock-images/`.
pub fn save_image(uri: &str) -> Result<String, String> {
    use base64::Engine;
    let (media, data) = super::decode_data_uri(uri).ok_or("无法识别的图片数据")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| format!("图片解码失败：{e}"))?;
    let ext = match media {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    };
    let dir = std::env::temp_dir().join("memdock-images");
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法保存图片：{e}"))?;
    let path = dir.join(format!("{}.{ext}", uuid::Uuid::new_v4()));
    std::fs::write(&path, bytes).map_err(|e| format!("无法保存图片：{e}"))?;
    Ok(path.to_string_lossy().into_owned())
}

fn kill(child: &Mutex<Child>) {
    proc::kill_tree(&mut child.lock().unwrap());
}

impl Driver for CodexDriver {
    fn send(&mut self, text: &str, images: &[String]) -> Result<(), String> {
        let paths = images.iter().map(|uri| save_image(uri)).collect::<Result<Vec<_>, _>>()?;
        let msgs = self.mapper.lock().unwrap().send(text, &paths);
        write_all(&self.stdin, &msgs)
    }

    fn respond(&mut self, request_id: &str, decision: Decision) -> Result<(), String> {
        let msg = self.mapper.lock().unwrap().respond(request_id, decision)?;
        write_all(&self.stdin, &[msg])
    }

    fn interrupt(&mut self) -> Result<(), String> {
        let msgs = self.mapper.lock().unwrap().interrupt();
        write_all(&self.stdin, &msgs)
    }

    fn stop(&mut self) {
        self.mapper.lock().unwrap().mark_stopped();
        self.stdin.lock().unwrap().take();
        kill(&self.child);
    }
}

/// Arguments of `codex app-server [-c model_reasoning_effort=x] [-c k=v ...]`.
pub(crate) fn app_server_args(opts: &StartOptions, is_batch: bool) -> Result<Vec<String>, String> {
    let mut args = vec!["app-server".to_string()];
    if let Some(e) = super::clean_effort(&opts.effort) {
        args.push("-c".into());
        args.push(format!("model_reasoning_effort={e}"));
    }
    for kv in opts.extra_args.iter().map(|a| a.trim()).filter(|a| !a.is_empty()) {
        if is_batch && kv.contains('%') {
            return Err(format!("配置项不能包含 %：{kv}"));
        }
        args.push("-c".into());
        args.push(kv.to_string());
    }
    Ok(args)
}

/// Builds `codex app-server [-c k=v ...]`.
fn command(opts: &StartOptions) -> Result<Command, String> {
    let dirs = path_dirs();
    let program = find_program(std::ffi::OsStr::new("codex"), &dirs)
        .ok_or("未找到 codex 命令，请先安装 Codex CLI（npm install -g @openai/codex）")?;
    let is_batch = program
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    let args = app_server_args(opts, is_batch)?;
    let mut cmd = Command::new(&program);
    cmd.args(&args)
        .current_dir(&opts.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // npm-installed CLIs need the same augmented PATH (node) used to find them.
    if cfg!(target_os = "macos") {
        if let Ok(p) = std::env::join_paths(&dirs) {
            cmd.env("PATH", p);
        }
    }
    Ok(cmd)
}

/// Waits (briefly, without holding the lock for long) for the exit code.
fn exit_code(child: &Mutex<Child>) -> Option<i32> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(Some(st)) = child.lock().unwrap().try_wait() {
            return st.code();
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Launches the CLI and returns its driver; the reader threads report through `emit`.
pub fn spawn(opts: &StartOptions, emit: Emit) -> Result<Box<dyn Driver>, String> {
    let mut cmd = command(opts)?;
    let mut child = proc::spawn_hidden(&mut cmd).map_err(|e| format!("启动 Codex 失败：{e}"))?;
    let stdout = child.stdout.take().ok_or("无法读取 Codex 输出")?;
    let stderr = child.stderr.take();
    let stdin = Arc::new(Mutex::new(child.stdin.take()));
    let child = Arc::new(Mutex::new(child));
    let mapper = Arc::new(Mutex::new(Mapper::new(opts, env!("CARGO_PKG_VERSION"))));

    let tail = Arc::new(Mutex::new(String::new()));
    if let Some(mut err) = stderr {
        let tail = tail.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = err.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut t = tail.lock().unwrap();
                t.push_str(&String::from_utf8_lossy(&buf[..n]));
                if t.len() > STDERR_TAIL * 2 {
                    let mut cut = t.len() - STDERR_TAIL;
                    while !t.is_char_boundary(cut) {
                        cut += 1;
                    }
                    t.drain(..cut);
                }
            }
        });
    }

    let first = mapper.lock().unwrap().begin();
    if let Err(e) = write_all(&stdin, &first) {
        kill(&child);
        return Err(e);
    }

    {
        let (mapper, stdin, child) = (mapper.clone(), stdin.clone(), child.clone());
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = Vec::new();
            loop {
                line.clear();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let text = String::from_utf8_lossy(&line);
                // Lock only for the mapping; emit after releasing it.
                let step = mapper.lock().unwrap().on_line(&text);
                if let Err(e) = write_all(&stdin, &step.writes) {
                    emit(LiveEvent::Error { message: e, fatal: false });
                }
                for ev in step.events {
                    emit(ev);
                }
                if step.kill {
                    stdin.lock().unwrap().take();
                    kill(&child);
                }
            }
            stdin.lock().unwrap().take();
            let code = exit_code(&child);
            let stderr_tail = tail.lock().unwrap().clone();
            let events = mapper.lock().unwrap().on_exit(code, &stderr_tail);
            for ev in events {
                emit(ev);
            }
        });
    }

    Ok(Box::new(CodexDriver { mapper, stdin, child }))
}
