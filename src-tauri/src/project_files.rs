//! Files of a project folder for the composer's `@` menu.
//!
//! The list comes from `git ls-files -co --exclude-standard` when the folder is
//! in a git work tree (so `.gitignore` is respected), otherwise from a walk that
//! skips heavy folders (`node_modules`, `target`, `.git`, `dist`, `build`).
//! Directories are derived from the file paths. Lists are cached per folder for
//! 30 s; matching is a case-insensitive subsequence, ranked by basename hit,
//! then shorter path.
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::proc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectFile {
    /// Relative to the folder, forward slashes; directories end with `/`.
    pub path: String,
    pub is_dir: bool,
}

const MAX_ENTRIES: usize = 20_000;
const MAX_DEPTH: usize = 8;
const TTL: Duration = Duration::from_secs(30);
const GIT_TIMEOUT: Duration = Duration::from_secs(10);
const SKIP_DIRS: [&str; 5] = ["node_modules", "target", ".git", "dist", "build"];
pub const DEFAULT_LIMIT: usize = 50;

type Cache = HashMap<String, (Instant, Arc<Vec<ProjectFile>>)>;
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// Relative file paths from git, `None` when `cwd` is not in a work tree / git is missing.
fn git_files(cwd: &Path) -> Option<Vec<String>> {
    let mut cmd = Command::new("git");
    cmd.args(["-c", "core.quotepath=off", "ls-files", "-co", "--exclude-standard", "-z"])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = proc::spawn_hidden(&mut cmd).ok()?;
    let out = proc::wait_with_timeout(child, GIT_TIMEOUT, None).ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        out.stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .take(MAX_ENTRIES)
            .map(|p| String::from_utf8_lossy(p).replace('\\', "/"))
            .collect(),
    )
}

/// Relative file paths by walking `cwd` (heavy folders skipped, depth 8, 20000 entries).
fn walk_files(cwd: &Path) -> Vec<String> {
    walkdir::WalkDir::new(cwd)
        .max_depth(MAX_DEPTH)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !(e.file_type().is_dir() && SKIP_DIRS.iter().any(|d| e.file_name() == *d)))
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.path().strip_prefix(cwd).ok().map(|p| p.to_string_lossy().replace('\\', "/")))
        .take(MAX_ENTRIES)
        .collect()
}

/// Files plus every parent directory (`a/`, `a/b/`), sorted by path.
pub fn with_dirs(files: &[String]) -> Vec<ProjectFile> {
    let mut dirs = BTreeSet::new();
    for f in files {
        let mut end = 0;
        while let Some(i) = f[end..].find('/') {
            end += i + 1;
            dirs.insert(f[..end].to_string());
        }
    }
    let mut out: Vec<ProjectFile> = dirs.into_iter().map(|path| ProjectFile { path, is_dir: true }).collect();
    out.extend(files.iter().map(|f| ProjectFile { path: f.clone(), is_dir: false }));
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

/// Last path segment (`a/b/` -> `b`).
fn basename(path: &str) -> &str {
    let p = path.trim_end_matches('/');
    p.rsplit('/').next().unwrap_or(p)
}

/// Whether every char of `query` occurs in `text` in order (both already lowercase).
fn is_subsequence(query: &str, text: &str) -> bool {
    let mut it = text.chars();
    query.chars().all(|q| it.any(|c| c == q))
}

/// Entries matching `query` (case-insensitive subsequence of the path, spaces
/// ignored), best first: basename contains the query, then shorter path, then path.
pub fn rank(entries: &[ProjectFile], query: &str, limit: usize) -> Vec<ProjectFile> {
    let q: String = query.trim().trim_start_matches('@').to_lowercase().replace('\\', "/").chars().filter(|c| !c.is_whitespace()).collect();
    let mut hits: Vec<(bool, usize, &ProjectFile)> = entries
        .iter()
        .filter_map(|e| {
            let p = e.path.to_lowercase();
            is_subsequence(&q, &p).then(|| (!basename(&p).contains(&q), e.path.chars().count(), e))
        })
        .collect();
    hits.sort_by(|a, b| (a.0, a.1, &a.2.path).cmp(&(b.0, b.1, &b.2.path)));
    hits.into_iter().take(limit).map(|(_, _, e)| e.clone()).collect()
}

/// Cached file list of `cwd`.
pub fn list(cwd: &Path) -> Result<Arc<Vec<ProjectFile>>, String> {
    if !cwd.is_dir() {
        return Err(format!("目录不存在：{}", cwd.display()));
    }
    let key = cwd.to_string_lossy().into_owned();
    if let Some((at, v)) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(&key)) {
        if at.elapsed() < TTL {
            return Ok(v.clone());
        }
    }
    let files = git_files(cwd).unwrap_or_else(|| walk_files(cwd));
    let v = Arc::new(with_dirs(&files));
    let mut cache = CACHE.lock().unwrap();
    let cache = cache.get_or_insert_with(HashMap::new);
    cache.retain(|_, (at, _)| at.elapsed() < TTL);
    cache.insert(key, (Instant::now(), v.clone()));
    Ok(v)
}

