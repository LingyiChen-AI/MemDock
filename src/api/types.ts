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

export type CommandErrorCode = "disk_changed" | "io" | "forbidden" | "invalid";

/** Error thrown by failing commands. `current_hash` is set for `disk_changed`. */
export interface CommandError {
  code: CommandErrorCode;
  message: string;
  current_hash?: string;
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

export type Theme = "dark" | "light" | "system";

export interface Settings {
  language: string | null;
  theme: string | null;
  manual_projects: string[];
}

export interface MemoryChangedPayload {
  paths: string[];
}
