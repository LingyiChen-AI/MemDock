import * as React from "react";
import { liveInterrupt, liveStop, type TurnUsage } from "@/api/live";
import { fmt } from "@/pages/memory/paths";
import { errorMessage } from "@/pages/skills/skillUtils";
import type { Messages } from "@/i18n";
import { refreshLiveList } from "./liveStore";

export function useTurnInterrupt(
  liveId: string | undefined,
  running: boolean,
  lastUsage: TurnUsage | null | undefined,
  c: Messages["chat"],
  onError: (error: string | null) => void,
) {
  const [stopping, setStopping] = React.useState(false);
  const [canForce, setCanForce] = React.useState(false);
  const request = React.useRef<ReturnType<typeof setTimeout> | null>(null);

  React.useEffect(() => {
    setStopping(false);
    setCanForce(false);
    return () => {
      if (request.current !== null) clearTimeout(request.current);
      request.current = null;
    };
  }, [liveId, running, lastUsage]);

  function stop() {
    if (!liveId || !running || request.current !== null) return;
    onError(null);
    setStopping(true);
    // Refresh the summary in case the completion arrived but its UI event was missed.
    const timer = setTimeout(() => {
      void refreshLiveList();
      request.current = null;
      setStopping(false);
      setCanForce(true);
    }, 10000);
    request.current = timer;
    const action = canForce ? liveStop : liveInterrupt;
    void action(liveId).then(() => {
      if (canForce) void refreshLiveList();
    }).catch((error) => {
      if (request.current !== timer) return;
      clearTimeout(timer);
      request.current = null;
      setStopping(false);
      setCanForce(true);
      onError(fmt(c.actionFailed, { error: errorMessage(error) }));
    });
  }

  const label = stopping ? c.stopping : canForce ? c.forceStop : c.stop;
  return { stopping, label, hint: canForce ? c.forceStopHint : c.stopHint, stop };
}
