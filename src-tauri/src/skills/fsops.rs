//! Safe file operations for skill install / update: copy, backup, atomic
//! replace and zip extraction. These write into users' real skill folders, so
//! every step either completes or restores the original state.

use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::model::Tool;
use crate::projects::normalize_path;
use crate::skills::model::SkillScope;

/// Names never copied by [`copy_dir`] (and ignored by the content hash).
const EXCLUDED: [&str; 4] = [
    ".git",
    "node_modules",
    ".memdock-complete",
    ".memdock-meta.json",
];
/// Describes a backup (see [`BackupMeta`]). A dot name so it cannot collide
/// with a skill's own `meta.json`; never restored.
pub const META_FILE: &str = ".memdock-meta.json";
/// Backups kept per skill.
pub const MAX_BACKUPS: usize = 10;
/// Upper bound for the uncompressed size of one zip.
pub const MAX_ZIP_BYTES: u64 = 200 * 1024 * 1024;
/// Upper bound for the number of entries in one zip.
pub const MAX_ZIP_ENTRIES: usize = 20_000;
/// Deepest directory nesting copied (guards against symlink loops).
const MAX_DEPTH: usize = 40;
/// Age after which leftover temp directories are removed.
const LEFTOVER_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(3600);

pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
pub struct BackupInfo {
    /// Directory name under `<backups_root>/<bucket>/`, the creation time in ms.
    pub id: String,
    pub created_at: i64,
    pub size_bytes: u64,
}

/// What `copy_tree` skips.
struct CopyOpts<'a> {
    /// Skip `.git`, `node_modules` and `.memdock-complete`.
    exclude: bool,
    /// When set, symlinks are followed only if they resolve inside this
    /// (canonical) directory; others are skipped.
    confine: Option<&'a Path>,
}

fn copy_tree(src: &Path, dst: &Path, opts: &CopyOpts, depth: usize) -> io::Result<()> {
    if depth > MAX_DEPTH {
        return Err(io::Error::other(format!(
            "directory nesting too deep (symlink loop?): {}",
            src.display()
        )));
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if opts.exclude && EXCLUDED.iter().any(|x| name == *x) {
            continue;
        }
        let path = entry.path();
        let to = dst.join(&name);
        let lmeta = fs::symlink_metadata(&path)?;
        let meta = if lmeta.is_symlink() {
            if let Some(root) = opts.confine {
                match fs::canonicalize(&path) {
                    Ok(real) if real.starts_with(root) => {}
                    _ => continue,
                }
            }
            match fs::metadata(&path) {
                Ok(m) => m,
                Err(_) => continue, // dangling link
            }
        } else {
            lmeta
        };
        if meta.is_dir() {
            copy_tree(&path, &to, opts, depth + 1)?;
        } else if meta.is_file() {
            fs::copy(&path, &to)?;
        }
    }
    Ok(())
}

/// Copies `src` into `dst` (created if missing), skipping `.git`,
/// `node_modules` and `.memdock-complete`. Symlinks are not copied as links:
/// the content they point to is copied.
pub fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    copy_tree(
        src,
        dst,
        &CopyOpts {
            exclude: true,
            confine: None,
        },
        0,
    )
}

/// Like [`copy_dir`] for untrusted trees (git clones): symlinks that resolve
/// outside `root` are skipped instead of followed.
pub fn copy_dir_confined(src: &Path, dst: &Path, root: &Path) -> io::Result<()> {
    let root = fs::canonicalize(root)?;
    copy_tree(
        src,
        dst,
        &CopyOpts {
            exclude: true,
            confine: Some(&root),
        },
        0,
    )
}

pub fn dir_size(dir: &Path) -> u64 {
    WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Removes a file, symlink (to a file or directory) or directory tree without
/// ever following a link.
pub fn remove_any(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else if meta.is_symlink() {
        // Windows directory links need remove_dir; file links remove_file.
        fs::remove_file(path).or_else(|_| fs::remove_dir(path))
    } else {
        fs::remove_file(path)
    }
}

/// Symlinks that could not be recreated inside a backup (Windows without the
/// symlink privilege). The target is recorded, never followed.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct LinkRecord {
    /// `/`-separated path relative to the skill directory.
    pub path: String,
    pub target: String,
}

/// Describes whose backup a bucket holds; written next to the copied files.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct BackupMeta {
    /// The skill directory the backup was taken from (the link for symlinked skills).
    pub dir: String,
    pub name: String,
    pub tool: Tool,
    pub scope: SkillScope,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<LinkRecord>,
}

impl BackupMeta {
    pub fn new(dir: &str, name: &str, tool: Tool, scope: SkillScope) -> Self {
        BackupMeta {
            dir: dir.to_string(),
            name: name.to_string(),
            tool,
            scope,
            links: Vec::new(),
        }
    }
}

/// A skill whose directory is gone but whose backups remain.
#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct DeletedSkill {
    pub bucket: String,
    pub name: String,
    pub tool: Tool,
    pub scope: SkillScope,
    pub dir: String,
    /// Newest first.
    pub backups: Vec<BackupInfo>,
}

