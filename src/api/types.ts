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
  | "local_changes"
  // Memory extraction
  | "already_running"
  | "not_installed"
  | "not_logged_in"
  | "stale"
  | "not_found"
  | "not_pending"
  /** The extraction data files could not be loaded; `reset_extract_data` recovers. */
  | "store_unavailable"
  // Environment install
  | "npm_missing"
  | "busy";

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
  claude_quota_enabled?: boolean;
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
  /** A subfolder of a larger git repository: not updatable from here. */
  not_updatable_subdir: boolean;
  /** The repository root when `not_updatable_subdir`. */
  repo_root: string | null;
  error: string | null;
}

export interface BackupInfo {
  /** Creation time in ms as a string; "" = no backup was made (delete). */
  id: string;
  created_at: number;
  size_bytes: number;
}

/** A deleted skill whose backups remain (restorable to `dir`). */
export interface DeletedSkill {
  bucket: string;
  name: string;
  tool: Tool;
  scope: SkillScope;
  /** Original skill directory. */
  dir: string;
  /** Newest first. */
  backups: BackupInfo[];
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
  | {
      type: "git";
      url: string;
      clone_url: string;
      git_ref?: string | null;
      skill_path: string;
      /** Install exactly this (inspected) commit. */
      commit?: string | null;
    }
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

// ---- Memory extraction and review (mirror the Rust serde names exactly)

/** `weekday` 0 = Sunday .. 6 = Saturday. */
export type ExtractSchedule =
  | { kind: "off" }
  | { kind: "daily"; hh: number; mm: number }
  | { kind: "weekly"; weekday: number; hh: number; mm: number };

export type ExtractProjectFilter = { kind: "all" } | { kind: "selected"; ids: string[] };

export interface ExtractConfig {
  /** null = automatic (prefer claude). */
  tool: Tool | null;
  schedule: ExtractSchedule;
  projects: ExtractProjectFilter;
  include_global: boolean;
  /** 1..=90. */
  lookback_days: number;
  lang: string | null;
}

export interface RunnerInfo {
  tool: Tool;
  available: boolean;
  program?: string;
}

export type RunTrigger = "manual" | "scheduled";
export type RunStatus = "running" | "succeeded" | "failed" | "cancelled" | "interrupted";
/** Why a run failed; mapped to localized text (the raw `error` stays as detail). */
export type RunErrorCode = "not_installed" | "not_logged_in" | "timeout" | "cancelled" | "invalid_output" | "io" | "other";

export interface RunRecord {
  id: string;
  started_at: number;
  finished_at: number | null;
  trigger: RunTrigger;
  tool: Tool;
  status: RunStatus;
  sessions_read: number;
  projects: number;
  suggestions_new: number;
  warnings: string[];
  /** Raw (English) detail. */
  error: string | null;
  /** Absent in records written by older builds. */
  error_code?: RunErrorCode | null;
}

export interface ExtractProgress {
  project: string;
  /** 1-based. */
  index: number;
  total: number;
}

export type SuggestionKind = "add" | "modify" | "obsolete";
export type SuggestionStatus = "pending" | "accepted" | "rejected" | "stale";

export interface SuggestionEvidence {
  tool: Tool;
  session_id: string;
  ts: number;
  quote: string;
}

export interface Suggestion {
  id: string;
  fingerprint: string;
  kind: SuggestionKind;
  file_id: string;
  path: string;
  tool_of_file: Tool;
  old_text: string | null;
  new_text: string | null;
  reason: string;
  evidence: SuggestionEvidence[];
  source_tool: Tool;
  created_at: number;
  base_hash: string;
  status: SuggestionStatus;
}

/** Keys of `by_project` are project ids; `__global__` for files outside any project. */
export interface PendingSummary {
  total: number;
  by_project: Record<string, number>;
}

export type ApplyOutcome =
  /** `saved: false`: written, but the status could not be persisted. */
  | { kind: "applied"; hash: string; saved: boolean }
  | { kind: "needs_confirm"; reason: string }
  | { kind: "stale" }
  | { kind: "already_applied" };

export interface AcceptResult {
  id: string;
  outcome?: ApplyOutcome;
  error?: CommandError;
}

export interface ContextTurn {
  tool: Tool;
  session_id: string;
  project_id: string | null;
  ts: number;
  role: "user" | "assistant";
  text: string;
}

export interface SuggestionContext {
  evidence: SuggestionEvidence;
  turns: ContextTurn[];
}

// ---- Usage statistics (plan 5) ----

export type UsageView = "today" | "daily" | "monthly";

export interface UsageTotals {
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
  total_tokens: number;
  /** Estimated USD for the priced part. */
  cost: number;
  /** Tokens of models without a known price (not in `cost`). */
  unpriced_tokens: number;
}

export interface UsageBreakdown {
  tool: Tool;
  model: string;
  totals: UsageTotals;
}

export interface UsageRow {
  /** `HH` (today), `YYYY-MM-DD` (daily) or `YYYY-MM` (monthly). */
  key: string;
  totals: UsageTotals;
  claude: UsageTotals;
  codex: UsageTotals;
  breakdown: UsageBreakdown[];
}

export interface UsageSummary {
  totals: UsageTotals;
  days: number;
  daily_avg_cost: number;
  daily_avg_tokens: number;
  peak: { key: string; cost: number; tokens: number } | null;
  cache_hit_rate: number | null;
}

export interface UsageReport {
  view: UsageView;
  from: string;
  to: string;
  rows: UsageRow[];
  summary: UsageSummary;
  models: string[];
  unknown_models: string[];
  pricing_updated: string;
  scanning: boolean;
}

export interface UsageQuery {
  view: UsageView;
  from: string;
  to: string;
  tools: Tool[];
  models: string[];
}

export interface QuotaWindow {
  used_percent: number;
  /** Epoch ms. */
  resets_at: number | null;
  window_minutes: number | null;
}

export interface ToolQuota {
  status: "ok" | "disabled" | "unavailable" | "no_data";
  five_hour: QuotaWindow | null;
  weekly: QuotaWindow | null;
  updated_at: number | null;
  reason?: string;
}

export interface QuotaDto {
  claude: ToolQuota;
  codex: ToolQuota;
}

// ---- Environment (plan 5) ----

export type EnvToolName = "claude" | "codex" | "node" | "npm" | "git";

export interface EnvToolStatus {
  name: EnvToolName;
  installed: boolean;
  path: string | null;
  version: string | null;
  logged_in: boolean | null;
  desktop_app: string | null;
}

export interface EnvReport {
  tools: EnvToolStatus[];
  data_dir: string;
  installing: string | null;
}

export type InstallEvent =
  | { kind: "log"; tool: string; line: string }
  | { kind: "done"; tool: string; ok: boolean; code: number | null; error: string | null };
