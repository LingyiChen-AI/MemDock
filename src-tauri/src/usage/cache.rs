//! Per-file parse cache (`<appdata>/usage-cache.json`): a log file is only
//! re-parsed when its size or mtime changed.
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::{claude, codex, RateSnapshot, UsageEvent};
use crate::model::Tool;
use crate::projects::open_jsonl;

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileEntry {
    pub size: u64,
    pub mtime: i64,
    pub tool: Tool,
    pub events: Vec<UsageEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<RateSnapshot>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct UsageCache {
    pub version: u32,
    pub files: BTreeMap<String, FileEntry>,
}

fn stat(p: &Path) -> Option<(u64, i64)> {
    let m = fs::metadata(p).ok()?;
    let mtime = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Some((m.len(), mtime))
}

impl UsageCache {
    /// Missing, unreadable or old-format caches start empty (it is only a cache).
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<UsageCache>(&b).ok())
            .filter(|c| c.version == VERSION)
            .unwrap_or_else(|| UsageCache { version: VERSION, files: BTreeMap::new() })
    }

    /// Compact JSON, written to a temp file then renamed.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec(self).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, path)
    }

    /// Re-parses new / changed files and drops vanished ones. Returns whether anything changed.
    pub fn refresh(&mut self, claude_home: &Path, codex_home: &Path) -> bool {
        let mut wanted: Vec<(PathBuf, Tool)> = claude::list_files(claude_home)
            .into_iter()
            .map(|p| (p, Tool::Claude))
            .collect();
        wanted.extend(codex::list_files(codex_home).into_iter().map(|p| (p, Tool::Codex)));
        let mut changed = false;
        let mut keep = std::collections::HashSet::new();
        for (path, tool) in wanted {
            let key = path.to_string_lossy().into_owned();
            let Some((size, mtime)) = stat(&path) else { continue };
            keep.insert(key.clone());
            if self
                .files
                .get(&key)
                .is_some_and(|e| e.size == size && e.mtime == mtime)
            {
                continue;
            }
            let Some(entry) = parse_file(&path, tool, size, mtime) else { continue };
            self.files.insert(key, entry);
            changed = true;
        }
        let before = self.files.len();
        self.files.retain(|k, _| keep.contains(k));
        changed || self.files.len() != before
    }

    pub fn events(&self) -> impl Iterator<Item = &UsageEvent> {
        self.files.values().flat_map(|f| f.events.iter())
    }

    /// Newest Codex rate-limit snapshot over all files.
    pub fn latest_rate(&self) -> Option<&RateSnapshot> {
        self.files
            .values()
            .filter_map(|f| f.rate.as_ref())
            .max_by_key(|r| r.ts)
    }
}

fn parse_file(path: &Path, tool: Tool, size: u64, mtime: i64) -> Option<FileEntry> {
    match tool {
        Tool::Claude => {
            let f = fs::File::open(path).ok()?;
            Some(FileEntry { size, mtime, tool, events: claude::parse_reader(BufReader::new(f)), rate: None })
        }
        Tool::Codex => {
            let r = open_jsonl(path).ok()?;
            let p = codex::parse_reader(r);
            Some(FileEntry { size, mtime, tool, events: p.events, rate: p.rate })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE_LINE: &str = r#"{"type":"assistant","requestId":"r1","timestamp":"2026-11-05T10:00:01.000Z","message":{"model":"claude-opus-5-5","id":"m1","usage":{"input_tokens":2,"output_tokens":10}}}"#;
    const CODEX_LINES: &str = "{\"timestamp\":\"2026-11-05T10:00:00Z\",\"type\":\"turn_context\",\"payload\":{\"model\":\"gpt-5\"}}\n{\"timestamp\":\"2026-11-05T10:00:01Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":5,\"output_tokens\":1}},\"rate_limits\":{\"primary\":{\"used_percent\":3.0,\"window_minutes\":300,\"resets_at\":1793599691}}}}\n";

    #[test]
    fn refresh_only_reparses_changed_files_and_roundtrips() {
        let tmp = tempfile::tempdir().unwrap();
        let ch = tmp.path().join(".claude");
        let xh = tmp.path().join(".codex");
        let proj = ch.join("projects").join("p1");
        fs::create_dir_all(&proj).unwrap();
        let day = xh.join("sessions").join("2026").join("11").join("05");
        fs::create_dir_all(&day).unwrap();
        fs::write(proj.join("s1.jsonl"), format!("{CLAUDE_LINE}\n")).unwrap();
        let zst = zstd::encode_all(CODEX_LINES.as_bytes(), 3).unwrap();
        fs::write(day.join("rollout-2026-11-05T10-00-00-abc.jsonl.zst"), zst).unwrap();
        fs::write(day.join("other.jsonl"), CODEX_LINES).unwrap(); // not a rollout: ignored

        let mut c = UsageCache::load(&tmp.path().join("missing.json"));
        assert!(c.refresh(&ch, &xh));
        assert_eq!(c.files.len(), 2);
        assert_eq!(c.events().count(), 2);
        assert_eq!(c.latest_rate().unwrap().primary.as_ref().unwrap().used_percent, 3.0);
        assert!(!c.refresh(&ch, &xh), "unchanged files are not re-parsed");

        let cache_path = tmp.path().join("data").join("usage-cache.json");
        c.save(&cache_path).unwrap();
        let mut c2 = UsageCache::load(&cache_path);
        assert_eq!(c2.events().count(), 2);
        fs::remove_file(proj.join("s1.jsonl")).unwrap();
        assert!(c2.refresh(&ch, &xh));
        assert_eq!(c2.events().count(), 1);

        fs::write(&cache_path, "{ broken").unwrap();
        assert!(UsageCache::load(&cache_path).files.is_empty());
    }
}
