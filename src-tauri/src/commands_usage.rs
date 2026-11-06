//! Usage statistics commands and the background log scanner.
//!
//! Event: `usage://updated` (no payload) after the first scan and after log
//! changes (throttled: 5 s after the first change event).
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use crate::commands::{lock, modify_settings, AppState, CommandError};
use crate::model::Tool;
use crate::settings::{load_settings, Settings};
use crate::usage::aggregate::{build_report, local_time, to_csv, Query, UsageReport, View};
use crate::usage::cache::UsageCache;
use crate::usage::{claude_quota, codex_log_quota, codex_quota, pricing, QuotaDto, ToolQuota};

pub const EVENT_UPDATED: &str = "usage://updated";
const THROTTLE: Duration = Duration::from_secs(5);
const FALLBACK_RESCAN: Duration = Duration::from_secs(300);
const CLAUDE_QUOTA_TTL: Duration = Duration::from_secs(60);
/// The live Codex lookup is cached a little longer (the page polls every minute).
const CODEX_QUOTA_TTL: Duration = Duration::from_secs(180);

pub type Emit = Arc<dyn Fn(&str, Value) + Send + Sync>;

pub struct UsageState {
    app: Arc<AppState>,
    cache: Mutex<UsageCache>,
    cache_path: PathBuf,
    /// Serializes refreshes (parsing happens outside the cache lock).
    refresh_lock: Mutex<()>,
    scanning: AtomicBool,
    claude_quota: Mutex<Option<(Instant, ToolQuota)>>,
    codex_quota: Mutex<Option<(Instant, ToolQuota)>>,
}

impl UsageState {
    pub fn new(app: Arc<AppState>) -> Self {
        let cache_path = app.data_dir.join("usage-cache.json");
        Self {
            app,
            cache: Mutex::new(UsageCache::default()),
            cache_path,
            refresh_lock: Mutex::new(()),
            scanning: AtomicBool::new(true),
            claude_quota: Mutex::new(None),
            codex_quota: Mutex::new(None),
        }
    }

    /// Loads the on-disk cache (only before the first refresh).
    pub fn load_cache(&self) {
        *lock(&self.cache) = UsageCache::load(&self.cache_path);
    }

    /// Re-parses changed logs; saves and returns true when something changed.
    pub fn refresh(&self) -> bool {
        let _r = lock(&self.refresh_lock);
        let mut work = lock(&self.cache).clone();
        let changed = work.refresh(&self.app.paths.claude_home, &self.app.paths.codex_home);
        if changed {
            if let Err(e) = work.save(&self.cache_path) {
                eprintln!("memdock: saving usage cache failed: {e}");
            }
            *lock(&self.cache) = work;
        }
        self.scanning.store(false, Ordering::SeqCst);
        changed
    }

    /// Background thread: load cache, first scan, then watch the log folders.
    pub fn start(self: &Arc<Self>, emit: Emit) {
        let me = self.clone();
        std::thread::spawn(move || {
            me.load_cache();
            me.refresh();
            emit(EVENT_UPDATED, Value::Null);
            me.watch_loop(&emit);
        });
    }

    fn watch_loop(&self, emit: &Emit) {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok() {
                let _ = tx.send(());
            }
        });
        let mut watcher = match watcher {
            Ok(w) => Some(w),
            Err(e) => {
                eprintln!("memdock: usage watcher unavailable: {e}");
                None
            }
        };
        if let Some(w) = watcher.as_mut() {
            let p = &self.app.paths;
            for dir in [
                p.claude_home.join("projects"),
                p.codex_home.join("sessions"),
                p.codex_home.join("archived_sessions"),
            ] {
                if dir.is_dir() {
                    let _ = w.watch(&dir, RecursiveMode::Recursive);
                }
            }
        }
        loop {
            match rx.recv_timeout(FALLBACK_RESCAN) {
                Ok(()) => {
                    std::thread::sleep(THROTTLE);
                    while rx.try_recv().is_ok() {}
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    // Watcher failed: fall back to periodic scans.
                    std::thread::sleep(FALLBACK_RESCAN);
                }
            }
            if self.refresh() {
                emit(EVENT_UPDATED, Value::Null);
            }
        }
    }
}

