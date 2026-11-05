use super::snapshot::SnapshotStore;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Clone, Debug)]
pub struct FileContent {
    pub text: String,
    pub hash: String,
    pub has_bom: bool,
    pub crlf: bool,
}

#[derive(Debug)]
pub enum WriteError {
    DiskChanged { current_hash: String },
    Io(io::Error),
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteError::DiskChanged { .. } => write!(f, "file changed on disk"),
            WriteError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for WriteError {
    fn from(e: io::Error) -> Self {
        WriteError::Io(e)
    }
}

const BOM: &[u8] = b"\xEF\xBB\xBF";

fn hash_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Reads the file bytes; `None` when it does not exist.
fn read_bytes(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn decode(bytes: &[u8]) -> io::Result<FileContent> {
    let has_bom = bytes.starts_with(BOM);
    let body = if has_bom { &bytes[BOM.len()..] } else { bytes };
    let raw = std::str::from_utf8(body).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "file is not valid UTF-8")
    })?;
    // `crlf` is true when the original contains any "\r\n".
    let crlf = raw.contains("\r\n");
    Ok(FileContent {
        text: raw.replace("\r\n", "\n"),
        hash: hash_bytes(bytes),
        has_bom,
        crlf,
    })
}

/// Missing file yields empty text and hash "".
pub fn read_memory(path: &Path) -> io::Result<FileContent> {
    match read_bytes(path)? {
        Some(bytes) => decode(&bytes),
        None => Ok(FileContent {
            text: String::new(),
            hash: String::new(),
            has_bom: false,
            crlf: false,
        }),
    }
}

fn encode(text: &str, has_bom: bool, crlf: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 3);
    if has_bom {
        out.extend_from_slice(BOM);
    }
    if crlf {
        // Normalize first so any stray "\r\n" is not doubled to "\r\r\n".
        out.extend_from_slice(text.replace("\r\n", "\n").replace('\n', "\r\n").as_bytes());
    } else {
        out.extend_from_slice(text.as_bytes());
    }
    out
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".{name}.memdock-tmp-{}-{nanos}", std::process::id()));

    let result = (|| {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&tmp, meta.permissions());
        }
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Writes `text` to `path` after verifying the on-disk hash still equals
/// `expected_hash` ("" for a file that must not exist yet). An existing file is
/// snapshotted first; BOM / CRLF of the original are preserved.
pub fn write_memory(
    path: &Path,
    text: &str,
    expected_hash: &str,
    snapshots: &SnapshotStore,
) -> Result<FileContent, WriteError> {
    // Disk-hash check immediately before writing.
    let original = read_bytes(path)?;
    let current_hash = original.as_deref().map(hash_bytes).unwrap_or_default();
    if current_hash != expected_hash {
        return Err(WriteError::DiskChanged { current_hash });
    }

    let (has_bom, crlf) = match &original {
        Some(bytes) => {
            snapshots.save(path, bytes)?;
            let body = bytes.strip_prefix(BOM).unwrap_or(bytes);
            (
                bytes.starts_with(BOM),
                body.windows(2).any(|w| w == b"\r\n"),
            )
        }
        None => (false, false),
    };
    let bytes = encode(text, has_bom, crlf);

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let is_symlink = fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if is_symlink {
        // Writing through the link updates the target and keeps the link.
        fs::write(path, &bytes)?;
    } else {
        write_atomic(path, &bytes)?;
    }
    Ok(read_memory(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn store(dir: &Path) -> SnapshotStore {
        SnapshotStore::new(dir.join("snaps"))
    }

    #[test]
    fn read_strips_bom_and_normalizes_crlf() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("CLAUDE.md");
        fs::write(&p, b"\xEF\xBB\xBFa\r\nb").unwrap();
        let c = read_memory(&p).unwrap();
        assert_eq!(c.text, "a\nb");
        assert!(c.has_bom);
        assert!(c.crlf);
        assert_eq!(c.hash.len(), 64);
        let missing = read_memory(&dir.path().join("nope.md")).unwrap();
        assert_eq!(missing.text, "");
        assert_eq!(missing.hash, "");
    }

    #[test]
    fn write_preserves_bom_and_crlf() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("CLAUDE.md");
        fs::write(&p, b"\xEF\xBB\xBFx\r\ny").unwrap();
        let before = read_memory(&p).unwrap();
        let after = write_memory(&p, "a\nb", &before.hash, &store(dir.path())).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"\xEF\xBB\xBFa\r\nb");
        assert_eq!(after.text, "a\nb");
        assert!(after.has_bom && after.crlf);
        assert_eq!(after.hash, read_memory(&p).unwrap().hash);
    }

    #[test]
    fn write_rejects_when_disk_changed() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("CLAUDE.md");
        fs::write(&p, "one").unwrap();
        let before = read_memory(&p).unwrap();
        fs::write(&p, "external").unwrap();
        let err = write_memory(&p, "mine", &before.hash, &store(dir.path())).unwrap_err();
        match err {
            WriteError::DiskChanged { current_hash } => {
                assert_eq!(current_hash, read_memory(&p).unwrap().hash)
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(fs::read_to_string(&p).unwrap(), "external");
    }

    #[test]
    fn write_new_file_rejected_if_created_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("CLAUDE.md");
        fs::write(&p, "appeared").unwrap();
        let err = write_memory(&p, "mine", "", &store(dir.path())).unwrap_err();
        assert!(matches!(err, WriteError::DiskChanged { .. }));
        assert_eq!(fs::read_to_string(&p).unwrap(), "appeared");
    }

    #[test]
    fn write_creates_snapshot_of_previous() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("CLAUDE.md");
        fs::write(&p, "old\r\ncontent").unwrap();
        let before = read_memory(&p).unwrap();
        let s = store(dir.path());
        write_memory(&p, "new", &before.hash, &s).unwrap();
        let list = s.list(&p).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(s.read(&p, &list[0].id).unwrap(), b"old\r\ncontent");
        assert_eq!(fs::read(&p).unwrap(), b"new");
    }

    #[test]
    fn write_new_file_creates_parent_dirs_no_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a").join("b").join("AGENTS.md");
        let s = store(dir.path());
        let c = write_memory(&p, "hi\nthere", "", &s).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"hi\nthere");
        assert!(!c.has_bom && !c.crlf);
        assert_eq!(s.list(&p).unwrap().len(), 0);
    }

    #[test]
    fn no_temp_files_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("CLAUDE.md");
        fs::write(&p, "a").unwrap();
        let h = read_memory(&p).unwrap().hash;
        write_memory(&p, "b", &h, &store(dir.path())).unwrap();
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("memdock-tmp"))
            .collect();
        assert!(names.is_empty(), "{names:?}");
    }

    #[cfg(unix)]
    #[test]
    fn write_through_symlink_keeps_link() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.md");
        let link = dir.path().join("CLAUDE.md");
        fs::write(&target, "old").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let h = read_memory(&link).unwrap().hash;
        write_memory(&link, "new", &h, &store(dir.path())).unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
    }
}
