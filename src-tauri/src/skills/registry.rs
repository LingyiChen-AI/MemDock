use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::model::Tool;
use crate::projects::normalize_path;
use crate::skills::model::{SkillEntry, SkillScope, SkillSource, SourceOrigin};
use crate::skills::source::{normalize_url, parse_url};

/// One place a registered skill is installed.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct InstallRecord {
    pub tool: Tool,
    pub scope: SkillScope,
    pub dir: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct RegistryEntry {
    pub name: String,
    pub source: SkillSource,
    /// Address used for network git calls; `source.url` stays the identity.
    #[serde(default)]
    pub clone_url: Option<String>,
    pub installs: Vec<InstallRecord>,
    /// Hash of the directory contents right after install / update.
    pub content_hash: String,
    pub installed_at: i64,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Registry {
    pub version: u32,
    pub skills: Vec<RegistryEntry>,
}

impl Default for Registry {
    fn default() -> Self {
        Registry {
            version: 1,
            skills: Vec::new(),
        }
    }
}

/// Loads the registry. A missing file -> empty. Any other read or parse failure
/// -> the file is copied to `<path>.bak` (timestamped if one exists) and an
/// empty registry is returned; if that backup fails an error is returned so the
/// caller never overwrites data it could not preserve.
pub fn load_registry(path: &Path) -> Result<Registry, String> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Registry::default()),
        Err(_) => {
            backup_corrupt(path)?;
            return Ok(Registry::default());
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(r) => Ok(r),
        Err(_) => {
            backup_corrupt(path)?;
            Ok(Registry::default())
        }
    }
}

fn backup_corrupt(path: &Path) -> Result<(), String> {
    let base = path
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("skills-registry.json"));
    let mut bak = base.clone();
    bak.push(".bak");
    let mut target = path.with_file_name(bak);
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
        .map_err(|e| format!("cannot back up unreadable registry {}: {e}", path.display()))
}

/// Saves atomically: unique temp file in the same directory, fsync, rename.
pub fn save_registry(path: &Path, r: &Registry) -> io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let json = serde_json::to_string_pretty(r)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut tmp_name = path
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("skills-registry.json"));
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

