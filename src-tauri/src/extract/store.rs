//! Persistence for the extraction config, suggestions, run records and
//! per-project watermarks.
//!
//! Files under `<appdata>`: `extract-config.json` (config),
//! `suggestions.json` (suggestions) and `extract-runs.json` (run records
//! **and** the watermarks). Each carries `"version": 1`. Corrupt files are
//! backed up to `.bak` and `load` fails with an error (never an empty store);
//! a newer version is backed up and refused. The caller wraps `Store` in a
//! `Mutex`; there is no internal locking.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::prompt::{Suggestion, SuggestionStatus};
use crate::fsutil::{load_json_or_backup, save_json_atomic, Loaded};
use crate::model::Tool;

/// Highest file format this build understands (all three files).
pub const STORE_VERSION: u32 = 1;
/// Run records kept (oldest dropped first).
pub const MAX_RUNS: usize = 100;
/// Non-pending suggestions kept (oldest dropped first); pending ones never.
pub const MAX_NON_PENDING: usize = 2000;

const CONFIG_FILE: &str = "extract-config.json";
const SUGGESTIONS_FILE: &str = "suggestions.json";
const RUNS_FILE: &str = "extract-runs.json";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    #[default]
    Off,
    Daily { hh: u8, mm: u8 },
    /// `weekday` 0-6.
    Weekly { weekday: u8, hh: u8, mm: u8 },
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(tag = "kind", content = "ids", rename_all = "snake_case")]
pub enum ProjectFilter {
    #[default]
    All,
    Selected(Vec<String>),
}

fn one() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

