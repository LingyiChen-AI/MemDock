//! Skill install flow: prepare content in a staging area, validate, then place
//! it into each target with backup + atomic replace.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

use crate::model::{Project, Tool};
use crate::paths::AgentPaths;
use crate::projects::normalize_path;
use crate::skills::frontmatter::parse_frontmatter;
use crate::skills::fsops::{
    MAX_ZIP_BYTES, atomic_replace_dir, backup_dir, copy_dir, copy_dir_confined, extract_zip,
    now_millis, remove_any,
};
use crate::skills::git::{GitError, GitRunner};
use crate::skills::git_cache::{CACHE_MAX_AGE, cache_dir_for, cleanup_cache, is_complete_cache};
use crate::skills::model::{SkillEntry, SkillScope, SkillSource, SourceOrigin};
use crate::skills::registry::{
    InstallRecord, RegistryEntry, dir_content_hash, load_registry, save_registry,
};
use crate::skills::scan::scan_skills;

/// Everything the operations need from the app.
pub struct OpsCtx {
    pub paths: AgentPaths,
    pub home: PathBuf,
    pub projects: Vec<Project>,
    pub appdata: PathBuf,
    pub git: Arc<dyn GitRunner>,
    pub registry_path: PathBuf,
}

impl OpsCtx {
    pub fn backups_root(&self) -> PathBuf {
        self.appdata.join("skill-backups")
    }

    pub fn cache_root(&self) -> PathBuf {
        self.appdata.join("git-cache")
    }

