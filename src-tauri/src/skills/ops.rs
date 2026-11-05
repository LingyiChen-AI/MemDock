//! Skill install flow: prepare content in a staging area, validate, then place
//! it into each target with backup + atomic replace.

use std::collections::HashMap;
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
    BackupInfo, MAX_ZIP_BYTES, atomic_replace_dir, backup_dir, cleanup_leftovers, copy_dir,
    copy_dir_confined, extract_zip, now_millis, prune_backups, remove_any,
};
use crate::skills::git::{GitError, GitRunner};
use crate::skills::git_cache::{CACHE_MAX_AGE, cache_dir_for, cleanup_cache, is_complete_cache};
use crate::skills::model::{SkillEntry, SkillScope, SkillSource, SourceOrigin};
use crate::skills::registry::{
    InstallRecord, Registry, RegistryEntry, dir_content_hash, load_registry, register_manual,
    save_registry,
};
use crate::skills::scan::scan_skills;
use crate::skills::source::{infer_source, read_skill_lock};

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
            // A symlinked skill directory must not lead outside the clone.
            let real_src = fs::canonicalize(&skill_src)?;
            let real_cache = fs::canonicalize(&cache)?;
            if !real_src.starts_with(&real_cache) {
                return Err(invalid(format!(
                    "skill path resolves outside the repository: {skill_path}"
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
                        git_ref: git_ref.map(str::to_string),
                        commit: Some(commit),
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
    cleanup_leftovers(parent);
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
            e.name == name
                && e.source.url == src.url
                && e.source.skill_path == src.skill_path
                && e.source.commit == src.commit
        }) {
            e.installs.extend(records);
            e.source.git_ref = src.git_ref.clone();
            e.source.commit = src.commit.clone();
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

// ---------------------------------------------------------------------------
// Check updates / update / rollback
// ---------------------------------------------------------------------------

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct UpdateStatus {
    pub skill_id: String,
    /// Installed commit, when known.
    pub current: Option<String>,
    /// Commit of the tracking ref on the remote.
    pub latest: Option<String>,
    pub has_update: bool,
    /// True for real local edits and also when the state is unknown.
    pub local_modified: bool,
    /// No registry content hash to compare against: treated as modified.
    pub local_state_unknown: bool,
    pub error: Option<String>,
}

/// Scans skills and attaches their inferred source.
fn scan_with_sources(ctx: &OpsCtx) -> Result<(Vec<SkillEntry>, Registry), OpsError> {
    let reg = load_registry(&ctx.registry_path).map_err(io_msg)?;
    let lock = read_skill_lock(&ctx.home);
    let mut all = scan_skills(&ctx.paths, &ctx.home, &ctx.projects);
    for e in all.iter_mut().filter(|e| !e.readonly) {
        e.source = infer_source(e, &reg, &lock, ctx.git.as_ref());
    }
    Ok((all, reg))
}

fn find_reg<'a>(reg: &'a Registry, id: &str) -> Option<&'a RegistryEntry> {
    reg.skills
        .iter()
        .find(|r| r.installs.iter().any(|i| normalize_path(&i.dir) == id))
}

/// `(local_modified, state_unknown)`.
fn local_state(entry: &SkillEntry, reg: Option<&RegistryEntry>) -> (bool, bool) {
    match reg {
        Some(r) if !r.content_hash.is_empty() => match dir_content_hash(Path::new(&entry.dir)) {
            Ok(h) => (h != r.content_hash, false),
            Err(_) => (true, true),
        },
        _ => (true, true),
    }
}

fn is_pull_origin(o: SourceOrigin) -> bool {
    matches!(o, SourceOrigin::GitDir | SourceOrigin::SymlinkRepo)
}

/// Directory that holds the files (the link target for symlinked skills).
fn real_dir(entry: &SkillEntry) -> PathBuf {
    entry
        .symlink_target
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&entry.dir))
}

/// Address handed to `git` for network calls.
fn network_url(
    ctx: &OpsCtx,
    entry: &SkillEntry,
    source: &SkillSource,
    reg: Option<&RegistryEntry>,
) -> String {
    if let Some(u) = reg.and_then(|r| r.clone_url.clone()) {
        return u;
    }
    if is_pull_origin(source.origin) {
        if let Some(u) = ctx.git.remote_url(&real_dir(entry)) {
            return u;
        }
    }
    source.url.clone()
}

fn git_msg(e: &GitError) -> String {
    match e {
        GitError::NotInstalled => "git is not installed".to_string(),
        GitError::Timeout => "git timed out".to_string(),
        GitError::Failed { stderr } => stderr.clone(),
    }
}

/// Compares the installed commit of every updatable skill with its remote.
/// One `ls-remote` per distinct (address, ref); failures are per skill.
pub fn check_updates(ctx: &OpsCtx, entries: &[SkillEntry]) -> Vec<UpdateStatus> {
    let reg = load_registry(&ctx.registry_path).unwrap_or_default();
    let git_ok = ctx.git.available();
    let mut remote: HashMap<(String, Option<String>), Result<String, String>> = HashMap::new();
    let mut out = Vec::new();
    for e in entries.iter().filter(|e| !e.readonly) {
        let Some(source) = &e.source else { continue };
        let reg_entry = find_reg(&reg, &e.id);
        let (local_modified, unknown) = local_state(e, reg_entry);
        let current = source.commit.clone();
        let mut st = UpdateStatus {
            skill_id: e.id.clone(),
            current: current.clone(),
            latest: None,
            has_update: false,
            local_modified,
            local_state_unknown: unknown,
            error: None,
        };
        if !git_ok {
            st.error = Some(git_msg(&GitError::NotInstalled));
            out.push(st);
            continue;
        }
        let url = network_url(ctx, e, source, reg_entry);
        let key = (url.clone(), source.git_ref.clone());
        let res = remote
            .entry(key)
            .or_insert_with(|| {
                ctx.git
                    .ls_remote_head(&url, source.git_ref.as_deref())
                    .map_err(|e| git_msg(&e))
            })
            .clone();
        match res {
            Ok(latest) => {
                st.has_update = current.as_deref() != Some(latest.as_str());
                st.latest = Some(latest);
            }
            Err(m) => st.error = Some(m),
        }
        out.push(st);
    }
    out
}

/// Backup bucket / temp-name key: the skill directory's own name, never the
/// (untrusted) frontmatter name.
fn skill_key(entry: &SkillEntry) -> Result<String, OpsError> {
    let name = Path::new(&entry.dir)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    validate_name(&name).map_err(invalid)?;
    Ok(name)
}

const REGISTRY_NOT_SAVED: &str = "files updated, registry not saved";

/// Writes the post-update / post-rollback state of the install at `entry.dir`
/// into the registry. An entry shared by several installs is split so every
/// install keeps its own commit and hash. `create` adds a record for skills
/// that were not registered yet.
fn record_state(
    ctx: &OpsCtx,
    entry: &SkillEntry,
    source: &SkillSource,
    clone_url: &str,
    commit: Option<String>,
    create: bool,
) -> Result<(), OpsError> {
    record_state_inner(ctx, entry, source, clone_url, commit, create)
        .map_err(|e| io_msg(format!("{REGISTRY_NOT_SAVED}: {e:?}")))
}

