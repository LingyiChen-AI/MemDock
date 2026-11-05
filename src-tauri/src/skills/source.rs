use std::fs;
use std::path::Path;

use crate::projects::normalize_path;
use crate::skills::git::GitRunner;
use crate::skills::model::{SkillEntry, SkillSource, SourceOrigin};
use crate::skills::registry::Registry;

/// Splits a git URL into `(lowercase host without www., repo path without .git)`.
/// Handles http(s)/ssh/git schemes (user info and ssh port dropped) and scp-style
/// `user@host:owner/repo`. Returns `None` when there is no host or no path.
pub fn parse_url(url: &str) -> Option<(String, String)> {
    let u = url.trim();
    let (host_raw, path_raw) = if let Some((scheme, rest)) = u.split_once("://") {
        let (auth, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = auth.rsplit_once('@').map_or(auth, |(_, h)| h);
        let host = if scheme.eq_ignore_ascii_case("ssh") {
            host.split(':').next().unwrap_or("")
        } else {
            host
        };
        (host, path)
    } else {
        let (auth, path) = u.split_once(':')?;
        (auth.rsplit_once('@').map_or(auth, |(_, h)| h), path)
    };
    let host = host_raw.trim().to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let mut path = path_raw.trim().trim_matches('/');
    if let Some(p) = path.strip_suffix(".git") {
        path = p.trim_end_matches('/');
    }
    if host.is_empty() || path.is_empty() || host.contains(char::is_whitespace) {
        return None;
    }
    Some((host, path.to_string()))
}

/// Normalizes a git URL to `https://<host>/<path>` (no `.git`, no trailing `/`).
pub fn normalize_url(url: &str) -> String {
    match parse_url(url) {
        Some((host, path)) => format!("https://{host}/{path}"),
        None => url.trim().trim_end_matches('/').to_string(),
    }
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
                commit: None,
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
        git_ref: None,
        commit: git.head_commit(dir),
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
    // skill-lock keys are directory names under `~/.agents/skills/`; only skills
    // living there (or symlinked to it) are attributed, never same-named local ones.
    let key = [Some(entry.dir.as_str()), entry.symlink_target.as_deref()]
        .into_iter()
        .flatten()
        .find_map(agents_skill_key)?;
    lock.iter().find(|(n, _)| *n == key).map(|(_, s)| s.clone())
}

/// `<anything>/.agents/skills/<key>` -> `<key>`.
fn agents_skill_key(path: &str) -> Option<String> {
    let p = path.replace('\\', "/");
    let parts: Vec<&str> = p.trim_end_matches('/').split('/').collect();
    let n = parts.len();
    if n >= 3 && parts[n - 2] == "skills" && parts[n - 3] == ".agents" && !parts[n - 1].is_empty() {
        Some(parts[n - 1].to_string())
    } else {
        None
    }
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
        fn pull_ff_only(&self, _: &Path) -> Result<String, GitError> {
            Err(GitError::NotInstalled)
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
                commit: None,
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
        for u in [
            "ssh://git@github.com/a/b.git",
            "ssh://git@github.com:22/a/b",
            "http://github.com/a/b",
            "https://user:tok@github.com/a/b.git/",
            "https://www.GitHub.com/a/b",
            "git@GitHub.com:a/b.git",
        ] {
            assert_eq!(normalize_url(u), "https://github.com/a/b", "{u}");
        }
        assert!(parse_url("https://").is_none());
        assert!(parse_url("git@x").is_none());
        assert!(parse_url("https://github.com/").is_none());
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
                    commit: None,
                    skill_path: "p".into(),
                    origin: SourceOrigin::Manual,
                },
                clone_url: None,
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
        assert_eq!(s.commit.as_deref(), Some("c0ffee"));
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
    fn infer_from_skill_lock_by_dir_name() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".agents").join("skills").join("demo-dir");
        fs::create_dir_all(&dir).unwrap();
        // Frontmatter name differs from the directory name (the lock key).
        let e = entry("Fancy Name", &dir);
        let s = infer_source(
            &e,
            &Registry::default(),
            &[lock_entry("demo-dir")],
            &FakeGit::none(),
        )
        .unwrap();
        assert_eq!(s.origin, SourceOrigin::SkillLock);
        assert_eq!(s.url, "https://github.com/l/l");
        // A symlink pointing into ~/.agents/skills/<key> is attributed too.
        let link = tmp.path().join(".claude").join("skills").join("alias");
        let mut l = entry("alias", &link);
        l.is_symlink = true;
        l.symlink_target = Some(dir.to_string_lossy().into_owned());
        assert!(
            infer_source(
                &l,
                &Registry::default(),
                &[lock_entry("demo-dir")],
                &FakeGit::none()
            )
            .is_some()
        );
    }

    #[test]
    fn infer_skill_lock_ignores_same_named_local_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".claude").join("skills").join("demo");
        fs::create_dir_all(&dir).unwrap();
        let e = entry("demo", &dir);
        assert!(
            infer_source(
                &e,
                &Registry::default(),
                &[lock_entry("demo")],
                &FakeGit::none()
            )
            .is_none()
        );
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
