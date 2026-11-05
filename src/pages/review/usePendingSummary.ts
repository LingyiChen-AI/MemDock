import * as React from "react";
import { onSuggestionsChanged, pendingSummary } from "@/api/extract";
import type { PendingSummary } from "@/api/types";

/** Pending-review counts: loaded once, then kept live by the `suggestions-changed` event. */
export function usePendingSummary(): PendingSummary | null {
  const [summary, setSummary] = React.useState<PendingSummary | null>(null);
  React.useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    // The event may arrive before the first read finishes; the newer value wins.
    let gotEvent = false;
    pendingSummary()
      .then((s) => alive && !gotEvent && setSummary(s))
      .catch(() => {});
    onSuggestionsChanged((s) => {
      if (!alive) return;
      gotEvent = true;
      setSummary(s);
    })
      .then((u) => (alive ? (unlisten = u) : u()))
      .catch(() => {});
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);
  return summary;
}
