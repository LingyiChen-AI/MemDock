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
  // GitHub sync
  | "unauthorized"
  | "network"
  | "github_error"
  | "git_missing"
  | "git_error"
  | "keychain"
  | "sync_busy"
  | "unresolved"
  | "secrets"
  | "local_changed"
  | "remote_moved"
  | "push_failed"
  | "manifest"
  | "repo_not_ours"
  | "public_not_acked"
  | "plan_not_found"
  // Environment install
  | "npm_missing"
  | "busy"
  // Session conversion
  /** The session was converted to the target tool before; retry with `force`. `paths` lists the earlier files. */
  | "already_converted"
  /** The session has no user message to convert. */
  | "empty_session"
  // GitHub import
  | "import_bad_url"
  | "import_not_found"
  | "import_rate_limited"
  | "import_too_large"
  | "import_expired";

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
  /** Set for sync `unresolved` / `local_changed`: the files involved. */
  paths?: string[];
  /** Set for sync `secrets`: where secrets were found (never the matched text). */
  secret_hits?: FileSecretHit[];
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
  /** Opt-in live Codex quota lookup with the local ChatGPT login (`~/.codex/auth.json`). */
  codex_quota_enabled?: boolean;
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
  /** The user's extraction rules replacing the built-in ones; null = built-in. */
  custom_prompt?: string | null;
}

export interface PromptDefaults {
  /** Built-in editable rules (with the placeholder). */
  rules: string;
  /** Always appended; not editable. */
  fixed: string;
  placeholder: string;
  max_chars: number;
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
  /** Set for suggestions that did not come from an extraction run. */
  origin?: SuggestionOrigin | null;
  /** The extraction run that produced it; absent for imports and older data. */
  run_id?: string | null;
}

export type SuggestionOrigin = {
  type: "github";
  /** `owner/repo`. */
  repo: string;
  path: string;
  git_ref: string;
  commit: string;
  url: string;
};

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

// ------------------------------------------------------------- GitHub sync

export type SyncNeeds = "public_ack" | "not_ours" | "relogin";

export interface SyncRepo {
  owner: string;
  name: string;
  url: string;
  /** `null` when GitHub's answer is not known (e.g. `not_ours`). */
  private: boolean | null;
}

export interface SyncStatus {
  logged_in: boolean;
  login: string | null;
  repo: SyncRepo | null;
  /** Milliseconds since the epoch. */
  last_sync_at: number | null;
  /** Local memory files that differ from the last sync. */
  pending: number;
  needs: SyncNeeds | null;
  /** Never synced here while the cloud already holds memories (a new computer). */
  remote_has_memory: boolean;
}

export interface DeviceCodeInfo {
  user_code: string;
  verification_uri: string;
  /** Seconds. */
  expires_in: number;
}

export type LoginStatus = "ok" | "expired" | "denied" | "cancelled" | "error" | "relogin";

/** Payload of `sync://login`. */
export interface LoginEvent {
  status: LoginStatus;
  needs?: SyncNeeds;
  message?: string;
}

export interface FileSecretHit {
  repo_path: string;
  line: number;
  rule: string;
}

export interface SyncConflict {
  repo_path: string;
  local_path: string;
  /** `null`: the file does not exist on that side. */
  local: string | null;
  remote: string | null;
  base: string | null;
  /** diff3 text with conflict markers. */
  merged: string;
  conflicts: number;
}

export interface SyncDeletedFile {
  repo_path: string;
  local_path: string | null;
}

export interface SyncHeldProject {
  project_key: string;
  files: SyncDeletedFile[];
}

export type SkippedReason =
  | "not_utf8_local"
  | "not_utf8_remote"
  | "not_utf8_base"
  | "case_collision"
  | "duplicate_local"
  | "not_regular_file"
  | "secret";

export interface SyncSkipped {
  repo_path: string;
  reason: SkippedReason;
}

/** A cloud file a download could not place on this computer. */
export interface SyncAbsentFile {
  repo_path: string;
  project_key: string;
  /** `no_project`: the project is not on this computer; `no_place`: no local location yet. */
  reason: "no_project" | "no_place";
}