fn default_lookback() -> u32 {
    7
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct ExtractConfig {
    /// `None` = automatic (prefer claude).
    #[serde(default)]
    pub tool: Option<Tool>,
    #[serde(default)]
    pub schedule: Schedule,
    #[serde(default)]
    pub projects: ProjectFilter,
    #[serde(default = "default_true")]
    pub include_global: bool,
    /// 1..=90.
    #[serde(default = "default_lookback")]
    pub lookback_days: u32,
    #[serde(default)]
    pub lang: Option<String>,
    /// The user's extraction rules replacing the built-in ones (`None` = built-in).
    #[serde(default)]
    pub custom_prompt: Option<String>,
}

impl Default for ExtractConfig {
    fn default() -> Self {
        ExtractConfig {
            tool: None,
            schedule: Schedule::Off,
            projects: ProjectFilter::All,
            include_global: true,
            lookback_days: 7,
            lang: None,
            custom_prompt: None,
        }
    }
}

impl ExtractConfig {
    fn normalized(mut self) -> Self {
        self.lookback_days = self.lookback_days.clamp(1, 90);
        self.custom_prompt = self
            .custom_prompt
            .map(|p| p.chars().take(super::prompt::MAX_RULES_CHARS).collect::<String>())
            .filter(|p| !p.trim().is_empty());
        self.schedule = match self.schedule {
            Schedule::Off => Schedule::Off,
            Schedule::Daily { hh, mm } => Schedule::Daily { hh: hh.min(23), mm: mm.min(59) },
            Schedule::Weekly { weekday, hh, mm } => Schedule::Weekly {
                weekday: weekday.min(6),
                hh: hh.min(23),
                mm: mm.min(59),
            },
        };
        self
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum RunTrigger {
    Manual,
    Scheduled,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

/// Machine-readable cause of a failed run; the UI maps it to localized text
/// and keeps `RunRecord::error` (raw English detail) as a tooltip.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum RunErrorCode {
    NotInstalled,
    NotLoggedIn,
    Timeout,
    Cancelled,
    InvalidOutput,
    Io,
    Other,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct RunRecord {
    pub id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub trigger: RunTrigger,
    pub tool: Tool,
    pub status: RunStatus,
    pub sessions_read: u32,
    pub projects: u32,
    pub suggestions_new: u32,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
    /// Set together with `error`; absent in records written by older builds.
    #[serde(default)]
    pub error_code: Option<RunErrorCode>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
struct Watermark {
    project_id: String,
    tool: Tool,
    ts: i64,
}

#[derive(Serialize, Deserialize)]
struct ConfigFile {
    #[serde(default = "one")]
    version: u32,
    #[serde(default)]
    config: ExtractConfig,
}

#[derive(Serialize, Deserialize)]
struct SuggestionsFile {
    #[serde(default = "one")]
    version: u32,
    #[serde(default)]
    suggestions: Vec<Suggestion>,
}

/// Run records and watermarks share one file.
#[derive(Serialize, Deserialize)]
struct RunsFile {
    #[serde(default = "one")]
    version: u32,
    #[serde(default)]
    runs: Vec<RunRecord>,
    #[serde(default)]
    watermarks: Vec<Watermark>,
}

pub struct Store {
    dir: PathBuf,
    config: ExtractConfig,
    suggestions: Vec<Suggestion>,
    runs: Vec<RunRecord>,
    watermarks: HashMap<(String, Tool), i64>,
}

fn load_file<T: serde::de::DeserializeOwned>(dir: &Path, name: &str) -> Result<Option<T>, String> {
    let path = dir.join(name);
    match load_json_or_backup::<T>(&path, STORE_VERSION, name)? {
        Loaded::Ok(v) => Ok(Some(v)),
        Loaded::Missing => Ok(None),
        Loaded::Recovered => Err(format!(
            "{} is corrupt; it was backed up to {}.bak and left in place",
            path.display(),
            path.display()
        )),
    }
}

/// True when a backup next to `path` (`<name>.bak` or `<name>.<ms>.bak`, as
/// written by `load_json_or_backup`) has exactly the bytes of `path`.
fn has_identical_backup(path: &Path) -> bool {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
        return false;
    };
    let Ok(bytes) = std::fs::read(path) else { return false };
    let prefix = format!("{name}.");
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    entries.flatten().any(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        n.starts_with(&prefix)
            && n.ends_with(".bak")
            && std::fs::read(e.path()).is_ok_and(|b| b == bytes)
    })
}

/// Removes `path` only after verifying an identical backup exists.
fn remove_backed_up(path: &Path) -> Result<(), String> {
    if !has_identical_backup(path) {
        return Err(format!("no verified backup of {}; nothing was removed", path.display()));
    }
    std::fs::remove_file(path).map_err(|e| format!("cannot remove {}: {e}", path.display()))
}

/// Recovery for a store that failed to load: removes every store file that
/// cannot be loaded (corrupt or too new), after verifying that each has an
/// identical `.bak` copy next to it (a missing backup is made first, as on a
/// normal load). Loadable files are kept. When any backup cannot be verified
/// nothing is removed. Returns the removed paths.
pub fn reset_unloadable(appdata: &Path) -> Result<Vec<PathBuf>, String> {
    let bad = [
        (CONFIG_FILE, load_file::<ConfigFile>(appdata, CONFIG_FILE).is_err()),
        (SUGGESTIONS_FILE, load_file::<SuggestionsFile>(appdata, SUGGESTIONS_FILE).is_err()),
        (RUNS_FILE, load_file::<RunsFile>(appdata, RUNS_FILE).is_err()),
    ];
    let paths: Vec<PathBuf> = bad
        .iter()
        .filter(|(_, b)| *b)
        .map(|(n, _)| appdata.join(n))
        .filter(|p| p.exists())
        .collect();
    if let Some(p) = paths.iter().find(|p| !has_identical_backup(p)) {
        return Err(format!("no verified backup of {}; nothing was removed", p.display()));
    }
    for p in &paths {
        remove_backed_up(p)?;
    }
    Ok(paths)
}

impl Store {
    /// Loads all three files from `appdata`. Missing files give defaults;
    /// corrupt or too-new files are backed up and reported as an error.
    /// Runs left in `Running` by a previous process become `Interrupted`.
    pub fn load(appdata: &Path) -> Result<Store, String> {
        let config = load_file::<ConfigFile>(appdata, CONFIG_FILE)?
            .map(|f| f.config.normalized())
            .unwrap_or_default();
        let suggestions = load_file::<SuggestionsFile>(appdata, SUGGESTIONS_FILE)?
            .map(|f| f.suggestions)
            .unwrap_or_default();
        let (mut runs, wm) = load_file::<RunsFile>(appdata, RUNS_FILE)?
            .map(|f| (f.runs, f.watermarks))
            .unwrap_or_default();
        for r in runs.iter_mut() {
            if r.status == RunStatus::Running {
                r.status = RunStatus::Interrupted;
            }
        }
        let watermarks = wm.into_iter().map(|w| ((w.project_id, w.tool), w.ts)).collect();
        Ok(Store { dir: appdata.to_path_buf(), config, suggestions, runs, watermarks })
    }

    pub fn save(&self) -> io::Result<()> {
        save_json_atomic(
            &self.dir.join(CONFIG_FILE),
            &ConfigFile { version: STORE_VERSION, config: self.config.clone() },
        )?;
        save_json_atomic(
            &self.dir.join(SUGGESTIONS_FILE),
            &SuggestionsFile { version: STORE_VERSION, suggestions: self.suggestions.clone() },
        )?;
        let mut wm: Vec<Watermark> = self
            .watermarks
            .iter()
            .map(|((p, t), ts)| Watermark { project_id: p.clone(), tool: *t, ts: *ts })
            .collect();
        wm.sort_by(|a, b| {
            (a.project_id.as_str(), a.tool as u8).cmp(&(b.project_id.as_str(), b.tool as u8))
        });
        save_json_atomic(
            &self.dir.join(RUNS_FILE),
            &RunsFile { version: STORE_VERSION, runs: self.runs.clone(), watermarks: wm },
        )
    }

    // ------------------------------------------------------------ config

    pub fn config(&self) -> &ExtractConfig {
        &self.config
    }

    /// Stores the config, clamping `lookback_days` to 1..=90 and times to valid ranges.
    pub fn set_config(&mut self, c: ExtractConfig) {
        self.config = c.normalized();
    }

    // ------------------------------------------------------- suggestions

    pub fn suggestions(&self) -> &[Suggestion] {
        &self.suggestions
    }

    /// Adds suggestions whose fingerprint is not already present in any
    /// non-Stale suggestion (or earlier in the same batch). Returns how many
    /// were added, then applies the retention cap.
    pub fn add_suggestions(&mut self, items: Vec<Suggestion>) -> u32 {
        let mut known: HashSet<String> = self
            .suggestions
            .iter()
            .filter(|s| s.status != SuggestionStatus::Stale)
            .map(|s| s.fingerprint.clone())
            .collect();
        let mut added = 0u32;
        for s in items {
            if known.insert(s.fingerprint.clone()) {
                self.suggestions.push(s);
                added += 1;
            }
        }
        self.enforce_retention();
        added
    }

    fn enforce_retention(&mut self) {
        let non_pending = self
            .suggestions
            .iter()
            .filter(|s| s.status != SuggestionStatus::Pending)
            .count();
        if non_pending <= MAX_NON_PENDING {
            return;
        }
        let mut idx: Vec<(bool, i64, usize)> = self
            .suggestions
            .iter()
            .enumerate()
            .filter(|(_, s)| s.status != SuggestionStatus::Pending)
            .map(|(i, s)| (s.status == SuggestionStatus::Rejected, s.created_at, i))
            .collect();
        // Accepted/Stale first (oldest first), Rejected last: rejected ones
        // are what stops the same suggestion from coming back.
        idx.sort();
        let drop: HashSet<usize> = idx
            .into_iter()
            .take(non_pending - MAX_NON_PENDING)
            .map(|(_, _, i)| i)
            .collect();
        let mut i = 0usize;
        self.suggestions.retain(|_| {
            let keep = !drop.contains(&i);
            i += 1;
            keep
        });
    }

    /// Returns false when no suggestion has this id.
    pub fn set_status(&mut self, id: &str, status: SuggestionStatus) -> bool {
        match self.suggestions.iter_mut().find(|s| s.id == id) {
            Some(s) => {
                s.status = status;
                true
            }
            None => false,
        }
    }

    /// After MemDock itself rewrote `path` from `old_hash` to `new_hash`:
    /// Pending suggestions on that file that were based on exactly `old_hash`
    /// now count as based on `new_hash` (MemDock's own write is not an
    /// external change). Suggestions based on any other hash are left alone,
    /// so external edits still require confirmation. Returns how many moved.
    pub fn rebase_pending(&mut self, path: &str, old_hash: &str, new_hash: &str) -> u32 {
        let mut n = 0;
        for s in self.suggestions.iter_mut() {
            if s.status == SuggestionStatus::Pending && s.path == path && s.base_hash == old_hash {
                s.base_hash = new_hash.to_string();
                n += 1;
            }
        }
        n
    }

    pub fn pending_count(&self) -> u32 {
        self.suggestions
            .iter()
            .filter(|s| s.status == SuggestionStatus::Pending)
            .count() as u32
    }

    /// Pending suggestions per project; `project_of` maps a suggestion to its
    /// project id (None for global files, which are not counted).
    pub fn pending_by_project(
        &self,
        project_of: impl Fn(&Suggestion) -> Option<String>,
    ) -> HashMap<String, u32> {
        let mut m: HashMap<String, u32> = HashMap::new();
        for s in self.suggestions.iter().filter(|s| s.status == SuggestionStatus::Pending) {
            if let Some(p) = project_of(s) {
                *m.entry(p).or_default() += 1;
            }
        }
        m
    }

    // -------------------------------------------------------------- runs

    pub fn runs(&self) -> &[RunRecord] {
        &self.runs
    }

    /// Appends a run (oldest first); keeps at most `MAX_RUNS`.
    pub fn push_run(&mut self, r: RunRecord) {
        self.runs.push(r);
        if self.runs.len() > MAX_RUNS {
            let extra = self.runs.len() - MAX_RUNS;
            self.runs.drain(..extra);
        }
    }

    /// Replaces the run with the same id; false when it is not found.
    pub fn update_run(&mut self, r: RunRecord) -> bool {
        match self.runs.iter_mut().find(|x| x.id == r.id) {
            Some(x) => {
                *x = r;
                true
            }
            None => false,
        }
    }

    // -------------------------------------------------------- watermarks

    /// Newest session timestamp (ms) already processed; 0 when none.
    pub fn watermark(&self, project_id: &str, tool: Tool) -> i64 {
        self.watermarks.get(&(project_id.to_string(), tool)).copied().unwrap_or(0)
    }

    pub fn set_watermark(&mut self, project_id: &str, tool: Tool, ts: i64) {
        self.watermarks.insert((project_id.to_string(), tool), ts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::prompt::{SuggestionKind, SuggestionStatus};
    use std::fs;

    fn sug(id: &str, fp: &str) -> Suggestion {
        Suggestion {
            id: id.into(),
            fingerprint: fp.into(),
            kind: SuggestionKind::Add,
            file_id: "f".into(),
            path: "/p/CLAUDE.md".into(),
            tool_of_file: Tool::Claude,
            old_text: None,
            new_text: Some("x".into()),
            reason: "r".into(),
            evidence: vec![],
            source_tool: Tool::Claude,
            created_at: 1,
            base_hash: "h".into(),
            status: SuggestionStatus::Pending,
            origin: None,
        }
    }

    fn run(id: &str) -> RunRecord {
        RunRecord {
            id: id.into(),
            started_at: 1,
            finished_at: None,
            trigger: RunTrigger::Manual,
            tool: Tool::Claude,
            status: RunStatus::Running,
            sessions_read: 0,
            projects: 0,
            suggestions_new: 0,
            warnings: vec![],
            error: None,
            error_code: None,
        }
    }

    #[test]
    fn config_defaults_off() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::load(tmp.path()).unwrap();
        let c = s.config();
        assert_eq!(c.schedule, Schedule::Off);
        assert_eq!(c.tool, None);
        assert_eq!(c.projects, ProjectFilter::All);
        assert!(c.include_global);
        assert_eq!(c.lookback_days, 7);
        assert_eq!(c.lang, None);
    }

    #[test]
    fn config_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = Store::load(tmp.path()).unwrap();
        let c = ExtractConfig {
            tool: Some(Tool::Codex),
            schedule: Schedule::Weekly { weekday: 3, hh: 9, mm: 30 },
            projects: ProjectFilter::Selected(vec!["a".into(), "b".into()]),
            include_global: false,
            lookback_days: 30,
            lang: Some("zh".into()),
            custom_prompt: Some("rules".into()),
        };
        s.set_config(c.clone());
        s.save().unwrap();
        let s2 = Store::load(tmp.path()).unwrap();
        assert_eq!(s2.config(), &c);
        let raw = fs::read_to_string(tmp.path().join("extract-config.json")).unwrap();
        assert!(raw.contains("\"version\": 1"));
    }

    #[test]
    fn config_values_are_clamped() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = Store::load(tmp.path()).unwrap();
        let mut c = ExtractConfig::default();
        c.lookback_days = 500;
        c.schedule = Schedule::Daily { hh: 30, mm: 99 };
        s.set_config(c);
        assert_eq!(s.config().lookback_days, 90);
        assert_eq!(s.config().schedule, Schedule::Daily { hh: 23, mm: 59 });
        let mut c = ExtractConfig::default();
        c.lookback_days = 0;
        s.set_config(c);
        assert_eq!(s.config().lookback_days, 1);
    }

    #[test]
    fn suggestions_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = Store::load(tmp.path()).unwrap();
        assert_eq!(s.add_suggestions(vec![sug("1", "a"), sug("2", "b")]), 2);
        s.save().unwrap();
        let s2 = Store::load(tmp.path()).unwrap();
        assert_eq!(s2.suggestions(), s.suggestions());
        assert_eq!(s2.pending_count(), 2);
    }

    #[test]
    fn rejected_fingerprint_suppressed() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = Store::load(tmp.path()).unwrap();
        s.add_suggestions(vec![sug("1", "a"), sug("2", "b")]);
        assert!(s.set_status("1", SuggestionStatus::Rejected));
        assert!(s.set_status("2", SuggestionStatus::Accepted));
        assert_eq!(s.add_suggestions(vec![sug("3", "a"), sug("4", "b")]), 0);
        assert_eq!(s.suggestions().len(), 2);
        // A stale suggestion's fingerprint may be proposed again.
        assert!(s.set_status("1", SuggestionStatus::Stale));
        assert_eq!(s.add_suggestions(vec![sug("5", "a")]), 1);
        assert!(!s.set_status("nope", SuggestionStatus::Stale));
    }

    #[test]
    fn pending_duplicate_not_added() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = Store::load(tmp.path()).unwrap();
        assert_eq!(s.add_suggestions(vec![sug("1", "a")]), 1);
        assert_eq!(s.add_suggestions(vec![sug("2", "a")]), 0);
        // Duplicates inside one batch too.
        assert_eq!(s.add_suggestions(vec![sug("3", "c"), sug("4", "c")]), 1);
        assert_eq!(s.pending_count(), 2);
    }

    #[test]
    fn rebase_pending_moves_only_matching_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = Store::load(tmp.path()).unwrap();
        let a = sug("1", "a"); // /p/CLAUDE.md, base h
        let mut b = sug("2", "b");
        b.base_hash = "older".into();
        let mut c = sug("3", "c");
        c.path = "/q/CLAUDE.md".into();
        let mut d = sug("4", "d");
        d.status = SuggestionStatus::Rejected;
        s.add_suggestions(vec![a, b, c, d]);
        assert_eq!(s.rebase_pending("/p/CLAUDE.md", "h", "h2"), 1);
        let base = |id: &str| s.suggestions().iter().find(|x| x.id == id).unwrap().base_hash.clone();
        assert_eq!(base("1"), "h2");
        assert_eq!(base("2"), "older");
        assert_eq!(base("3"), "h");
        assert_eq!(base("4"), "h");
    }

    #[test]
    fn pending_by_project_counts_only_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = Store::load(tmp.path()).unwrap();
        let mut a = sug("1", "a");
        a.path = "/x/CLAUDE.md".into();
        let mut b = sug("2", "b");
        b.path = "/x/AGENTS.md".into();
        let mut c = sug("3", "c");
        c.path = "/y/CLAUDE.md".into();
        let d = sug("4", "d"); // global: no project
        s.add_suggestions(vec![a, b, c, d]);
        s.set_status("2", SuggestionStatus::Accepted);
        let m = s.pending_by_project(|x| {
            if x.path.starts_with("/x/") {
                Some("px".to_string())
            } else if x.path.starts_with("/y/") {
                Some("py".to_string())
            } else {
                None
            }
        });
        assert_eq!(m.len(), 2);
        assert_eq!(m["px"], 1);
        assert_eq!(m["py"], 1);
    }

    #[test]
    fn non_pending_retention_keeps_all_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = Store::load(tmp.path()).unwrap();
        let mut batch = Vec::new();
        for i in 0..(MAX_NON_PENDING + 50) {
            let mut x = sug(&format!("r{i}"), &format!("fr{i}"));
            x.status = SuggestionStatus::Rejected;
            x.created_at = i as i64;
            batch.push(x);
        }
        for i in 0..5 {
            let mut x = sug(&format!("p{i}"), &format!("fp{i}"));
            x.created_at = -1; // oldest of all, but pending
            batch.push(x);
        }
        // Rejected ones are added as given (status preserved by add).
        s.add_suggestions(batch);
        let non_pending = s.suggestions().iter().filter(|x| x.status != SuggestionStatus::Pending).count();
        assert_eq!(non_pending, MAX_NON_PENDING);
        assert_eq!(s.pending_count(), 5);
        // Oldest non-pending were dropped.
        assert!(!s.suggestions().iter().any(|x| x.id == "r0"));
        assert!(s.suggestions().iter().any(|x| x.id == format!("r{}", MAX_NON_PENDING + 49)));
    }

    #[test]
    fn runs_capped_at_100() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = Store::load(tmp.path()).unwrap();
        for i in 0..105 {
            s.push_run(run(&i.to_string()));
        }
        assert_eq!(s.runs().len(), 100);
        assert_eq!(s.runs()[0].id, "5");
        assert_eq!(s.runs()[99].id, "104");
        s.save().unwrap();
        assert_eq!(Store::load(tmp.path()).unwrap().runs().len(), 100);
    }

    #[test]
    fn update_run_replaces_by_id() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = Store::load(tmp.path()).unwrap();
        s.push_run(run("a"));
        let mut r = run("a");
        r.status = RunStatus::Succeeded;
        r.suggestions_new = 3;
        assert!(s.update_run(r));
        assert_eq!(s.runs().len(), 1);
        assert_eq!(s.runs()[0].status, RunStatus::Succeeded);
        assert!(!s.update_run(run("zzz")));
    }

    #[test]
    fn leftover_running_runs_become_interrupted_on_load() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = Store::load(tmp.path()).unwrap();
        s.push_run(run("a"));
        s.save().unwrap();
        let s2 = Store::load(tmp.path()).unwrap();
        assert_eq!(s2.runs()[0].status, RunStatus::Interrupted);
    }

    #[test]
    fn watermark_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = Store::load(tmp.path()).unwrap();
        assert_eq!(s.watermark("p", Tool::Claude), 0);
        s.set_watermark("p", Tool::Claude, 1234);
        s.set_watermark("p", Tool::Codex, 99);
        s.save().unwrap();
        let s2 = Store::load(tmp.path()).unwrap();
        assert_eq!(s2.watermark("p", Tool::Claude), 1234);
        assert_eq!(s2.watermark("p", Tool::Codex), 99);
        assert_eq!(s2.watermark("q", Tool::Claude), 0);
    }

    #[test]
    fn corrupt_store_backed_up_and_errors() {
        for name in ["extract-config.json", "suggestions.json", "extract-runs.json"] {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join(name);
            fs::write(&path, "{ oops").unwrap();
            let err = Store::load(tmp.path());
            assert!(err.is_err(), "{name}");
            assert_eq!(fs::read_to_string(tmp.path().join(format!("{name}.bak"))).unwrap(), "{ oops");
            assert_eq!(fs::read_to_string(&path).unwrap(), "{ oops", "original left in place");
        }
    }

    #[test]
    fn reset_unloadable_removes_only_backed_up_corrupt_files() {
        let tmp = tempfile::tempdir().unwrap();
        let mut good = Store::load(tmp.path()).unwrap();
        good.push_run(run("keep"));
        good.save().unwrap();
        fs::write(tmp.path().join("suggestions.json"), "{ oops").unwrap();
        assert!(Store::load(tmp.path()).is_err()); // makes the .bak
        let removed = reset_unloadable(tmp.path()).unwrap();
        assert_eq!(removed, vec![tmp.path().join("suggestions.json")]);
        assert_eq!(fs::read_to_string(tmp.path().join("suggestions.json.bak")).unwrap(), "{ oops");
        let s = Store::load(tmp.path()).unwrap();
        assert_eq!(s.runs().len(), 1, "loadable files are kept");
        // nothing to do on a healthy store
        assert!(reset_unloadable(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn remove_refuses_without_identical_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("suggestions.json");
        fs::write(&p, "{ oops").unwrap();
        assert!(remove_backed_up(&p).is_err());
        assert!(p.exists());
        // a backup with different bytes does not count
        fs::write(tmp.path().join("suggestions.json.bak"), "other").unwrap();
        assert!(remove_backed_up(&p).is_err());
        assert!(p.exists());
        fs::write(tmp.path().join("suggestions.json.123.bak"), "{ oops").unwrap();
        remove_backed_up(&p).unwrap();
        assert!(!p.exists());
    }

    #[test]
    fn newer_version_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("suggestions.json");
        fs::write(&path, r#"{"version": 2, "suggestions": []}"#).unwrap();
        let err = Store::load(tmp.path()).err().unwrap();
        assert!(err.contains("version 2"), "{err}");
        assert!(tmp.path().join("suggestions.json.bak").exists());
    }

    #[test]
    fn missing_version_field_means_one() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("extract-config.json"), r#"{"config": {"lookback_days": 3}}"#).unwrap();
        fs::write(tmp.path().join("suggestions.json"), r#"{"suggestions": []}"#).unwrap();
        fs::write(tmp.path().join("extract-runs.json"), r#"{"runs": []}"#).unwrap();
        let s = Store::load(tmp.path()).unwrap();
        assert_eq!(s.config().lookback_days, 3);
        assert!(!tmp.path().join("extract-config.json.bak").exists());
    }

    #[test]
    fn retention_drops_rejected_last() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = Store::load(tmp.path()).unwrap();
        let mut batch = Vec::new();
        for i in 0..MAX_NON_PENDING {
            let mut x = sug(&format!("r{i}"), &format!("fr{i}"));
            x.status = SuggestionStatus::Rejected;
            x.created_at = i as i64; // oldest overall
            batch.push(x);
        }
        for i in 0..10 {
            let mut x = sug(&format!("a{i}"), &format!("fa{i}"));
            x.status = if i % 2 == 0 { SuggestionStatus::Accepted } else { SuggestionStatus::Stale };
            x.created_at = 1_000_000 + i as i64; // newer than every rejected one
            batch.push(x);
        }
        s.add_suggestions(batch);
        // 10 over the cap: the 10 Accepted/Stale go, not the older Rejected.
        assert_eq!(s.suggestions().len(), MAX_NON_PENDING);
        assert!(s.suggestions().iter().all(|x| x.status == SuggestionStatus::Rejected));
        assert!(s.suggestions().iter().any(|x| x.id == "r0"));
    }
}