/// `<sanitized dir name>-<first 8 hex of sha256(normalize_path(dir))>`: the
/// backup bucket of the skill directory `dir`. Same-named skills in different
/// places never share a bucket, and the (untrusted) skill name is not used.
pub fn backup_bucket(dir: &Path) -> String {
    let base = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut safe: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    if safe.is_empty() {
        safe = "skill".into();
    }
    let digest = Sha256::digest(normalize_path(&dir.to_string_lossy()).as_bytes());
    let hex: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("{safe}-{hex}")
}

/// True for names that [`backup_bucket`] can produce (no separators, no `..`).
pub fn valid_bucket(b: &str) -> bool {
    !b.is_empty()
        && b != "."
        && b != ".."
        && b.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// Backup ids are millisecond timestamps.
pub fn valid_backup_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
}

fn make_link(target: &Path, link: &Path, is_dir: bool) -> io::Result<()> {
    #[cfg(unix)]
    {
        let _ = is_dir;
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        if is_dir {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        }
    }
}

fn join_rel(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.to_string()
    } else {
        format!("{rel}/{name}")
    }
}

/// Copies a tree keeping symlinks as links (never following them). Links that
/// cannot be created are appended to `failed`. `exclude` skips the names in
/// [`EXCLUDED`].
fn copy_keep_links(
    src: &Path,
    dst: &Path,
    rel: &str,
    exclude: bool,
    failed: &mut Vec<LinkRecord>,
    depth: usize,
) -> io::Result<()> {
    if depth > MAX_DEPTH {
        return Err(io::Error::other(format!(
            "directory nesting too deep: {}",
            src.display()
        )));
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == META_FILE || (exclude && EXCLUDED.iter().any(|x| name == *x)) {
            continue;
        }
        let path = entry.path();
        let to = dst.join(&name);
        let lmeta = fs::symlink_metadata(&path)?;
        let child_rel = join_rel(rel, &name.to_string_lossy());
        if lmeta.is_symlink() {
            let target = fs::read_link(&path)?;
            let is_dir = fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false);
            if make_link(&target, &to, is_dir).is_err() {
                failed.push(LinkRecord {
                    path: child_rel,
                    target: target.to_string_lossy().into_owned(),
                });
            }
        } else if lmeta.is_dir() {
            copy_keep_links(&path, &to, &child_rel, exclude, failed, depth + 1)?;
        } else if lmeta.is_file() {
            fs::copy(&path, &to)?;
        }
    }
    Ok(())
}

/// Restores the files of a backup into `dst` (created). Symlinks inside are
/// recreated as links where the OS allows it and skipped otherwise; the meta
/// file is never brought back. `.git` / `node_modules` only come back with
/// `keep_all` (a skill that no longer exists); a rollback keeps the live ones.
pub fn restore_dir(
    backup: &Path,
    dst: &Path,
    meta: Option<&BackupMeta>,
    keep_all: bool,
) -> io::Result<()> {
    let mut failed = Vec::new();
    copy_keep_links(backup, dst, "", !keep_all, &mut failed, 0)?;
    if let Some(m) = meta {
        for l in &m.links {
            let to = dst.join(l.path.replace('/', std::path::MAIN_SEPARATOR_STR));
            let _ = make_link(Path::new(&l.target), &to, true)
                .or_else(|_| make_link(Path::new(&l.target), &to, false));
        }
    }
    Ok(())
}

/// Copies `src` to `<backups_root>/<bucket>/<millis>/` (symlinks kept as links,
/// never followed) with a meta file, and keeps at most [`MAX_BACKUPS`] backups
/// per bucket (oldest are deleted). The bucket comes from `meta.dir`.
pub fn backup_dir(src: &Path, backups_root: &Path, meta: &BackupMeta) -> io::Result<BackupInfo> {
    let bucket = backup_bucket(Path::new(&meta.dir));
    let base = backups_root.join(&bucket);
    fs::create_dir_all(&base)?;
    let mut ms = now_millis();
    while base.join(ms.to_string()).exists() {
        ms += 1;
    }
    let dest = base.join(ms.to_string());
    let mut meta = meta.clone();
    meta.links.clear();
    let result = (|| -> io::Result<()> {
        copy_keep_links(src, &dest, "", false, &mut meta.links, 0)?;
        let json = serde_json::to_string_pretty(&meta).map_err(io::Error::other)?;
        fs::write(dest.join(META_FILE), json)
    })();
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&dest);
        return Err(e);
    }
    let info = BackupInfo {
        id: ms.to_string(),
        created_at: ms,
        size_bytes: dir_size(&dest),
    };
    prune_backups(backups_root, &bucket, &info.id);
    Ok(info)
}

