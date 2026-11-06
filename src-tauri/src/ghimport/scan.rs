//! Decides what in a repository tree can be imported.
//!
//! Rules (paths are compared case-insensitively, `.git` / `node_modules` are
//! skipped everywhere):
//! - **skill**: every directory holding a `SKILL.md`; all files below it belong
//!   to the skill and are never offered as memories.
//! - **memory** (`.md` files outside skills):
//!   `CLAUDE.md`, `CLAUDE.local.md`, `AGENTS.md` anywhere;
//!   any `.md` below a `.claude/` directory, except its `skills/`,
//!   `commands/`, `agents/`, `hooks/`, `plugins/` and `output-styles/`;
//!   any `.md` below a directory named `memory` or `memories`.
//! - A `blob` link to a single `.md` file offers that file whatever its name.

use serde::Serialize;

use super::fetch::{EntryKind, TreeEntry};

/// Memory files above this size are listed but cannot be imported.
pub const MAX_MEMORY_BYTES: u64 = 256 * 1024;
/// A skill above either limit is listed but cannot be installed.
pub const MAX_SKILL_FILES: usize = 300;
pub const MAX_SKILL_BYTES: u64 = 10 * 1024 * 1024;
/// At most this many items are listed.
pub const MAX_ITEMS: usize = 300;

const CLAUDE_EXCLUDED: [&str; 6] = ["skills", "commands", "agents", "hooks", "plugins", "output-styles"];

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MemoryFileKind {
    ClaudeMd,
    ClaudeLocal,
    AgentsMd,
    /// A file below `.claude/`.
    ClaudeDir,
    /// A file below `memory/` or `memories/`.
    MemoryDir,
    /// Picked directly through a blob link.
    File,
}

impl MemoryFileKind {
    fn rank(self) -> u8 {
        match self {
            MemoryFileKind::ClaudeMd => 0,
            MemoryFileKind::AgentsMd => 1,
            MemoryFileKind::ClaudeLocal => 2,
            MemoryFileKind::ClaudeDir => 3,
            MemoryFileKind::MemoryDir => 4,
            MemoryFileKind::File => 5,
        }
    }
}

#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemKind {
    Memory { memory_kind: MemoryFileKind },
    Skill { files: u32 },
}