/** A file left out of a sync and not synced since (persisted). */
export interface SyncSkippedRecord {
  repo_path: string;
  reason: SkippedReason;
  /** Milliseconds since the epoch. */
  at: number;
  local_path: string | null;
  /** Memory index id of the local file, when known. */
  file_id: string | null;
}

export type SyncDirection = "up" | "down";

export type ManifestScope = { type: "global" } | { type: "project"; key: string };

export interface ManifestSkill {
  name: string;
  url: string;
  git_ref: string | null;
  commit: string | null;
  skill_path: string;
  tool: Tool;
  scope: ManifestScope;
  installed_on: string[];
  removed_on: string[];
}

export interface SyncPlanCounts {
  pull: number;
  push: number;
  merged: number;
  conflicts: number;
  remote_deleted: number;
  local_all_deleted: number;
  skipped: number;
}

export interface SyncPlan {
  id: string;
  counts: SyncPlanCounts;
  conflicts: SyncConflict[];
  /** Deleted on another machine, unchanged here: keep (default) or delete locally. */
  remote_deleted: SyncDeletedFile[];
  /** Every memory file of these projects is gone here: keep remote or delete remote. */
  local_all_deleted: SyncHeldProject[];
  skipped: SyncSkipped[];
  secret_hits: FileSecretHit[];
  missing_skills: ManifestSkill[];
  removed_elsewhere: ManifestSkill[];
  /** A download: resolving writes local files only, nothing is pushed. */
  download: boolean;
  /** Download: local changes left as they are (not uploaded). */
  kept_local: string[];
  /** Download: cloud files with no place on this computer. */
  absent: SyncAbsentFile[];
}

export interface SyncSummary {
  commit: string | null;
  committed: boolean;
  pulled: number;
  pushed: number;
  merged: number;
}

export type SyncStartResult =
  | {
      kind: "done";
      summary: SyncSummary;
      /** Files left out of this sync, with reasons. */
      skipped: SyncSkipped[];
      /** Skills other machines have that this one lacks. */
      missing_skills: ManifestSkill[];
      /** Skills installed here that every other machine removed. */
      removed_elsewhere: ManifestSkill[];
      download: boolean;
      kept_local: string[];
      absent: SyncAbsentFile[];
    }
  | { kind: "needs_resolution"; plan: SyncPlan }
  | { kind: "blocked"; secret_hits: FileSecretHit[] };

/** Choice for one file of a plan, keyed by `repo_path`. */
export type SyncResolution =
  | { kind: "text"; text: string }
  | { kind: "delete_local" }
  | { kind: "keep_local" }
  | { kind: "delete_remote" }
  | { kind: "keep_remote" };

export interface SyncProject {
  project_id: string;
  name: string;
  /** From the project's git remote; `null` without one. */
  key: string | null;
  dir_key: string;
  /** Projects with a remote always take part; others only when included. */
  included: boolean;
}

export interface SyncCommit {
  id: string;
  message: string;
  /** Milliseconds since the epoch. */
  time: number;
  files: string[];
}
// ---- Usage statistics (plan 5) ----

export type UsageView = "today" | "daily" | "monthly";