/// Meta of one backup, if it has a readable one.
pub fn read_meta(backups_root: &Path, bucket: &str, id: &str) -> Option<BackupMeta> {
    if !valid_bucket(bucket) || !valid_backup_id(id) {
        return None;
    }
    let text = fs::read_to_string(backups_root.join(bucket).join(id).join(META_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Deletes the oldest backups of `bucket` beyond [`MAX_BACKUPS`], never `keep_id`.
pub fn prune_backups(backups_root: &Path, bucket: &str, keep_id: &str) {
    let base = backups_root.join(bucket);
    let others: Vec<BackupInfo> = list_backups(backups_root, bucket)
        .into_iter()
        .filter(|b| b.id != keep_id)
        .collect();
    for old in others.iter().skip(MAX_BACKUPS - 1) {
        let _ = fs::remove_dir_all(base.join(&old.id));
    }
}

/// Backups of `bucket`, newest first.
pub fn list_backups(backups_root: &Path, bucket: &str) -> Vec<BackupInfo> {
    let Ok(rd) = fs::read_dir(backups_root.join(bucket)) else {
        return Vec::new();
    };
    let mut out: Vec<BackupInfo> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            let created_at = id.parse::<i64>().ok()?;
            Some(BackupInfo {
                size_bytes: dir_size(&e.path()),
                id,
                created_at,
            })
        })
        .collect();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out
}

/// Buckets whose skill directory no longer exists, newest backup first.
/// Buckets without a readable meta file are ignored.
pub fn list_deleted(backups_root: &Path) -> Vec<DeletedSkill> {
    let Ok(rd) = fs::read_dir(backups_root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let bucket = e.file_name().to_string_lossy().into_owned();
        if !valid_bucket(&bucket) || !e.path().is_dir() {
            continue;
        }
        let backups = list_backups(backups_root, &bucket);
        let Some(meta) = backups
            .iter()
            .find_map(|b| read_meta(backups_root, &bucket, &b.id))
        else {
            continue;
        };
        // A meta pointing at another bucket is not ours.
        if backup_bucket(Path::new(&meta.dir)) != bucket {
            continue;
        }
        if fs::symlink_metadata(&meta.dir).is_ok() {
            continue;
        }
        out.push(DeletedSkill {
            bucket,
            name: meta.name,
            tool: meta.tool,
            scope: meta.scope,
            dir: meta.dir,
            backups,
        });
    }
    out.sort_by(|a, b| {
        let t = |d: &DeletedSkill| d.backups.first().map(|x| x.created_at).unwrap_or(0);
        t(b).cmp(&t(a)).then_with(|| a.bucket.cmp(&b.bucket))
    });
    out
}

/// Names carried over from the live directory when it is replaced.
const CARRIED: [&str; 2] = [".git", "node_modules"];

/// Moves the live `.git` / `node_modules` of `live` into `new` (which must be
/// on the same volume) and returns what was moved. All or nothing.
pub fn carry_live_dirs(live: &Path, new: &Path) -> io::Result<Vec<&'static str>> {
    let mut moved = Vec::new();
    for n in CARRIED {
        let from = live.join(n);
        if fs::symlink_metadata(&from).is_err() {
            continue;
        }
        if let Err(e) = fs::rename(&from, new.join(n)) {
            uncarry_live_dirs(live, new, &moved);
            return Err(e);
        }
        moved.push(n);
    }
    Ok(moved)
}

/// Puts what [`carry_live_dirs`] moved back into `live`.
pub fn uncarry_live_dirs(live: &Path, new: &Path, moved: &[&str]) {
    for n in moved {
        let _ = fs::rename(new.join(n), live.join(n));
    }
}

fn sibling_named(target: &Path, tag: &str) -> PathBuf {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "skill".into());
    let ts = now_millis();
    let pid = std::process::id();
    target.with_file_name(format!(".{name}.memdock-{tag}-{pid}-{ts}"))
}

/// Replaces `target` with the contents of `prepared`. `prepared` must be on
/// the same volume as `target` (a sibling temp directory) and is consumed.
/// Any failure restores `target` to its original state.
///
/// - `target` missing: `prepared` is renamed into place.
/// - `target` a directory: rename `target` to `.<name>.memdock-old-*`, rename
///   `prepared` into place, delete the old directory; rolled back on failure.
/// - `target` a symlink: the link stays; the link target's contents are
///   replaced (with a safety copy that is restored on failure).
pub fn atomic_replace_dir(prepared: &Path, target: &Path) -> io::Result<()> {
    let meta = match fs::symlink_metadata(target) {
        Ok(m) => Some(m),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    match meta {
        None => fs::rename(prepared, target),
        Some(m) if m.is_symlink() => replace_through_link(prepared, target),
        Some(_) => {
            let old = sibling_named(target, "old");
            fs::rename(target, &old)?;
            if let Err(e) = fs::rename(prepared, target) {
                // Roll back: put the original directory back.
                if let Err(e2) = fs::rename(&old, target) {
                    return Err(io::Error::other(format!(
                        "replace failed ({e}) and restore failed ({e2}); original kept at {}",
                        old.display()
                    )));
                }
                return Err(e);
            }
            // New content is in place; the old copy is just garbage now.
            if let Err(e) = fs::remove_dir_all(&old) {
                eprintln!(
                    "memdock: could not remove old skill copy {}: {e}",
                    old.display()
                );
            }
            Ok(())
        }
    }
}

/// Empties `dir` but keeps the excluded entries (`.git`, `node_modules`, ...)
/// that [`copy_dir`] never brings back.
fn clear_dir_contents(dir: &Path) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if EXCLUDED.iter().any(|x| entry.file_name() == *x) {
            continue;
        }
        remove_any(&entry.path())?;
    }
    Ok(())
}

