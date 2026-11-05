//! Environment detection, one-click install and data-folder commands.
//!
//! Events: `env://install-log` ({kind:"log", tool, line}) and
//! `env://install-done` ({kind:"done", tool, ok, code, error}).
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::commands::{lock, AppState, CommandError};
use crate::env::{
    claude_keychain_present, codex_desktop_candidates, detect, npm_package, run_install, EnvCtx,
    InstallEvent, InstallExecutor, SystemExecutor, SystemProbe, ToolStatus, VersionProbe,
};
use crate::extract::runner::{find_program, path_dirs};

pub const EVENT_LOG: &str = "env://install-log";
pub const EVENT_DONE: &str = "env://install-done";

#[derive(Serialize, Debug, Clone)]
pub struct EnvReport {
    pub tools: Vec<ToolStatus>,
    pub data_dir: String,
    /// Tool currently being installed.
    pub installing: Option<String>,
}

pub struct EnvState {
    app: Arc<AppState>,
    probe: Arc<dyn VersionProbe>,
    executor: Arc<dyn InstallExecutor>,
    installing: Mutex<Option<String>>,
}

impl EnvState {
    pub fn new(app: Arc<AppState>) -> Self {
        Self::with(app, Arc::new(SystemProbe), Arc::new(SystemExecutor { dirs: path_dirs() }))
    }
    pub fn with(app: Arc<AppState>, probe: Arc<dyn VersionProbe>, executor: Arc<dyn InstallExecutor>) -> Self {
        Self { app, probe, executor, installing: Mutex::new(None) }
    }
}

pub fn env_detect_impl(es: &EnvState) -> EnvReport {
    let home = es.app.home.clone();
    let keychain = claude_keychain_present;
    let ctx = EnvCtx {
        dirs: path_dirs(),
        claude_home: es.app.paths.claude_home.clone(),
        codex_home: es.app.paths.codex_home.clone(),
        desktop: codex_desktop_candidates(&home),
        probe: es.probe.as_ref(),
        claude_keychain: &keychain,
    };
    EnvReport {
        tools: detect(&ctx),
        data_dir: es.app.data_dir.to_string_lossy().into_owned(),
        installing: lock(&es.installing).clone(),
    }
}

/// Starts `npm install -g` for `tool` on a background thread.
pub fn env_install_impl(
    es: &Arc<EnvState>,
    tool: &str,
    npm: Option<PathBuf>,
    emit: Arc<dyn Fn(InstallEvent) + Send + Sync>,
) -> Result<(), CommandError> {
    if npm_package(tool).is_none() {
        return Err(CommandError::new("invalid", format!("cannot install {tool}")));
    }
    let Some(npm) = npm else {
        return Err(CommandError::new("npm_missing", "npm was not found; install Node.js first"));
    };
    {
        let mut g = lock(&es.installing);
        if g.is_some() {
            return Err(CommandError::new("busy", "an installation is already running"));
        }
        *g = Some(tool.to_string());
    }
    let me = es.clone();
    let tool = tool.to_string();
    std::thread::spawn(move || {
        run_install(&tool, &npm, me.executor.as_ref(), &|e| emit(e));
        *lock(&me.installing) = None;
    });
    Ok(())
}

/// Opens `dir` in the system file manager.
fn open_folder(dir: &std::path::Path) -> std::io::Result<()> {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program).arg(dir).spawn().map(|_| ())
}

type Es<'a> = State<'a, Arc<EnvState>>;

#[tauri::command(async)]
pub fn env_detect(es: Es) -> EnvReport {
    env_detect_impl(&es)
}

#[tauri::command(async)]
pub fn env_install(es: Es, app: AppHandle, tool: String) -> Result<(), CommandError> {
    let npm = find_program(std::ffi::OsStr::new("npm"), &path_dirs());
    let emit = Arc::new(move |e: InstallEvent| {
        let name = match e {
            InstallEvent::Log { .. } => EVENT_LOG,
            InstallEvent::Done { .. } => EVENT_DONE,
        };
        let _ = app.emit(name, e);
    });
    env_install_impl(es.inner(), &tool, npm, emit)
}

#[tauri::command(async)]
pub fn open_data_dir(es: Es) -> Result<String, CommandError> {
    let dir = es.app.data_dir.clone();
    std::fs::create_dir_all(&dir)?;
    open_folder(&dir)?;
    Ok(dir.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AgentPaths;
    use std::path::Path;
    use std::sync::mpsc;

    struct NoProbe;
    impl VersionProbe for NoProbe {
        fn output(&self, _: &Path) -> Option<String> {
            None
        }
    }
    struct Exec;
    impl InstallExecutor for Exec {
        fn run(&self, _: &Path, _: &[String], on_line: &mut dyn FnMut(String)) -> Result<i32, String> {
            on_line("ok".into());
            Ok(0)
        }
    }

    #[test]
    fn install_requires_npm_and_streams_events() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AgentPaths::from_home(&tmp.path().join("home"));
        let app = Arc::new(AppState::with_home(paths, tmp.path().join("data"), tmp.path().join("home")));
        let es = Arc::new(EnvState::with(app, Arc::new(NoProbe), Arc::new(Exec)));
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let emit: Arc<dyn Fn(InstallEvent) + Send + Sync> = Arc::new(move |e| {
            let _ = lock(&tx).send(e);
        });
        let err = env_install_impl(&es, "claude", None, emit.clone()).unwrap_err();
        assert_eq!(err.code, "npm_missing");
        assert_eq!(env_install_impl(&es, "git", Some("npm".into()), emit.clone()).unwrap_err().code, "invalid");
        env_install_impl(&es, "claude", Some("npm".into()), emit).unwrap();
        let mut got = Vec::new();
        while let Ok(e) = rx.recv_timeout(std::time::Duration::from_secs(5)) {
            let done = matches!(e, InstallEvent::Done { .. });
            got.push(e);
            if done {
                break;
            }
        }
        assert_eq!(got.len(), 3);
        assert!(matches!(&got[2], InstallEvent::Done { ok: true, .. }));
        let report = env_detect_impl(&es);
        assert_eq!(report.tools.len(), 5);
    }
}
