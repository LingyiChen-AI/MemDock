import * as React from "react";
import { worktreeCheck, worktreeList, worktreeStatus, type WorktreeCheck, type WorktreeInfo, type WorktreeStatus } from "@/api/worktree";

/**
 * Worktree sessions ("独立工作区"): the registered worktrees, their change status (fetched
 * lazily, cached) and which folders are git repos. Module-level, shared by the list and the pane.
 */

const WT_SEGMENT = "/.memdock/worktrees/";

/** Forward slashes, no trailing slash; lower case for Windows drive paths. */
export function normPath(p: string): string {
  const x = p.replace(/\\/g, "/").replace(/\/+$/, "");
  return /^[a-zA-Z]:/.test(x) ? x.toLowerCase() : x;
}

/** `<repo>/.memdock/worktrees/<slug>` → its repo root and slug (display case kept). */
export function parseWorktreePath(cwd: string | null | undefined): { repoRoot: string; slug: string } | null {
  if (!cwd) return null;
  const x = cwd.replace(/\\/g, "/").replace(/\/+$/, "");
  const i = x.toLowerCase().lastIndexOf(WT_SEGMENT);
  if (i <= 0) return null;
  const slug = x.slice(i + WT_SEGMENT.length);
  if (!slug || slug.includes("/")) return null;
  return { repoRoot: x.slice(0, i), slug };
}

// ---------- git check (per folder, cached) ----------

const checks = new Map<string, Promise<WorktreeCheck | null>>();

/** The folder's repo, or null when it is not a git repo (cached per folder). */
export function checkWorktree(cwd: string): Promise<WorktreeCheck | null> {
  const key = normPath(cwd);
  let p = checks.get(key);
  if (!p) {
    p = worktreeCheck(cwd).catch(() => null);
    checks.set(key, p);
  }
  return p;
}

/** `checkWorktree` as a hook: undefined while checking. */
export function useGitRepo(cwd: string | null | undefined, enabled = true): boolean | undefined {
  const [ok, setOk] = React.useState<{ cwd: string; ok: boolean } | null>(null);
  React.useEffect(() => {
    if (!cwd || !enabled) return;
    let alive = true;
    void checkWorktree(cwd).then((r) => alive && setOk({ cwd, ok: !!r }));
    return () => {
      alive = false;
    };
  }, [cwd, enabled]);
  if (!cwd || !enabled) return false;
  return ok && ok.cwd === cwd ? ok.ok : undefined;
}

// ---------- list + status store ----------

export interface WorktreeState {
  list: WorktreeInfo[];
  /** The list was read at least once (before that, nothing counts as deleted). */
  loaded: boolean;
  /** Normalized paths removed in this app run. */
  removed: ReadonlySet<string>;
  statuses: ReadonlyMap<string, WorktreeStatus>;
}

const INITIAL: WorktreeState = { list: [], loaded: false, removed: new Set(), statuses: new Map() };
let state: WorktreeState = INITIAL;
const listeners = new Set<() => void>();
const inflight = new Map<string, Promise<void>>();

function set(next: Partial<WorktreeState>) {
  state = { ...state, ...next };
  listeners.forEach((l) => l());
}

export function getWorktreeState(): WorktreeState {
  return state;
}

export function useWorktreeState(): WorktreeState {
  const sub = React.useCallback((l: () => void) => {
    listeners.add(l);
    return () => {
      listeners.delete(l);
    };
  }, []);
  const get = React.useCallback(() => state, []);
  return React.useSyncExternalStore(sub, get, get);
}

/** Re-reads the registered worktrees (errors keep the old list). */
export async function refreshWorktrees(): Promise<void> {
  try {
    const list = await worktreeList();
    set({ list: Array.isArray(list) ? list : [], loaded: true });
  } catch {
    // not available (tests / older backend): keep what we have
  }
}

/** A worktree just created: known before the next list refresh. */
export function addWorktree(info: WorktreeInfo): void {
  const n = normPath(info.path);
  const removed = new Set(state.removed);
  removed.delete(n);
  set({ list: [...state.list.filter((x) => normPath(x.path) !== n), info], removed });
}

/** The worktree's folder is gone (merged with delete / discarded). */
export function markWorktreeRemoved(path: string): void {
  const n = normPath(path);
  const removed = new Set(state.removed);
  removed.add(n);
  const statuses = new Map(state.statuses);
  statuses.delete(n);
  set({ list: state.list.filter((x) => normPath(x.path) !== n), removed, statuses });
}

/** Fetches (again) the change status of a worktree. */
export function refreshWorktreeStatus(path: string): Promise<void> {
  const n = normPath(path);
  const cur = inflight.get(n);
  if (cur) return cur;
  const p = worktreeStatus(path)
    .then((st) => {
      if (state.removed.has(n)) return;
      const statuses = new Map(state.statuses);
      statuses.set(n, st);
      set({ statuses });
    })
    .catch(() => {})
    .finally(() => inflight.delete(n));
  inflight.set(n, p);
  return p;
}

/** The cached status of `path`, fetched on first use. */
export function useWorktreeStatus(path: string | null | undefined): WorktreeStatus | null {
  const st = useWorktreeState();
  const n = path ? normPath(path) : "";
  const cached = n ? (st.statuses.get(n) ?? null) : null;
  React.useEffect(() => {
    if (path && !getWorktreeState().statuses.has(normPath(path))) void refreshWorktreeStatus(path);
  }, [path]);
  return cached;
}

/** Refreshes the status when a turn of the session ends (an active state turns idle / exited). */
export function useStatusRefreshOnTurnEnd(path: string | null | undefined, active: boolean): void {
  const was = React.useRef(active);
  React.useEffect(() => {
    if (was.current && !active && path) void refreshWorktreeStatus(path);
    was.current = active;
  }, [active, path]);
}

/** How a session folder relates to a worktree. */
export interface WorktreeLink {
  /** The registered worktree; null once deleted. */
  info: WorktreeInfo | null;
  deleted: boolean;
  repoRoot: string;
  slug: string;
}

export function findWorktree(list: WorktreeInfo[], cwd: string): WorktreeInfo | undefined {
  const n = normPath(cwd);
  return list.find((x) => normPath(x.path) === n);
}

/** The worktree a session folder belongs to (also when it was deleted); null for ordinary folders. */
export function worktreeOf(st: WorktreeState, cwd: string | null | undefined): WorktreeLink | null {
  if (!cwd) return null;
  const info = findWorktree(st.list, cwd);
  if (info) return { info, deleted: false, repoRoot: info.repo_root, slug: info.slug };
  const parsed = parseWorktreePath(cwd);
  if (!parsed) return null;
  if (st.removed.has(normPath(cwd)) || st.loaded) return { info: null, deleted: true, ...parsed };
  return null;
}

/** Tests. */
export function resetWorktreeStore(): void {
  checks.clear();
  inflight.clear();
  state = INITIAL;
  listeners.forEach((l) => l());
}