/// Removes stale `.<name>.memdock-old-*` / `.memdock-new-*` / `.memdock-tmp-*`
/// leftovers in `root` that are older than one hour.
pub fn cleanup_leftovers(root: &Path) {
    let Ok(rd) = fs::read_dir(root) else { return };
    let now = SystemTime::now();
    for e in rd.flatten() {
        if !is_leftover_name(&e.file_name().to_string_lossy()) {
            continue;
        }
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age >= LEFTOVER_MAX_AGE);
        if old {
            if let Err(err) = remove_any(&e.path()) {
                eprintln!(
                    "memdock: could not remove leftover {}: {err}",
                    e.path().display()
                );
            }
        }
    }
}

/// True for temp directories created by install / replace (`.x.memdock-old-1`).
pub fn is_leftover_name(name: &str) -> bool {
    name.starts_with('.')
        && ["old", "new", "tmp"]
            .iter()
            .any(|k| name.contains(&format!(".memdock-{k}-")))
}

fn replace_through_link(prepared: &Path, link: &Path) -> io::Result<()> {
    let real = fs::canonicalize(link)?;
    if !real.is_dir() {
        return Err(io::Error::other(format!(
            "symlink target is not a directory: {}",
            real.display()
        )));
    }
    // The link target must not contain the link's own location or `prepared`
    // (clearing it would wipe the skills directory).
    let inside = |p: &Path| -> bool {
        let canon = p
            .parent()
            .and_then(|d| fs::canonicalize(d).ok())
            .map(|d| d.join(p.file_name().unwrap_or_default()))
            .or_else(|| fs::canonicalize(p).ok());
        canon.is_none_or(|c| c.starts_with(&real))
    };
    if inside(link) || inside(prepared) {
        return Err(io::Error::other(format!(
            "refusing to replace through a symlink whose target contains the link or the new content: {}",
            real.display()
        )));
    }
    // Excluded entries (.git, node_modules) stay untouched, so the safety
    // copy only needs the rest.
    let safety = sibling_named(&real, "old");
    copy_dir(&real, &safety).inspect_err(|_| {
        let _ = fs::remove_dir_all(&safety);
    })?;
    let result = clear_dir_contents(&real).and_then(|_| copy_dir(prepared, &real));
    match result {
        Ok(()) => {
            let _ = fs::remove_dir_all(&safety);
            let _ = fs::remove_dir_all(prepared);
            Ok(())
        }
        Err(e) => {
            let restored = clear_dir_contents(&real).and_then(|_| copy_dir(&safety, &real));
            match restored {
                Ok(()) => {
                    let _ = fs::remove_dir_all(&safety);
                    Err(e)
                }
                Err(e2) => Err(io::Error::other(format!(
                    "replace failed ({e}) and restore failed ({e2}); original kept at {}",
                    safety.display()
                ))),
            }
        }
    }
}

/// Sanitized relative path parts of a zip entry name, or an error for unsafe
/// names (absolute, drive prefix, `..`). `.` and empty parts are dropped.
fn zip_parts(name: &str) -> Result<Vec<String>, String> {
    let norm = name.replace('\\', "/");
    if norm.starts_with('/') {
        return Err(format!("unsafe zip entry (absolute path): {name}"));
    }
    let mut parts = Vec::new();
    for p in norm.split('/') {
        match p {
            "" | "." => {}
            ".." => return Err(format!("unsafe zip entry (..): {name}")),
            p if p.contains(':') => {
                return Err(format!("unsafe zip entry (drive or stream prefix): {name}"));
            }
            p => parts.push(p.to_string()),
        }
    }
    // Defense in depth: the parts must form a purely relative path.
    let joined: PathBuf = parts.iter().collect();
    if joined
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(format!("unsafe zip entry: {name}"));
    }
    Ok(parts)
}

fn is_junk(parts: &[String]) -> bool {
    parts.first().is_some_and(|p| p == "__MACOSX") || parts.last().is_some_and(|p| p == ".DS_Store")
}

