//! Read-only access to Codex's own memory database (`stage1_outputs` table).
//!
//! Codex owns this sqlite file and may be running at the same time, so it is
//! opened strictly read-only (`mode=ro`) and never written to.

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CodexMemoryEntry {
    pub thread_id: String,
    pub raw_memory: String,
    pub rollout_summary: String,
    pub generated_at: i64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CodexDbResult {
    Ok { entries: Vec<CodexMemoryEntry> },
    Unavailable { reason: String },
}

/// Reads all `stage1_outputs` rows, newest `generated_at` first.
/// Missing file, missing table, lock timeout, or any other failure yields
/// `Unavailable` instead of panicking.
pub fn read_codex_memories(db_path: &Path) -> CodexDbResult {
    // Check existence first so a missing db is reported without touching the path.
    if !db_path.exists() {
        return CodexDbResult::Unavailable {
            reason: format!("codex db not found: {}", db_path.display()),
        };
    }
    let uri = build_uri(&db_path.to_string_lossy(), cfg!(windows));
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = match Connection::open_with_flags(&uri, flags) {
        Ok(conn) => conn,
        Err(e) => {
            return CodexDbResult::Unavailable {
                reason: format!("open codex db failed: {e}"),
            };
        }
    };
    if let Err(e) = conn.busy_timeout(Duration::from_secs(1)) {
        return CodexDbResult::Unavailable {
            reason: format!("set busy timeout failed: {e}"),
        };
    }
    match query_entries(&conn) {
        Ok(entries) => CodexDbResult::Ok { entries },
        Err(e) => CodexDbResult::Unavailable {
            reason: format!("read codex memories failed: {e}"),
        },
    }
}

fn query_entries(conn: &Connection) -> rusqlite::Result<Vec<CodexMemoryEntry>> {
    let mut stmt = conn.prepare(
        "SELECT thread_id, raw_memory, rollout_summary, generated_at
         FROM stage1_outputs
         ORDER BY generated_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(CodexMemoryEntry {
            thread_id: row.get(0)?,
            raw_memory: row.get(1)?,
            rollout_summary: row.get(2)?,
            generated_at: row.get(3)?,
        })
    })?;
    rows.collect()
}

/// Builds a read-only sqlite URI (`file:...?mode=ro`) from an absolute path.
/// Windows: `C:\a\b` becomes `file:///C:/a/b`; Unix: `/a/b` becomes `file:/a/b`.
/// `%`, `?`, `#` and spaces in the path are percent-encoded.
fn build_uri(path: &str, windows: bool) -> String {
    let mut uri = String::from("file:");
    let body = if windows {
        uri.push_str("///");
        path.replace('\\', "/")
    } else {
        path.to_string()
    };
    for c in body.chars() {
        match c {
            '%' => uri.push_str("%25"),
            '?' => uri.push_str("%3F"),
            '#' => uri.push_str("%23"),
            ' ' => uri.push_str("%20"),
            _ => uri.push(c),
        }
    }
    uri.push_str("?mode=ro");
    uri
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    const SCHEMA: &str = "
        CREATE TABLE stage1_outputs (
            thread_id TEXT PRIMARY KEY,
            source_updated_at INTEGER NOT NULL,
            raw_memory TEXT NOT NULL,
            rollout_summary TEXT NOT NULL,
            rollout_slug TEXT,
            generated_at INTEGER NOT NULL,
            usage_count INTEGER,
            last_usage INTEGER,
            selected_for_phase2 INTEGER NOT NULL DEFAULT 0,
            selected_for_phase2_source_updated_at INTEGER
        );";

    fn make_db(dir: &Path) -> PathBuf {
        let path = dir.join("state.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        path
    }

    fn insert(path: &Path, thread_id: &str, generated_at: i64) {
        let conn = Connection::open(path).unwrap();
        conn.execute(
            "INSERT INTO stage1_outputs (thread_id, source_updated_at, raw_memory, rollout_summary, generated_at)
             VALUES (?1, 1, ?2, ?3, ?4)",
            rusqlite::params![
                thread_id,
                format!("raw {thread_id}"),
                format!("summary {thread_id}"),
                generated_at
            ],
        )
        .unwrap();
    }

    #[test]
    fn reads_entries_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_db(dir.path());
        insert(&path, "t-old", 100);
        insert(&path, "t-new", 300);
        insert(&path, "t-mid", 200);

        let result = read_codex_memories(&path);

        assert_eq!(
            result,
            CodexDbResult::Ok {
                entries: vec![
                    CodexMemoryEntry {
                        thread_id: "t-new".into(),
                        raw_memory: "raw t-new".into(),
                        rollout_summary: "summary t-new".into(),
                        generated_at: 300,
                    },
                    CodexMemoryEntry {
                        thread_id: "t-mid".into(),
                        raw_memory: "raw t-mid".into(),
                        rollout_summary: "summary t-mid".into(),
                        generated_at: 200,
                    },
                    CodexMemoryEntry {
                        thread_id: "t-old".into(),
                        raw_memory: "raw t-old".into(),
                        rollout_summary: "summary t-old".into(),
                        generated_at: 100,
                    },
                ],
            }
        );
    }

    #[test]
    fn missing_db_returns_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.sqlite");

        let result = read_codex_memories(&path);

        assert!(matches!(result, CodexDbResult::Unavailable { .. }));
        assert!(!path.exists(), "opening a missing db must not create it");
    }

    #[test]
    fn missing_table_returns_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite");
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE other (x INTEGER);")
            .unwrap();

        let result = read_codex_memories(&path);

        assert!(matches!(result, CodexDbResult::Unavailable { .. }));
    }

    #[test]
    fn opening_does_not_modify_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_db(dir.path());
        insert(&path, "t-1", 42);

        let bytes_before = fs::read(&path).unwrap();
        let mtime_before = fs::metadata(&path).unwrap().modified().unwrap();

        let result = read_codex_memories(&path);
        assert!(matches!(result, CodexDbResult::Ok { .. }));

        let bytes_after = fs::read(&path).unwrap();
        let mtime_after = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(bytes_before, bytes_after, "db bytes changed");
        assert_eq!(mtime_before, mtime_after, "db mtime changed");

        for suffix in ["-wal", "-shm", "-journal"] {
            let sibling = dir.path().join(format!("state.sqlite{suffix}"));
            assert!(!sibling.exists(), "unexpected sibling file {suffix} created");
        }
    }

    #[test]
    fn uri_for_windows_path() {
        assert_eq!(
            build_uri(r"C:\Users\Admin\.codex\state.sqlite", true),
            "file:///C:/Users/Admin/.codex/state.sqlite?mode=ro"
        );
    }

    #[test]
    fn uri_for_unix_path() {
        assert_eq!(
            build_uri("/home/u/.codex/state.sqlite", false),
            "file:/home/u/.codex/state.sqlite?mode=ro"
        );
    }

    #[test]
    fn uri_percent_encodes_special_chars() {
        assert_eq!(
            build_uri(r"C:\a b\c?d#e%f.sqlite", true),
            "file:///C:/a%20b/c%3Fd%23e%25f.sqlite?mode=ro"
        );
    }
}
