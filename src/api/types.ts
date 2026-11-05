// Hand-written mirror of the Rust types in src-tauri/src (serde names).

export type Tool = "claude" | "codex";
export type ProjectSource = "claude_sessions" | "codex_sessions" | "manual";

export interface Project {
  id: string;
  path: string;
  name: string;
  exists: boolean;
  sources: ProjectSource[];
}

export type Scope = { type: "global" } | { type: "project"; project_id: string };

export type MemoryKind =
  | "claude_instructions"
  | "claude_local"
  | "claude_auto"
  | "codex_agents"
  | "codex_db";

export interface MemoryFile {
  id: string;
  tool: Tool;
  scope: Scope;
  kind: MemoryKind;
  path: string;
  exists: boolean;
  is_symlink: boolean;
  size_bytes: number;
  /** Unix milliseconds. */
  modified_at: number | null;
}

export interface ProjectMemories {
  project: Project;
  files: MemoryFile[];
}

export interface MemoryIndex {
  global: MemoryFile[];
  projects: ProjectMemories[];
}

export interface ImportRef {
  raw: string;
  resolved: string;
  exists: boolean;
  /** 1-based line number. */
  line: number;
}

export interface FileContentDto {
  text: string;
  hash: string;
  has_bom: boolean;
  crlf: boolean;
  tokens: number;
  imports: ImportRef[];
}

export type CommandErrorCode =
  | "disk_changed"
  | "io"
  | "forbidden"
  | "invalid"
  // Skill management
  | "exists"
  | "read_only"
  | "git_unavailable"
  | "git_failed"
  | "local_changes";

/** Error thrown by failing commands. `current_hash` is set for `disk_changed`. */
export interface CommandError {
  code: CommandErrorCode;
  message: string;
  current_hash?: string;
  /** Set for skill `exists`: the first directory that is in the way. */
  dir?: string;
  /** Set for skill `exists`: every directory that would be overwritten. */
  dirs?: string[];
  /** Set for skill `local_changes`: no baseline exists, local edits cannot be ruled out. */
  unknown?: boolean;
}

export interface SearchHit {
  file_id: string;
  line: number;
  preview: string;
}

export interface SnapshotInfo {
  id: string;
  created_at: number;
  size_bytes: number;
}

export interface CodexMemoryEntry {
  thread_id: string;
  raw_memory: string;
  rollout_summary: string;
  generated_at: number;
}

export type CodexDbResult =
  | { status: "ok"; entries: CodexMemoryEntry[] }
  | { status: "unavailable"; reason: string };

export type Accent = "green" | "blue" | "violet" | "orange" | "teal";

export type Theme = "dark" | "light" | "system";

export interface Settings {
  language: string | null;
  theme: string | null;
  /** "green" | "blue" | "violet" | "orange" | "teal"; null = green. */
  accent: string | null;
  manual_projects: string[];
}

export interface MemoryChangedPayload {
  paths: string[];
}

// ---------------------------------------------------------------------------
// Skills (mirrors src-tauri/src/skills and commands_skills.rs serde names)
// ---------------------------------------------------------------------------

export type SkillKind = "user" | "plugin" | "codex_system" | "claude_synced";

export type SkillScope = { type: "global" } | { type: "project"; project_id: string };

export type SourceOrigin = "registry" | "git_dir" | "symlink_repo" | "skill_lock" | "manual";

export interface SkillSource {
  /** Normalized: https://github.com/owner/repo */
  url: string;
  /** Tracking ref (branch or tag); null = remote HEAD. */
  git_ref: string | null;
  commit: string | null;
  /** Path inside the repository; "" when the skill is the repo root. */
  skill_path: string;
  origin: SourceOrigin;
}

export interface SkillEntry {
  id: string;
  name: string;
  description: string;
  tool: Tool;
  scope: SkillScope;
  kind: SkillKind;
  dir: string;
  is_symlink: boolean;
  symlink_target: string | null;
  readonly: boolean;
  plugin: string | null;
  source: SkillSource | null;
  file_count: number;
  size_bytes: number;
  modified_at: number | null;
}

export interface UpdateStatus {
  skill_id: string;
  current: string | null;
  latest: string | null;
  has_update: boolean;
  local_modified: boolean;
  /** No recorded baseline: treated as modified. */
  local_state_unknown: boolean;
  error: string | null;
}

export interface BackupInfo {
  /** Creation time in ms as a string; "" = no backup was made (delete). */
  id: string;
  created_at: number;
  size_bytes: number;
}

export interface RepoSkill {
  skill_path: string;
  name: string;
  description: string;
}

/** Result of inspectGitSource. */
export interface InspectResult {
  url: string;
  clone_url: string;
  git_ref: string | null;
  commit: string;
  skills: RepoSkill[];
  /** Skill path pinned by the input (a tree/<ref>/<path> URL), if any. */
  preselected: string | null;
}

export type InstallFrom =
  | { type: "git"; url: string; clone_url: string; git_ref?: string | null; skill_path: string }
  | { type: "folder"; path: string }
  | { type: "zip"; path: string }
  | { type: "template"; name: string; description: string };

export interface InstallTarget {
  tool: Tool;
  scope: SkillScope;
}

export interface SkillFileInfo {
  rel_path: string;
  size_bytes: number;
}

export interface SkillFileList {
  files: SkillFileInfo[];
  /** True when the listing stopped at 5000 files. */
  truncated: boolean;
}

/** Binary / oversized files have `binary: true` and no text (read-only). */
export interface SkillFileDto {
  text: string;
  hash: string;
  has_bom: boolean;
  crlf: boolean;
  tokens: number;
  imports: ImportRef[];
  binary: boolean;
  too_large: boolean;
}

export interface SkillsChangedPayload {
  paths: string[];
}