/// Deterministic content hash: files sorted by `/`-separated relative path,
/// each contributing sha256(path + NUL + bytes); symlinks contribute
/// `symlink:<target>` instead. `.git/`, `node_modules/`, `.DS_Store` and `.memdock-complete` are skipped.
pub fn dir_content_hash(dir: &Path) -> io::Result<String> {
    let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
    let walker = WalkDir::new(dir).into_iter().filter_entry(|e| {
        let n = e.file_name();
        !(n == ".git" || n == ".DS_Store" || n == "node_modules" || n == ".memdock-complete")
    });
    for item in walker {
        let item = item.map_err(io::Error::other)?;
        let ft = item.file_type();
        if !ft.is_file() && !ft.is_symlink() {
            continue;
        }
        let rel = item
            .path()
            .strip_prefix(dir)
            .map_err(io::Error::other)?
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        files.push((rel, item.path().to_path_buf()));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut total = Sha256::new();
    for (rel, path) in files {
        let mut h = Sha256::new();
        h.update(rel.as_bytes());
        h.update([0u8]);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            let target = fs::read_link(&path)?;
            h.update(b"symlink:");
            h.update(target.to_string_lossy().replace('\\', "/").as_bytes());
        } else {
            h.update(fs::read(&path)?);
        }
        total.update(h.finalize());
    }
    Ok(total
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Records a user-supplied source for a skill that could not be inferred.
pub fn register_manual(
    registry: &mut Registry,
    entry: &SkillEntry,
    url: &str,
    skill_path: &str,
) -> Result<(), String> {
    let trimmed = url.trim();
    let scheme_ok = ["https://", "http://", "ssh://", "git@"]
        .iter()
        .any(|p| trimmed.starts_with(p));
    if !scheme_ok || parse_url(trimmed).is_none() {
        return Err(format!("unsupported url: {url}"));
    }
    let url = normalize_url(trimmed);
    let skill_path = skill_path.trim().trim_matches('/').to_string();
    let hash = dir_content_hash(Path::new(&entry.dir)).map_err(|e| e.to_string())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let install = InstallRecord {
        tool: entry.tool,
        scope: entry.scope.clone(),
        dir: entry.dir.clone(),
    };
    let source = SkillSource {
        url,
        git_ref: None,
        commit: None,
        skill_path,
        origin: SourceOrigin::Manual,
    };
    let key = normalize_path(&entry.dir);
    // Drop this directory from any other record so it has exactly one source.
    for e in registry.skills.iter_mut() {
        e.installs.retain(|i| normalize_path(&i.dir) != key);
    }
    registry.skills.retain(|e| !e.installs.is_empty());
    if let Some(e) = registry.skills.iter_mut().find(|e| {
        e.name == entry.name
            && e.source.url == source.url
            && e.source.skill_path == source.skill_path
    }) {
        e.installs.push(install);
        e.content_hash = hash;
        e.updated_at = now;
    } else {
        registry.skills.push(RegistryEntry {
            name: entry.name.clone(),
            source,
            clone_url: None,
            installs: vec![install],
            content_hash: hash,
            installed_at: now,
            updated_at: now,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(dir: &Path) -> SkillEntry {
        SkillEntry {
            id: normalize_path(&dir.to_string_lossy()),
            name: "demo".into(),
            description: String::new(),
            tool: Tool::Claude,
            scope: SkillScope::Global,
            kind: crate::skills::model::SkillKind::User,
            dir: dir.to_string_lossy().into_owned(),
            is_symlink: false,
            symlink_target: None,
            readonly: false,
            plugin: None,
            source: None,
            file_count: 1,
            size_bytes: 1,
            modified_at: None,
        }
    }

    fn sample() -> Registry {
        Registry {
            version: 1,
            skills: vec![RegistryEntry {
                name: "demo".into(),
                source: SkillSource {
                    url: "https://github.com/a/b".into(),
                    git_ref: Some("main".into()),
                    commit: Some("abc".into()),
                    skill_path: "skills/demo".into(),
                    origin: SourceOrigin::Registry,
                },
                clone_url: None,
                installs: vec![InstallRecord {
                    tool: Tool::Codex,
                    scope: SkillScope::Project {
                        project_id: "p".into(),
                    },
                    dir: "/x/demo".into(),
                }],
                content_hash: "h".into(),
                installed_at: 1,
                updated_at: 2,
            }],
        }
    }

    #[test]
    fn registry_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sub").join("reg.json");
        assert_eq!(load_registry(&path).unwrap(), Registry::default());
        save_registry(&path, &sample()).unwrap();
        assert_eq!(load_registry(&path).unwrap(), sample());
        assert!(!tmp.path().join("sub").join("reg.json.tmp").exists());
    }

    #[test]
    fn corrupt_registry_backed_up() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("reg.json");
        fs::write(&path, "{ oops").unwrap();
        assert_eq!(load_registry(&path).unwrap(), Registry::default());
        assert_eq!(
            fs::read_to_string(tmp.path().join("reg.json.bak")).unwrap(),
            "{ oops"
        );
        fs::write(&path, "garbage2").unwrap();
        load_registry(&path).unwrap();
        assert!(
            fs::read_to_string(tmp.path().join("reg.json.bak"))
                .unwrap()
                .contains("oops")
        );
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 3);
    }

    #[test]
    fn non_utf8_registry_is_backed_up() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("reg.json");
        fs::write(&path, [0xffu8, 0xfe, 0x00, 0x80]).unwrap();
        assert_eq!(load_registry(&path).unwrap(), Registry::default());
        assert_eq!(
            fs::read(tmp.path().join("reg.json.bak")).unwrap(),
            vec![0xffu8, 0xfe, 0x00, 0x80]
        );
    }

    #[test]
    fn unreadable_registry_without_backup_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        // A directory in place of the file: read fails (not NotFound) and cannot be copied.
        let path = tmp.path().join("reg.json");
        fs::create_dir(&path).unwrap();
        assert!(load_registry(&path).is_err());
    }

    #[test]
    fn content_hash_covers_symlinks_and_skips_node_modules() {
        let a = tempfile::tempdir().unwrap();
        fs::write(a.path().join("SKILL.md"), "x").unwrap();
        let base = dir_content_hash(a.path()).unwrap();
        fs::create_dir(a.path().join("node_modules")).unwrap();
        fs::write(a.path().join("node_modules").join("m.js"), "m").unwrap();
        assert_eq!(base, dir_content_hash(a.path()).unwrap());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("t1", a.path().join("l")).unwrap();
            let h1 = dir_content_hash(a.path()).unwrap();
            assert_ne!(base, h1);
            fs::remove_file(a.path().join("l")).unwrap();
            std::os::unix::fs::symlink("t2", a.path().join("l")).unwrap();
            assert_ne!(h1, dir_content_hash(a.path()).unwrap());
        }
    }

    #[test]
    fn content_hash_ignores_git_dir_and_is_order_independent() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        fs::write(a.path().join("SKILL.md"), "x").unwrap();
        fs::create_dir(a.path().join("sub")).unwrap();
        fs::write(a.path().join("sub").join("f.txt"), "y").unwrap();
        // Created in the opposite order, plus noise that must be ignored.
        fs::create_dir(b.path().join("sub")).unwrap();
        fs::write(b.path().join("sub").join("f.txt"), "y").unwrap();
        fs::write(b.path().join("SKILL.md"), "x").unwrap();
        fs::create_dir(b.path().join(".git")).unwrap();
        fs::write(b.path().join(".git").join("HEAD"), "ref").unwrap();
        fs::write(b.path().join(".DS_Store"), "junk").unwrap();
        fs::write(b.path().join(".memdock-complete"), "abc").unwrap();
        let ha = dir_content_hash(a.path()).unwrap();
        assert_eq!(ha, dir_content_hash(b.path()).unwrap());
        fs::write(b.path().join("sub").join("f.txt"), "changed").unwrap();
        assert_ne!(ha, dir_content_hash(b.path()).unwrap());
    }

    #[test]
    fn register_manual_rejects_bad_url() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("SKILL.md"), "x").unwrap();
        let e = entry(tmp.path());
        let mut reg = Registry::default();
        assert!(register_manual(&mut reg, &e, "ftp://github.com/a/b", "").is_err());
        assert!(register_manual(&mut reg, &e, "not a url", "").is_err());
        assert!(register_manual(&mut reg, &e, "https://", "").is_err());
        assert!(register_manual(&mut reg, &e, "git@x", "").is_err());
        assert!(register_manual(&mut reg, &e, "https://github.com/", "").is_err());
        assert!(reg.skills.is_empty());
        register_manual(&mut reg, &e, "git@github.com:a/b.git", "/skills/demo/").unwrap();
        let s = &reg.skills[0];
        assert_eq!(s.source.url, "https://github.com/a/b");
        assert_eq!(s.source.skill_path, "skills/demo");
        assert_eq!(s.source.origin, SourceOrigin::Manual);
        assert_eq!(s.content_hash, dir_content_hash(tmp.path()).unwrap());
        // Registering the same dir again does not duplicate installs.
        register_manual(&mut reg, &e, "https://github.com/a/b/", "skills/demo").unwrap();
        assert_eq!(reg.skills.len(), 1);
        assert_eq!(reg.skills[0].installs.len(), 1);
    }
}
