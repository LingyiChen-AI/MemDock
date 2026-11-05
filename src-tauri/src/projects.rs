use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::model::{Project, ProjectSource};
use crate::paths::AgentPaths;

/// Max lines read from the head of each session file when looking for a cwd.
const MAX_LINES: usize = 200;

/// Normalizes a path into a stable project id: `/` separators, no trailing `/`,
/// lowercased on Windows (case-insensitive file system).
pub fn normalize_path(p: &str) -> String {
    let mut s = p.replace('\\', "/");
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    if cfg!(windows) {
        s = s.to_lowercase();
    }
    s
}

/// Scans the first `MAX_LINES` lines of a jsonl file; returns the first string
/// produced by `pick` for a parsed line. Unparsable lines are skipped.
fn scan_jsonl<F>(path: &Path, limit: usize, pick: F) -> Option<String>
where
    F: Fn(&serde_json::Value) -> Option<String>,
{
    let reader = BufReader::new(File::open(path).ok()?);
    for line in reader.lines().take(limit) {
        let Ok(line) = line else { continue };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(found) = pick(&v) {
                return Some(found);
            }
        }
    }
    None
}

fn top_level_cwd(v: &serde_json::Value) -> Option<String> {
    v.get("cwd")?.as_str().filter(|s| !s.is_empty()).map(String::from)
}

/// Claude project dirs as (cwd, encoded dir), in directory-name order.
fn claude_entries(paths: &AgentPaths) -> Vec<(String, PathBuf)> {
    let Ok(rd) = fs::read_dir(paths.claude_home.join("projects")) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        files.sort();
        if let Some(cwd) = files
            .iter()
            .find_map(|f| scan_jsonl(f, MAX_LINES, top_level_cwd))
        {
            out.push((cwd, dir));
        }
    }
    out
}

/// Maps project id to the Claude encoded project directory (for auto-memory lookup).
pub fn claude_project_dirs(paths: &AgentPaths) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();
    for (cwd, dir) in claude_entries(paths) {
        map.entry(normalize_path(&cwd)).or_insert(dir);
    }
    map
}

fn codex_cwds(paths: &AgentPaths) -> Vec<String> {
    let mut files: Vec<PathBuf> = WalkDir::new(paths.codex_home.join("sessions"))
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            let n = e.file_name().to_string_lossy();
            n.starts_with("rollout-") && n.ends_with(".jsonl")
        })
        .map(|e| e.into_path())
        .collect();
    files.sort();
    files
        .iter()
        .filter_map(|f| {
            // The session_meta record is the first line.
            scan_jsonl(f, 1, |v| {
                if v.get("type")?.as_str()? != "session_meta" {
                    return None;
                }
                v.get("payload")?
                    .get("cwd")?
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(String::from)
            })
        })
        .collect()
}

fn project_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_string()
}

