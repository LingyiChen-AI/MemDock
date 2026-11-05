use std::path::Path;

use serde::Serialize;

use super::io::read_memory;
use crate::model::{MemoryIndex, MemoryKind};

/// Maximum number of characters kept in `SearchHit::preview`.
const PREVIEW_CHARS: usize = 200;

/// One matching line in a memory file.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SearchHit {
    pub file_id: String,
    /// 1-based line number.
    pub line: u32,
    /// The matching line, truncated to 200 characters.
    pub preview: String,
}

/// Case-insensitive substring search across all existing memory files.
///
/// Global files are searched first, then projects in index order; lines are
/// visited in order within each file. Files that fail to read are skipped.
/// Blank queries and `limit == 0` yield no hits.
pub fn search(index: &MemoryIndex, query: &str, limit: usize) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let needle = query.trim().to_lowercase();
    if needle.is_empty() || limit == 0 {
        return hits;
    }

    let files = index
        .global
        .iter()
        .chain(index.projects.iter().flat_map(|p| p.files.iter()));
    for file in files {
        if !file.exists || file.kind == MemoryKind::CodexDb {
            continue;
        }
        let Ok(content) = read_memory(Path::new(&file.path)) else {
            continue;
        };
        for (i, line) in content.text.lines().enumerate() {
            if !line.to_lowercase().contains(&needle) {
                continue;
            }
            hits.push(SearchHit {
                file_id: file.id.clone(),
                line: (i + 1) as u32,
                preview: line.chars().take(PREVIEW_CHARS).collect(),
            });
            if hits.len() >= limit {
                return hits;
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MemoryFile, Project, ProjectMemories, Scope, Tool};
    use std::fs;
    use std::path::PathBuf;

    fn file(id: &str, path: PathBuf, kind: MemoryKind, exists: bool) -> MemoryFile {
        MemoryFile {
            id: id.to_string(),
            tool: Tool::Claude,
            scope: Scope::Global,
            kind,
            path: path.to_string_lossy().into_owned(),
            exists,
            is_symlink: false,
            size_bytes: 0,
            modified_at: None,
        }
    }

    fn index_with(global: Vec<MemoryFile>, projects: Vec<ProjectMemories>) -> MemoryIndex {
        MemoryIndex { global, projects }
    }

    fn project(id: &str, files: Vec<MemoryFile>) -> ProjectMemories {
        ProjectMemories {
            project: Project {
                id: id.to_string(),
                path: format!("/work/{id}"),
                name: id.to_string(),
                exists: true,
                sources: vec![],
            },
            files,
        }
    }

    #[test]
    fn finds_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("CLAUDE.md");
        fs::write(&p, "Hello World\nno match here\nHELLO again\n").unwrap();
        let idx = index_with(
            vec![file("g1", p, MemoryKind::ClaudeInstructions, true)],
            vec![],
        );

        let hits = search(&idx, "hello", 10);

        assert_eq!(
            hits,
            vec![
                SearchHit {
                    file_id: "g1".into(),
                    line: 1,
                    preview: "Hello World".into(),
                },
                SearchHit {
                    file_id: "g1".into(),
                    line: 3,
                    preview: "HELLO again".into(),
                },
            ]
        );
    }

    #[test]
    fn finds_cjk() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("AGENTS.md");
        fs::write(&p, "第一行\n这里是记忆文件内容\n第三行").unwrap();
        let idx = index_with(
            vec![file("g1", p, MemoryKind::CodexAgents, true)],
            vec![],
        );

        let hits = search(&idx, " 记忆 ", 10);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 2);
        assert_eq!(hits[0].preview, "这里是记忆文件内容");
    }

    #[test]
    fn respects_limit() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("CLAUDE.md");
        fs::write(&p, "todo 1\ntodo 2\ntodo 3\ntodo 4\ntodo 5\n").unwrap();
        let idx = index_with(
            vec![file("g1", p, MemoryKind::ClaudeInstructions, true)],
            vec![],
        );

        let hits = search(&idx, "todo", 2);

        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].line, 1);
        assert_eq!(hits[1].line, 2);
    }

    #[test]
    fn blank_query_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("CLAUDE.md");
        fs::write(&p, "anything\n").unwrap();
        let idx = index_with(
            vec![file("g1", p, MemoryKind::ClaudeInstructions, true)],
            vec![],
        );

        assert!(search(&idx, "   ", 10).is_empty());
        assert!(search(&idx, "", 10).is_empty());
    }

    #[test]
    fn skips_missing_files_and_codex_db() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.md");
        let db = dir.path().join("state.sqlite");
        fs::write(&missing, "needle\n").unwrap();
        fs::write(&db, "needle\n").unwrap();
        let idx = index_with(
            vec![
                file("gone", missing, MemoryKind::ClaudeInstructions, false),
                file("db", db, MemoryKind::CodexDb, true),
            ],
            vec![],
        );

        assert!(search(&idx, "needle", 10).is_empty());
    }

    #[test]
    fn searches_global_before_projects_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let g = dir.path().join("global.md");
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        fs::write(&g, "key\n").unwrap();
        fs::write(&a, "key in a\n").unwrap();
        fs::write(&b, "key in b\n").unwrap();
        let idx = index_with(
            vec![file("g", g, MemoryKind::ClaudeInstructions, true)],
            vec![
                project("pa", vec![file("a", a, MemoryKind::ClaudeLocal, true)]),
                project("pb", vec![file("b", b, MemoryKind::ClaudeAuto, true)]),
            ],
        );

        let ids: Vec<String> = search(&idx, "key", 10)
            .into_iter()
            .map(|h| h.file_id)
            .collect();

        assert_eq!(ids, vec!["g", "a", "b"]);
    }

    #[test]
    fn preview_truncates_by_characters() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("CLAUDE.md");
        let long: String = "记".repeat(300);
        fs::write(&p, format!("{long}\n")).unwrap();
        let idx = index_with(
            vec![file("g1", p, MemoryKind::ClaudeInstructions, true)],
            vec![],
        );

        let hits = search(&idx, "记", 10);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].preview.chars().count(), 200);
        assert_eq!(hits[0].preview, "记".repeat(200));
    }
}
