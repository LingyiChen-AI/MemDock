//! Small shared helpers for versioned JSON data files: corruption backup on
//! load and atomic save (unique temp file + fsync + rename).

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Result of reading a versioned JSON file.
#[derive(Debug)]
pub enum Loaded<T> {
    /// The file does not exist.
    Missing,
    /// Parsed fine.
    Ok(T),
    /// The file was unreadable or unparsable; it has been copied to `.bak`.
    Recovered,
}

/// Reads `path`. Missing -> `Missing`. Unreadable / unparsable -> backed up to
/// `<path>.bak` (timestamped if a different one exists) and `Recovered`; if the
/// backup fails an error is returned. A `version` greater than `max_version`
/// is backed up and refused with an error, never overwritten.
pub fn load_json_or_backup<T: DeserializeOwned>(
    path: &Path,
    max_version: u32,
    label: &str,
) -> Result<Loaded<T>, String> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Loaded::Missing),
        Err(_) => {
            backup_corrupt(path, label)?;
            return Ok(Loaded::Recovered);
        }
    };
    match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(v) => {
            let version = v.get("version").and_then(|x| x.as_u64()).unwrap_or(1);
            if version > u64::from(max_version) {
                backup_corrupt(path, label)?;
                return Err(format!(
                    "{label} {} has version {version}, newer than this build supports ({max_version}); it was backed up and left untouched",
                    path.display()
                ));
            }
            match serde_json::from_value(v) {
                Ok(r) => Ok(Loaded::Ok(r)),
                Err(_) => {
                    backup_corrupt(path, label)?;
                    Ok(Loaded::Recovered)
                }
            }
        }
        Err(_) => {
            backup_corrupt(path, label)?;
            Ok(Loaded::Recovered)
        }
    }
}

fn backup_corrupt(path: &Path, label: &str) -> Result<(), String> {
    let base = path
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("data.json"));
    let mut bak = base.clone();
    bak.push(".bak");
    let mut target = path.with_file_name(bak);
    // Already backed up (the same unreadable file is loaded again and again).
    if let (Ok(a), Ok(b)) = (fs::read(path), fs::read(&target)) {
        if a == b {
            return Ok(());
        }
    }
    if target.exists() {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let mut named = base;
        named.push(format!(".{ms}.bak"));
        target = path.with_file_name(named);
    }
    fs::copy(path, &target)
        .map(|_| ())
        .map_err(|e| format!("cannot back up unreadable {label} {}: {e}", path.display()))
}

/// Saves atomically: unique temp file in the same directory, fsync, rename.
pub fn save_json_atomic<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut tmp_name = path
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("data.json"));
    tmp_name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp_path = path.with_file_name(tmp_name);
    let write = || -> io::Result<()> {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp_path, path)
    };
    let res = write();
    if res.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    res
}

/// Unique temporary sibling path of `dest` (`<name>.tmp-<pid>-<nanos>`), used
/// to build a directory next to its final location and rename it into place.
pub(crate) fn tmp_sibling(dest: &Path) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "clone".into());
    dest.with_file_name(format!("{name}.tmp-{}-{nanos}", std::process::id()))
}
