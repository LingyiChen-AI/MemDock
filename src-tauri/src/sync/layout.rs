//! Repository layout and the mapping between local memory files and repo paths.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{MemoryFile, MemoryIndex, MemoryKind, Scope};
use crate::paths::AgentPaths;
use crate::projects::{claude_project_dirs, normalize_path};

/// A local memory file that takes part in sync.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SyncFile {
    pub repo_path: String,
    pub local_path: PathBuf,
    pub file_id: String,
    pub project_key: Option<String>,
}

// ------------------------------------------------------------ project keys

/// Reads the `[remote "origin"]` url out of git config text.
fn origin_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            let compact: String = t.chars().filter(|c| !c.is_whitespace()).collect();
            in_origin = compact.eq_ignore_ascii_case("[remote\"origin\"]");
            continue;
        }
        if in_origin {
            if let Some((k, v)) = t.split_once('=') {
                if k.trim().eq_ignore_ascii_case("url") {
                    let v = v.trim().trim_matches('"').trim();
                    if !v.is_empty() {
                        return Some(v.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Normalizes a git remote url to `host/owner/repo`.
fn normalize_remote(url: &str) -> Option<String> {
    let url = url.trim();
    let (host, path) = if let Some((scheme, rest)) = url.split_once("://") {
        if scheme.eq_ignore_ascii_case("file") {
            return None;
        }
        let (auth, path) = rest.split_once('/')?;
        let hostport = auth.rsplit_once('@').map_or(auth, |x| x.1);
        let host = hostport.split(':').next().unwrap_or("");
        (host.to_string(), path.to_string())
    } else {
        // scp style: [user@]host:path
        let (left, path) = url.split_once(':')?;
        if left.len() == 1 || left.contains('/') || left.contains('\\') {
            return None; // drive letter or local path
        }
        let host = left.rsplit_once('@').map_or(left, |x| x.1);
        (host.to_string(), path.to_string())
    };
    let host = host.trim().to_lowercase();
    let mut path = path.trim().trim_matches('/').to_string();
    if let Some(p) = path.strip_suffix(".git") {
        path = p.trim_end_matches('/').to_string();
    }
    let path = path.trim_start_matches('/').to_string();
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("{host}/{path}"))
}

/// Resolves the directory holding the git config of `project_dir`.
fn git_config_path(project_dir: &Path) -> Option<PathBuf> {
    let dot_git = project_dir.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let text = fs::read_to_string(&dot_git).ok()?;
        let target = text.lines().find_map(|l| l.trim().strip_prefix("gitdir:"))?.trim();
        let p = PathBuf::from(target);
        if p.is_absolute() { p } else { project_dir.join(p) }
    };
    if git_dir.join("config").is_file() {
        return Some(git_dir.join("config"));
    }
    // linked worktree: the shared config lives in the common dir
    let common = fs::read_to_string(git_dir.join("commondir")).ok()?;
    let common = PathBuf::from(common.trim());
    let common = if common.is_absolute() { common } else { git_dir.join(common) };
    let cfg = common.join("config");
    cfg.is_file().then_some(cfg)
}

/// `host/owner/repo` of the project's `origin` remote, if it has one.
pub fn project_key(project_dir: &Path) -> Option<String> {
    let cfg = fs::read_to_string(git_config_path(project_dir)?).ok()?;
    normalize_remote(&origin_url(&cfg)?)
}

/// Key of a project without remote: `local/<directory name>`.
pub fn dir_key(project_dir: &Path) -> String {
    let name = project_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("local/{name}")
}

fn safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Accepts only `/`-separated segments of `[A-Za-z0-9._-]`; anything else
/// (traversal, empty segments, spaces) means the project does not sync.
pub fn safe_key(key: &str) -> Option<String> {
    key.split('/').all(safe_segment).then(|| key.to_string())
}

// ------------------------------------------------------------ file mapping

fn to_slash(p: &str) -> String {
    p.replace('\\', "/")
}

/// `file` path relative to `base` (case-insensitive prefix, original case kept).
fn rel_to(base: &str, file: &str) -> Option<String> {
    let base = to_slash(base);
    let base = base.trim_end_matches('/');
    let file = to_slash(file);
    let head = file.get(..base.len())?;
    if !head.eq_ignore_ascii_case(base) {
        return None;
    }
    file.get(base.len()..)?.strip_prefix('/').map(String::from)
}

/// Path of an auto-memory file relative to its `memory` directory
/// (`.../projects/<enc>/memory/<rel>`).
fn auto_rel(path: &str) -> Option<String> {
    let p = to_slash(path);
    let comps: Vec<&str> = p.split('/').collect();
    let i = (2..comps.len().saturating_sub(1))
        .find(|&i| comps[i] == "memory" && comps[i - 2] == "projects")?;
    let rel = comps[i + 1..].join("/");
    (!rel.is_empty()).then_some(rel)
}

fn project_repo_path(key: &str, kind: MemoryKind, local: &str, project_dir: &str) -> Option<String> {
    let base = format!("projects/{key}");
    match kind {
        MemoryKind::ClaudeInstructions => {
            let rel = rel_to(project_dir, local)?;
            if rel.eq_ignore_ascii_case("CLAUDE.md") {
                Some(format!("{base}/claude/CLAUDE.md"))
            } else if rel.eq_ignore_ascii_case(".claude/CLAUDE.md") {
                Some(format!("{base}/claude/dot-claude/CLAUDE.md"))
            } else {
                None
            }
        }
        MemoryKind::ClaudeLocal => {
            let rel = rel_to(project_dir, local)?;
            rel.eq_ignore_ascii_case("CLAUDE.local.md")
                .then(|| format!("{base}/claude/CLAUDE.local.md"))
        }
        MemoryKind::CodexAgents => {
            let rel = rel_to(project_dir, local)?;
            rel.eq_ignore_ascii_case("AGENTS.md").then(|| format!("{base}/codex/AGENTS.md"))
        }
        MemoryKind::ClaudeAuto => {
            let rel = auto_rel(local)?;
            rel.split('/').all(safe_rel_segment).then(|| format!("{base}/claude/memory/{rel}"))
        }
        MemoryKind::CodexDb => None,
    }
}

fn safe_rel_segment(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && !s.contains(['\\', ':', '\0'])
}

fn sync_file(f: &MemoryFile, repo_path: String, key: Option<String>) -> SyncFile {
    SyncFile {
        repo_path,
        local_path: PathBuf::from(&f.path),
        file_id: f.id.clone(),
        project_key: key,
    }
}

/// Maps local memory files to repo paths. Projects without a remote take part
/// only when their `dir_key` is in `included_local`.
pub fn map_files(index: &MemoryIndex, included_local: &[String]) -> Vec<SyncFile> {
    let mut out = Vec::new();
    for f in &index.global {
        if f.is_symlink || !matches!(f.scope, Scope::Global) {
            continue;
        }
        let rp = match f.kind {
            MemoryKind::ClaudeInstructions => "global/claude/CLAUDE.md",
            MemoryKind::CodexAgents => "global/codex/AGENTS.md",
            _ => continue,
        };
        out.push(sync_file(f, rp.to_string(), None));
    }
    for pm in &index.projects {
        let dir = Path::new(&pm.project.path);
        let key = match project_key(dir) {
            Some(k) => k,
            None => {
                let dk = dir_key(dir);
                if !included_local.contains(&dk) {
                    continue;
                }
                dk
            }
        };
        let Some(key) = safe_key(&key) else { continue };
        for f in &pm.files {
            if f.is_symlink {
                continue;
            }
            if let Some(rp) = project_repo_path(&key, f.kind, &f.path, &pm.project.path) {
                out.push(sync_file(f, rp, Some(key.clone())));
            }
        }
    }
    out
}

/// Where a repo file that is not on this machine yet should be written.
/// `None` when the project is not on this machine or the path is not valid.
pub fn expected_local_path(
    repo_path: &str,
    projects: &[(String, PathBuf)],
    paths: &AgentPaths,
) -> Option<PathBuf> {
    match repo_path {
        "global/claude/CLAUDE.md" => return Some(paths.claude_home.join("CLAUDE.md")),
        "global/codex/AGENTS.md" => return Some(paths.codex_home.join("AGENTS.md")),
        _ => {}
    }
    let rest = repo_path.strip_prefix("projects/")?;
    let (key, dir) = projects
        .iter()
        .filter(|(k, _)| rest.strip_prefix(k.as_str()).is_some_and(|t| t.starts_with('/')))
        .max_by_key(|(k, _)| k.len())
        .map(|(k, d)| (k, d))?;
    let tail = &rest[key.len() + 1..];
    match tail {
        "claude/CLAUDE.md" => Some(dir.join("CLAUDE.md")),
        "claude/dot-claude/CLAUDE.md" => Some(dir.join(".claude").join("CLAUDE.md")),
        "claude/CLAUDE.local.md" => Some(dir.join("CLAUDE.local.md")),
        "codex/AGENTS.md" => Some(dir.join("AGENTS.md")),
        _ => {
            let rel = tail.strip_prefix("claude/memory/")?;
            if !rel.split('/').all(safe_rel_segment) {
                return None;
            }
            let enc = claude_project_dirs(paths).get(&normalize_path(&dir.to_string_lossy()))?.clone();
            let mut p = enc.join("memory");
            for seg in rel.split('/') {
                p.push(seg);
            }
            Some(p)
        }
    }
}

/// Text as stored in the repo: no BOM, LF newlines, lossy UTF-8.
pub fn normalize_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

// ----------------------------------------------------------------- machine

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Machine {
    pub id: String,
    pub name: String,
}

fn hostname() -> String {
    ["COMPUTERNAME", "HOSTNAME", "HOST"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Reads `machine.json`, creating it (uuid + host name) on first use.
pub fn load_or_create_machine(path: &Path) -> io::Result<Machine> {
    if let Ok(text) = fs::read_to_string(path) {
        if let Ok(m) = serde_json::from_str::<Machine>(&text) {
            if !m.id.is_empty() {
                return Ok(m);
            }
        }
    }
    let m = Machine { id: uuid::Uuid::new_v4().to_string(), name: hostname() };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::fsutil::save_json_atomic(path, &m)?;
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Project, ProjectMemories, ProjectSource};

    fn init_git(dir: &Path, url: Option<&str>) {
        fs::create_dir_all(dir.join(".git")).unwrap();
        let mut cfg = String::from("[core]\n\tbare = false\n");
        if let Some(u) = url {
            cfg.push_str(&format!("[remote \"origin\"]\n\turl = {u}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n"));
        }
        fs::write(dir.join(".git").join("config"), cfg).unwrap();
    }

    #[test]
    fn project_key_variants() {
        let tmp = tempfile::tempdir().unwrap();
        for (i, url) in [
            "https://github.com/Foo/Bar.git",
            "git@github.com:Foo/Bar.git",
            "ssh://git@github.com:22/Foo/Bar",
            "https://user:tok@GitHub.com/Foo/Bar/",
        ]
        .iter()
        .enumerate()
        {
            let d = tmp.path().join(format!("p{i}"));
            init_git(&d, Some(url));
            assert_eq!(project_key(&d).as_deref(), Some("github.com/Foo/Bar"), "{url}");
        }
        let none = tmp.path().join("none");
        init_git(&none, None);
        assert_eq!(project_key(&none), None);
        assert_eq!(project_key(&tmp.path().join("missing")), None);
    }

    #[test]
    fn project_key_follows_gitdir_file() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("realgit");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("config"), "[remote \"origin\"]\n\turl = git@gitlab.com:a/b.git\n").unwrap();
        let proj = tmp.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        fs::write(proj.join(".git"), format!("gitdir: {}\n", real.display())).unwrap();
        assert_eq!(project_key(&proj).as_deref(), Some("gitlab.com/a/b"));
    }

    #[test]
    fn safe_key_rejects_traversal() {
        assert_eq!(safe_key("github.com/Foo/Bar").as_deref(), Some("github.com/Foo/Bar"));
        assert_eq!(safe_key("local/my-proj_1").as_deref(), Some("local/my-proj_1"));
        for bad in ["github.com/../x", "a//b", "a/b c", "", "/a", "a/", "a/./b"] {
            assert_eq!(safe_key(bad), None, "{bad}");
        }
    }

    fn mf(path: &Path, kind: MemoryKind, scope: Scope, symlink: bool) -> MemoryFile {
        let p = path.to_string_lossy().into_owned();
        MemoryFile {
            id: normalize_path(&p),
            tool: if matches!(kind, MemoryKind::CodexAgents | MemoryKind::CodexDb) {
                crate::model::Tool::Codex
            } else {
                crate::model::Tool::Claude
            },
            scope,
            kind,
            path: p,
            exists: true,
            is_symlink: symlink,
            size_bytes: 1,
            modified_at: Some(1),
        }
    }

    fn proj(dir: &Path) -> Project {
        let s = dir.to_string_lossy().into_owned();
        Project { id: normalize_path(&s), path: s, name: "p".into(), exists: true, sources: vec![ProjectSource::Manual] }
    }

    fn build_index(tmp: &Path, with_remote: bool) -> (MemoryIndex, PathBuf) {
        let home = tmp.join("home");
        let claude = home.join(".claude");
        let codex = home.join(".codex");
        let dir = tmp.join("work").join("myproj");
        init_git(&dir, with_remote.then_some("https://github.com/Foo/Bar.git"));
        let p = proj(&dir);
        let mem = claude.join("projects").join("enc").join("memory");
        let g = |k, path: PathBuf| mf(&path, k, Scope::Global, false);
        let s = || Scope::Project { project_id: p.id.clone() };
        let files = vec![
            mf(&dir.join("CLAUDE.md"), MemoryKind::ClaudeInstructions, s(), false),
            mf(&dir.join(".claude").join("CLAUDE.md"), MemoryKind::ClaudeInstructions, s(), false),
            mf(&dir.join("CLAUDE.local.md"), MemoryKind::ClaudeLocal, s(), false),
            mf(&mem.join("a.md"), MemoryKind::ClaudeAuto, s(), false),
            mf(&mem.join("sub").join("b.md"), MemoryKind::ClaudeAuto, s(), false),
            mf(&mem.join("MEMORY.md"), MemoryKind::ClaudeAuto, s(), false),
            mf(&dir.join("AGENTS.md"), MemoryKind::CodexAgents, s(), false),
            mf(&dir.join("linked.md"), MemoryKind::ClaudeLocal, s(), true),
            mf(&dir.join("db.sqlite"), MemoryKind::CodexDb, s(), false),
        ];
        let index = MemoryIndex {
            global: vec![
                g(MemoryKind::ClaudeInstructions, claude.join("CLAUDE.md")),
                g(MemoryKind::CodexAgents, codex.join("AGENTS.md")),
                g(MemoryKind::CodexDb, codex.join("memories_1.sqlite")),
            ],
            projects: vec![ProjectMemories { project: p, files }],
        };
        (index, dir)
    }

    #[test]
    fn map_files_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let (index, _) = build_index(tmp.path(), true);
        let m = map_files(&index, &[]);
        let paths: Vec<&str> = m.iter().map(|f| f.repo_path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "global/claude/CLAUDE.md",
                "global/codex/AGENTS.md",
                "projects/github.com/Foo/Bar/claude/CLAUDE.md",
                "projects/github.com/Foo/Bar/claude/dot-claude/CLAUDE.md",
                "projects/github.com/Foo/Bar/claude/CLAUDE.local.md",
                "projects/github.com/Foo/Bar/claude/memory/a.md",
                "projects/github.com/Foo/Bar/claude/memory/sub/b.md",
                "projects/github.com/Foo/Bar/claude/memory/MEMORY.md",
                "projects/github.com/Foo/Bar/codex/AGENTS.md",
            ]
        );
        assert_eq!(m[0].project_key, None);
        assert_eq!(m[2].project_key.as_deref(), Some("github.com/Foo/Bar"));
        assert_eq!(m[2].file_id, index.projects[0].files[0].id);
    }

    #[test]
    fn no_remote_project_needs_inclusion() {
        let tmp = tempfile::tempdir().unwrap();
        let (index, _) = build_index(tmp.path(), false);
        let none = map_files(&index, &[]);
        assert!(none.iter().all(|f| f.project_key.is_none()), "{none:?}");
        assert_eq!(none.len(), 2);
        let inc = map_files(&index, &["local/myproj".to_string()]);
        assert!(inc.iter().any(|f| f.repo_path == "projects/local/myproj/claude/CLAUDE.md"));
        assert!(inc.iter().any(|f| f.repo_path == "projects/local/myproj/claude/memory/sub/b.md"));
        let other = map_files(&index, &["local/other".to_string()]);
        assert_eq!(other.len(), 2);
    }

    #[test]
    fn expected_local_path_maps_back() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let paths = AgentPaths::from_home(&home);
        let dir = tmp.path().join("work").join("myproj");
        fs::create_dir_all(&dir).unwrap();
        let projects = vec![("github.com/Foo/Bar".to_string(), dir.clone())];
        assert_eq!(
            expected_local_path("global/claude/CLAUDE.md", &projects, &paths),
            Some(paths.claude_home.join("CLAUDE.md"))
        );
        assert_eq!(
            expected_local_path("global/codex/AGENTS.md", &projects, &paths),
            Some(paths.codex_home.join("AGENTS.md"))
        );
        let k = "projects/github.com/Foo/Bar";
        assert_eq!(expected_local_path(&format!("{k}/claude/CLAUDE.md"), &projects, &paths), Some(dir.join("CLAUDE.md")));
        assert_eq!(
            expected_local_path(&format!("{k}/claude/dot-claude/CLAUDE.md"), &projects, &paths),
            Some(dir.join(".claude").join("CLAUDE.md"))
        );
        assert_eq!(expected_local_path(&format!("{k}/codex/AGENTS.md"), &projects, &paths), Some(dir.join("AGENTS.md")));
        assert_eq!(expected_local_path("projects/github.com/Other/X/claude/CLAUDE.md", &projects, &paths), None);
        assert_eq!(expected_local_path(&format!("{k}/claude/unknown.md"), &projects, &paths), None);
        assert_eq!(expected_local_path(&format!("{k}/claude/memory/../x.md"), &projects, &paths), None);
        // auto memory: no Claude project dir yet -> None
        assert_eq!(expected_local_path(&format!("{k}/claude/memory/a.md"), &projects, &paths), None);
        // with a Claude session for the project
        let enc = paths.claude_home.join("projects").join("enc");
        fs::create_dir_all(&enc).unwrap();
        let cwd = dir.to_string_lossy().replace('\\', "\\\\");
        fs::write(enc.join("s.jsonl"), format!("{{\"type\":\"user\",\"cwd\":\"{cwd}\"}}\n")).unwrap();
        assert_eq!(
            expected_local_path(&format!("{k}/claude/memory/sub/a.md"), &projects, &paths),
            Some(enc.join("memory").join("sub").join("a.md"))
        );
    }

    #[test]
    fn normalize_text_strips_bom_and_crlf() {
        assert_eq!(normalize_text(b"\xEF\xBB\xBFa\r\nb\r\n"), "a\nb\n");
        assert_eq!(normalize_text(b"plain\n"), "plain\n");
        assert!(normalize_text(&[0x61, 0xFF, 0x62]).contains('\u{FFFD}'));
    }

    #[test]
    fn machine_id_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("sync").join("machine.json");
        let a = load_or_create_machine(&p).unwrap();
        let b = load_or_create_machine(&p).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.id.len(), 36);
        assert!(!a.name.is_empty());
    }
}
