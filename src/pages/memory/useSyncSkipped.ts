import * as React from "react";
import { onSyncStatusChanged, syncSkippedFiles } from "@/api/sync";
import type { SkippedReason } from "@/api/types";

/**
 * Memory file ids the last syncs left out (file id -> reason). Reloaded whenever the sync status
 * changes (every sync emits it). Failures (no backend, not logged in) mean "nothing skipped".
 */
export function useSyncSkipped(): Record<string, SkippedReason> {
  const [skipped, setSkipped] = React.useState<Record<string, SkippedReason>>({});
  React.useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    const load = () =>
      Promise.resolve()
        .then(() => syncSkippedFiles())
        .then((list) => {
          if (!alive) return;
          const out: Record<string, SkippedReason> = {};
          for (const r of Array.isArray(list) ? list : []) if (r.file_id) out[r.file_id] = r.reason;
          setSkipped(out);
        })
        .catch(() => alive && setSkipped({}));
    void load();
    Promise.resolve()
      .then(() => onSyncStatusChanged(() => void load()))
      .then((u) => {
        if (typeof u !== "function") return;
        if (alive) unlisten = u;
        else u();
      })
      .catch(() => {});
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);
  return skipped;
}