#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
pub struct ImportItem {
    /// `m:<path>` or `s:<dir>`.
    pub id: String,
    /// File path (memory) or skill directory (`""` = repository root).
    pub path: String,
    /// Display name: file name, or skill directory name.
    pub name: String,
    /// Bytes (memory: the file; skill: all its files).
    pub size: u64,
    #[serde(flatten)]
    pub kind: ItemKind,
    /// Over the size / file count limits: shown, not importable.
    pub too_large: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Classified {
    pub items: Vec<ImportItem>,
    /// More than `MAX_ITEMS` candidates were found.
    pub truncated: bool,
}

fn lower_segments(p: &str) -> Vec<String> {
    p.split('/').map(|s| s.to_ascii_lowercase()).collect()
}

fn skipped(p: &str) -> bool {
    p.split('/').any(|s| s == ".git" || s == "node_modules")
}

fn under(path: &str, dir: &str) -> bool {
    dir.is_empty() || path.eq_ignore_ascii_case(dir) || {
        let pl = path.to_ascii_lowercase();
        pl.starts_with(&format!("{}/", dir.to_ascii_lowercase()))
    }
}

fn base_name(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// The memory kind of a `.md` path, or `None` when it is not a memory file.
pub fn memory_kind(path: &str) -> Option<MemoryFileKind> {
    let segs = lower_segments(path);
    let name = segs.last()?;
    if !name.ends_with(".md") {
        return None;
    }
    match name.as_str() {
        "claude.md" => return Some(MemoryFileKind::ClaudeMd),
        "claude.local.md" => return Some(MemoryFileKind::ClaudeLocal),
        "agents.md" => return Some(MemoryFileKind::AgentsMd),
        _ => {}
    }
    let dirs = &segs[..segs.len() - 1];
    if let Some(i) = dirs.iter().position(|s| s == ".claude") {
        let excluded = dirs.get(i + 1).is_some_and(|d| CLAUDE_EXCLUDED.contains(&d.as_str()));
        if !excluded {
            return Some(MemoryFileKind::ClaudeDir);
        }
    }
    if dirs.iter().any(|s| s == "memory" || s == "memories") {
        return Some(MemoryFileKind::MemoryDir);
    }
    None
}

/// Directory of `path` (`""` for a root file).
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// Classifies `entries` limited to `scope` (`""` = whole repository). With
/// `single_file`, only that file is considered (a blob link).
pub fn classify(entries: &[TreeEntry], scope: &str, single_file: Option<&str>) -> Classified {
    let blobs: Vec<&TreeEntry> = entries
        .iter()
        .filter(|e| e.kind == EntryKind::Blob && !skipped(&e.path))
        .collect();

    if let Some(file) = single_file {
        let Some(e) = blobs.iter().find(|e| e.path == file) else {
            return Classified::default();
        };
        // A link to a SKILL.md means the skill directory.
        if base_name(&e.path).eq_ignore_ascii_case("SKILL.md") {
            let dir = parent(&e.path);
            return Classified { items: vec![skill_item(dir, &blobs)], truncated: false };
        }
        if !e.path.to_ascii_lowercase().ends_with(".md") {
            return Classified::default();
        }
        let kind = memory_kind(&e.path).unwrap_or(MemoryFileKind::File);
        return Classified { items: vec![memory_item(e, kind)], truncated: false };
    }

    let in_scope: Vec<&TreeEntry> = blobs.iter().copied().filter(|e| under(&e.path, scope)).collect();
    let mut skill_dirs: Vec<String> = in_scope
        .iter()
        .filter(|e| base_name(&e.path).eq_ignore_ascii_case("SKILL.md"))
        .map(|e| parent(&e.path).to_string())
        .collect();
    skill_dirs.sort();
    skill_dirs.dedup();
    let in_skill = |p: &str| skill_dirs.iter().any(|d| under(p, d));

    let mut memories: Vec<ImportItem> = in_scope
        .iter()
        .filter(|e| !in_skill(&e.path))
        .filter_map(|e| memory_kind(&e.path).map(|k| memory_item(e, k)))
        .collect();
    memories.sort_by(|a, b| {
        let rank = |i: &ImportItem| match i.kind {
            ItemKind::Memory { memory_kind } => memory_kind.rank(),
            ItemKind::Skill { .. } => 9,
        };
        (rank(a), a.path.matches('/').count(), &a.path).cmp(&(rank(b), b.path.matches('/').count(), &b.path))
    });
    let skills: Vec<ImportItem> = skill_dirs.iter().map(|d| skill_item(d, &blobs)).collect();

    let mut items: Vec<ImportItem> = memories.into_iter().chain(skills).collect();
    let truncated = items.len() > MAX_ITEMS;
    items.truncate(MAX_ITEMS);
    Classified { items, truncated }
}

fn memory_item(e: &TreeEntry, kind: MemoryFileKind) -> ImportItem {
    ImportItem {
        id: format!("m:{}", e.path),
        path: e.path.clone(),
        name: base_name(&e.path).to_string(),
        size: e.size,
        kind: ItemKind::Memory { memory_kind: kind },
        too_large: e.size > MAX_MEMORY_BYTES,
    }
}

/// Files of the skill at `dir` (every blob below it), as paths relative to the repo root.
pub fn skill_files<'a>(dir: &str, entries: &'a [TreeEntry]) -> Vec<&'a TreeEntry> {
    entries
        .iter()
        .filter(|e| e.kind == EntryKind::Blob && !skipped(&e.path) && under(&e.path, dir) && e.path != dir)
        .collect()
}

