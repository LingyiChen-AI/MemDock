use serde::{Deserialize, Serialize};

/// The coding agent tools Memdock manages.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    Claude,
    Codex,
}

/// Where a project was discovered from.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSource {
    ClaudeSessions,
    CodexSessions,
    Manual,
}

/// A project directory known to Memdock.
#[derive(Serialize, Clone, Debug)]
pub struct Project {
    pub id: String,
    pub path: String,
    pub name: String,
    pub exists: bool,
    pub sources: Vec<ProjectSource>,
}

/// Where a memory file lives.
#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Scope {
    Global,
    Project { project_id: String },
}

/// What kind of memory a file holds.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    ClaudeInstructions,
    ClaudeLocal,
    ClaudeAuto,
    CodexAgents,
    CodexDb,
}

/// A memory file (or the Codex memory database) known to Memdock.
#[derive(Serialize, Clone, Debug)]
pub struct MemoryFile {
    pub id: String,
    pub tool: Tool,
    pub scope: Scope,
    pub kind: MemoryKind,
    pub path: String,
    pub exists: bool,
    pub is_symlink: bool,
    pub size_bytes: u64,
    /// Unix milliseconds.
    pub modified_at: Option<i64>,
}

/// A project together with its memory files.
#[derive(Serialize, Clone, Debug)]
pub struct ProjectMemories {
    pub project: Project,
    pub files: Vec<MemoryFile>,
}

/// All memory files: global ones plus one group per project.
#[derive(Serialize, Clone, Debug)]
pub struct MemoryIndex {
    pub global: Vec<MemoryFile>,
    pub projects: Vec<ProjectMemories>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&Tool::Claude).unwrap(), "\"claude\"");
        assert_eq!(serde_json::to_string(&Tool::Codex).unwrap(), "\"codex\"");
    }
}
