import type { RunRecord, Suggestion } from "@/api/types";

/** The part of a run needed to tell which suggestions it produced. */
export type RunRef = Pick<RunRecord, "id" | "started_at" | "finished_at">;

/**
 * Whether `s` came from `run`: by its recorded run id, or (older suggestions
 * without one) by being created during the run. Imports never belong to a run.
 */
export function belongsToRun(s: Suggestion, run: RunRef): boolean {
  if (s.run_id) return s.run_id === run.id;
  if (s.origin) return false;
  const end = run.finished_at ?? Number.POSITIVE_INFINITY;
  return s.created_at >= run.started_at && s.created_at <= end;
}

export interface RunProgress {
  total: number;
  pending: number;
  accepted: number;
  rejected: number;
  /** Outdated before anyone handled them. */
  stale: number;
}

export function runProgress(run: RunRef, suggestions: Suggestion[]): RunProgress {
  const p: RunProgress = { total: 0, pending: 0, accepted: 0, rejected: 0, stale: 0 };
  for (const s of suggestions) {
    if (!belongsToRun(s, run)) continue;
    p.total++;
    if (s.status === "pending") p.pending++;
    else if (s.status === "accepted") p.accepted++;
    else if (s.status === "rejected") p.rejected++;
    else p.stale++;
  }
  return p;
}