pub fn search(cwd: &Path, query: &str, limit: usize) -> Result<Vec<ProjectFile>, String> {
    Ok(rank(&list(cwd)?, query, limit))
}

/// Files and folders of `cwd` matching `query`, for the `@` menu.
#[tauri::command]
pub async fn project_files(cwd: String, query: String, limit: Option<u32>) -> Result<Vec<ProjectFile>, String> {
    let limit = limit.map(|l| l as usize).unwrap_or(DEFAULT_LIMIT);
    tauri::async_runtime::spawn_blocking(move || search(Path::new(&cwd), &query, limit))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(v: &[ProjectFile]) -> Vec<&str> {
        v.iter().map(|e| e.path.as_str()).collect()
    }

    #[test]
    fn derives_dirs_from_files() {
        let v = with_dirs(&["src/api/live.ts".into(), "src/main.ts".into(), "README.md".into()]);
        assert_eq!(paths(&v), vec!["README.md", "src/", "src/api/", "src/api/live.ts", "src/main.ts"]);
        assert!(v[1].is_dir && !v[0].is_dir);
    }

    #[test]
    fn subsequence_is_ordered() {
        assert!(is_subsequence("slt", "src/api/live.ts"));
        assert!(is_subsequence("", "x"));
        assert!(!is_subsequence("tls", "src/api/live.ts"));
    }

    #[test]
    fn ranks_basename_hits_then_shorter_paths() {
        let v = with_dirs(&[
            "src/live/models.rs".into(),
            "models.md".into(),
            "src/my/odd/elements/s.rs".into(),
            "docs/Models/readme.md".into(),
        ]);
        let r = rank(&v, "Models", 10);
        // Basename hits first (shortest first), then subsequence-only matches.
        assert_eq!(
            paths(&r),
            vec![
                "models.md",
                "docs/Models/",
                "src/live/models.rs",
                "src/my/odd/elements/",
                "docs/Models/readme.md",
                "src/my/odd/elements/s.rs"
            ]
        );
        assert_eq!(rank(&v, "models", 2).len(), 2);
        assert!(rank(&v, "zzz", 10).is_empty());
        // `@` prefix and case are ignored; empty query lists the shortest paths first.
        assert_eq!(paths(&rank(&v, "@MODELS.MD", 1)), vec!["models.md"]);
        assert_eq!(paths(&rank(&v, "", 2)), vec!["src/", "docs/"]);
    }

    #[test]
    fn walk_skips_heavy_dirs() {
        let d = tempfile::tempdir().unwrap();
        for f in ["a/b.txt", "node_modules/x/y.js", "target/debug/z", "c.rs"] {
            let p = d.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let mut f = walk_files(d.path());
        f.sort();
        assert_eq!(f, vec!["a/b.txt", "c.rs"]);
    }

    /// `cargo test --lib project_files::tests::real -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real() {
        let cwd = std::env::var("MEMDOCK_FILES_CWD").unwrap_or_else(|_| "C:/codes/cc-memory".into());
        let t = Instant::now();
        let all = list(Path::new(&cwd)).unwrap();
        println!("{} entries in {:?}", all.len(), t.elapsed());
        assert!(!all.iter().any(|e| e.path.starts_with("node_modules/") || e.path.contains("/target/")));
        for q in ["", "models", "live.ts", "srcpty"] {
            let t = Instant::now();
            let r = search(Path::new(&cwd), q, 8).unwrap();
            println!("{q:?} ({:?}): {:?}", t.elapsed(), paths(&r));
        }
    }
}