    fn staging_root(&self) -> PathBuf {
        self.appdata.join("skill-tmp")
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum OpsError {
    Exists {
        dir: String,
    },
    ReadOnly,
    GitUnavailable,
    Git(GitError),
    /// The skill has local edits that an update would overwrite.
    LocalChanges,
    Invalid {
        message: String,
    },
    Io {
        message: String,
    },
}

impl From<io::Error> for OpsError {
    fn from(e: io::Error) -> Self {
        OpsError::Io {
            message: e.to_string(),
        }
    }
}

fn invalid(m: impl Into<String>) -> OpsError {
    OpsError::Invalid { message: m.into() }
}

fn io_msg(m: impl Into<String>) -> OpsError {
    OpsError::Io { message: m.into() }
}

fn git_err(e: GitError) -> OpsError {
    match e {
        GitError::NotInstalled => OpsError::GitUnavailable,
        e => OpsError::Git(e),
    }
}

#[derive(Debug, Clone)]
pub enum InstallFrom {
    Git {
        /// Normalized identity URL.
        url: String,
        /// Address handed to `git`.
        clone_url: String,
        git_ref: Option<String>,
        /// `""` when the skill is the repository root.
        skill_path: String,
    },
    Folder {
        path: String,
    },
    Zip {
        path: String,
    },
    Template {
        name: String,
        description: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallTarget {
    pub tool: Tool,
    pub scope: SkillScope,
}

/// Removes a staging directory when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(root: &Path) -> io::Result<TempDir> {
        fs::create_dir_all(root)?;
        let p = root.join(format!(
            "install-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&p)?;
        Ok(TempDir(p))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `^[a-z0-9][a-z0-9-_.]{0,63}$`, plus names Windows cannot create.
pub fn validate_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let ok_first = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let ok_rest = chars
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' || c == '.');
    if !ok_first || !ok_rest || name.len() > 64 {
        return Err(format!("invalid skill name: {name:?}"));
    }
    if name.ends_with('.') {
        return Err(format!("invalid skill name (trailing dot): {name:?}"));
    }
    let stem = name.split('.').next().unwrap_or(name);
    let reserved = ["con", "prn", "aux", "nul"].iter().any(|r| stem == *r)
        || ["com", "lpt"].iter().any(|p| {
            stem.strip_prefix(p)
                .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit() && n != "0")
        });
    if reserved {
        return Err(format!(
            "invalid skill name (reserved on Windows): {name:?}"
        ));
    }
    Ok(())
}

/// Where a skill named `name` is installed for `t`.
pub fn target_dir(
    t: &InstallTarget,
    paths: &AgentPaths,
    home: &Path,
    projects: &[Project],
    name: &str,
) -> Result<PathBuf, String> {
    let _ = home;
    validate_name(name)?;
    Ok(match &t.scope {
        SkillScope::Global => match t.tool {
            Tool::Claude => paths.claude_home.join("skills").join(name),
            Tool::Codex => paths.codex_home.join("skills").join(name),
        },
        SkillScope::Project { project_id } => {
            let key = normalize_path(project_id);
            let p = projects
                .iter()
                .find(|p| p.id == *project_id || normalize_path(&p.path) == key)
                .ok_or_else(|| format!("unknown project: {project_id}"))?;
            let root = Path::new(&p.path);
            match t.tool {
                Tool::Claude => root.join(".claude").join("skills").join(name),
                Tool::Codex => root.join(".agents").join("skills").join(name),
            }
        }
    })
}

/// Skill folders Memdock must never write into.
fn is_readonly_location(t: &InstallTarget, name: &str) -> bool {
    t.tool == Tool::Claude && t.scope == SkillScope::Global && name == "synced"
}

fn last_segment(s: &str) -> String {
    s.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim_end_matches(".git")
        .to_string()
}

/// Safe relative path inside a repository.
fn repo_subpath(skill_path: &str) -> Result<PathBuf, OpsError> {
    let mut out = PathBuf::new();
    for part in skill_path.replace('\\', "/").split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(invalid("skill path must not contain '..'")),
            p if p.contains(':') => return Err(invalid("invalid skill path")),
            p => out.push(p),
        }
    }
    Ok(out)
}

struct Prepared {
    /// Directory holding the skill files.
    content: PathBuf,
    /// Directory name of the source, used when frontmatter has no valid name.
    fallback_name: String,
    /// Forced name (templates).
    forced_name: Option<String>,
    /// Source to record in the registry.
    source: Option<(SkillSource, String)>,
}

fn prepare(ctx: &OpsCtx, from: &InstallFrom, staging: &Path) -> Result<Prepared, OpsError> {
    let content = staging.join("content");
    match from {
        InstallFrom::Folder { path } => {
            let src = Path::new(path);
            if !src.is_dir() {
                return Err(invalid(format!("not a folder: {path}")));
            }
            copy_dir(src, &content)?;
            Ok(Prepared {
                content,
                fallback_name: src
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                forced_name: None,
                source: None,
            })
        }
        InstallFrom::Zip { path } => {
            let zp = Path::new(path);
            extract_zip(zp, &content, MAX_ZIP_BYTES).map_err(invalid)?;
            Ok(Prepared {
                content,
                fallback_name: zp
                    .file_stem()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                forced_name: None,
                source: None,
            })
        }
        InstallFrom::Template { name, description } => {
            validate_name(name).map_err(invalid)?;
            fs::create_dir_all(&content)?;
            let desc = description.split_whitespace().collect::<Vec<_>>().join(" ");
            let desc = serde_json::to_string(&desc).map_err(|e| io_msg(e.to_string()))?;
            let text = format!(
                "---\nname: {name}\ndescription: {desc}\n---\n\n# {name}\n\n在这里写这个 skill 的使用说明：什么时候用、怎么做、有哪些注意事项。\n"
            );
            fs::write(content.join("SKILL.md"), text)?;
            Ok(Prepared {
                content,
                fallback_name: name.clone(),
                forced_name: Some(name.clone()),
                source: None,
            })
        }
        InstallFrom::Git {
            url,
            clone_url,
            git_ref,
            skill_path,
        } => {
            let sub = repo_subpath(skill_path)?;
            if !ctx.git.available() {
                return Err(OpsError::GitUnavailable);
            }
            let git_ref = git_ref.as_deref();
            let head = ctx
                .git
                .ls_remote_head(clone_url, git_ref)
                .map_err(git_err)?;
            let cache = cache_dir_for(&ctx.cache_root(), url, &head);
            let commit = if is_complete_cache(&cache) {
                head
            } else {
                if cache.exists() {
                    remove_any(&cache)?;
                }
                ctx.git
                    .shallow_clone(clone_url, git_ref, &cache)
                    .map_err(git_err)?
            };
            let skill_src = cache.join(&sub);
            if !skill_src.is_dir() {
                return Err(invalid(format!(
                    "skill path not found in repository: {skill_path}"
                )));
            }
            copy_dir_confined(&skill_src, &content, &cache)?;
            let fallback = if sub.as_os_str().is_empty() {
                last_segment(url)
            } else {
                sub.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            };
            Ok(Prepared {
                content,
                fallback_name: fallback,
                forced_name: None,
                source: Some((
                    SkillSource {
                        url: url.clone(),
                        git_ref: Some(commit),
                        skill_path: sub
                            .components()
                            .map(|c| c.as_os_str().to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                            .join("/"),
                        origin: SourceOrigin::Registry,
                    },
                    clone_url.clone(),
                )),
            })
        }
    }
}

/// Copies `content` next to `dir`, then moves it into place (backing up and
/// atomically replacing an existing directory).
fn place(
    ctx: &OpsCtx,
    content: &Path,
    dir: &Path,
    name: &str,
    overwrite: bool,
) -> Result<(), OpsError> {
    let exists = fs::symlink_metadata(dir).is_ok();
    if exists && !overwrite {
        return Err(OpsError::Exists {
            dir: dir.to_string_lossy().into_owned(),
        });
    }
    let parent = dir
        .parent()
        .ok_or_else(|| invalid("target has no parent"))?;
    fs::create_dir_all(parent)?;
    let sibling = parent.join(format!(
        ".{name}.memdock-new-{}-{}",
        std::process::id(),
        now_millis()
    ));
    let result = (|| -> Result<(), OpsError> {
        copy_dir(content, &sibling)?;
        if exists {
            backup_dir(dir, &ctx.backups_root(), name)?;
            atomic_replace_dir(&sibling, dir)?;
        } else {
            fs::rename(&sibling, dir)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&sibling);
    }
    result
}

/// Records git installs and drops stale records for overwritten directories.
fn update_registry(
    ctx: &OpsCtx,
    installed: &[(InstallTarget, PathBuf)],
    name: &str,
    source: Option<&(SkillSource, String)>,
) -> Result<(), OpsError> {
    if installed.is_empty() {
        return Ok(());
    }
    let mut reg = load_registry(&ctx.registry_path).map_err(io_msg)?;
    let keys: Vec<String> = installed
        .iter()
        .map(|(_, d)| normalize_path(&d.to_string_lossy()))
        .collect();
    for e in reg.skills.iter_mut() {
        e.installs
            .retain(|i| !keys.contains(&normalize_path(&i.dir)));
    }
    reg.skills.retain(|e| !e.installs.is_empty());
    if let Some((src, clone_url)) = source {
        let now = now_millis();
        let records: Vec<InstallRecord> = installed
            .iter()
            .map(|(t, d)| InstallRecord {
                tool: t.tool,
                scope: t.scope.clone(),
                dir: d.to_string_lossy().into_owned(),
            })
            .collect();
        let hash = dir_content_hash(&installed[0].1)?;
        if let Some(e) = reg.skills.iter_mut().find(|e| {
            e.name == name && e.source.url == src.url && e.source.skill_path == src.skill_path
        }) {
            e.installs.extend(records);
            e.source.git_ref = src.git_ref.clone();
            e.clone_url = Some(clone_url.clone());
            e.content_hash = hash;
            e.updated_at = now;
        } else {
            reg.skills.push(RegistryEntry {
                name: name.to_string(),
                source: src.clone(),
                clone_url: Some(clone_url.clone()),
                installs: records,
                content_hash: hash,
                installed_at: now,
                updated_at: now,
            });
        }
    }
    save_registry(&ctx.registry_path, &reg)?;
    Ok(())
}

/// Installs a skill into every target. Existing directories are an error
/// unless `overwrite`, in which case they are backed up and atomically
/// replaced. Nothing is written when any target already exists and
/// `overwrite` is false.
pub fn install(
    ctx: &OpsCtx,
    from: InstallFrom,
    targets: &[InstallTarget],
    overwrite: bool,
) -> Result<Vec<SkillEntry>, OpsError> {
    if targets.is_empty() {
        return Err(invalid("no install target"));
    }
    // Fail early on an unreadable registry, before touching any skill folder.
    load_registry(&ctx.registry_path).map_err(io_msg)?;

    let staging = TempDir::new(&ctx.staging_root())?;
    let prepared = prepare(ctx, &from, &staging.0)?;
    let skill_md = prepared.content.join("SKILL.md");
    if !skill_md.is_file() {
        return Err(invalid("SKILL.md not found at the root of the skill"));
    }
    let name = match &prepared.forced_name {
        Some(n) => n.clone(),
        None => {
            let text = fs::read_to_string(&skill_md)
                .map_err(|e| invalid(format!("cannot read SKILL.md: {e}")))?;
            let fm_name = parse_frontmatter(&text).name;
            match fm_name.filter(|n| validate_name(n).is_ok()) {
                Some(n) => n,
                None => {
                    validate_name(&prepared.fallback_name).map_err(invalid)?;
                    prepared.fallback_name.clone()
                }
            }
        }
    };
    validate_name(&name).map_err(invalid)?;

    // Resolve and pre-check every target before writing anything.
    let mut dirs: Vec<(InstallTarget, PathBuf)> = Vec::new();
    for t in targets {
        if is_readonly_location(t, &name) {
            return Err(OpsError::ReadOnly);
        }
        let dir = target_dir(t, &ctx.paths, &ctx.home, &ctx.projects, &name).map_err(invalid)?;
        if dirs.iter().any(|(_, d)| *d == dir) {
            continue;
        }
        if !overwrite && fs::symlink_metadata(&dir).is_ok() {
            return Err(OpsError::Exists {
                dir: dir.to_string_lossy().into_owned(),
            });
        }
        dirs.push((t.clone(), dir));
    }

    let mut done: Vec<(InstallTarget, PathBuf)> = Vec::new();
    let mut failure = None;
    for (t, dir) in dirs {
        match place(ctx, &prepared.content, &dir, &name, overwrite) {
            Ok(()) => done.push((t, dir)),
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }
    // Record what was installed even if a later target failed.
    let reg_result = update_registry(ctx, &done, &name, prepared.source.as_ref());
    if matches!(from, InstallFrom::Git { .. }) {
        cleanup_cache(&ctx.cache_root(), CACHE_MAX_AGE);
    }
    if let Some(e) = failure {
        return Err(e);
    }
    reg_result?;

    let all = scan_skills(&ctx.paths, &ctx.home, &ctx.projects);
    Ok(done
        .iter()
        .filter_map(|(t, d)| {
            let id = normalize_path(&d.to_string_lossy());
            all.iter()
                .find(|e| e.id == id && e.tool == t.tool && e.scope == t.scope)
                .cloned()
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::git::SystemGit;
    use crate::skills::git::testutil::{git, make_remote};
    use crate::skills::registry::Registry;

    struct Env {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        ctx: OpsCtx,
    }

    fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let ctx = OpsCtx {
            paths: AgentPaths::from_home(&home),
            home,
            projects: Vec::new(),
            appdata: root.join("appdata"),
            git: Arc::new(SystemGit::new()),
            registry_path: root.join("appdata").join("skills-registry.json"),
        };
        Env {
            _tmp: tmp,
            root,
            ctx,
        }
    }

    fn write(p: &Path, text: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn claude() -> InstallTarget {
        InstallTarget {
            tool: Tool::Claude,
            scope: SkillScope::Global,
        }
    }

    fn codex() -> InstallTarget {
        InstallTarget {
            tool: Tool::Codex,
            scope: SkillScope::Global,
        }
    }

    fn folder_skill(e: &Env, dir_name: &str, fm_name: &str, body: &str) -> String {
        let d = e.root.join("src").join(dir_name);
        write(
            &d.join("SKILL.md"),
            &format!("---\nname: {fm_name}\ndescription: demo skill\n---\n{body}\n"),
        );
        write(&d.join("ref/a.md"), "ref");
        write(&d.join(".git/HEAD"), "x");
        d.to_string_lossy().into_owned()
    }

    #[test]
    fn install_from_folder_to_claude_global() {
        let e = env();
        let src = folder_skill(&e, "demo-dir", "demo", "v1");
        let out = install(
            &e.ctx,
            InstallFrom::Folder { path: src },
            &[claude()],
            false,
        )
        .unwrap();
        let dir = e.ctx.paths.claude_home.join("skills").join("demo");
        assert!(dir.join("SKILL.md").is_file());
        assert!(dir.join("ref/a.md").is_file());
        assert!(!dir.join(".git").exists());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "demo");
        assert_eq!(out[0].tool, Tool::Claude);
        // No leftovers next to the skill or in staging.
        let names: Vec<_> = fs::read_dir(dir.parent().unwrap())
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(names.len(), 1);
        assert_eq!(fs::read_dir(e.ctx.staging_root()).unwrap().count(), 0);
        // Non-git installs are not registered.
        assert!(
            load_registry(&e.ctx.registry_path)
                .unwrap()
                .skills
                .is_empty()
        );
    }

    fn repo_with_skill(e: &Env) -> (String, PathBuf) {
        let (url, work) = make_remote(&e.root);
        write(
            &work.join("skills/x/SKILL.md"),
            "---\nname: x\ndescription: from repo\n---\nbody\n",
        );
        write(&work.join("skills/x/ref.md"), "r");
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "skill"]);
        git(&work, &["push", "-q", "origin", "main"]);
        (url, work)
    }

    fn git_from(url: &str, path: &str) -> InstallFrom {
        InstallFrom::Git {
            url: url.to_string(),
            clone_url: url.to_string(),
            git_ref: None,
            skill_path: path.to_string(),
        }
    }

    #[test]
    fn install_from_repo_subpath() {
        let e = env();
        let (url, work) = repo_with_skill(&e);
        let head = git(&work, &["rev-parse", "HEAD"]);
        let out = install(&e.ctx, git_from(&url, "skills/x"), &[claude()], false).unwrap();
        let dir = e.ctx.paths.claude_home.join("skills").join("x");
        assert!(dir.join("SKILL.md").is_file() && dir.join("ref.md").is_file());
        assert!(!dir.join(".memdock-complete").exists());
        assert_eq!(out.len(), 1);
        let reg = load_registry(&e.ctx.registry_path).unwrap();
        assert_eq!(reg.skills.len(), 1);
        let s = &reg.skills[0];
        assert_eq!(s.source.url, url);
        assert_eq!(s.source.skill_path, "skills/x");
        assert_eq!(s.source.git_ref.as_deref(), Some(head.as_str()));
        assert_eq!(s.clone_url.as_deref(), Some(url.as_str()));
        assert_eq!(s.content_hash, dir_content_hash(&dir).unwrap());
        // Second install reuses the complete cache (remote may vanish).
        fs::remove_dir_all(dir).unwrap();
        install(&e.ctx, git_from(&url, "skills/x"), &[codex()], false).unwrap();
    }

    #[test]
    fn install_repo_bad_subpath_rejected() {
        let e = env();
        let (url, _w) = repo_with_skill(&e);
        for p in ["../x", "skills/missing"] {
            let r = install(&e.ctx, git_from(&url, p), &[claude()], false);
            assert!(matches!(r, Err(OpsError::Invalid { .. })), "{p}: {r:?}");
        }
    }

    #[test]
    fn git_unavailable_is_reported() {
        let mut e = env();
        e.ctx.git = Arc::new(SystemGit::with_program(PathBuf::from(
            "no-such-git-program",
        )));
        let r = install(&e.ctx, git_from("file:///x", ""), &[claude()], false);
        assert_eq!(r.unwrap_err(), OpsError::GitUnavailable);
    }

    #[test]
    fn install_template_creates_skill_md() {
        let e = env();
        let out = install(
            &e.ctx,
            InstallFrom::Template {
                name: "my-skill".into(),
                description: "Use when X: do \"Y\"\nsecond line".into(),
            },
            &[codex()],
            false,
        )
        .unwrap();
        let dir = e.ctx.paths.codex_home.join("skills").join("my-skill");
        let text = fs::read_to_string(dir.join("SKILL.md")).unwrap();
        let fm = parse_frontmatter(&text);
        assert_eq!(fm.name.as_deref(), Some("my-skill"));
        assert_eq!(
            fm.description.as_deref(),
            Some("Use when X: do \"Y\" second line")
        );
        assert!(text.contains("使用说明"));
        assert_eq!(out[0].description, "Use when X: do \"Y\" second line");
    }

    #[test]
    fn install_existing_without_overwrite_errors() {
        let e = env();
        let src = folder_skill(&e, "demo", "demo", "v1");
        install(
            &e.ctx,
            InstallFrom::Folder { path: src.clone() },
            &[claude()],
            false,
        )
        .unwrap();
        let dir = e.ctx.paths.claude_home.join("skills").join("demo");
        write(&dir.join("mine.txt"), "mine");
        let r = install(
            &e.ctx,
            InstallFrom::Folder { path: src.clone() },
            &[claude()],
            false,
        );
        assert_eq!(
            r.unwrap_err(),
            OpsError::Exists {
                dir: dir.to_string_lossy().into_owned()
            }
        );
        assert!(dir.join("mine.txt").exists());
        // A second target already existing blocks all targets.
        let r = install(
            &e.ctx,
            InstallFrom::Folder { path: src },
            &[codex(), claude()],
            false,
        );
        assert!(matches!(r, Err(OpsError::Exists { .. })));
        assert!(!e.ctx.paths.codex_home.join("skills").join("demo").exists());
    }

    #[test]
    fn install_overwrite_backs_up_first() {
        let e = env();
        let src1 = folder_skill(&e, "demo", "demo", "v1");
        install(
            &e.ctx,
            InstallFrom::Folder { path: src1 },
            &[claude()],
            false,
        )
        .unwrap();
        let dir = e.ctx.paths.claude_home.join("skills").join("demo");
        write(&dir.join("mine.txt"), "mine");
        let src2 = folder_skill(&e, "demo2", "demo", "v2");
        install(
            &e.ctx,
            InstallFrom::Folder { path: src2 },
            &[claude()],
            true,
        )
        .unwrap();
        assert!(
            fs::read_to_string(dir.join("SKILL.md"))
                .unwrap()
                .contains("v2")
        );
        assert!(!dir.join("mine.txt").exists());
        let backups = crate::skills::fsops::list_backups(&e.ctx.backups_root(), "demo");
        assert_eq!(backups.len(), 1);
        let b = e.ctx.backups_root().join("demo").join(&backups[0].id);
        assert!(
            fs::read_to_string(b.join("SKILL.md"))
                .unwrap()
                .contains("v1")
        );
        assert_eq!(fs::read_to_string(b.join("mine.txt")).unwrap(), "mine");
        let names: Vec<_> = fs::read_dir(dir.parent().unwrap())
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(names.len(), 1, "no temp dirs left");
    }

    #[test]
    fn install_both_tools_records_two_installs() {
        let e = env();
        let (url, _w) = repo_with_skill(&e);
        let out = install(
            &e.ctx,
            git_from(&url, "skills/x"),
            &[claude(), codex()],
            false,
        )
        .unwrap();
        assert_eq!(out.len(), 2);
        let reg: Registry = load_registry(&e.ctx.registry_path).unwrap();
        assert_eq!(reg.skills.len(), 1);
        let tools: Vec<Tool> = reg.skills[0].installs.iter().map(|i| i.tool).collect();
        assert_eq!(tools, vec![Tool::Claude, Tool::Codex]);
        // Overwriting does not duplicate install records.
        install(&e.ctx, git_from(&url, "skills/x"), &[claude()], true).unwrap();
        let reg = load_registry(&e.ctx.registry_path).unwrap();
        assert_eq!(reg.skills.len(), 1);
        assert_eq!(reg.skills[0].installs.len(), 2);
    }

    #[test]
    fn install_into_project_scope() {
        let mut e = env();
        let proj = e.root.join("proj");
        fs::create_dir_all(&proj).unwrap();
        let p = Project {
            id: normalize_path(&proj.to_string_lossy()),
            path: proj.to_string_lossy().into_owned(),
            name: "proj".into(),
            exists: true,
            sources: Vec::new(),
        };
        let scope = SkillScope::Project {
            project_id: p.id.clone(),
        };
        e.ctx.projects = vec![p];
        let src = folder_skill(&e, "demo", "demo", "v1");
        let targets = [
            InstallTarget {
                tool: Tool::Claude,
                scope: scope.clone(),
            },
            InstallTarget {
                tool: Tool::Codex,
                scope,
            },
        ];
        let out = install(&e.ctx, InstallFrom::Folder { path: src }, &targets, false).unwrap();
        assert!(proj.join(".claude/skills/demo/SKILL.md").is_file());
        assert!(proj.join(".agents/skills/demo/SKILL.md").is_file());
        assert_eq!(out.len(), 2);
        let unknown = InstallTarget {
            tool: Tool::Claude,
            scope: SkillScope::Project {
                project_id: "nope".into(),
            },
        };
        let src = folder_skill(&e, "demo", "demo", "v1");
        assert!(matches!(
            install(&e.ctx, InstallFrom::Folder { path: src }, &[unknown], false),
            Err(OpsError::Invalid { .. })
        ));
    }

    #[test]
    fn invalid_name_rejected() {
        let e = env();
        for bad in [
            "Bad Name", "../evil", "-lead", "con", "nul.txt", "lpt1", "trail.", "A",
        ] {
            assert!(validate_name(bad).is_err(), "{bad}");
        }
        for good in ["a", "my-skill_1.2", "com10", "console"] {
            assert!(validate_name(good).is_ok(), "{good}");
        }
        assert!(validate_name(&"a".repeat(65)).is_err());
        // Frontmatter name invalid and the directory name invalid too.
        let src = folder_skill(&e, "Bad Dir", "Bad Name!", "v1");
        let r = install(
            &e.ctx,
            InstallFrom::Folder { path: src },
            &[claude()],
            false,
        );
        assert!(matches!(r, Err(OpsError::Invalid { .. })));
        // Frontmatter invalid but the directory name valid -> directory name.
        let src = folder_skill(&e, "good-dir", "Bad Name!", "v1");
        install(
            &e.ctx,
            InstallFrom::Folder { path: src },
            &[claude()],
            false,
        )
        .unwrap();
        assert!(e.ctx.paths.claude_home.join("skills/good-dir").is_dir());
        // Templates validate their given name.
        let r = install(
            &e.ctx,
            InstallFrom::Template {
                name: "NUL".into(),
                description: String::new(),
            },
            &[claude()],
            false,
        );
        assert!(matches!(r, Err(OpsError::Invalid { .. })));
        // Missing SKILL.md.
        let empty = e.root.join("empty");
        fs::create_dir_all(&empty).unwrap();
        let r = install(
            &e.ctx,
            InstallFrom::Folder {
                path: empty.to_string_lossy().into_owned(),
            },
            &[claude()],
            false,
        );
        assert!(matches!(r, Err(OpsError::Invalid { .. })));
        // Claude's synced folder is read-only.
        let src = folder_skill(&e, "synced", "synced", "v1");
        let r = install(
            &e.ctx,
            InstallFrom::Folder { path: src },
            &[claude()],
            false,
        );
        assert_eq!(r.unwrap_err(), OpsError::ReadOnly);
    }

    #[test]
    fn install_from_zip() {
        use std::io::Write;
        let e = env();
        let zp = e.root.join("pack.zip");
        let f = fs::File::create(&zp).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("zipped/SKILL.md", o).unwrap();
        z.write_all(b"---\nname: zipped\ndescription: z\n---\n")
            .unwrap();
        z.finish().unwrap();
        install(
            &e.ctx,
            InstallFrom::Zip {
                path: zp.to_string_lossy().into_owned(),
            },
            &[claude()],
            false,
        )
        .unwrap();
        assert!(
            e.ctx
                .paths
                .claude_home
                .join("skills/zipped/SKILL.md")
                .is_file()
        );
    }

    #[test]
    fn ops_error_serializes_with_code() {
        let v = serde_json::to_value(OpsError::Exists { dir: "/x".into() }).unwrap();
        assert_eq!(v["code"], "exists");
        assert_eq!(v["dir"], "/x");
        assert_eq!(
            serde_json::to_value(OpsError::LocalChanges).unwrap()["code"],
            "local_changes"
        );
        assert_eq!(
            serde_json::to_value(OpsError::GitUnavailable).unwrap()["code"],
            "git_unavailable"
        );
        let v = serde_json::to_value(OpsError::Invalid {
            message: "m".into(),
        })
        .unwrap();
        assert_eq!(
            (v["code"].as_str(), v["message"].as_str()),
            (Some("invalid"), Some("m"))
        );
        let v = serde_json::to_value(OpsError::Git(GitError::Timeout)).unwrap();
        assert_eq!(v["code"], "git");
        assert_eq!(v["kind"], "timeout");
    }
}
