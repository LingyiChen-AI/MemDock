use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use crate::model::{
    MemoryFile, MemoryIndex, MemoryKind, Project, ProjectMemories, Scope, Tool,
};
use crate::paths::AgentPaths;
use crate::projects::{claude_project_dirs, normalize_path};

/// Builds a `MemoryFile` entry for `path`; missing files get size 0 and no mtime.
fn entry(path: &Path, tool: Tool, scope: Scope, kind: MemoryKind) -> MemoryFile {
    let path_str = path.to_string_lossy().into_owned();
    let link_meta = fs::symlink_metadata(path).ok();
    let is_symlink = link_meta
        .as_ref()
        .is_some_and(|m| m.file_type().is_symlink());
    // Follow symlinks for size/mtime; a dangling link falls back to the link itself.
    let meta = fs::metadata(path).ok().or(link_meta.clone());
    let exists = link_meta.is_some();
    let (size_bytes, modified_at) = match (&meta, exists) {
        (Some(m), true) => (
            m.len(),
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64),
        ),
        _ => (0, None),
    };
    MemoryFile {
        id: normalize_path(&path_str),
        tool,
        scope,
        kind,
        path: path_str,
        exists,
        is_symlink,
        size_bytes,
        modified_at,
    }
}

fn project_files(project: &Project, claude_dir: Option<&Path>) -> Vec<MemoryFile> {
    let scope = || Scope::Project {
        project_id: project.id.clone(),
    };
    let root = Path::new(&project.path);
    let mut files = Vec::new();
    let candidates = [
        (root.join("CLAUDE.md"), Tool::Claude, MemoryKind::ClaudeInstructions),
        (
            root.join(".claude").join("CLAUDE.md"),
            Tool::Claude,
            MemoryKind::ClaudeInstructions,
        ),
        (root.join("CLAUDE.local.md"), Tool::Claude, MemoryKind::ClaudeLocal),
        (root.join("AGENTS.md"), Tool::Codex, MemoryKind::CodexAgents),
    ];
    for (path, tool, kind) in candidates {
        // symlink_metadata so dangling links are still listed.
        if fs::symlink_metadata(&path).is_ok() {
            files.push(entry(&path, tool, scope(), kind));
        }
    }
    if let Some(dir) = claude_dir {
        let mut autos: Vec<_> = fs::read_dir(dir.join("memory"))
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "md") && p.is_file())
            .collect();
        autos.sort_by_key(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
            let name = name.unwrap_or_default();
            (name != "MEMORY.md", name)
        });
        for p in autos {
            files.push(entry(&p, Tool::Claude, scope(), MemoryKind::ClaudeAuto));
        }
    }
    files
}

