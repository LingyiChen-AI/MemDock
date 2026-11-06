//! Conversion records in MemDock's data directory (`session-convert.json`), used
//! to warn before converting the same session twice and to label sessions.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil::{load_json_or_backup, save_json_atomic, Loaded};
use crate::model::Tool;

const VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRef {
    pub tool: Tool,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub source: SessionRef,
    pub target: SessionRef,
    pub target_path: String,
    pub cwd: String,
    /// Unix milliseconds.
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConvertStore {
    pub version: u32,
    pub records: Vec<Record>,
}

impl Default for ConvertStore {
    fn default() -> Self {
        Self { version: VERSION, records: Vec::new() }
    }
}

pub fn store_path(data_dir: &Path) -> PathBuf {
    data_dir.join("session-convert.json")
}

pub fn load(path: &Path) -> Result<ConvertStore, String> {
    match load_json_or_backup::<ConvertStore>(path, VERSION, "conversion records")? {
        Loaded::Ok(s) => Ok(s),
        Loaded::Missing | Loaded::Recovered => Ok(ConvertStore::default()),
    }
}

pub fn save(path: &Path, store: &ConvertStore) -> std::io::Result<()> {
    save_json_atomic(path, store)
}

impl ConvertStore {
    /// Earlier conversions of `source` into `target` whose file still exists.
    pub fn existing(&self, source: &SessionRef, target: Tool) -> Vec<&Record> {
        self.records
            .iter()
            .filter(|r| &r.source == source && r.target.tool == target && Path::new(&r.target_path).exists())
            .collect()
    }

    pub fn source_of(&self, target: &SessionRef) -> Option<&SessionRef> {
        self.records.iter().find(|r| &r.target == target).map(|r| &r.source)
    }

    pub fn targets_of(&self, source: &SessionRef) -> Vec<SessionRef> {
        self.records
            .iter()
            .filter(|r| &r.source == source && Path::new(&r.target_path).exists())
            .map(|r| r.target.clone())
            .collect()
    }
}
