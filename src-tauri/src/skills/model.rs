use serde::Serialize;

use crate::model::Tool;

/// Where a skill comes from, which decides whether it can be edited.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SkillKind {
    User,
    Plugin,
    CodexSystem,
    ClaudeSynced,
}

/// Whether a skill is installed globally or inside one project.
#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SkillScope {
    Global,
    Project { project_id: String },
}

/// Origin of an installed skill (filled in by the source-tracking step).
#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
pub struct SkillSource {
    pub url: String,
}

/// A skill directory containing a `SKILL.md`.
#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct SkillEntry {
    /// `normalize_path(dir)`.
    pub id: String,
    /// Frontmatter `name`, falling back to the directory name.
    pub name: String,
    pub description: String,
    pub tool: Tool,
    pub scope: SkillScope,
    pub kind: SkillKind,
    pub dir: String,
    pub is_symlink: bool,
    pub symlink_target: Option<String>,
    /// True for everything except user skills.
    pub readonly: bool,
    /// Plugin key such as `superpowers@claude-plugins-official`.
    pub plugin: Option<String>,
    pub source: Option<SkillSource>,
    pub file_count: u32,
    pub size_bytes: u64,
    /// Unix milliseconds of `SKILL.md`.
    pub modified_at: Option<i64>,
}
