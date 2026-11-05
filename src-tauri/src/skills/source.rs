use std::fs;
use std::path::Path;

use crate::projects::normalize_path;
use crate::skills::git::GitRunner;
use crate::skills::model::{SkillEntry, SkillSource, SourceOrigin};
use crate::skills::registry::Registry;

/// Normalizes a git URL: `git@host:owner/repo(.git)` -> `https://host/owner/repo`,
/// and strips a trailing `.git` and `/`.
pub fn normalize_url(url: &str) -> String {
    let mut u = url.trim().to_string();
    if let Some(rest) = u.strip_prefix("git@") {
        if let Some((host, path)) = rest.split_once(':') {
            u = format!("https://{host}/{path}");
        }
    }
    while u.ends_with('/') {
        u.pop();
    }
    if let Some(s) = u.strip_suffix(".git") {
        u = s.to_string();
    }
    while u.ends_with('/') {
        u.pop();
    }
    u
}

/// Reads `home/.agents/.skill-lock.json` (v3). Missing or malformed -> empty.
pub fn read_skill_lock(home: &Path) -> Vec<(String, SkillSource)> {
    let Ok(text) = fs::read_to_string(home.join(".agents").join(".skill-lock.json")) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(skills) = v.get("skills").and_then(|s| s.as_object()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (name, item) in skills {
        let Some(url) = item.get("sourceUrl").and_then(|u| u.as_str()) else {
            continue;
        };
        let raw = item
            .get("skillPath")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .replace('\\', "/");
        let raw = raw.trim_matches('/');
        let skill_path = match raw.rsplit_once('/') {
            Some((parent, last)) if last.eq_ignore_ascii_case("SKILL.md") => parent.to_string(),
            None if raw.eq_ignore_ascii_case("SKILL.md") => String::new(),
            _ => raw.to_string(),
        };
        out.push((
            name.clone(),
            SkillSource {
                url: normalize_url(url),
                git_ref: None,
                skill_path,
                origin: SourceOrigin::SkillLock,
            },
        ));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Path of `dir` relative to `root`, `/`-separated; `""` when equal.
fn rel_path(root: &Path, dir: &Path) -> Option<String> {
    if let Ok(r) = dir.strip_prefix(root) {
        return Some(
            r.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
        );
    }
    // Fall back to a normalized (slash / case-insensitive on Windows) comparison.
    let r = normalize_path(&root.to_string_lossy());
    let d = normalize_path(&dir.to_string_lossy());
    if d == r {
        return Some(String::new());
    }
    let prefix = format!("{}/", r.trim_end_matches('/'));
    if d.starts_with(&prefix) {
        let orig = dir.to_string_lossy().replace('\\', "/");
        return Some(orig.get(prefix.len()..)?.to_string());
    }
    None
}

fn from_repo(dir: &Path, origin: SourceOrigin, git: &dyn GitRunner) -> Option<SkillSource> {
    let url = git.remote_url(dir)?;
    let root = git.repo_root(dir)?;
    Some(SkillSource {
        url: normalize_url(&url),
        git_ref: git.head_commit(dir),
        skill_path: rel_path(&root, dir)?,
        origin,
    })
}

/// Works out where a skill came from. Priority: registry -> own `.git` ->
/// symlink target inside a repo -> skill-lock entry of the same name.
pub fn infer_source(
    entry: &SkillEntry,
    registry: &Registry,
    lock: &[(String, SkillSource)],
    git: &dyn GitRunner,
) -> Option<SkillSource> {
    let key = normalize_path(&entry.dir);
    if let Some(r) = registry
        .skills
        .iter()
        .find(|r| r.installs.iter().any(|i| normalize_path(&i.dir) == key))
    {
        let mut s = r.source.clone();
        s.origin = SourceOrigin::Registry;
        return Some(s);
    }
    let dir = Path::new(&entry.dir);
    if dir.join(".git").exists() {
        if let Some(s) = from_repo(dir, SourceOrigin::GitDir, git) {
            return Some(s);
        }
    }
    if let Some(target) = &entry.symlink_target {
        if let Some(s) = from_repo(Path::new(target), SourceOrigin::SymlinkRepo, git) {
            return Some(s);
        }
    }
    lock.iter()
        .find(|(n, _)| *n == entry.name)
        .map(|(_, s)| s.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Tool;
    use crate::skills::git::{GitError, GitRunner};
    use crate::skills::model::{SkillKind, SkillScope};
    use crate::skills::registry::{InstallRecord, RegistryEntry};
    use std::path::PathBuf;

    struct FakeGit {
        remote: Option<String>,
        root: Option<PathBuf>,
        head: Option<String>,
    }
    impl FakeGit {
        fn none() -> Self {
            FakeGit {
                remote: None,
                root: None,
                head: None,
            }
        }
    }
    impl GitRunner for FakeGit {
        fn available(&self) -> bool {
            true
        }
        fn ls_remote_head(&self, _: &str, _: Option<&str>) -> Result<String, GitError> {
            Err(GitError::NotInstalled)
        }
        fn shallow_clone(&self, _: &str, _: Option<&str>, _: &Path) -> Result<String, GitError> {
            Err(GitError::NotInstalled)
        }
        fn remote_url(&self, _: &Path) -> Option<String> {
            self.remote.clone()
        }
        fn head_commit(&self, _: &Path) -> Option<String> {
            self.head.clone()
        }
        fn repo_root(&self, _: &Path) -> Option<PathBuf> {
            self.root.clone()
        }
    }

    fn entry(name: &str, dir: &Path) -> SkillEntry {
        SkillEntry {
            id: normalize_path(&dir.to_string_lossy()),
            name: name.into(),
            description: String::new(),
            tool: Tool::Claude,
            scope: SkillScope::Global,
            kind: SkillKind::User,
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

    fn lock_entry(name: &str) -> (String, SkillSource) {
        (
            name.into(),
            SkillSource {
                url: "https://github.com/l/l".into(),
                git_ref: None,
                skill_path: "skills/x".into(),
                origin: SourceOrigin::SkillLock,
            },
        )
    }

    #[test]
    fn normalizes_urls() {
        assert_eq!(
            normalize_url("https://github.com/a/b.git"),
            "https://github.com/a/b"
        );
        assert_eq!(
            normalize_url("https://github.com/a/b/"),
            "https://github.com/a/b"
        );
        assert_eq!(
            normalize_url("git@github.com:a/b.git"),
            "https://github.com/a/b"
        );
        assert_eq!(
            normalize_url("git@github.com:a/b"),
            "https://github.com/a/b"
        );
    }

    #[test]
    fn reads_skill_lock_v3() {
        let tmp = tempfile::tempdir().unwrap();
        let agents = tmp.path().join(".agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(
            agents.join(".skill-lock.json"),
            r#"{"version":3,"skills":{"huashu-motion":{"source":"alchaincyf/huashu-motion","sourceType":"github","sourceUrl":"https://github.com/alchaincyf/huashu-motion.git","skillPath":"skills/huashu-motion/SKILL.md","skillFolderHash":"fac"},"root-skill":{"sourceUrl":"https://github.com/o/r","skillPath":"SKILL.md"}},"dismissed":{}}"#,
        )
        .unwrap();
        let lock = read_skill_lock(tmp.path());
        assert_eq!(lock.len(), 2);
        let (n, s) = lock.iter().find(|(n, _)| n == "huashu-motion").unwrap();
        assert_eq!(n, "huashu-motion");
        assert_eq!(s.url, "https://github.com/alchaincyf/huashu-motion");
        assert_eq!(s.skill_path, "skills/huashu-motion");
        assert_eq!(s.origin, SourceOrigin::SkillLock);
        assert_eq!(
            lock.iter()
                .find(|(n, _)| n == "root-skill")
                .unwrap()
                .1
                .skill_path,
            ""
        );
        assert!(read_skill_lock(&tmp.path().join("nope")).is_empty());
    }

    #[test]
    fn infer_prefers_registry() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join(".git")).unwrap();
        let e = entry("demo", tmp.path());
        let reg = Registry {
            version: 1,
            skills: vec![RegistryEntry {
                name: "demo".into(),
                source: SkillSource {
                    url: "https://github.com/reg/reg".into(),
                    git_ref: Some("r1".into()),
                    skill_path: "p".into(),
                    origin: SourceOrigin::Manual,
                },
                installs: vec![InstallRecord {
                    tool: Tool::Claude,
                    scope: SkillScope::Global,
                    dir: e.dir.replace('\\', "/"),
                }],
                content_hash: String::new(),
                installed_at: 0,
                updated_at: 0,
            }],
        };
        let git = FakeGit {
            remote: Some("https://github.com/git/git".into()),
            root: Some(tmp.path().to_path_buf()),
            head: None,
        };
        let s = infer_source(&e, &reg, &[lock_entry("demo")], &git).unwrap();
        assert_eq!(s.url, "https://github.com/reg/reg");
        assert_eq!(s.git_ref.as_deref(), Some("r1"));
        assert_eq!(s.origin, SourceOrigin::Registry);
    }

    #[test]
    fn infer_from_git_dir_with_subpath() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("skills").join("demo");
        fs::create_dir_all(dir.join(".git")).unwrap();
        let e = entry("demo", &dir);
        let git = FakeGit {
            remote: Some("git@github.com:o/r.git".into()),
            root: Some(tmp.path().to_path_buf()),
            head: Some("c0ffee".into()),
        };
        let s = infer_source(&e, &Registry::default(), &[lock_entry("demo")], &git).unwrap();
        assert_eq!(s.url, "https://github.com/o/r");
        assert_eq!(s.skill_path, "skills/demo");
        assert_eq!(s.git_ref.as_deref(), Some("c0ffee"));
        assert_eq!(s.origin, SourceOrigin::GitDir);
        // Skill at the repo root -> empty path.
        let root = entry("demo", tmp.path());
        fs::create_dir_all(tmp.path().join(".git")).unwrap();
        let s = infer_source(&root, &Registry::default(), &[], &git).unwrap();
        assert_eq!(s.skill_path, "");
    }

    #[test]
    fn infer_from_symlink_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let target = repo.join("skills").join("demo");
        fs::create_dir_all(&target).unwrap();
        let link = tmp.path().join("link");
        let mut e = entry("demo", &link);
        e.is_symlink = true;
        e.symlink_target = Some(target.to_string_lossy().into_owned());
        let git = FakeGit {
            remote: Some("https://github.com/o/r.git".into()),
            root: Some(repo),
            head: Some("abc".into()),
        };
        let s = infer_source(&e, &Registry::default(), &[], &git).unwrap();
        assert_eq!(s.url, "https://github.com/o/r");
        assert_eq!(s.skill_path, "skills/demo");
        assert_eq!(s.origin, SourceOrigin::SymlinkRepo);
    }

    #[test]
    fn infer_from_skill_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let e = entry("demo", tmp.path());
        let s = infer_source(
            &e,
            &Registry::default(),
            &[lock_entry("demo")],
            &FakeGit::none(),
        )
        .unwrap();
        assert_eq!(s.origin, SourceOrigin::SkillLock);
        assert_eq!(s.url, "https://github.com/l/l");
    }

    #[test]
    fn infer_none_for_local() {
        let tmp = tempfile::tempdir().unwrap();
        let e = entry("demo", tmp.path());
        assert!(
            infer_source(
                &e,
                &Registry::default(),
                &[lock_entry("other")],
                &FakeGit::none()
            )
            .is_none()
        );
    }
}
