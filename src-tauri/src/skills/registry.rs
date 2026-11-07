use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::fsutil::{load_json_or_backup, save_json_atomic, Loaded};
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
            version: REGISTRY_VERSION,
            skills: Vec::new(),
        }
    }
}

/// Highest registry format this build understands.
pub const REGISTRY_VERSION: u32 = 1;

/// Loads the registry. A missing file -> empty. Any other read or parse failure
/// -> the file is copied to `<path>.bak` (timestamped if one exists) and an
/// empty registry is returned; if that backup fails an error is returned so the
/// caller never overwrites data it could not preserve. A registry written by a
/// newer version is backed up and refused (error), never read as the current
/// format and never overwritten.
pub fn load_registry(path: &Path) -> Result<Registry, String> {
    match load_json_or_backup::<Registry>(path, REGISTRY_VERSION, "skills registry")? {
        Loaded::Ok(r) => Ok(r),
        Loaded::Missing | Loaded::Recovered => Ok(Registry::default()),
    }
}

/// Saves atomically: unique temp file in the same directory, fsync, rename.
pub fn save_registry(path: &Path, r: &Registry) -> io::Result<()> {
    save_json_atomic(path, r)
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
    // Join an existing record only when it has the very same baseline (no
    // commit, same content): a shared record's hash and commit are never
    // rewritten, and the new install never inherits another install's commit.
    if let Some(e) = registry.skills.iter_mut().find(|e| {
        e.name == entry.name
            && e.source.url == source.url
            && e.source.skill_path == source.skill_path
            && e.source.commit.is_none()
            && e.content_hash == hash
    }) {
        e.installs.push(install);
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
            plugin_version: None,
            plugin_install_path: None,
            plugin_local_scope: false,
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

    fn manual_url() -> &'static str {
        "https://github.com/o/r"
    }

    #[test]
    fn register_manual_never_rewrites_a_shared_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        for d in [&a, &b] {
            fs::create_dir_all(d).unwrap();
            fs::write(d.join("SKILL.md"), "same").unwrap();
        }
        let hash = dir_content_hash(&a).unwrap();
        let rec = |d: &Path| InstallRecord {
            tool: Tool::Claude,
            scope: SkillScope::Global,
            dir: d.to_string_lossy().into_owned(),
        };
        let shared = RegistryEntry {
            name: "demo".into(),
            source: SkillSource {
                url: normalize_url(manual_url()),
                git_ref: None,
                commit: Some("c1".into()),
                skill_path: "skills/demo".into(),
                origin: SourceOrigin::Registry,
            },
            clone_url: Some(manual_url().into()),
            installs: vec![rec(&a), rec(&b)],
            content_hash: "baseline-of-install-a".into(),
            installed_at: 1,
            updated_at: 2,
        };
        let mut reg = Registry {
            version: 1,
            skills: vec![shared.clone()],
        };
        register_manual(&mut reg, &entry(&b), manual_url(), "skills/demo").unwrap();
        // The shared entry lost only the registered install; hash and commit stay.
        let old = reg.skills.iter().find(|e| e.source.commit.is_some()).unwrap();
        assert_eq!(old.installs, vec![rec(&a)]);
        assert_eq!(old.content_hash, "baseline-of-install-a");
        assert_eq!(old.source.commit.as_deref(), Some("c1"));
        assert_eq!(old.updated_at, 2);
        // The registered install has its own entry: no commit, current content as baseline.
        let own = reg.skills.iter().find(|e| e.source.origin == SourceOrigin::Manual).unwrap();
        assert_eq!(own.installs, vec![rec(&b)]);
        assert_eq!(own.source.commit, None);
        assert_eq!(own.content_hash, hash);
        assert_eq!(reg.skills.len(), 2);
    }

    #[test]
    fn register_manual_only_joins_an_identical_baseline() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        for d in [&a, &b] {
            fs::create_dir_all(d).unwrap();
            fs::write(d.join("SKILL.md"), "same").unwrap();
        }
        let mut reg = Registry::default();
        register_manual(&mut reg, &entry(&a), manual_url(), "p").unwrap();
        register_manual(&mut reg, &entry(&b), manual_url(), "p").unwrap();
        assert_eq!(reg.skills.len(), 1, "same content, no commit: one record");
        assert_eq!(reg.skills[0].installs.len(), 2);
        // Different content: its own record, the first one's hash is untouched.
        let c = tmp.path().join("c");
        fs::create_dir_all(&c).unwrap();
        fs::write(c.join("SKILL.md"), "edited").unwrap();
        let first_hash = reg.skills[0].content_hash.clone();
        register_manual(&mut reg, &entry(&c), manual_url(), "p").unwrap();
        assert_eq!(reg.skills.len(), 2);
        assert_eq!(reg.skills[0].content_hash, first_hash);
    }

    #[test]
    fn newer_registry_version_is_backed_up_and_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("skills-registry.json");
        let future = r#"{"version": 2, "skills": [], "extra": {"x": 1}}"#;
        fs::write(&path, future).unwrap();
        let err = load_registry(&path).unwrap_err();
        assert!(err.contains("version 2"), "{err}");
        assert_eq!(fs::read_to_string(tmp.path().join("skills-registry.json.bak")).unwrap(), future);
        assert_eq!(fs::read_to_string(&path).unwrap(), future, "never overwritten");
        // Loading again does not pile up more backups.
        assert!(load_registry(&path).is_err());
        let baks: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".bak"))
            .collect();
        assert_eq!(baks.len(), 1);
        // Version 1 and a missing version still load.
        fs::write(&path, r#"{"version": 1, "skills": []}"#).unwrap();
        assert_eq!(load_registry(&path).unwrap(), Registry::default());
    }
}