fn skill_item(dir: &str, blobs: &[&TreeEntry]) -> ImportItem {
    let files: Vec<&&TreeEntry> = blobs.iter().filter(|e| under(&e.path, dir) && e.path != dir).collect();
    let size: u64 = files.iter().map(|e| e.size).sum();
    ImportItem {
        id: format!("s:{dir}"),
        path: dir.to_string(),
        name: if dir.is_empty() { String::new() } else { base_name(dir).to_string() },
        size,
        kind: ItemKind::Skill { files: files.len() as u32 },
        too_large: files.len() > MAX_SKILL_FILES || size > MAX_SKILL_BYTES,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(p: &str, size: u64) -> TreeEntry {
        TreeEntry { path: p.into(), kind: EntryKind::Blob, size }
    }

    fn tree() -> Vec<TreeEntry> {
        vec![
            blob("CLAUDE.md", 10),
            blob("README.md", 10),
            blob("AGENTS.md", 10),
            blob("docs/CLAUDE.local.md", 10),
            blob(".claude/CLAUDE.md", 10),
            blob(".claude/rules/style.md", 10),
            blob(".claude/commands/deploy.md", 10),
            blob(".claude/skills/pdf/SKILL.md", 10),
            blob(".claude/skills/pdf/ref/forms.md", 10),
            blob(".claude/skills/pdf/scripts/x.py", 30),
            blob("memory/MEMORY.md", 10),
            blob("notes/memories/user.md", 10),
            blob("notes/memories/data.json", 10),
            blob("skills/memory/SKILL.md", 10),
            blob("skills/memory/memory/notes.md", 10),
            blob("node_modules/x/CLAUDE.md", 10),
            blob("big/CLAUDE.md", MAX_MEMORY_BYTES + 1),
            TreeEntry { path: "memory".into(), kind: EntryKind::Tree, size: 0 },
        ]
    }

    fn ids(c: &Classified) -> Vec<String> {
        c.items.iter().map(|i| i.id.clone()).collect()
    }

    #[test]
    fn memory_kinds() {
        assert_eq!(memory_kind("a/CLAUDE.md"), Some(MemoryFileKind::ClaudeMd));
        assert_eq!(memory_kind("claude.local.md"), Some(MemoryFileKind::ClaudeLocal));
        assert_eq!(memory_kind("x/Agents.md"), Some(MemoryFileKind::AgentsMd));
        assert_eq!(memory_kind(".claude/rules/a.md"), Some(MemoryFileKind::ClaudeDir));
        assert_eq!(memory_kind(".claude/agents/a.md"), None);
        assert_eq!(memory_kind("Memories/a.md"), Some(MemoryFileKind::MemoryDir));
        assert_eq!(memory_kind("memory/a.txt"), None);
        assert_eq!(memory_kind("README.md"), None);
    }

    #[test]
    fn classifies_whole_repo() {
        let c = classify(&tree(), "", None);
        assert_eq!(
            ids(&c),
            vec![
                "m:CLAUDE.md",
                "m:.claude/CLAUDE.md",
                "m:big/CLAUDE.md",
                "m:AGENTS.md",
                "m:docs/CLAUDE.local.md",
                "m:.claude/rules/style.md",
                "m:memory/MEMORY.md",
                "m:notes/memories/user.md",
                "s:.claude/skills/pdf",
                "s:skills/memory",
            ]
        );
        let big = c.items.iter().find(|i| i.path == "big/CLAUDE.md").unwrap();
        assert!(big.too_large);
        let pdf = c.items.iter().find(|i| i.id == "s:.claude/skills/pdf").unwrap();
        assert_eq!(pdf.kind, ItemKind::Skill { files: 3 });
        assert_eq!(pdf.size, 50);
        assert_eq!(pdf.name, "pdf");
        assert!(!c.truncated);
    }

    #[test]
    fn scope_limits_to_subdir() {
        let c = classify(&tree(), "notes", None);
        assert_eq!(ids(&c), vec!["m:notes/memories/user.md"]);
        let c = classify(&tree(), ".claude/skills/pdf", None);
        assert_eq!(ids(&c), vec!["s:.claude/skills/pdf"]);
        // A prefix that is not a directory boundary does not match.
        let c = classify(&tree(), "note", None);
        assert!(c.items.is_empty());
    }

    #[test]
    fn single_file_links() {
        let c = classify(&tree(), "", Some("README.md"));
        assert_eq!(ids(&c), vec!["m:README.md"]);
        assert_eq!(c.items[0].kind, ItemKind::Memory { memory_kind: MemoryFileKind::File });
        let c = classify(&tree(), "", Some(".claude/skills/pdf/SKILL.md"));
        assert_eq!(ids(&c), vec!["s:.claude/skills/pdf"]);
        let c = classify(&tree(), "", Some(".claude/skills/pdf/scripts/x.py"));
        assert!(c.items.is_empty());
        let c = classify(&tree(), "", Some("missing.md"));
        assert!(c.items.is_empty());
    }

    #[test]
    fn root_skill_and_limits() {
        let mut t = vec![blob("SKILL.md", 1), blob("CLAUDE.md", 1)];
        let c = classify(&t, "", None);
        // Everything is inside the root skill.
        assert_eq!(ids(&c), vec!["s:"]);
        for i in 0..MAX_SKILL_FILES {
            t.push(blob(&format!("f{i}.txt"), 1));
        }
        let c = classify(&t, "", None);
        assert!(c.items[0].too_large);
        let many: Vec<TreeEntry> = (0..MAX_ITEMS + 5).map(|i| blob(&format!("memory/{i}.md"), 1)).collect();
        let c = classify(&many, "", None);
        assert_eq!(c.items.len(), MAX_ITEMS);
        assert!(c.truncated);
    }
}
