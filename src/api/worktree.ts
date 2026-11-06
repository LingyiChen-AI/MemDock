// Worktree sessions ("独立工作区"). Mirrors `src-tauri/src/worktree.rs`; errors are Chinese strings.
import { invoke } from "@tauri-apps/api/core";

export interface WorktreeInfo {
  /** Forward slashes. */
  path: string;
  branch: string;
  slug: string;
  repo_root: string;
  base_branch: string;
  base_commit: string;
  /** Unix epoch milliseconds. */
  created_at: number;
}

export interface WorktreeCheck {
  repo_root: string;
  base_branch: string | null;
}

export interface ChangedFile {
  path: string;
  status: "A" | "M" | "D" | "R" | string;
  added: number;
  deleted: number;
}

export interface WorktreeStatus {
  files: ChangedFile[];
  ahead: number;
  dirty: boolean;
}

export type MergeMode = "squash" | "merge" | "keep";

export interface MergeResult {
  merged: boolean;
  conflicts: string[];
  auto_committed: boolean;
  commit: string | null;
}

/** Rejects when `cwd` is not inside a git repository (or HEAD is detached). */
export function worktreeCheck(cwd: string): Promise<WorktreeCheck> {
  return invoke<WorktreeCheck>("worktree_check", { cwd });
}

/** Creates `memdock/<slug>` at `<toplevel>/.memdock/worktrees/<slug>` from the current HEAD. */
export function worktreeCreate(cwd: string, hint: string): Promise<WorktreeInfo> {
  return invoke<WorktreeInfo>("worktree_create", { cwd, hint });
}

/** Registered worktrees whose folder still exists. */
export function worktreeList(): Promise<WorktreeInfo[]> {
  return invoke<WorktreeInfo[]>("worktree_list");
}

export function worktreeStatus(path: string): Promise<WorktreeStatus> {
  return invoke<WorktreeStatus>("worktree_status", { path });
}

/** Unified diff vs the base commit (incl. uncommitted changes); one file or all. */
export function worktreeDiff(path: string, file?: string | null): Promise<string> {
  return invoke<string>("worktree_diff", { path, file: file ?? null });
}

export function worktreeMerge(path: string, mode: MergeMode, message: string, deleteAfter: boolean): Promise<MergeResult> {
  return invoke<MergeResult>("worktree_merge", { path, mode, message, deleteAfter });
}

export function worktreeRemove(path: string, deleteBranch: boolean): Promise<void> {
  return invoke<void>("worktree_remove", { path, deleteBranch });
}
