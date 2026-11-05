use serde::Serialize;

/// The coding agent tools Memdock manages.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&Tool::Claude).unwrap(), "\"claude\"");
        assert_eq!(serde_json::to_string(&Tool::Codex).unwrap(), "\"codex\"");
    }
}