fn query(view: View, from: String, to: String, tools: Vec<Tool>, models: Vec<String>) -> Query {
    Query { view, from, to, tools, models }
}

pub fn usage_summary_impl(us: &UsageState, q: &Query) -> Result<UsageReport, CommandError> {
    let cache = lock(&us.cache);
    let mut r = build_report(cache.events(), q, pricing::table(), local_time)
        .map_err(|e| CommandError::new("invalid", e))?;
    r.scanning = us.scanning.load(Ordering::SeqCst);
    Ok(r)
}

pub fn usage_export_csv_impl(us: &UsageState, q: &Query, path: &Path) -> Result<(), CommandError> {
    let r = usage_summary_impl(us, q)?;
    // BOM so spreadsheet apps read UTF-8 model names correctly.
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(to_csv(&r).as_bytes());
    std::fs::write(path, bytes)?;
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

type Fetch<'a> = &'a dyn Fn(&Path, i64) -> ToolQuota;

/// Returns the cached result when it is younger than `ttl`, otherwise fetches.
fn cached_fetch(slot: &Mutex<Option<(Instant, ToolQuota)>>, ttl: Duration, fetch: impl FnOnce() -> ToolQuota) -> ToolQuota {
    if let Some((_, q)) = lock(slot).as_ref().filter(|(at, _)| at.elapsed() < ttl) {
        return q.clone();
    }
    let q = fetch();
    *lock(slot) = Some((Instant::now(), q.clone()));
    q
}

/// The fetchers are injected so tests never touch the network or real
/// credentials. Both lookups are opt-in; Codex falls back to the newest log
/// snapshot when its live lookup is off or fails (`live_error` says why).
pub fn usage_quota_impl(us: &UsageState, fetch_claude: Fetch, fetch_codex: Fetch) -> QuotaDto {
    let now = now_ms();
    let settings = load_settings(&us.app.settings_path);
    let claude = if !settings.claude_quota_enabled {
        ToolQuota::with_status("disabled", None)
    } else {
        cached_fetch(&us.claude_quota, CLAUDE_QUOTA_TTL, || fetch_claude(&us.app.paths.claude_home, now))
    };
    let log = codex_log_quota(lock(&us.cache).latest_rate(), now);
    let codex = if !settings.codex_quota_enabled {
        log
    } else {
        let live = cached_fetch(&us.codex_quota, CODEX_QUOTA_TTL, || fetch_codex(&us.app.paths.codex_home, now));
        if live.status == "ok" {
            live
        } else if log.status == "ok" {
            ToolQuota { live_error: live.reason.clone(), ..log }
        } else {
            live
        }
    };
    QuotaDto { claude, codex }
}

pub fn set_claude_quota_enabled_impl(us: &UsageState, enabled: bool) -> Result<Settings, CommandError> {
    *lock(&us.claude_quota) = None;
    modify_settings(&us.app, |s| s.claude_quota_enabled = enabled)
}

pub fn set_codex_quota_enabled_impl(us: &UsageState, enabled: bool) -> Result<Settings, CommandError> {
    *lock(&us.codex_quota) = None;
    modify_settings(&us.app, |s| s.codex_quota_enabled = enabled)
}

type Us<'a> = State<'a, Arc<UsageState>>;

#[tauri::command(async)]
pub fn usage_summary(
    us: Us,
    view: View,
    from: String,
    to: String,
    tools: Vec<Tool>,
    models: Vec<String>,
) -> Result<UsageReport, CommandError> {
    usage_summary_impl(&us, &query(view, from, to, tools, models))
}

#[tauri::command(async)]
pub fn usage_quota(us: Us) -> QuotaDto {
    usage_quota_impl(&us, &claude_quota::fetch, &codex_quota::fetch)
}

#[tauri::command(async)]
pub fn usage_export_csv(
    us: Us,
    view: View,
    from: String,
    to: String,
    tools: Vec<Tool>,
    models: Vec<String>,
    path: String,
) -> Result<(), CommandError> {
    usage_export_csv_impl(&us, &query(view, from, to, tools, models), Path::new(&path))
}

#[tauri::command(async)]
pub fn set_claude_quota_enabled(us: Us, enabled: bool) -> Result<Settings, CommandError> {
    set_claude_quota_enabled_impl(&us, enabled)
}

#[tauri::command(async)]
pub fn set_codex_quota_enabled(us: Us, enabled: bool) -> Result<Settings, CommandError> {
    set_codex_quota_enabled_impl(&us, enabled)
}

/// Rescans now (the "refresh" button); returns whether anything changed.
#[tauri::command(async)]
pub fn usage_rescan(us: Us, app: AppHandle) -> bool {
    let changed = us.refresh();
    if changed {
        let _ = app.emit(EVENT_UPDATED, ());
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AgentPaths;
    use std::cell::Cell;
    use std::fs;

    fn state(tmp: &Path) -> UsageState {
        let paths = AgentPaths::from_home(&tmp.join("home"));
        fs::create_dir_all(paths.claude_home.join("projects").join("p")).unwrap();
        fs::create_dir_all(&paths.codex_home).unwrap();
        fs::write(
            paths.claude_home.join("projects").join("p").join("s.jsonl"),
            r#"{"type":"assistant","requestId":"r","timestamp":"2026-11-05T10:00:00Z","message":{"model":"claude-opus-5-5","id":"m","usage":{"input_tokens":10,"output_tokens":5}}}"#,
        )
        .unwrap();
        let app = Arc::new(AppState::with_home(paths, tmp.join("data"), tmp.join("home")));
        UsageState::new(app)
    }

    #[test]
    fn summary_after_refresh_and_quota_is_off_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let us = state(tmp.path());
        assert!(us.refresh());
        assert!(tmp.path().join("data").join("usage-cache.json").exists());
        let q = query(View::Monthly, "2026-01-01".into(), "2026-12-31".into(), vec![], vec![]);
        let r = usage_summary_impl(&us, &q).unwrap();
        assert!(!r.scanning);
        assert_eq!(r.summary.totals.total_tokens, 15);
        assert_eq!(r.summary.totals.requests, 1);

        let calls = Cell::new(0);
        let fake = |_: &Path, _: i64| {
            calls.set(calls.get() + 1);
            ToolQuota::with_status("ok", None)
        };
        let codex_calls = Cell::new(0);
        let fake_codex = |_: &Path, _: i64| {
            codex_calls.set(codex_calls.get() + 1);
            ToolQuota::with_status("unavailable", Some("codex_login_expired".into()))
        };
        let dto = usage_quota_impl(&us, &fake, &fake_codex);
        assert_eq!(dto.claude.status, "disabled");
        assert_eq!(dto.codex.status, "no_data");
        assert_eq!(calls.get(), 0, "no network call while the switch is off");
        assert_eq!(codex_calls.get(), 0, "no Codex network call while its switch is off");

        set_claude_quota_enabled_impl(&us, true).unwrap();
        assert_eq!(usage_quota_impl(&us, &fake, &fake_codex).claude.status, "ok");
        usage_quota_impl(&us, &fake, &fake_codex);
        assert_eq!(calls.get(), 1, "cached for a minute");

        // Codex live on but failing and no log snapshot: the error is shown.
        set_codex_quota_enabled_impl(&us, true).unwrap();
        let dto = usage_quota_impl(&us, &fake, &fake_codex);
        assert_eq!(dto.codex.status, "unavailable");
        assert_eq!(dto.codex.reason.as_deref(), Some("codex_login_expired"));
        usage_quota_impl(&us, &fake, &fake_codex);
        assert_eq!(codex_calls.get(), 1, "Codex result is cached");

        let out = tmp.path().join("u.csv");
        usage_export_csv_impl(&us, &q, &out).unwrap();
        let text = fs::read_to_string(&out).unwrap();
        assert!(text.contains("2026-11,Claude Code,claude-opus-5-5,10,5,0,0,15,"));
    }
}