fn record_state_inner(
    ctx: &OpsCtx,
    entry: &SkillEntry,
    source: &SkillSource,
    clone_url: &str,
    commit: Option<String>,
    create: bool,
) -> Result<(), OpsError> {
    let mut reg = load_registry(&ctx.registry_path).map_err(io_msg)?;
    let hash = dir_content_hash(Path::new(&entry.dir))?;
    let now = now_millis();
    let pos = reg.skills.iter().position(|r| {
        r.installs
            .iter()
            .any(|i| normalize_path(&i.dir) == entry.id)
    });
    match pos {
        Some(p) => {
            if reg.skills[p].installs.len() > 1 {
                let mut split = reg.skills[p].clone();
                reg.skills[p]
                    .installs
                    .retain(|i| normalize_path(&i.dir) != entry.id);
                split
                    .installs
                    .retain(|i| normalize_path(&i.dir) == entry.id);
                reg.skills.push(split);
            }
            let idx = if reg.skills[p]
                .installs
                .iter()
                .any(|i| normalize_path(&i.dir) == entry.id)
            {
                p
            } else {
                reg.skills.len() - 1
            };
            let r = &mut reg.skills[idx];
            r.source.commit = commit;
            r.content_hash = hash;
            r.updated_at = now;
        }
        None if create => {
            let mut s = source.clone();
            s.origin = SourceOrigin::Registry;
            s.commit = commit;
            reg.skills.push(RegistryEntry {
                name: entry.name.clone(),
                source: s,
                clone_url: Some(clone_url.to_string()),
                installs: vec![InstallRecord {
                    tool: entry.tool,
                    scope: entry.scope.clone(),
                    dir: entry.dir.clone(),
                }],
                content_hash: hash,
                installed_at: now,
                updated_at: now,
            });
        }
        None => return Ok(()),
    }
    save_registry(&ctx.registry_path, &reg)?;
    Ok(())
}

fn rescan_entry(ctx: &OpsCtx, id: &str) -> Result<SkillEntry, OpsError> {
    scan_with_sources(ctx)?
        .0
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| io_msg("skill not found after the operation"))
}

fn lookup(ctx: &OpsCtx, skill_id: &str) -> Result<(SkillEntry, Registry), OpsError> {
    let (all, reg) = scan_with_sources(ctx)?;
    let entry = all
        .into_iter()
        .find(|e| e.id == skill_id)
        .ok_or_else(|| invalid(format!("skill not found: {skill_id}")))?;
    Ok((entry, reg))
}

/// Updates a skill to the latest commit of its tracking ref. The previous
/// state is backed up first; on any failure the skill directory is unchanged.
pub fn update(
    ctx: &OpsCtx,
    skill_id: &str,
    accept_local_changes: bool,
) -> Result<SkillEntry, OpsError> {
    let (entry, reg) = lookup(ctx, skill_id)?;
    if entry.readonly {
        return Err(OpsError::ReadOnly);
    }
    let source = entry
        .source
        .clone()
        .ok_or_else(|| invalid("skill has no known source"))?;
    let reg_entry = find_reg(&reg, &entry.id);
    let (modified, _) = local_state(&entry, reg_entry);
    if modified && !accept_local_changes {
        return Err(OpsError::LocalChanges);
    }
    if !ctx.git.available() {
        return Err(OpsError::GitUnavailable);
    }
    if is_pull_origin(source.origin) {
        update_by_pull(ctx, &entry)?;
    } else {
        let url = network_url(ctx, &entry, &source, reg_entry);
        update_by_clone(ctx, &entry, &source, &url)?;
    }
    rescan_entry(ctx, &entry.id)
}

/// Skills that live in a git working tree: fast-forward the repository.
fn update_by_pull(ctx: &OpsCtx, entry: &SkillEntry) -> Result<(), OpsError> {
    let base = real_dir(entry);
    let root = ctx
        .git
        .repo_root(&base)
        .ok_or_else(|| invalid("skill is not inside a git repository"))?;
    let key = skill_key(entry)?;
    // Back up outside the real bucket so a failed pull neither leaves a
    // useless backup nor prunes older ones.
    let stage = TempDir::new(&ctx.staging_root())?;
    let info = backup_dir(&base, &stage.0, &key)?;
    ctx.git.pull_ff_only(&root).map_err(git_err)?;
    let dest_base = ctx.backups_root().join(&key);
    fs::create_dir_all(&dest_base)?;
    let mut id = info.id.parse::<i64>().unwrap_or_else(|_| now_millis());
    while dest_base.join(id.to_string()).exists() {
        id += 1;
    }
    fs::rename(
        stage.0.join(&key).join(&info.id),
        dest_base.join(id.to_string()),
    )?;
    prune_backups(&ctx.backups_root(), &key, &id.to_string());
    Ok(())
}

/// Registry-installed skills: shallow-clone the latest, replace atomically.
fn update_by_clone(
    ctx: &OpsCtx,
    entry: &SkillEntry,
    source: &SkillSource,
    url: &str,
) -> Result<(), OpsError> {
    let sub = repo_subpath(&source.skill_path)?;
    let git_ref = source.git_ref.as_deref();
    let head = ctx.git.ls_remote_head(url, git_ref).map_err(git_err)?;
    let cache = cache_dir_for(&ctx.cache_root(), &source.url, &head);
    let commit = if is_complete_cache(&cache) {
        head
    } else {
        if cache.exists() {
            remove_any(&cache)?;
        }
        ctx.git
            .shallow_clone(url, git_ref, &cache)
            .map_err(git_err)?
    };
    let skill_src = cache.join(&sub);
    if !skill_src.is_dir() {
        return Err(invalid(format!(
            "skill path not found in repository: {}",
            source.skill_path
        )));
    }
    let real_src = fs::canonicalize(&skill_src)?;
    let real_cache = fs::canonicalize(&cache)?;
    if !real_src.starts_with(&real_cache) {
        return Err(invalid(format!(
            "skill path resolves outside the repository: {}",
            source.skill_path
        )));
    }
    let staging = TempDir::new(&ctx.staging_root())?;
    let content = staging.0.join("content");
    copy_dir_confined(&skill_src, &content, &cache)?;
    if !content.join("SKILL.md").is_file() {
        return Err(invalid("SKILL.md not found in the updated skill"));
    }
    let key = skill_key(entry)?;
    place(ctx, &content, Path::new(&entry.dir), &key, true)?;
    let recorded = record_state(ctx, entry, source, url, Some(commit), true);
    cleanup_cache(&ctx.cache_root(), CACHE_MAX_AGE);
    recorded
}

