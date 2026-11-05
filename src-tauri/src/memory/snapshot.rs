use crate::projects::normalize_path;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_SNAPSHOTS: usize = 50;

#[derive(Serialize, Clone, Debug)]
pub struct SnapshotInfo {
    pub id: String,
    pub created_at: i64,
    pub size_bytes: u64,
}

pub struct SnapshotStore {
    root: PathBuf,
}

impl SnapshotStore {
    pub fn new(root: PathBuf) -> Self {
        SnapshotStore { root }
    }

    /// `root/<first 16 hex of sha256(normalize_path(path))>`
    fn dir_for(&self, path: &Path) -> PathBuf {
        let key = normalize_path(&path.to_string_lossy());
        let digest = Sha256::digest(key.as_bytes());
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        self.root.join(&hex[..16])
    }

    /// Stores `bytes` as a new snapshot of `path` (id = unix millis, bumped by 1
    /// until unique) and prunes to the newest `MAX_SNAPSHOTS`.
    pub fn save(&self, path: &Path, bytes: &[u8]) -> io::Result<SnapshotInfo> {
        let dir = self.dir_for(path);
        fs::create_dir_all(&dir)?;
        let meta = serde_json::json!({ "path": path.to_string_lossy() });
        fs::write(dir.join("meta.json"), meta.to_string())?;

        let mut id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        loop {
            // create_new makes the uniqueness check atomic.
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(dir.join(format!("{id}.bak")))
            {
                Ok(mut f) => {
                    let res = f.write_all(bytes).and_then(|_| f.sync_all());
                    if let Err(e) = res {
                        drop(f);
                        let _ = fs::remove_file(dir.join(format!("{id}.bak")));
                        return Err(e);
                    }
                    break;
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => id += 1,
                Err(e) => return Err(e),
            }
        }

        // Pruning is best-effort: the snapshot itself is already safely saved.
        if let Ok(list) = self.list(path) {
            for old in list.iter().skip(MAX_SNAPSHOTS) {
                let _ = fs::remove_file(dir.join(format!("{}.bak", old.id)));
            }
        }
        Ok(SnapshotInfo {
            id: id.to_string(),
            created_at: id,
            size_bytes: bytes.len() as u64,
        })
    }

    /// Snapshots of `path`, newest first. Missing directory means none.
    pub fn list(&self, path: &Path) -> io::Result<Vec<SnapshotInfo>> {
        let dir = self.dir_for(path);
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".bak") else {
                continue;
            };
            let Ok(created_at) = stem.parse::<i64>() else {
                continue;
            };
            out.push(SnapshotInfo {
                id: stem.to_string(),
                created_at,
                size_bytes: entry.metadata()?.len(),
            });
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(out)
    }

    pub fn read(&self, path: &Path, id: &str) -> io::Result<Vec<u8>> {
        // ids are digits only; rejects path traversal like "../meta".
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid snapshot id",
            ));
        }
        fs::read(self.dir_for(path).join(format!("{id}.bak")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_keeps_at_most_50() {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(dir.path().join("snaps"));
        let p = Path::new("/proj/CLAUDE.md");
        for i in 0..55u32 {
            store.save(p, format!("v{i}").as_bytes()).unwrap();
        }
        let list = store.list(p).unwrap();
        assert_eq!(list.len(), MAX_SNAPSHOTS);
        // newest first, oldest 5 dropped
        assert_eq!(store.read(p, &list[0].id).unwrap(), b"v54");
        assert_eq!(store.read(p, &list[49].id).unwrap(), b"v5");
        let ids: Vec<i64> = list.iter().map(|s| s.id.parse().unwrap()).collect();
        assert!(ids.windows(2).all(|w| w[0] > w[1]));
    }

    #[test]
    fn save_records_meta_and_size_and_isolates_paths() {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(dir.path().join("snaps"));
        let a = Path::new("/proj/a/CLAUDE.md");
        let b = Path::new("/proj/b/CLAUDE.md");
        let info = store.save(a, b"hello").unwrap();
        assert_eq!(info.size_bytes, 5);
        assert!(info.created_at > 0);
        assert_eq!(store.list(b).unwrap().len(), 0);
        let key = crate::projects::normalize_path(&a.to_string_lossy());
        let metas: Vec<_> = std::fs::read_dir(dir.path().join("snaps"))
            .unwrap()
            .map(|e| e.unwrap().path().join("meta.json"))
            .collect();
        assert_eq!(metas.len(), 1);
        let meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&metas[0]).unwrap()).unwrap();
        assert_eq!(meta["path"], a.to_string_lossy().as_ref());
        assert_eq!(metas[0].parent().unwrap().file_name().unwrap().len(), 16);
        let _ = key;
    }

    #[test]
    fn read_rejects_bad_ids() {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(dir.path().join("snaps"));
        let p = Path::new("/proj/CLAUDE.md");
        store.save(p, b"x").unwrap();
        assert!(store.read(p, "../meta.json").is_err());
        assert!(store.read(p, "123").is_err());
    }
}
