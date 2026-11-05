//! Persistent sync state (`<appdata>/sync/state.json`). Never holds the token.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::fsutil::{load_json_or_backup, save_json_atomic, Loaded};

fn default_repo_name() -> String {
    "MemDockHub".to_string()
}

/// A project that took part in the last successful sync, with the local
/// directory it had then (`projects::normalize_path`). The base commit is only
/// trusted for a project whose key and directory both still match.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct SyncedProject {
    pub key: String,
    pub dir: String,
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
    /// `owner/name` of the public repository the user agreed to sync into.
    /// An acknowledgement never carries over to another repository or account.
    #[serde(default)]
    pub public_ack_for: Option<String>,
    #[serde(default)]
    pub included_local: Vec<String>,
    #[serde(default)]
    pub synced_projects: Vec<SyncedProject>,
    /// Repo paths left out of the last sync (skipped): their base is not
    /// trusted, so they can only be pulled or conflict, never deleted.
    #[serde(default)]
    pub unsynced_paths: BTreeSet<String>,
    /// Project keys whose memory files were all deleted locally and are held
    /// until the user decides (no deletion of their synced files is pushed).
    #[serde(default)]
    pub held_projects: BTreeSet<String>,
    /// What blocks sync until the user acts: `public_ack` / `not_ours` / `relogin`.
    #[serde(default)]
    pub needs: Option<String>,
    /// `owner/name` of the repository cloned into `<appdata>/sync/repo`.
    #[serde(default)]
    pub clone_of: Option<String>,
    /// Visibility of the bound repository as GitHub last reported it.
    #[serde(default)]
    pub repo_private: Option<bool>,
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
            public_ack_for: None,
            included_local: Vec::new(),
            synced_projects: Vec::new(),
            unsynced_paths: BTreeSet::new(),
            held_projects: BTreeSet::new(),
            needs: None,
            clone_of: None,
            repo_private: None,
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
            public_ack_for: Some("octo/MemDockHub".into()),
            included_local: vec!["C:/x".into()],
            synced_projects: vec![SyncedProject {
                key: "k1".into(),
                dir: "c:/w/p".into(),
            }],
            unsynced_paths: BTreeSet::from(["projects/k1/claude/CLAUDE.md".to_string()]),
            held_projects: BTreeSet::from(["k1".to_string()]),
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