/// Lists all global and per-project memory files.
pub fn scan_all(paths: &AgentPaths, projects: &[Project]) -> MemoryIndex {
    let mut global = vec![
        entry(
            &paths.claude_home.join("CLAUDE.md"),
            Tool::Claude,
            Scope::Global,
            MemoryKind::ClaudeInstructions,
        ),
        entry(
            &paths.codex_home.join("AGENTS.md"),
            Tool::Codex,
            Scope::Global,
            MemoryKind::CodexAgents,
        ),
    ];
    let db = paths.codex_home.join("memories_1.sqlite");
    if fs::symlink_metadata(&db).is_ok() {
        let mut f = entry(&db, Tool::Codex, Scope::Global, MemoryKind::CodexDb);
        f.id = "codex-db".to_string();
        global.push(f);
    }
    let dirs = claude_project_dirs(paths);
    let projects = projects
        .iter()
        .map(|p| ProjectMemories {
            project: p.clone(),
            files: project_files(p, dirs.get(&p.id).map(|d| d.as_path())),
        })
        .collect();
    MemoryIndex { global, projects }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProjectSource;
    use tempfile::TempDir;

    fn setup() -> (TempDir, AgentPaths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AgentPaths::from_home(tmp.path());
        fs::create_dir_all(&paths.claude_home).unwrap();
        fs::create_dir_all(&paths.codex_home).unwrap();
        (tmp, paths)
    }

    fn project(path: &Path) -> Project {
        let s = path.to_string_lossy().into_owned();
        Project {
            id: normalize_path(&s),
            path: s,
            name: "proj".into(),
            exists: path.exists(),
            sources: vec![ProjectSource::Manual],
        }
    }

    /// Fakes a Claude session so `claude_project_dirs` maps `proj` to `<claude>/projects/enc`.
    fn fake_session(paths: &AgentPaths, proj: &Path) -> std::path::PathBuf {
        let dir = paths.claude_home.join("projects").join("enc");
        fs::create_dir_all(&dir).unwrap();
        let cwd = proj.to_string_lossy().replace('\\', "\\\\");
        fs::write(
            dir.join("s.jsonl"),
            format!("{{\"type\":\"user\",\"cwd\":\"{cwd}\"}}\n"),
        )
        .unwrap();
        dir
    }

    #[test]
    fn lists_global_even_if_missing() {
        let (_t, paths) = setup();
        let idx = scan_all(&paths, &[]);
        assert_eq!(idx.global.len(), 2);
        assert_eq!(idx.global[0].kind, MemoryKind::ClaudeInstructions);
        assert_eq!(idx.global[1].kind, MemoryKind::CodexAgents);
        assert!(idx.global.iter().all(|f| !f.exists && f.size_bytes == 0));
        assert!(idx.global.iter().all(|f| f.modified_at.is_none()));
        assert!(idx.global.iter().all(|f| f.scope == Scope::Global));
    }

    #[test]
    fn existing_global_has_size_and_mtime() {
        let (_t, paths) = setup();
        fs::write(paths.claude_home.join("CLAUDE.md"), "hello").unwrap();
        let idx = scan_all(&paths, &[]);
        let f = &idx.global[0];
        assert!(f.exists);
        assert_eq!(f.size_bytes, 5);
        assert!(f.modified_at.unwrap() > 0);
        assert_eq!(f.id, normalize_path(&f.path));
    }

    #[test]
    fn lists_project_instruction_files() {
        let (t, paths) = setup();
        let proj = t.path().join("work");
        fs::create_dir_all(&proj).unwrap();
        for n in ["CLAUDE.md", "CLAUDE.local.md", "AGENTS.md"] {
            fs::write(proj.join(n), "x").unwrap();
        }
        let idx = scan_all(&paths, &[project(&proj)]);
        let files = &idx.projects[0].files;
        let kinds: Vec<_> = files.iter().map(|f| f.kind).collect();
        assert_eq!(
            kinds,
            vec![
                MemoryKind::ClaudeInstructions,
                MemoryKind::ClaudeLocal,
                MemoryKind::CodexAgents
            ]
        );
        assert_eq!(files[2].tool, Tool::Codex);
    }

    #[test]
    fn lists_dot_claude_claude_md() {
        let (t, paths) = setup();
        let proj = t.path().join("work");
        fs::create_dir_all(proj.join(".claude")).unwrap();
        fs::write(proj.join(".claude").join("CLAUDE.md"), "x").unwrap();
        let idx = scan_all(&paths, &[project(&proj)]);
        assert_eq!(idx.projects[0].files.len(), 1);
        assert_eq!(idx.projects[0].files[0].kind, MemoryKind::ClaudeInstructions);
    }

    #[test]
    fn lists_auto_memory_with_index_first() {
        let (t, paths) = setup();
        let proj = t.path().join("work");
        fs::create_dir_all(&proj).unwrap();
        let enc = fake_session(&paths, &proj);
        let mem = enc.join("memory");
        fs::create_dir_all(&mem).unwrap();
        for n in ["b.md", "MEMORY.md", "a.md", "notes.txt"] {
            fs::write(mem.join(n), "m").unwrap();
        }
        let idx = scan_all(&paths, &[project(&proj)]);
        let names: Vec<_> = idx.projects[0]
            .files
            .iter()
            .map(|f| {
                assert_eq!(f.kind, MemoryKind::ClaudeAuto);
                Path::new(&f.path).file_name().unwrap().to_string_lossy().into_owned()
            })
            .collect();
        assert_eq!(names, vec!["MEMORY.md", "a.md", "b.md"]);
    }

    #[test]
    fn missing_project_dir_is_flagged() {
        let (t, paths) = setup();
        let proj = t.path().join("gone");
        let enc = fake_session(&paths, &proj);
        fs::create_dir_all(enc.join("memory")).unwrap();
        fs::write(enc.join("memory").join("MEMORY.md"), "m").unwrap();
        let idx = scan_all(&paths, &[project(&proj)]);
        let pm = &idx.projects[0];
        assert!(!pm.project.exists);
        assert_eq!(pm.files.len(), 1);
        assert_eq!(pm.files[0].kind, MemoryKind::ClaudeAuto);
    }

    #[test]
    fn codex_db_listed_when_present() {
        let (_t, paths) = setup();
        assert_eq!(scan_all(&paths, &[]).global.len(), 2);
        fs::write(paths.codex_home.join("memories_1.sqlite"), "db").unwrap();
        let idx = scan_all(&paths, &[]);
        assert_eq!(idx.global.len(), 3);
        let db = &idx.global[2];
        assert_eq!(db.kind, MemoryKind::CodexDb);
        assert_eq!(db.id, "codex-db");
        assert!(db.exists);
        assert_eq!(db.size_bytes, 2);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_flagged() {
        let (t, paths) = setup();
        let target = t.path().join("real.md");
        fs::write(&target, "abc").unwrap();
        std::os::unix::fs::symlink(&target, paths.claude_home.join("CLAUDE.md")).unwrap();
        let idx = scan_all(&paths, &[]);
        let f = &idx.global[0];
        assert!(f.is_symlink && f.exists);
        assert_eq!(f.size_bytes, 3);
        assert!(f.path.ends_with("CLAUDE.md"));
    }
}
