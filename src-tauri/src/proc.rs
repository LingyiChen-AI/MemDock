//! Shared child-process helpers: hidden spawn, pipe draining, timeout /
//! cancel polling and process-tree kill. Used by `skills::git` and the
//! extraction CLI runners.
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// Why waiting for a child did not produce an exit status.
#[derive(Debug, Clone, PartialEq)]
pub enum WaitError {
    Timeout,
    Cancelled,
    Io(String),
}

/// Spawns `cmd` without a console window; on Unix it leads its own process group
/// so the whole tree can be killed. Stdout/stderr must be configured by the caller.
pub fn spawn_hidden(cmd: &mut Command) -> std::io::Result<Child> {
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
    cmd.spawn()
}

/// Reader thread handle; `join` waits at most `DRAIN_JOIN_WAIT` so a grandchild
/// still holding the pipe cannot hang the caller.
struct Drain(std::sync::mpsc::Receiver<Vec<u8>>);

const DRAIN_JOIN_WAIT: Duration = Duration::from_secs(5);

impl Drain {
    fn join(self) -> Vec<u8> {
        self.0.recv_timeout(DRAIN_JOIN_WAIT).unwrap_or_default()
    }
}

fn drain<R: Read + Send + 'static>(mut r: R) -> Drain {
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    Drain(rx)
}

/// Kills the child and its descendants, then reaps it.
pub fn kill_tree(child: &mut Child) {
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

/// Waits for `child` (draining stdout/stderr on reader threads), polling the
/// optional `cancel` flag and the timeout every 20 ms. On timeout / cancel the
/// process tree is killed.
pub fn wait_with_timeout(
    child: Child,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<Output, WaitError> {
    wait_with_stdout_lines(child, timeout, cancel, None)
}

pub(crate) fn wait_with_stdout_lines(
    mut child: Child,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
    on_line: Option<Box<dyn FnMut(&[u8]) + Send>>,
) -> Result<Output, WaitError> {
    let out = child.stdout.take().map(|stdout| {
        let Some(mut on_line) = on_line else { return drain(stdout) };
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = Vec::new();
            let mut buffer = Vec::new();
            while reader.read_until(b'\n', &mut line).is_ok_and(|size| size > 0) {
                on_line(&line);
                buffer.append(&mut line);
            }
            let _ = tx.send(buffer);
        });
        Drain(rx)
    });
    let err = child.stderr.take().map(drain);
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if cancel.is_some_and(|c| c.load(Ordering::SeqCst)) {
                    kill_tree(&mut child);
                    return Err(WaitError::Cancelled);
                }
                if start.elapsed() >= timeout {
                    kill_tree(&mut child);
                    return Err(WaitError::Timeout);
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                kill_tree(&mut child);
                return Err(WaitError::Io(e.to_string()));
            }
        }
    };
    let stdout = out.map(|h| h.join()).unwrap_or_default();
    let stderr = err.map(|h| h.join()).unwrap_or_default();
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}