/// Discovers projects from Claude/Codex session logs plus manual entries,
/// deduplicated by normalized path and sorted by name.
pub fn discover_projects(paths: &AgentPaths, manual: &[String]) -> Vec<Project> {
    let mut by_id: HashMap<String, Project> = HashMap::new();
    let mut add = |path: String, source: ProjectSource| {
        let id = normalize_path(&path);
        let entry = by_id.entry(id.clone()).or_insert_with(|| Project {
            id,
            name: project_name(&path),
            exists: Path::new(&path).is_dir(),
            path,
            sources: Vec::new(),
        });
        if !entry.sources.contains(&source) {
            entry.sources.push(source);
        }
    };
    for (cwd, _) in claude_entries(paths) {
        add(cwd, ProjectSource::ClaudeSessions);
    }
    for cwd in codex_cwds(paths) {
        add(cwd, ProjectSource::CodexSessions);
    }
    for m in manual {
        if !m.trim().is_empty() {
            add(m.clone(), ProjectSource::Manual);
        }
    }
    let mut list: Vec<Project> = by_id.into_values().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn write_lines(path: &Path, lines: &[String]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, lines.join("\n") + "\n").unwrap();
    }

    fn claude_file(paths: &AgentPaths, dir: &str, file: &str, lines: &[String]) {
        write_lines(&paths.claude_home.join("projects").join(dir).join(file), lines);
    }

    fn codex_file(paths: &AgentPaths, name: &str, cwd: &str) {
        let line = json!({"type":"session_meta","payload":{"cwd":cwd}}).to_string();
        write_lines(
            &paths.codex_home.join("sessions/2026/11/05").join(name),
            &[line],
        );
    }

    #[test]
    fn discovers_from_claude_jsonl_cwd() {
        let tmp = TempDir::new().unwrap();
        let paths = AgentPaths::from_home(tmp.path());
        let proj = tmp.path().join("a");
        fs::create_dir_all(&proj).unwrap();
        let cwd = proj.to_string_lossy().to_string();
        claude_file(
            &paths,
            "C--x-a",
            "s1.jsonl",
            &[
                json!({"type":"summary"}).to_string(),
                json!({"cwd": cwd}).to_string(),
            ],
        );
        let list = discover_projects(&paths, &[]);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "a");
        assert_eq!(list[0].sources, vec![ProjectSource::ClaudeSessions]);
        assert!(list[0].exists);
        let dirs = claude_project_dirs(&paths);
        assert_eq!(
            dirs.get(&list[0].id).unwrap(),
            &paths.claude_home.join("projects").join("C--x-a")
        );
    }

    #[test]
    fn discovers_from_codex_session_meta() {
        let tmp = TempDir::new().unwrap();
        let paths = AgentPaths::from_home(tmp.path());
        codex_file(&paths, "rollout-2026-11-05T10-00-00-abc.jsonl", "/work/proj-b");
        codex_file(&paths, "other.jsonl", "/work/ignored");
        let list = discover_projects(&paths, &[]);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "proj-b");
        assert_eq!(list[0].sources, vec![ProjectSource::CodexSessions]);
        assert!(!list[0].exists);
    }

    #[cfg(windows)]
    #[test]
    fn dedupes_same_project_different_forms() {
        let tmp = TempDir::new().unwrap();
        let paths = AgentPaths::from_home(tmp.path());
        claude_file(
            &paths,
            "C--codes-x",
            "s.jsonl",
            &[json!({"cwd": "C:\\codes\\x"}).to_string()],
        );
        codex_file(&paths, "rollout-1.jsonl", "c:/codes/x");
        let list = discover_projects(&paths, &[]);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].path, "C:\\codes\\x");
        assert!(list[0].sources.contains(&ProjectSource::ClaudeSessions));
        assert!(list[0].sources.contains(&ProjectSource::CodexSessions));
    }

    #[test]
    fn dedupes_trailing_slash() {
        let tmp = TempDir::new().unwrap();
        let paths = AgentPaths::from_home(tmp.path());
        claude_file(
            &paths,
            "d",
            "s.jsonl",
            &[json!({"cwd": "/a/b/"}).to_string()],
        );
        codex_file(&paths, "rollout-2.jsonl", "/a/b");
        let list = discover_projects(&paths, &[]);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "b");
        assert_eq!(list[0].sources.len(), 2);
        assert_eq!(normalize_path("/a/b/"), normalize_path("/a/b"));
    }

    #[test]
    fn manual_projects_included() {
        let tmp = TempDir::new().unwrap();
        let paths = AgentPaths::from_home(tmp.path());
        let list = discover_projects(&paths, &["/m/one".to_string()]);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].sources, vec![ProjectSource::Manual]);
        assert_eq!(list[0].name, "one");
    }

    #[test]
    fn reads_at_most_200_lines() {
        let tmp = TempDir::new().unwrap();
        let paths = AgentPaths::from_home(tmp.path());
        let mut lines: Vec<String> = (0..200).map(|_| json!({"type":"x"}).to_string()).collect();
        lines.push(json!({"cwd": "/late/proj"}).to_string());
        claude_file(&paths, "late", "s.jsonl", &lines);
        assert!(discover_projects(&paths, &[]).is_empty());
    }
}
