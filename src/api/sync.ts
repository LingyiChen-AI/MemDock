import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  DeviceCodeInfo,
  LoginEvent,
  SyncCommit,
  SyncProject,
  SyncDirection,
  SyncResolution,
  SyncSkippedRecord,
  SyncStartResult,
  SyncStatus,
  SyncSummary,
} from "./types";

// Every function rejects with a `CommandError` ({ code, message, paths?, secret_hits? }).
// Sync codes: not_logged_in, unauthorized, network, github_error, git_missing, git_error,
// keychain, sync_busy, unresolved, secrets, local_changed, remote_moved, push_failed,
// manifest, repo_not_ours, public_not_acked, plan_not_found.

export const LOGIN_EVENT = "sync://login";
export const STATUS_EVENT = "sync://status-changed";

/** Local only (no network): login, bound repository and pending count. */
export function syncStatus(): Promise<SyncStatus> {
  return invoke<SyncStatus>("sync_status");
}

/** Starts the device flow; the result arrives later as a `sync://login` event. */
export function loginStart(): Promise<DeviceCodeInfo> {
  return invoke<DeviceCodeInfo>("login_start");
}

export function loginCancel(): Promise<void> {
  return invoke<void>("login_cancel");
}

/** Forgets the token and the login; local data and the repository clone stay. */
export function logout(): Promise<SyncStatus> {
  return invoke<SyncStatus>("logout");
}

/** Binds another repository name (after `not_ours`). */
export function setRepoName(name: string): Promise<SyncStatus> {
  return invoke<SyncStatus>("set_repo_name", { name });
}

/** Confirms syncing into a public repository. */
export function ackPublicRepo(): Promise<SyncStatus> {
  return invoke<SyncStatus>("ack_public_repo");
}

/** Opens https://github.com/login/device in the default browser. */
export function openVerificationUrl(): Promise<void> {
  return invoke<void>("open_verification_url");
}

/**
 * `done` also reports skipped files and skill differences (nothing needed a decision).
 * `secrets`: what to do with files that look like they hold secrets (default: stop and report).
 * `direction`: `up` merges the cloud and pushes this computer's changes; `down` only brings the
 * cloud to this computer (pushes nothing, secrets are not checked).
 */
export function syncStart(
  secrets: "block" | "allow" | "skip" = "block",
  direction: SyncDirection = "up",
): Promise<SyncStartResult> {
  return invoke<SyncStartResult>("sync_start", { secrets, direction });
}

/** Files left out of a sync and not synced since, mapped to local files (local only). */
export function syncSkippedFiles(): Promise<SyncSkippedRecord[]> {
  return invoke<SyncSkippedRecord[]>("sync_skipped_files");
}

/** Resolutions are keyed by `repo_path`. */
export function syncFinish(
  planId: string,
  resolutions: Record<string, SyncResolution>,
): Promise<SyncSummary> {
  return invoke<SyncSummary>("sync_finish", { planId, resolutions });
}

export function syncDiscard(planId: string): Promise<void> {
  return invoke<void>("sync_discard", { planId });
}

/** Directory keys of projects without a git remote that should take part. */
export function setIncludedLocal(keys: string[]): Promise<SyncProject[]> {
  return invoke<SyncProject[]>("set_included_local", { keys });
}

export function syncProjects(): Promise<SyncProject[]> {
  return invoke<SyncProject[]>("sync_projects");
}

export function syncHistory(limit: number): Promise<SyncCommit[]> {
  return invoke<SyncCommit[]>("sync_history", { limit });
}

/** Text of `repoPath` at `commit`; `null` when absent there. */
export function syncFileAt(commit: string, repoPath: string): Promise<string | null> {
  return invoke<string | null>("sync_file_at", { commit, repoPath });
}

/** Writes that version to the local file (snapshot first); synced next time. */
export function restoreFromHistory(commit: string, repoPath: string): Promise<void> {
  return invoke<void>("restore_from_history", { commit, repoPath });
}

export function onLoginEvent(cb: (e: LoginEvent) => void): Promise<UnlistenFn> {
  return listen<LoginEvent>(LOGIN_EVENT, (e) => cb(e.payload));
}

export function onSyncStatusChanged(cb: (s: SyncStatus) => void): Promise<UnlistenFn> {
  return listen<SyncStatus>(STATUS_EVENT, (e) => cb(e.payload));
}
