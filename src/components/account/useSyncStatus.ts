import * as React from "react";
import { onSyncStatusChanged, syncStatus } from "@/api/sync";
import type { SyncStatus } from "@/api/types";

/** Sync login/pending state: read once, then kept live by `sync://status-changed`. */
export function useSyncStatus(): [SyncStatus | null, (s: SyncStatus) => void, () => void] {
  const [status, setStatus] = React.useState<SyncStatus | null>(null);
  const alive = React.useRef(true);
  const reload = React.useCallback(() => {
    syncStatus()
      .then((s) => alive.current && setStatus(s))
      .catch(() => {});
  }, []);
  React.useEffect(() => {
    alive.current = true;
    let unlisten: (() => void) | undefined;
    reload();
    onSyncStatusChanged((s) => alive.current && setStatus(s))
      .then((u) => (alive.current ? (unlisten = u) : u()))
      .catch(() => {});
    return () => {
      alive.current = false;
      unlisten?.();
    };
  }, [reload]);
  return [status, setStatus, reload];
}
