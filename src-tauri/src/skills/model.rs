use serde::{Deserialize, Serialize};

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
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SkillScope {
    Global,
    Project { project_id: String },
}

/// How a skill's source was determined.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SourceOrigin {
    Registry,
    GitDir,
    SymlinkRepo,
    SkillLock,
    Manual,
}

/// Origin of an installed skill.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct SkillSource {
    /// Normalized: `https://github.com/owner/repo` (no `.git`, no trailing `/`).
    pub url: String,
    /// Tracking ref (branch or tag) requested at install; `None` = remote HEAD.
    pub git_ref: Option<String>,
    /// Commit the install corresponds to, when known.
    #[serde(default)]
    pub commit: Option<String>,
    /// Path inside the repository; `""` when the skill is the repo root.
    pub skill_path: String,
    pub origin: SourceOrigin,
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