/// Extracts `zip_path` into `dest` (created if needed). Rejects zip-slip,
/// symlink entries, more than [`MAX_ZIP_ENTRIES`] entries and more than
/// `max_total_bytes` uncompressed. When the archive has a single top-level
/// directory, that level is removed. On error `dest` may hold partial output;
/// the caller discards it.
pub fn extract_zip(zip_path: &Path, dest: &Path, max_total_bytes: u64) -> Result<(), String> {
    let file = fs::File::open(zip_path).map_err(|e| format!("cannot open zip: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("invalid zip: {e}"))?;
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(format!("zip has too many entries ({})", archive.len()));
    }
    // Pass 1: validate everything before writing anything.
    struct Item {
        index: usize,
        parts: Vec<String>,
        is_dir: bool,
    }
    let mut items: Vec<Item> = Vec::new();
    let mut declared: u64 = 0;
    for i in 0..archive.len() {
        let f = archive
            .by_index(i)
            .map_err(|e| format!("invalid zip: {e}"))?;
        let ename = f
            .name()
            .map_err(|e| format!("invalid zip entry name: {e}"))?
            .into_owned();
        let parts = zip_parts(&ename)?;
        if let Some(mode) = f.unix_mode() {
            if mode & 0o170000 == 0o120000 {
                return Err(format!("zip contains a symlink entry: {ename}"));
            }
        }
        declared = declared.saturating_add(f.size());
        if declared > max_total_bytes {
            return Err("zip is too large when extracted".into());
        }
        if parts.is_empty() || is_junk(&parts) {
            continue;
        }
        items.push(Item {
            index: i,
            parts,
            is_dir: f.is_dir(),
        });
    }
    let first = items.first().map(|i| i.parts[0].clone());
    let strip = match first {
        Some(top) => {
            items.iter().all(|i| i.parts[0] == top)
                && items.iter().any(|i| i.parts.len() > 1 || i.is_dir)
                && items.iter().all(|i| i.parts.len() > 1 || i.is_dir)
        }
        None => false,
    };
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    // Pass 2: extract, counting the bytes actually written.
    let mut written: u64 = 0;
    for item in items {
        let rel: PathBuf = if strip {
            item.parts[1..].iter().collect()
        } else {
            item.parts.iter().collect()
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&rel);
        if item.is_dir {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut f = archive
            .by_index(item.index)
            .map_err(|e| format!("invalid zip: {e}"))?;
        let mut w = fs::File::create(&out).map_err(|e| e.to_string())?;
        let remaining = max_total_bytes - written;
        let n = io::copy(&mut (&mut f).take(remaining + 1), &mut w).map_err(|e| e.to_string())?;
        written += n;
        if written > max_total_bytes {
            return Err("zip is too large when extracted".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(p: &Path, text: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    /// Meta for a skill directory called `name` under `root/skills`.
    fn meta(root: &Path, name: &str) -> BackupMeta {
        BackupMeta::new(
            &root.join("skills").join(name).to_string_lossy(),
            name,
            Tool::Claude,
            SkillScope::Global,
        )
    }

    fn bucket(root: &Path, name: &str) -> String {
        backup_bucket(&root.join("skills").join(name))
    }

    fn make_zip(path: &Path, entries: &[(&str, &str)]) {
        let f = fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, body) in entries {
            if name.ends_with('/') {
                z.add_directory(*name, opts).unwrap();
            } else {
                z.start_file(*name, opts).unwrap();
                z.write_all(body.as_bytes()).unwrap();
            }
        }
        z.finish().unwrap();
    }

    /// Creates a directory symlink; returns false when the OS refuses.
    fn try_dir_symlink(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        let r = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let r = std::os::windows::fs::symlink_dir(target, link).or_else(|_| {
            // No symlink privilege: a junction behaves the same for these tests.
            let ok = std::process::Command::new("cmd")
                .arg("/C")
                .arg("mklink")
                .arg("/J")
                .arg(link.to_string_lossy().replace('/', "\\"))
                .arg(target.to_string_lossy().replace('/', "\\"))
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if ok {
                Ok(())
            } else {
                Err(io::Error::other("no junction"))
            }
        });
        r.is_ok()
    }

    #[test]
    fn copy_dir_skips_excluded_and_nested() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        write(&src.join("SKILL.md"), "a");
        write(&src.join("sub/f.txt"), "b");
        write(&src.join(".git/HEAD"), "x");
        write(&src.join("node_modules/m.js"), "x");
        write(&src.join("sub/.memdock-complete"), "x");
        let dst = t.path().join("dst");
        copy_dir(&src, &dst).unwrap();
        assert_eq!(fs::read_to_string(dst.join("SKILL.md")).unwrap(), "a");
        assert_eq!(fs::read_to_string(dst.join("sub/f.txt")).unwrap(), "b");
        assert!(!dst.join(".git").exists());
        assert!(!dst.join("node_modules").exists());
        assert!(!dst.join("sub/.memdock-complete").exists());
    }

    #[test]
    fn copy_dir_copies_symlink_content_not_link() {
        let t = tempfile::tempdir().unwrap();
        let real = t.path().join("real");
        write(&real.join("f.txt"), "linked");
        let src = t.path().join("src");
        write(&src.join("SKILL.md"), "a");
        if !try_dir_symlink(&real, &src.join("lnk")) {
            eprintln!("skipped: cannot create symlinks");
            return;
        }
        let dst = t.path().join("dst");
        copy_dir(&src, &dst).unwrap();
        let m = fs::symlink_metadata(dst.join("lnk")).unwrap();
        assert!(m.is_dir() && !m.is_symlink());
        assert_eq!(fs::read_to_string(dst.join("lnk/f.txt")).unwrap(), "linked");
        // Confined copy skips links leaving the root.
        let dst2 = t.path().join("dst2");
        copy_dir_confined(&src, &dst2, &src).unwrap();
        assert!(dst2.join("SKILL.md").exists());
        assert!(!dst2.join("lnk").exists());
    }

    #[test]
    fn backup_keeps_ten_newest_first() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        write(&src.join("SKILL.md"), "12345");
        write(&src.join(".git/HEAD"), "keep");
        let root = t.path().join("bk");
        let mut last = None;
        for _ in 0..12 {
            last = Some(backup_dir(&src, &root, &meta(t.path(), "demo")).unwrap());
        }
        let list = list_backups(&root, &bucket(t.path(), "demo"));
        assert_eq!(list.len(), MAX_BACKUPS);
        assert_eq!(list[0].id, last.unwrap().id);
        assert!(list.windows(2).all(|w| w[0].created_at > w[1].created_at));
        assert!(list[0].size_bytes >= 5 + 4, "files plus the meta file");
        // Backups are full copies, including .git.
        assert!(
            root.join(bucket(t.path(), "demo"))
                .join(&list[0].id)
                .join(".git/HEAD")
                .exists()
        );
        assert!(list_backups(&root, "none-00000000").is_empty());
    }

    #[test]
    fn atomic_replace_swaps_and_leaves_no_temp() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("skill");
        write(&target.join("old.txt"), "old");
        let prepared = t.path().join(".skill.memdock-new");
        write(&prepared.join("new.txt"), "new");
        atomic_replace_dir(&prepared, &target).unwrap();
        assert!(target.join("new.txt").exists());
        assert!(!target.join("old.txt").exists());
        assert!(!prepared.exists());
        let names: Vec<_> = fs::read_dir(t.path()).unwrap().flatten().collect();
        assert_eq!(names.len(), 1);
        // Missing target: plain rename.
        let t2 = t.path().join("fresh");
        let p2 = t.path().join(".fresh.new");
        write(&p2.join("a"), "a");
        atomic_replace_dir(&p2, &t2).unwrap();
        assert!(t2.join("a").exists());
    }

    #[test]
    fn atomic_replace_failure_restores_original() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("skill");
        write(&target.join("old.txt"), "old");
        // `prepared` does not exist, so the second rename fails.
        let err = atomic_replace_dir(&t.path().join("missing"), &target);
        assert!(err.is_err());
        assert_eq!(fs::read_to_string(target.join("old.txt")).unwrap(), "old");
        let names: Vec<_> = fs::read_dir(t.path()).unwrap().flatten().collect();
        assert_eq!(names.len(), 1, "no temp dirs left");
    }

    #[test]
    fn atomic_replace_through_symlink_keeps_link() {
        let t = tempfile::tempdir().unwrap();
        let real = t.path().join("repo/skill");
        write(&real.join("old.txt"), "old");
        write(&real.join("sub/deep.txt"), "old");
        let link = t.path().join("link");
        if !try_dir_symlink(&real, &link) {
            eprintln!("skipped: cannot create symlinks");
            return;
        }
        let prepared = t.path().join(".link.new");
        write(&prepared.join("new.txt"), "new");
        atomic_replace_dir(&prepared, &link).unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(fs::read_to_string(real.join("new.txt")).unwrap(), "new");
        assert!(!real.join("old.txt").exists());
        assert!(!real.join("sub").exists());
        assert!(!prepared.exists());
        let leftovers: Vec<_> = fs::read_dir(t.path().join("repo"))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(leftovers.len(), 1, "no safety copy left");
    }

    #[test]
    fn zip_single_top_dir_flattened() {
        let t = tempfile::tempdir().unwrap();
        let z = t.path().join("a.zip");
        make_zip(
            &z,
            &[
                ("demo/", ""),
                ("demo/SKILL.md", "hi"),
                ("demo/sub/x.txt", "x"),
                ("__MACOSX/demo/._SKILL.md", "junk"),
            ],
        );
        let out = t.path().join("out");
        extract_zip(&z, &out, MAX_ZIP_BYTES).unwrap();
        assert_eq!(fs::read_to_string(out.join("SKILL.md")).unwrap(), "hi");
        assert!(out.join("sub/x.txt").exists());
        assert!(!out.join("demo").exists());
        // Two top-level entries are kept as is.
        let z2 = t.path().join("b.zip");
        make_zip(&z2, &[("SKILL.md", "hi"), ("ref/a.md", "a")]);
        let out2 = t.path().join("out2");
        extract_zip(&z2, &out2, MAX_ZIP_BYTES).unwrap();
        assert!(out2.join("SKILL.md").exists() && out2.join("ref/a.md").exists());
    }

    #[test]
    fn zip_slip_rejected() {
        let t = tempfile::tempdir().unwrap();
        for (i, bad) in [
            "../evil.txt",
            "a/../../evil.txt",
            "/abs/evil.txt",
            "C:/evil.txt",
            "..\\evil.txt",
        ]
        .iter()
        .enumerate()
        {
            let z = t.path().join(format!("{i}.zip"));
            make_zip(&z, &[("ok/SKILL.md", "x"), (bad, "boom")]);
            let out = t.path().join(format!("out{i}"));
            assert!(extract_zip(&z, &out, MAX_ZIP_BYTES).is_err(), "{bad}");
            assert!(!t.path().join("evil.txt").exists());
            assert!(!out.exists(), "nothing written before validation passes");
        }
    }

    #[test]
    fn zip_too_large_rejected() {
        let t = tempfile::tempdir().unwrap();
        let z = t.path().join("a.zip");
        make_zip(&z, &[("SKILL.md", "0123456789")]);
        assert!(extract_zip(&z, &t.path().join("o1"), 5).is_err());
        assert!(extract_zip(&z, &t.path().join("o2"), 10).is_ok());
    }

    #[test]
    fn zip_symlink_entry_rejected() {
        let t = tempfile::tempdir().unwrap();
        let z = t.path().join("a.zip");
        let f = fs::File::create(&z).unwrap();
        let mut w = zip::ZipWriter::new(f);
        w.start_file("SKILL.md", zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(b"x").unwrap();
        w.add_symlink("lnk", "/etc", zip::write::SimpleFileOptions::default())
            .unwrap();
        w.finish().unwrap();
        assert!(extract_zip(&z, &t.path().join("o"), MAX_ZIP_BYTES).is_err());
    }

    #[test]
    fn symlink_overwrite_preserves_git_and_node_modules() {
        let t = tempfile::tempdir().unwrap();
        let real = t.path().join("repo/skill");
        write(&real.join("old.txt"), "old");
        write(&real.join(".git/HEAD"), "ref");
        write(&real.join("node_modules/x"), "x");
        let skills = t.path().join("skills");
        fs::create_dir_all(&skills).unwrap();
        let link = skills.join("link");
        if !try_dir_symlink(&real, &link) {
            eprintln!("skipped: cannot create symlinks");
            return;
        }
        let prepared = skills.join(".link.memdock-new-1");
        write(&prepared.join("new.txt"), "new");
        atomic_replace_dir(&prepared, &link).unwrap();
        assert_eq!(fs::read_to_string(real.join("new.txt")).unwrap(), "new");
        assert!(!real.join("old.txt").exists());
        assert_eq!(fs::read_to_string(real.join(".git/HEAD")).unwrap(), "ref");
        assert!(real.join("node_modules/x").exists());
    }

    #[test]
    fn symlink_to_ancestor_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let skills = t.path().join("skills");
        write(&skills.join("other/SKILL.md"), "keep");
        let link = skills.join("loop");
        if !try_dir_symlink(&skills, &link) {
            eprintln!("skipped: cannot create symlinks");
            return;
        }
        let prepared = skills.join(".loop.memdock-new-1");
        write(&prepared.join("new.txt"), "new");
        assert!(atomic_replace_dir(&prepared, &link).is_err());
        assert_eq!(
            fs::read_to_string(skills.join("other/SKILL.md")).unwrap(),
            "keep"
        );
        assert!(prepared.exists());
    }

    #[test]
    fn backup_never_prunes_the_new_one() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        write(&src.join("SKILL.md"), "x");
        let root = t.path().join("bk");
        let future = now_millis() + 10_000_000_000;
        for i in 0..MAX_BACKUPS as i64 {
            write(
                &root
                    .join(bucket(t.path(), "demo"))
                    .join((future + i).to_string())
                    .join("f"),
                "f",
            );
        }
        let info = backup_dir(&src, &root, &meta(t.path(), "demo")).unwrap();
        let b = bucket(t.path(), "demo");
        assert!(root.join(&b).join(&info.id).join("SKILL.md").exists());
        assert_eq!(list_backups(&root, &b).len(), MAX_BACKUPS);
    }

    #[test]
    fn cleanup_leftovers_removes_only_old_memdock_temp() {
        let t = tempfile::tempdir().unwrap();
        let old = t.path().join(".x.memdock-old-1-2");
        let fresh = t.path().join(".x.memdock-new-1-2");
        let keep = t.path().join("x");
        let dotfile = t.path().join(".hidden");
        write(&old, "o");
        write(&fresh.join("f"), "f");
        write(&keep.join("f"), "k");
        write(&dotfile, "h");
        let f = fs::File::options().write(true).open(&old).unwrap();
        f.set_modified(SystemTime::now() - std::time::Duration::from_secs(7200))
            .unwrap();
        drop(f);
        cleanup_leftovers(t.path());
        assert!(!old.exists());
        assert!(fresh.exists() && keep.exists() && dotfile.exists());
    }

    #[test]
    fn bucket_is_sanitized_stable_and_per_directory() {
        let a = backup_bucket(Path::new("/h/.claude/skills/My Skill"));
        assert!(a.starts_with("my-skill-") && a.len() == "my-skill-".len() + 8, "{a}");
        assert!(valid_bucket(&a));
        // Different place, same name: different bucket.
        assert_ne!(a, backup_bucket(Path::new("/h/.codex/skills/My Skill")));
        // Same place spelled differently: same bucket.
        assert_eq!(a, backup_bucket(Path::new("/h/.claude/skills/My Skill/")));
        let long = backup_bucket(Path::new(&format!("/s/{}", "A".repeat(100))));
        assert_eq!(long.len(), 64 + 1 + 8);
        let odd = backup_bucket(Path::new("/s/\u{4e2d}\u{6587}.v1"));
        assert!(valid_bucket(&odd) && odd.starts_with("--.v1-"), "{odd}");
        assert!(!valid_bucket("..") && !valid_bucket("a/b") && !valid_bucket(""));
    }

    #[test]
    fn backup_writes_meta_and_it_is_not_restored() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("skills").join("demo");
        write(&src.join("SKILL.md"), "a");
        write(&src.join("meta.json"), "skill's own");
        write(&src.join(".git/HEAD"), "ref");
        let root = t.path().join("bk");
        let info = backup_dir(&src, &root, &meta(t.path(), "demo")).unwrap();
        let b = bucket(t.path(), "demo");
        let m = read_meta(&root, &b, &info.id).unwrap();
        assert_eq!(m.name, "demo");
        assert_eq!(m.dir, src.to_string_lossy());
        assert!(m.links.is_empty());
        // The skill's own meta.json is untouched; ours has a reserved name.
        let bdir = root.join(&b).join(&info.id);
        assert_eq!(fs::read_to_string(bdir.join("meta.json")).unwrap(), "skill's own");
        assert!(bdir.join(META_FILE).is_file());
        // Restoring: no meta file; .git only with keep_all.
        let a = t.path().join("a");
        restore_dir(&bdir, &a, Some(&m), false).unwrap();
        assert!(a.join("SKILL.md").is_file() && a.join("meta.json").is_file());
        assert!(!a.join(META_FILE).exists() && !a.join(".git").exists());
        let b2 = t.path().join("b");
        restore_dir(&bdir, &b2, Some(&m), true).unwrap();
        assert!(b2.join(".git/HEAD").is_file() && !b2.join(META_FILE).exists());
        // Wrong or hostile bucket / id never reads anything.
        assert!(read_meta(&root, "../x", &info.id).is_none());
        assert!(read_meta(&root, &b, "../1").is_none());
    }

    #[test]
    fn backup_never_follows_links_even_to_an_ancestor() {
        let t = tempfile::tempdir().unwrap();
        let outside = t.path().join("outside");
        write(&outside.join("secret.txt"), "do not copy");
        let src = t.path().join("skills").join("demo");
        write(&src.join("SKILL.md"), "a");
        write(&src.join("sub/f.txt"), "f");
        if !try_dir_symlink(t.path(), &src.join("loop")) {
            eprintln!("skipped: cannot create symlinks");
            return;
        }
        // Also a link to a sibling directory with content.
        assert!(try_dir_symlink(&outside, &src.join("sub/out")));
        let root = t.path().join("bk");
        let info = backup_dir(&src, &root, &meta(t.path(), "demo")).unwrap();
        let b = root.join(bucket(t.path(), "demo")).join(&info.id);
        assert!(b.join("SKILL.md").is_file() && b.join("sub/f.txt").is_file());
        // Links are kept as links or recorded in the meta; their content is not copied.
        let m = read_meta(&root, &bucket(t.path(), "demo"), &info.id).unwrap();
        for (rel, target) in [("loop", t.path()), ("sub/out", outside.as_path())] {
            let p = b.join(rel);
            let kept_as_link = fs::symlink_metadata(&p).is_ok_and(|m| m.is_symlink());
            let recorded = m.links.iter().any(|l| l.path == rel && Path::new(&l.target) == target);
            assert!(kept_as_link || recorded, "{rel}: {m:?}");
            if !kept_as_link {
                assert!(!p.exists());
            }
        }
        assert!(!b.join("loop/outside").exists() || fs::symlink_metadata(b.join("loop")).unwrap().is_symlink());
        assert!(dir_size(&b) < 1000, "no followed content");
        // Restoring does not recurse either.
        let r = t.path().join("restored");
        restore_dir(&b, &r, Some(&m), true).unwrap();
        assert!(r.join("SKILL.md").is_file());
    }

    #[test]
    fn carry_live_dirs_moves_both_and_puts_them_back() {
        let t = tempfile::tempdir().unwrap();
        let live = t.path().join("live");
        write(&live.join("SKILL.md"), "s");
        write(&live.join(".git/HEAD"), "ref");
        write(&live.join("node_modules/m.js"), "m");
        let new = t.path().join("new");
        fs::create_dir_all(&new).unwrap();
        let moved = carry_live_dirs(&live, &new).unwrap();
        assert_eq!(moved, vec![".git", "node_modules"]);
        assert!(new.join(".git/HEAD").is_file() && new.join("node_modules/m.js").is_file());
        assert!(!live.join(".git").exists() && live.join("SKILL.md").is_file());
        uncarry_live_dirs(&live, &new, &moved);
        assert!(live.join(".git/HEAD").is_file() && live.join("node_modules/m.js").is_file());
        // A failure halfway returns what was already moved.
        write(&new.join("node_modules/block"), "x");
        assert!(carry_live_dirs(&live, &new).is_err());
        assert!(live.join(".git/HEAD").is_file() && live.join("node_modules/m.js").is_file());
        // Nothing to carry: nothing moved.
        let bare = t.path().join("bare");
        write(&bare.join("SKILL.md"), "s");
        assert!(carry_live_dirs(&bare, &new).unwrap().is_empty());
    }

    #[test]
    fn list_deleted_only_lists_missing_dirs_with_meta() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("bk");
        let mk = |name: &str| {
            let d = t.path().join("skills").join(name);
            write(&d.join("SKILL.md"), name);
            d
        };
        let gone = mk("gone");
        let alive = mk("alive");
        backup_dir(&gone, &root, &meta(t.path(), "gone")).unwrap();
        backup_dir(&alive, &root, &meta(t.path(), "alive")).unwrap();
        // A bucket without meta (not ours) and a stray file are ignored.
        write(&root.join("legacy-12345678/1/SKILL.md"), "x");
        write(&root.join("stray.txt"), "x");
        fs::remove_dir_all(&gone).unwrap();
        let d = list_deleted(&root);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].name, "gone");
        assert_eq!(d[0].bucket, bucket(t.path(), "gone"));
        assert_eq!(d[0].backups.len(), 1);
        assert!(list_deleted(&t.path().join("none")).is_empty());
    }
}