/// Restores a backup. The current state is backed up first, so a rollback can
/// itself be undone.
pub fn rollback(ctx: &OpsCtx, skill_id: &str, backup_id: &str) -> Result<SkillEntry, OpsError> {
    let (entry, reg) = lookup(ctx, skill_id)?;
    if entry.readonly {
        return Err(OpsError::ReadOnly);
    }
    if backup_id.is_empty() || !backup_id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid("invalid backup id"));
    }
    let key = skill_key(&entry)?;
    let backup = ctx.backups_root().join(&key).join(backup_id);
    if !backup.is_dir() {
        return Err(invalid(format!("backup not found: {backup_id}")));
    }
    let dir = PathBuf::from(&entry.dir);
    let parent = dir
        .parent()
        .ok_or_else(|| invalid("target has no parent"))?;
    cleanup_leftovers(parent);
    let sibling = parent.join(format!(
        ".{}.memdock-new-{}-{}",
        key,
        std::process::id(),
        now_millis()
    ));
    let is_link = fs::symlink_metadata(&dir)
        .map(|m| m.is_symlink())
        .unwrap_or(false);
    let live_git = dir.join(".git");
    let mut moved_git = false;
    let result = (|| -> Result<(), OpsError> {
        copy_dir(&backup, &sibling)?;
        backup_dir(&dir, &ctx.backups_root(), &key)?;
        // copy_dir never brings `.git` back; carry the live repository over.
        if !is_link && fs::symlink_metadata(&live_git).is_ok() {
            fs::rename(&live_git, sibling.join(".git"))?;
            moved_git = true;
        }
        atomic_replace_dir(&sibling, &dir)?;
        Ok(())
    })();
    if result.is_err() {
        if moved_git {
            let _ = fs::rename(sibling.join(".git"), &live_git);
        }
        let _ = fs::remove_dir_all(&sibling);
    }
    result?;
    // The restored files no longer match the recorded commit.
    if let Some(src) = &entry.source {
        let url = find_reg(&reg, &entry.id)
            .and_then(|r| r.clone_url.clone())
            .unwrap_or_else(|| src.url.clone());
        record_state(ctx, &entry, src, &url, None, false)?;
    }
    rescan_entry(ctx, &entry.id)
}

// ---------------------------------------------------------------------------
// Delete / cross-tool sync / manual source registration
// ---------------------------------------------------------------------------

/// Deletes a skill after backing it up. A symlinked skill loses only the link;
/// the link target (and its files) stay. The registry forgets the install.
pub fn delete(ctx: &OpsCtx, skill_id: &str) -> Result<BackupInfo, OpsError> {
    let (entry, _) = lookup(ctx, skill_id)?;
    if entry.readonly {
        return Err(OpsError::ReadOnly);
    }
    let key = skill_key(&entry)?;
    // Fail early on an unreadable registry, before touching any file.
    load_registry(&ctx.registry_path).map_err(io_msg)?;
    let dir = PathBuf::from(&entry.dir);
    // Back up the real contents (the link target for symlinked skills).
    let info = backup_dir(&real_dir(&entry), &ctx.backups_root(), &key)?;
    // remove_any never follows a link: remove_file / remove_dir for links,
    // remove_dir_all only for real directories.
    remove_any(&dir)?;
    forget_install(ctx, &entry.id).map_err(|e| io_msg(format!("{REGISTRY_NOT_SAVED}: {e:?}")))?;
    Ok(info)
}

fn forget_install(ctx: &OpsCtx, id: &str) -> Result<(), OpsError> {
    let mut reg = load_registry(&ctx.registry_path).map_err(io_msg)?;
    let before = reg.clone();
    for e in reg.skills.iter_mut() {
        e.installs.retain(|i| normalize_path(&i.dir) != id);
    }
    reg.skills.retain(|e| !e.installs.is_empty());
    if reg != before {
        save_registry(&ctx.registry_path, &reg)?;
    }
    Ok(())
}

/// Copies a skill (read-only ones included) to `target`; the copy is a user
/// skill. The original source, when known, is recorded for the copy.
pub fn sync_to(
    ctx: &OpsCtx,
    skill_id: &str,
    target: InstallTarget,
    overwrite: bool,
) -> Result<SkillEntry, OpsError> {
    let (entry, reg) = lookup(ctx, skill_id)?;
    let key = skill_key(&entry)?;
    if is_readonly_location(&target, &key) {
        return Err(OpsError::ReadOnly);
    }
    let dir = target_dir(&target, &ctx.paths, &ctx.home, &ctx.projects, &key).map_err(invalid)?;
    let same = |a: &Path, b: &Path| {
        normalize_path(&a.to_string_lossy()) == normalize_path(&b.to_string_lossy())
    };
    if same(&dir, Path::new(&entry.dir)) || same(&dir, &real_dir(&entry)) {
        return Err(invalid("source and target are the same directory"));
    }
    if !overwrite && fs::symlink_metadata(&dir).is_ok() {
        return Err(OpsError::Exists {
            dir: dir.to_string_lossy().into_owned(),
        });
    }
    let source = entry.source.clone().map(|mut s| {
        if s.origin != SourceOrigin::Manual {
            s.origin = SourceOrigin::Registry;
        }
        let clone_url = find_reg(&reg, &entry.id)
            .and_then(|r| r.clone_url.clone())
            .unwrap_or_else(|| s.url.clone());
        (s, clone_url)
    });
    let staging = TempDir::new(&ctx.staging_root())?;
    let content = staging.0.join("content");
    copy_dir(&real_dir(&entry), &content)?;
    if !content.join("SKILL.md").is_file() {
        return Err(invalid("SKILL.md not found in the skill"));
    }
    place(ctx, &content, &dir, &key, overwrite)?;
    let installed = [(target, dir.clone())];
    update_registry(ctx, &installed, &key, source.as_ref())
        .map_err(|e| io_msg(format!("{REGISTRY_NOT_SAVED}: {e:?}")))?;
    rescan_entry(ctx, &normalize_path(&dir.to_string_lossy()))
}

