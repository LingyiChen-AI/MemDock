//! Persistent sync state (`<appdata>/sync/state.json`). Never holds the token.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::fsutil::{load_json_or_backup, save_json_atomic, Loaded};

fn default_repo_name() -> String {
    "MemDockHub".to_string()
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct SyncState {
    #[serde(default)]
    pub login: Option<String>,
    #[serde(default)]
    pub repo_owner: Option<String>,
    #[serde(default = "default_repo_name")]
    pub repo_name: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub last_sync_commit: Option<String>,
    #[serde(default)]
    pub last_sync_at: Option<i64>,
    #[serde(default)]
    pub public_ack: bool,
    #[serde(default)]
    pub included_local: Vec<String>,
    #[serde(default)]
    pub synced_projects: Vec<String>,
}

impl Default for SyncState {
    fn default() -> Self {
        SyncState {
            login: None,
            repo_owner: None,
            repo_name: default_repo_name(),
            branch: None,
            last_sync_commit: None,
            last_sync_at: None,
            public_ack: false,
            included_local: Vec::new(),
            synced_projects: Vec::new(),
        }
    }
}

/// Missing file -> default; corrupt file -> backed up, default.
pub fn load_state(path: &Path) -> Result<SyncState, String> {
    match load_json_or_backup::<SyncState>(path, 1, "sync state")? {
        Loaded::Ok(s) => Ok(s),
        Loaded::Missing | Loaded::Recovered => Ok(SyncState::default()),
    }
}

pub fn save_state(path: &Path, state: &SyncState) -> io::Result<()> {
    save_json_atomic(path, state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_roundtrip_and_corrupt_backup() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sync").join("state.json");
        assert_eq!(load_state(&p).unwrap(), SyncState::default());
        assert_eq!(SyncState::default().repo_name, "MemDockHub");

        let s = SyncState {
            login: Some("octo".into()),
            repo_owner: Some("octo".into()),
            branch: Some("main".into()),
            last_sync_commit: Some("abc".into()),
            last_sync_at: Some(123),
            public_ack: true,
            included_local: vec!["C:/x".into()],
            synced_projects: vec!["k1".into()],
            ..SyncState::default()
        };
        save_state(&p, &s).unwrap();
        assert_eq!(load_state(&p).unwrap(), s);

        // Partial file: defaults fill in.
        std::fs::write(&p, r#"{"login":"a"}"#).unwrap();
        let partial = load_state(&p).unwrap();
        assert_eq!(partial.login.as_deref(), Some("a"));
        assert_eq!(partial.repo_name, "MemDockHub");

        // Corrupt: backed up, default returned.
        std::fs::write(&p, "{not json").unwrap();
        assert_eq!(load_state(&p).unwrap(), SyncState::default());
        let bak = p.with_file_name("state.json.bak");
        assert_eq!(std::fs::read_to_string(bak).unwrap(), "{not json");
    }
}
