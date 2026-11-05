//! Shared child-process helpers: hidden spawn, pipe draining, timeout /
//! cancel polling and process-tree kill. Used by `skills::git` and the
//! extraction CLI runners.
use std::io::Read;
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

fn drain<R: Read + Send + 'static>(mut r: R) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        buf
    })
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
    mut child: Child,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<Output, WaitError> {
    let out = child.stdout.take().map(drain);
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
    let stdout = out.map(|h| h.join().unwrap_or_default()).unwrap_or_default();
    let stderr = err.map(|h| h.join().unwrap_or_default()).unwrap_or_default();
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}