/// Records a user-supplied source for a skill whose origin is unknown.
pub fn register_source(
    ctx: &OpsCtx,
    skill_id: &str,
    url: &str,
    skill_path: &str,
) -> Result<SkillEntry, OpsError> {
    let (entry, _) = lookup(ctx, skill_id)?;
    if entry.readonly {
        return Err(OpsError::ReadOnly);
    }
    let mut reg = load_registry(&ctx.registry_path).map_err(io_msg)?;
    register_manual(&mut reg, &entry, url, skill_path).map_err(invalid)?;
    save_registry(&ctx.registry_path, &reg)?;
    rescan_entry(ctx, &entry.id)
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
        assert_eq!(s.source.commit.as_deref(), Some(head.as_str()));
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

    #[test]
    fn registry_keeps_tracking_ref_and_old_commit_after_remote_moves() {
        let e = env();
        let (url, work) = repo_with_skill(&e);
        let old_head = git(&work, &["rev-parse", "HEAD"]);
        let from = InstallFrom::Git {
            url: url.clone(),
            clone_url: url.clone(),
            git_ref: Some("main".into()),
            skill_path: "skills/x".into(),
        };
        install(&e.ctx, from, &[claude()], false).unwrap();
        crate::skills::git::testutil::commit_file(&work, "later.txt");
        git(&work, &["push", "-q", "origin", "main"]);
        let reg = load_registry(&e.ctx.registry_path).unwrap();
        let s = &reg.skills[0].source;
        assert_eq!(s.git_ref.as_deref(), Some("main"));
        assert_eq!(s.commit.as_deref(), Some(old_head.as_str()));
        let remote_head = e
            .ctx
            .git
            .ls_remote_head(&url, s.git_ref.as_deref())
            .unwrap();
        assert_ne!(remote_head, old_head);
    }

    #[test]
    fn symlinked_skill_dir_outside_repo_rejected() {
        let e = env();
        let (url, _w) = repo_with_skill(&e);
        install(&e.ctx, git_from(&url, "skills/x"), &[claude()], false).unwrap();
        // Turn the cached skills/x into a link that leaves the clone.
        let cache = fs::read_dir(e.ctx.cache_root())
            .unwrap()
            .flatten()
            .map(|d| d.path())
            .next()
            .unwrap();
        let outside = e.root.join("outside");
        write(&outside.join("SKILL.md"), "---\nname: x\n---\n");
        let inner = cache.join("skills/x");
        fs::remove_dir_all(&inner).unwrap();
        #[cfg(unix)]
        let ok = std::os::unix::fs::symlink(&outside, &inner).is_ok();
        #[cfg(windows)]
        let ok = std::os::windows::fs::symlink_dir(&outside, &inner).is_ok()
            || std::process::Command::new("cmd")
                .arg("/C")
                .arg("mklink")
                .arg("/J")
                .arg(inner.to_string_lossy().replace('/', "\\"))
                .arg(outside.to_string_lossy().replace('/', "\\"))
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
        if !ok {
            eprintln!("skipped: cannot create symlinks");
            return;
        }
        let r = install(&e.ctx, git_from(&url, "skills/x"), &[codex()], false);
        assert!(matches!(r, Err(OpsError::Invalid { .. })), "{r:?}");
        assert!(!e.ctx.paths.codex_home.join("skills/x").exists());
    }

    // ---------- Task 5: check updates / update / rollback ----------

    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// Real git underneath, with call counting and failure injection.
    struct WrapGit {
        inner: SystemGit,
        ls_calls: AtomicUsize,
        fail_clone: AtomicBool,
        /// After a successful clone, turn this path into a directory.
        sabotage: std::sync::Mutex<Option<PathBuf>>,
    }

    impl WrapGit {
        fn new() -> Arc<WrapGit> {
            Arc::new(WrapGit {
                inner: SystemGit::new(),
                ls_calls: AtomicUsize::new(0),
                fail_clone: AtomicBool::new(false),
                sabotage: std::sync::Mutex::new(None),
            })
        }
    }

    impl GitRunner for WrapGit {
        fn available(&self) -> bool {
            self.inner.available()
        }
        fn ls_remote_head(&self, url: &str, r: Option<&str>) -> Result<String, GitError> {
            self.ls_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.ls_remote_head(url, r)
        }
        fn shallow_clone(
            &self,
            url: &str,
            r: Option<&str>,
            dest: &Path,
        ) -> Result<String, GitError> {
            if self.fail_clone.load(Ordering::SeqCst) {
                return Err(GitError::Failed {
                    stderr: "injected clone failure".into(),
                });
            }
            let res = self.inner.shallow_clone(url, r, dest);
            if let Some(p) = self.sabotage.lock().unwrap().take() {
                let _ = fs::remove_file(&p);
                fs::create_dir_all(&p).unwrap();
            }
            res
        }
        fn remote_url(&self, dir: &Path) -> Option<String> {
            self.inner.remote_url(dir)
        }
        fn head_commit(&self, dir: &Path) -> Option<String> {
            self.inner.head_commit(dir)
        }
        fn repo_root(&self, dir: &Path) -> Option<PathBuf> {
            self.inner.repo_root(dir)
        }
        fn pull_ff_only(&self, dir: &Path) -> Result<String, GitError> {
            self.inner.pull_ff_only(dir)
        }
    }

    fn env_wrap() -> (Env, Arc<WrapGit>) {
        let mut e = env();
        let w = WrapGit::new();
        e.ctx.git = w.clone();
        (e, w)
    }

    fn tracked(url: &str, path: &str) -> InstallFrom {
        InstallFrom::Git {
            url: url.to_string(),
            clone_url: url.to_string(),
            git_ref: Some("main".into()),
            skill_path: path.to_string(),
        }
    }

    fn all_entries(e: &Env) -> Vec<SkillEntry> {
        scan_with_sources(&e.ctx).unwrap().0
    }

    fn push_change(work: &Path, rel: &str, text: &str) -> String {
        write(&work.join(rel), text);
        git(work, &["add", "."]);
        git(work, &["commit", "-q", "-m", rel]);
        git(work, &["push", "-q", "origin", "main"]);
        git(work, &["rev-parse", "HEAD"])
    }

    fn installed_x(e: &Env) -> PathBuf {
        e.ctx.paths.claude_home.join("skills").join("x")
    }

    #[test]
    fn check_reports_update_when_remote_moved() {
        let e = env();
        let (url, work) = repo_with_skill(&e);
        let old = git(&work, &["rev-parse", "HEAD"]);
        install(&e.ctx, tracked(&url, "skills/x"), &[claude()], false).unwrap();
        let st = check_updates(&e.ctx, &all_entries(&e));
        assert_eq!(st.len(), 1);
        assert!(!st[0].has_update && !st[0].local_modified && !st[0].local_state_unknown);
        assert_eq!(st[0].current.as_deref(), Some(old.as_str()));
        assert_eq!(st[0].error, None);

        let new = push_change(&work, "skills/x/more.md", "more");
        let st = check_updates(&e.ctx, &all_entries(&e));
        assert!(st[0].has_update);
        assert_eq!(st[0].latest.as_deref(), Some(new.as_str()));
        assert_eq!(st[0].current.as_deref(), Some(old.as_str()));

        // A local edit is reported.
        write(&installed_x(&e).join("mine.txt"), "m");
        let st = check_updates(&e.ctx, &all_entries(&e));
        assert!(st[0].local_modified && !st[0].local_state_unknown);

        // One failing remote does not abort the others; error is per skill.
        let mut es = all_entries(&e);
        let mut bad = es[0].clone();
        bad.id = "zzz".into();
        if let Some(s) = bad.source.as_mut() {
            s.url = "file:///does/not/exist".into();
        }
        bad.dir = e.root.join("nowhere").to_string_lossy().into_owned();
        es.push(bad);
        let st = check_updates(&e.ctx, &es);
        assert_eq!(st.len(), 2);
        assert!(st[0].error.is_none() && st[0].has_update);
        assert!(st[1].error.is_some() && !st[1].has_update);
    }

    #[test]
    fn check_groups_ls_remote_by_url() {
        let (e, w) = env_wrap();
        let (url, work) = repo_with_skill(&e);
        push_change(
            &work,
            "skills/y/SKILL.md",
            "---\nname: y\ndescription: y\n---\n",
        );
        install(&e.ctx, tracked(&url, "skills/x"), &[claude()], false).unwrap();
        install(&e.ctx, tracked(&url, "skills/y"), &[claude()], false).unwrap();
        let es = all_entries(&e);
        assert_eq!(es.len(), 2);
        w.ls_calls.store(0, Ordering::SeqCst);
        let st = check_updates(&e.ctx, &es);
        assert_eq!(st.len(), 2);
        assert_eq!(w.ls_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn update_detects_local_changes() {
        let e = env();
        let (url, work) = repo_with_skill(&e);
        install(&e.ctx, tracked(&url, "skills/x"), &[claude()], false).unwrap();
        let dir = installed_x(&e);
        push_change(&work, "skills/x/more.md", "more");
        write(&dir.join("mine.txt"), "mine");
        let id = normalize_path(&dir.to_string_lossy());
        assert_eq!(
            update(&e.ctx, &id, false).unwrap_err(),
            OpsError::LocalChanges
        );
        assert_eq!(fs::read_to_string(dir.join("mine.txt")).unwrap(), "mine");
        assert!(!dir.join("more.md").exists());
        update(&e.ctx, &id, true).unwrap();
        assert!(!dir.join("mine.txt").exists());
        assert!(dir.join("more.md").is_file());
        let backups = crate::skills::fsops::list_backups(&e.ctx.backups_root(), "x");
        assert_eq!(backups.len(), 1);
        let b = e.ctx.backups_root().join("x").join(&backups[0].id);
        assert_eq!(fs::read_to_string(b.join("mine.txt")).unwrap(), "mine");
    }

    #[test]
    fn update_applies_and_records_commit() {
        let e = env();
        let (url, work) = repo_with_skill(&e);
        install(&e.ctx, tracked(&url, "skills/x"), &[claude()], false).unwrap();
        let dir = installed_x(&e);
        let before = load_registry(&e.ctx.registry_path).unwrap().skills[0].clone();
        let new = push_change(&work, "skills/x/more.md", "more");
        std::thread::sleep(std::time::Duration::from_millis(5));
        let id = normalize_path(&dir.to_string_lossy());
        let out = update(&e.ctx, &id, false).unwrap();
        assert_eq!(out.id, id);
        assert!(dir.join("more.md").is_file());
        assert!(!dir.join(".memdock-complete").exists());
        let reg = load_registry(&e.ctx.registry_path).unwrap();
        assert_eq!(reg.skills.len(), 1);
        let s = &reg.skills[0];
        assert_eq!(s.source.commit.as_deref(), Some(new.as_str()));
        assert_eq!(s.source.git_ref.as_deref(), Some("main"));
        assert_eq!(s.content_hash, dir_content_hash(&dir).unwrap());
        assert_ne!(s.content_hash, before.content_hash);
        assert!(s.updated_at >= before.updated_at);
        assert_eq!(s.installed_at, before.installed_at);
        let st = check_updates(&e.ctx, &all_entries(&e));
        assert!(!st[0].has_update && !st[0].local_modified);
        let names: Vec<_> = fs::read_dir(dir.parent().unwrap())
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(names.len(), 1, "no temp dirs left");
        assert_eq!(fs::read_dir(e.ctx.staging_root()).unwrap().count(), 0);
    }

    #[test]
    fn update_failure_leaves_skill_intact() {
        let (e, w) = env_wrap();
        let (url, work) = repo_with_skill(&e);
        install(&e.ctx, tracked(&url, "skills/x"), &[claude()], false).unwrap();
        let dir = installed_x(&e);
        let hash = dir_content_hash(&dir).unwrap();
        let reg_before = load_registry(&e.ctx.registry_path).unwrap();
        push_change(&work, "skills/x/more.md", "more");
        w.fail_clone.store(true, Ordering::SeqCst);
        let id = normalize_path(&dir.to_string_lossy());
        let r = update(&e.ctx, &id, false);
        assert!(matches!(r, Err(OpsError::Git(_))), "{r:?}");
        assert_eq!(dir_content_hash(&dir).unwrap(), hash);
        assert_eq!(load_registry(&e.ctx.registry_path).unwrap(), reg_before);
        let names: Vec<_> = fs::read_dir(dir.parent().unwrap())
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(names.len(), 1);
        assert_eq!(fs::read_dir(e.ctx.staging_root()).unwrap().count(), 0);
    }

    /// A skill dir that is itself a clone of the remote, plus the work repo.
    fn git_dir_skill(e: &Env) -> (PathBuf, PathBuf) {
        let (url, work) = make_remote(&e.root);
        push_change(
            &work,
            "SKILL.md",
            "---\nname: gd\ndescription: git dir\n---\nv1\n",
        );
        let dir = e.ctx.paths.claude_home.join("skills").join("gd");
        fs::create_dir_all(dir.parent().unwrap()).unwrap();
        git(&e.root, &["clone", "-q", &url, &dir.to_string_lossy()]);
        (dir, work)
    }

    #[test]
    fn update_git_dir_skill_uses_pull() {
        let e = env();
        let (dir, work) = git_dir_skill(&e);
        let id = normalize_path(&dir.to_string_lossy());
        let old = git(&dir, &["rev-parse", "HEAD"]);
        let new = push_change(&work, "extra.md", "extra");

        let st = check_updates(&e.ctx, &all_entries(&e));
        assert_eq!(st.len(), 1);
        assert!(st[0].has_update, "{:?}", st[0]);
        assert_eq!(st[0].latest.as_deref(), Some(new.as_str()));
        assert_eq!(st[0].current.as_deref(), Some(old.as_str()));
        // No registry hash -> local state unknown, treated as modified.
        assert!(st[0].local_modified && st[0].local_state_unknown);

        assert_eq!(
            update(&e.ctx, &id, false).unwrap_err(),
            OpsError::LocalChanges
        );
        assert_eq!(git(&dir, &["rev-parse", "HEAD"]), old);
        update(&e.ctx, &id, true).unwrap();
        assert_eq!(git(&dir, &["rev-parse", "HEAD"]), new);
        assert!(dir.join("extra.md").is_file() && dir.join(".git").exists());
        let backups = crate::skills::fsops::list_backups(&e.ctx.backups_root(), "gd");
        assert_eq!(backups.len(), 1);

        // Rolling back restores the files but keeps the repository.
        rollback(&e.ctx, &id, &backups[0].id).unwrap();
        assert!(!dir.join("extra.md").exists());
        assert!(dir.join(".git").exists());
        assert_eq!(git(&dir, &["rev-parse", "HEAD"]), new);
    }

    #[test]
    fn update_git_dir_pull_failure_keeps_head() {
        let e = env();
        let (dir, work) = git_dir_skill(&e);
        let id = normalize_path(&dir.to_string_lossy());
        // Local and remote diverge: --ff-only must refuse.
        write(&dir.join("local.md"), "l");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "local"]);
        let local_head = git(&dir, &["rev-parse", "HEAD"]);
        push_change(&work, "remote.md", "r");
        let r = update(&e.ctx, &id, true);
        assert!(matches!(r, Err(OpsError::Git(_))), "{r:?}");
        assert_eq!(git(&dir, &["rev-parse", "HEAD"]), local_head);
        assert!(dir.join("local.md").is_file() && !dir.join("remote.md").exists());
    }

    #[test]
    fn update_symlinked_skill_pulls_in_repo_root() {
        let e = env();
        let (url, work) = make_remote(&e.root);
        push_change(
            &work,
            "skills/s/SKILL.md",
            "---\nname: s\ndescription: linked\n---\nv1\n",
        );
        let repo = e.root.join("repo");
        git(&e.root, &["clone", "-q", &url, &repo.to_string_lossy()]);
        let link = e.ctx.paths.claude_home.join("skills").join("s");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        let target = repo.join("skills/s");
        #[cfg(unix)]
        let ok = std::os::unix::fs::symlink(&target, &link).is_ok();
        #[cfg(windows)]
        let ok = std::os::windows::fs::symlink_dir(&target, &link).is_ok();
        if !ok {
            eprintln!("skipped: cannot create symlinks");
            return;
        }
        let new = push_change(&work, "skills/s/more.md", "m");
        let id = normalize_path(&link.to_string_lossy());
        let es = all_entries(&e);
        assert_eq!(
            es[0].source.as_ref().unwrap().origin,
            SourceOrigin::SymlinkRepo
        );
        let st = check_updates(&e.ctx, &es);
        assert!(st[0].has_update && st[0].local_state_unknown, "{:?}", st[0]);
        assert_eq!(
            update(&e.ctx, &id, false).unwrap_err(),
            OpsError::LocalChanges
        );
        update(&e.ctx, &id, true).unwrap();
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]), new);
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(link.join("more.md").is_file());
        assert_eq!(
            crate::skills::fsops::list_backups(&e.ctx.backups_root(), "s").len(),
            1
        );
    }

    #[test]
    fn rollback_restores_backup_and_backs_up_current() {
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
        let src2 = folder_skill(&e, "demo2", "demo", "v2");
        install(
            &e.ctx,
            InstallFrom::Folder { path: src2 },
            &[claude()],
            true,
        )
        .unwrap();
        let backups = crate::skills::fsops::list_backups(&e.ctx.backups_root(), "demo");
        assert_eq!(backups.len(), 1);
        let id = normalize_path(&dir.to_string_lossy());
        let out = rollback(&e.ctx, &id, &backups[0].id).unwrap();
        assert_eq!(out.id, id);
        let text = fs::read_to_string(dir.join("SKILL.md")).unwrap();
        assert!(text.contains("v1") && !text.contains("v2"));
        let after = crate::skills::fsops::list_backups(&e.ctx.backups_root(), "demo");
        assert_eq!(after.len(), 2);
        // The new backup holds v2 (the state before the rollback).
        let newest = e.ctx.backups_root().join("demo").join(&after[0].id);
        assert!(
            fs::read_to_string(newest.join("SKILL.md"))
                .unwrap()
                .contains("v2")
        );
        // Unsafe / unknown ids are rejected.
        for bad in ["../x", "", "999", "1/2"] {
            assert!(rollback(&e.ctx, &id, bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn update_and_rollback_reject_readonly() {
        let e = env();
        let d = e.ctx.paths.codex_home.join("skills/.system/sys");
        write(&d.join("SKILL.md"), "---\nname: sys\ndescription: s\n---\n");
        let id = normalize_path(&d.to_string_lossy());
        assert_eq!(update(&e.ctx, &id, true).unwrap_err(), OpsError::ReadOnly);
        assert_eq!(rollback(&e.ctx, &id, "1").unwrap_err(), OpsError::ReadOnly);
        // check_updates skips read-only skills.
        let es = all_entries(&e);
        assert!(check_updates(&e.ctx, &es).is_empty());
    }

    #[test]
    fn untrusted_frontmatter_name_cannot_escape_backup_or_sibling_paths() {
        for bad in ["../evil", "a/b", r"..\evil"] {
            let e = env();
            let (url, work) = repo_with_skill(&e);
            install(&e.ctx, tracked(&url, "skills/x"), &[claude()], false).unwrap();
            let dir = installed_x(&e);
            let id = normalize_path(&dir.to_string_lossy());
            push_change(
                &work,
                "skills/x/SKILL.md",
                &format!(
                    "---
name: {bad}
description: evil
---
v2
"
                ),
            );
            update(&e.ctx, &id, false).unwrap();
            let backups = crate::skills::fsops::list_backups(&e.ctx.backups_root(), "x");
            assert_eq!(backups.len(), 1, "{bad}");
            rollback(&e.ctx, &id, &backups[0].id).unwrap();
            // Nothing named after the malicious value exists anywhere.
            for ent in walkdir::WalkDir::new(&e.root).into_iter().flatten() {
                let n = ent.file_name().to_string_lossy().into_owned();
                assert!(!n.contains("evil") || n == "SKILL.md", "{bad}: {n}");
            }
            let created: Vec<_> = fs::read_dir(e.ctx.backups_root())
                .unwrap()
                .flatten()
                .map(|d| d.file_name().to_string_lossy().into_owned())
                .collect();
            assert_eq!(created, vec!["x".to_string()], "{bad}");
        }
    }

    #[test]
    fn rollback_keeps_gitfile_pointer() {
        let e = env();
        let src = folder_skill(&e, "demo", "demo", "v1");
        install(
            &e.ctx,
            InstallFrom::Folder { path: src },
            &[claude()],
            false,
        )
        .unwrap();
        let dir = e.ctx.paths.claude_home.join("skills").join("demo");
        let src2 = folder_skill(&e, "demo2", "demo", "v2");
        install(
            &e.ctx,
            InstallFrom::Folder { path: src2 },
            &[claude()],
            true,
        )
        .unwrap();
        fs::write(
            dir.join(".git"),
            "gitdir: /somewhere
",
        )
        .unwrap();
        let b = crate::skills::fsops::list_backups(&e.ctx.backups_root(), "demo");
        let id = normalize_path(&dir.to_string_lossy());
        rollback(&e.ctx, &id, &b[0].id).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join(".git")).unwrap(),
            "gitdir: /somewhere
"
        );
        assert!(
            fs::read_to_string(dir.join("SKILL.md"))
                .unwrap()
                .contains("v1")
        );
    }

    #[test]
    fn install_does_not_merge_into_entry_with_other_commit() {
        let e = env();
        let (url, work) = repo_with_skill(&e);
        install(
            &e.ctx,
            tracked(&url, "skills/x"),
            &[claude(), codex()],
            false,
        )
        .unwrap();
        let c1 = load_registry(&e.ctx.registry_path).unwrap().skills[0]
            .source
            .commit
            .clone()
            .unwrap();
        let c2 = push_change(&work, "skills/x/more.md", "m");
        let id = normalize_path(&installed_x(&e).to_string_lossy());
        update(&e.ctx, &id, false).unwrap();
        // Reinstall into the updated target: must not touch the codex entry.
        install(&e.ctx, tracked(&url, "skills/x"), &[claude()], true).unwrap();
        let reg = load_registry(&e.ctx.registry_path).unwrap();
        let codex_dir = normalize_path(&e.ctx.paths.codex_home.join("skills/x").to_string_lossy());
        let b = find_reg(&reg, &codex_dir).unwrap();
        assert_eq!(b.source.commit.as_deref(), Some(c1.as_str()));
        let a = find_reg(&reg, &id).unwrap();
        assert_eq!(a.source.commit.as_deref(), Some(c2.as_str()));
        assert_eq!(reg.skills.len(), 2);
    }

    #[test]
    fn failed_pull_does_not_leave_or_evict_backups() {
        let e = env();
        let (dir, work) = git_dir_skill(&e);
        let id = normalize_path(&dir.to_string_lossy());
        for _ in 0..10 {
            crate::skills::fsops::backup_dir(&dir, &e.ctx.backups_root(), "gd").unwrap();
        }
        let before: Vec<String> = crate::skills::fsops::list_backups(&e.ctx.backups_root(), "gd")
            .into_iter()
            .map(|b| b.id)
            .collect();
        assert_eq!(before.len(), 10);
        write(&dir.join("local.md"), "l");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "local"]);
        push_change(&work, "remote.md", "r");
        assert!(update(&e.ctx, &id, true).is_err());
        let after: Vec<String> = crate::skills::fsops::list_backups(&e.ctx.backups_root(), "gd")
            .into_iter()
            .map(|b| b.id)
            .collect();
        assert_eq!(after, before);
        assert_eq!(fs::read_dir(e.ctx.staging_root()).unwrap().count(), 0);
    }

    #[test]
    fn registry_save_failure_reports_files_updated() {
        let (e, w) = env_wrap();
        let (url, work) = repo_with_skill(&e);
        install(&e.ctx, tracked(&url, "skills/x"), &[claude()], false).unwrap();
        let dir = installed_x(&e);
        push_change(&work, "skills/x/more.md", "m");
        *w.sabotage.lock().unwrap() = Some(e.ctx.registry_path.clone());
        let id = normalize_path(&dir.to_string_lossy());
        match update(&e.ctx, &id, false) {
            Err(OpsError::Io { message }) => {
                assert!(
                    message.contains("files updated, registry not saved"),
                    "{message}"
                )
            }
            other => panic!("{other:?}"),
        }
        assert!(dir.join("more.md").is_file());
    }

    // ---------- Task 6: delete / sync / register ----------

    fn install_demo(e: &Env, tgt: InstallTarget) -> (PathBuf, String) {
        let src = folder_skill(e, "demo", "demo", "v1");
        let out = install(&e.ctx, InstallFrom::Folder { path: src }, &[tgt], false).unwrap();
        (PathBuf::from(&out[0].dir), out[0].id.clone())
    }

    /// Directory symlink, falling back to a junction on Windows.
    fn make_link(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(target, link).is_ok()
                || std::process::Command::new("cmd")
                    .arg("/C")
                    .arg("mklink")
                    .arg("/J")
                    .arg(link.to_string_lossy().replace('/', "\\"))
                    .arg(target.to_string_lossy().replace('/', "\\"))
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
        }
    }

    #[test]
    fn delete_backs_up_and_removes() {
        let e = env();
        let (dir, id) = install_demo(&e, claude());
        write(&dir.join("mine.txt"), "mine");
        let info = delete(&e.ctx, &id).unwrap();
        assert!(!dir.exists());
        let b = e.ctx.backups_root().join("demo").join(&info.id);
        assert_eq!(fs::read_to_string(b.join("mine.txt")).unwrap(), "mine");
        assert!(b.join("SKILL.md").is_file() && b.join("ref/a.md").is_file());
        let names: Vec<_> = fs::read_dir(dir.parent().unwrap())
            .unwrap()
            .flatten()
            .collect();
        assert!(names.is_empty());
    }

    #[test]
    fn delete_symlink_keeps_target() {
        let e = env();
        let target = e.root.join("real-skill");
        write(
            &target.join("SKILL.md"),
            "---\nname: linked\ndescription: l\n---\nbody\n",
        );
        write(&target.join("sub/f.txt"), "f");
        let link = e.ctx.paths.claude_home.join("skills").join("linked");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        if !make_link(&target, &link) {
            eprintln!("skipped: cannot create symlinks or junctions");
            return;
        }
        let id = normalize_path(&link.to_string_lossy());
        let entry = all_entries(&e).into_iter().find(|s| s.id == id).unwrap();
        assert!(entry.is_symlink, "scan should see the link");
        let info = delete(&e.ctx, &id).unwrap();
        assert!(fs::symlink_metadata(&link).is_err(), "link removed");
        assert!(target.join("SKILL.md").is_file());
        assert_eq!(fs::read_to_string(target.join("sub/f.txt")).unwrap(), "f");
        // The backup is a real copy of the contents.
        let b = e.ctx.backups_root().join("linked").join(&info.id);
        assert!(b.join("SKILL.md").is_file() && b.join("sub/f.txt").is_file());
        assert!(!fs::symlink_metadata(&b).unwrap().file_type().is_symlink());
    }

    #[test]
    fn delete_readonly_rejected() {
        let e = env();
        let d = e.ctx.paths.codex_home.join("skills/.system/sys");
        write(&d.join("SKILL.md"), "---\nname: sys\ndescription: s\n---\n");
        let id = normalize_path(&d.to_string_lossy());
        assert_eq!(delete(&e.ctx, &id).unwrap_err(), OpsError::ReadOnly);
        assert!(d.join("SKILL.md").is_file());
        assert!(!e.ctx.backups_root().join("sys").exists());
    }

    #[test]
    fn delete_updates_registry() {
        let e = env();
        let (url, _w) = repo_with_skill(&e);
        install(
            &e.ctx,
            tracked(&url, "skills/x"),
            &[claude(), codex()],
            false,
        )
        .unwrap();
        let id = normalize_path(&installed_x(&e).to_string_lossy());
        delete(&e.ctx, &id).unwrap();
        let reg = load_registry(&e.ctx.registry_path).unwrap();
        assert_eq!(reg.skills.len(), 1);
        assert_eq!(reg.skills[0].installs.len(), 1);
        assert_eq!(reg.skills[0].installs[0].tool, Tool::Codex);
        let id2 = normalize_path(&e.ctx.paths.codex_home.join("skills/x").to_string_lossy());
        delete(&e.ctx, &id2).unwrap();
        assert!(
            load_registry(&e.ctx.registry_path)
                .unwrap()
                .skills
                .is_empty()
        );
        assert!(delete(&e.ctx, &id2).is_err());
    }

    #[test]
    fn sync_claude_to_codex_global() {
        let e = env();
        let (_dir, id) = install_demo(&e, claude());
        let out = sync_to(&e.ctx, &id, codex(), false).unwrap();
        assert_eq!(out.tool, Tool::Codex);
        assert_eq!(out.kind, crate::skills::model::SkillKind::User);
        assert!(!out.readonly);
        let d = e.ctx.paths.codex_home.join("skills/demo");
        assert!(d.join("SKILL.md").is_file() && d.join("ref/a.md").is_file());
        assert!(!d.join(".git").exists());
        // Source untouched; no registry entry without a source.
        assert!(
            e.ctx
                .paths
                .claude_home
                .join("skills/demo/SKILL.md")
                .is_file()
        );
        assert!(
            load_registry(&e.ctx.registry_path)
                .unwrap()
                .skills
                .is_empty()
        );
        assert_eq!(fs::read_dir(e.ctx.staging_root()).unwrap().count(), 0);
    }

    #[test]
    fn sync_keeps_source_in_registry() {
        let e = env();
        let (url, _w) = repo_with_skill(&e);
        install(&e.ctx, tracked(&url, "skills/x"), &[claude()], false).unwrap();
        let id = normalize_path(&installed_x(&e).to_string_lossy());
        sync_to(&e.ctx, &id, codex(), false).unwrap();
        let reg = load_registry(&e.ctx.registry_path).unwrap();
        assert_eq!(reg.skills.len(), 1);
        assert_eq!(reg.skills[0].installs.len(), 2);
        assert_eq!(reg.skills[0].source.url, url);
        let cdir = e.ctx.paths.codex_home.join("skills/x");
        assert_eq!(reg.skills[0].content_hash, dir_content_hash(&cdir).unwrap());
        // The copy now reports a source and tracks updates.
        let cid = normalize_path(&cdir.to_string_lossy());
        let es = all_entries(&e);
        let c = es.iter().find(|s| s.id == cid).unwrap();
        assert_eq!(c.source.as_ref().unwrap().url, url);
    }

    #[test]
    fn sync_existing_without_overwrite_errors() {
        let e = env();
        let (_dir, id) = install_demo(&e, claude());
        sync_to(&e.ctx, &id, codex(), false).unwrap();
        let d = e.ctx.paths.codex_home.join("skills").join("demo");
        write(&d.join("mine.txt"), "mine");
        let r = sync_to(&e.ctx, &id, codex(), false);
        assert_eq!(
            r.unwrap_err(),
            OpsError::Exists {
                dir: d.to_string_lossy().into_owned()
            }
        );
        assert!(d.join("mine.txt").exists());
        sync_to(&e.ctx, &id, codex(), true).unwrap();
        assert!(!d.join("mine.txt").exists());
        let b = crate::skills::fsops::list_backups(&e.ctx.backups_root(), "demo");
        assert_eq!(b.len(), 1);
        assert!(
            e.ctx
                .backups_root()
                .join("demo")
                .join(&b[0].id)
                .join("mine.txt")
                .is_file()
        );
        // Syncing onto itself is rejected.
        assert!(matches!(
            sync_to(&e.ctx, &id, claude(), true),
            Err(OpsError::Invalid { .. })
        ));
    }

    #[test]
    fn sync_readonly_plugin_skill_creates_user_copy() {
        let e = env();
        let install_path = e.root.join("cache/superpowers/7.0.0");
        write(
            &install_path.join("skills/brainstorming/SKILL.md"),
            "---\nname: brainstorming\ndescription: Plan first\n---\nbody\n",
        );
        write(&install_path.join("skills/brainstorming/ref.md"), "r");
        let json = serde_json::json!({
            "version": 2,
            "plugins": { "superpowers@claude-plugins-official": [
                { "scope": "user", "installPath": install_path.to_string_lossy(), "version": "7.0.0" }
            ] }
        });
        write(
            &e.ctx
                .paths
                .claude_home
                .join("plugins/installed_plugins.json"),
            &json.to_string(),
        );
        let entry = all_entries(&e)
            .into_iter()
            .find(|s| s.name == "brainstorming")
            .unwrap();
        assert!(entry.readonly);
        let out = sync_to(&e.ctx, &entry.id, codex(), false).unwrap();
        assert_eq!(out.kind, crate::skills::model::SkillKind::User);
        assert!(!out.readonly);
        assert!(
            e.ctx
                .paths
                .codex_home
                .join("skills/brainstorming/ref.md")
                .is_file()
        );
        // The plugin copy is untouched.
        assert!(install_path.join("skills/brainstorming/SKILL.md").is_file());
    }

    #[test]
    fn register_source_then_check_updates_works() {
        /// Maps a public-looking url onto the local bare repository.
        struct RewriteGit(SystemGit, String);
        impl GitRunner for RewriteGit {
            fn available(&self) -> bool {
                self.0.available()
            }
            fn ls_remote_head(&self, url: &str, r: Option<&str>) -> Result<String, GitError> {
                let u = if url == FAKE_URL {
                    self.1.as_str()
                } else {
                    url
                };
                self.0.ls_remote_head(u, r)
            }
            fn shallow_clone(
                &self,
                u: &str,
                r: Option<&str>,
                d: &Path,
            ) -> Result<String, GitError> {
                self.0.shallow_clone(u, r, d)
            }
            fn remote_url(&self, d: &Path) -> Option<String> {
                self.0.remote_url(d)
            }
            fn head_commit(&self, d: &Path) -> Option<String> {
                self.0.head_commit(d)
            }
            fn repo_root(&self, d: &Path) -> Option<PathBuf> {
                self.0.repo_root(d)
            }
            fn pull_ff_only(&self, d: &Path) -> Result<String, GitError> {
                self.0.pull_ff_only(d)
            }
        }
        const FAKE_URL: &str = "https://example.com/o/r";
        let mut e = env();
        let (real_url, work) = repo_with_skill(&e);
        e.ctx.git = Arc::new(RewriteGit(SystemGit::new(), real_url));
        let url = FAKE_URL.to_string();
        let (dir, id) = install_demo(&e, claude());
        let head = git(&work, &["rev-parse", "HEAD"]);
        let out = register_source(&e.ctx, &id, &url, "skills/x").unwrap();
        let s = out.source.expect("source after register");
        assert_eq!(s.url, url);
        assert_eq!(s.skill_path, "skills/x");
        let reg = load_registry(&e.ctx.registry_path).unwrap();
        assert_eq!(reg.skills[0].source.origin, SourceOrigin::Manual);
        assert_eq!(reg.skills.len(), 1);
        assert_eq!(reg.skills[0].content_hash, dir_content_hash(&dir).unwrap());
        // Remote has a commit, none recorded locally -> update available.
        let st = check_updates(&e.ctx, &all_entries(&e));
        assert_eq!(st.len(), 1);
        assert_eq!(st[0].error, None);
        assert_eq!(st[0].latest.as_deref(), Some(head.as_str()));
        assert!(st[0].has_update);
        // Bad urls and read-only skills are rejected.
        assert!(matches!(
            register_source(&e.ctx, &id, "ftp://nope", ""),
            Err(OpsError::Invalid { .. })
        ));
        let d = e.ctx.paths.codex_home.join("skills/.system/sys");
        write(&d.join("SKILL.md"), "---\nname: sys\ndescription: s\n---\n");
        let sid = normalize_path(&d.to_string_lossy());
        assert_eq!(
            register_source(&e.ctx, &sid, &url, "").unwrap_err(),
            OpsError::ReadOnly
        );
    }

    #[test]
    fn delete_registry_save_failure_reports_files_updated() {
        let e = env();
        let (url, _w) = repo_with_skill(&e);
        install(&e.ctx, tracked(&url, "skills/x"), &[claude()], false).unwrap();
        let dir = installed_x(&e);
        let id = normalize_path(&dir.to_string_lossy());
        // Make the registry path unwritable by replacing it with a directory.
        fs::remove_file(&e.ctx.registry_path).unwrap();
        fs::create_dir_all(&e.ctx.registry_path).unwrap();
        // An unreadable registry makes delete refuse before touching files.
        let r = delete(&e.ctx, &id);
        assert!(matches!(r, Err(OpsError::Io { .. })), "{r:?}");
        assert!(dir.join("SKILL.md").is_file());
    }
}