export interface UsageTotals {
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
  total_tokens: number;
  /** Billed model requests after deduplication. */
  requests: number;
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
  /** cache_read / (input + cache_read + cache_write); null without input. */
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

/**
 * Why a quota lookup failed (`ToolQuota.reason` / `live_error`). `http_<status>`
 * codes are passed through for other HTTP failures.
 */
export type QuotaReason =
  /** Claude Code has no OAuth login on this machine. */
  | "not_logged_in"
  /** No ChatGPT login token in `~/.codex/auth.json`. */
  | "codex_not_logged_in"
  /** The endpoint answered 401/403: sign in again in Codex. */
  | "codex_login_expired"
  /** The endpoint answered with JSON MemDock cannot read. */
  | "unexpected_response"
  | "network"
  | `http_${number}`;

export interface ToolQuota {
  status: "ok" | "disabled" | "unavailable" | "no_data";
  five_hour: QuotaWindow | null;
  weekly: QuotaWindow | null;
  updated_at: number | null;
  reason?: QuotaReason | string;
  /** "live": quota endpoint; "log": newest local log snapshot (Codex). */
  source?: "live" | "log";
  /** The live lookup is on but failed; the log snapshot is shown instead. */
  live_error?: QuotaReason | string;
  /** Subscription plan reported by the endpoint ("plus", "pro", ...). */
  plan?: string;
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

// ---- Session conversion (src-tauri/src/convert, commands_sessions.rs) ----

export interface SessionRef {
  tool: Tool;
  id: string;
}

export interface SessionSummary {
  tool: Tool;
  id: string;
  path: string;
  cwd: string;
  project_id: string;
  project_name: string;
  title: string | null;
  first_prompt: string;
  /** Starts from an automatic summary of an earlier conversation. */
  continued: boolean;
  message_count: number;
  tool_calls: number;
  /** Unix milliseconds. */
  started_at: number | null;
  updated_at: number | null;
  model: string | null;
  /** Set when MemDock wrote this session. */
  converted_from: SessionRef | null;
  /** Sessions MemDock already wrote from this one (files that still exist). */
  converted_to: SessionRef[];
}

export interface PlanStats {
  user_messages: number;
  assistant_messages: number;
  tool_calls: number;
  /** Shell calls kept as shell calls in the target tool. */
  mapped_calls: number;
  /** Calls rewritten as assistant text. */
  degraded_calls: number;
}

export type PreviewRole = "user" | "assistant" | "shell";

export interface PreviewMessage {
  role: PreviewRole;
  ts: string;
  text: string;
}

export type ToolKind = "shell" | "read" | "edit" | "write" | "search" | "web" | "agent" | "todo" | "other";

/** The original conversation for the viewer (convert/timeline.rs). */
export type TimelineItem =
  | { type: "user"; ts: string; text: string; /** Pasted images as data: URIs. */ images?: string[] }
  | { type: "assistant"; ts: string; text: string }
  | {
      type: "tool";
      ts: string;
      name: string;
      kind: ToolKind;
      summary: string;
      input: string;
      output: string | null;
      is_error: boolean;
      /** Converting keeps it as a real shell call; otherwise it becomes text. */
      mapped: boolean;
    };

export interface TimelineCounts {
  user: number;
  assistant: number;
  tools: number;
  shell: number;
  files: number;
  errors: number;
}

export interface SessionPreview {
  summary: SessionSummary;
  messages: PreviewMessage[];
  total_messages: number;
  stats: PlanStats;
  timeline: TimelineItem[];
  counts: TimelineCounts;
  timeline_total: number;
}

export interface ConvertResult {
  tool: Tool;
  session_id: string;
  path: string;
  cwd: string;
  resume_command: string;
  /** Codex target: registered in Codex's thread list database. */
  indexed: boolean;
  /** `state_db_missing`, `state_db_schema`, or `<area>: <error>` strings. */
  warnings: string[];
  stats: PlanStats;
}

// ------------------------------------------------------------ GitHub import

export type GhMemoryKind = "claude_md" | "claude_local" | "agents_md" | "claude_dir" | "memory_dir" | "file";

export type GhImportItem = {
  /** `m:<path>` or `s:<dir>`. */
  id: string;
  path: string;
  name: string;
  size: number;
  too_large: boolean;
} & ({ type: "memory"; memory_kind: GhMemoryKind } | { type: "skill"; files: number });

export interface GhImportScan {
  session_id: string;
  repo: string;
  repo_url: string;
  git_ref: string;
  commit: string;
  scope: string;
  items: GhImportItem[];
  truncated: boolean;
  tree_truncated: boolean;
  authenticated: boolean;
}

export interface GhImportPreview {
  text: string;
  binary: boolean;
  files: { path: string; size: number }[];
}

export type GhImportTarget =
  | { type: "global_claude" }
  | { type: "global_agents" }
  | { type: "project_claude"; project_id: string }
  | { type: "project_agents"; project_id: string };

export interface GhMemoryPick {
  item_id: string;
  target: GhImportTarget;
}

export interface GhImportMemoriesResult {
  added: number;
  duplicates: number;
  skipped: { item_id: string; reason: "binary" | "empty" | "too_large" }[];
}
