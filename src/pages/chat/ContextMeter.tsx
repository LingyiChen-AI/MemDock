import * as React from "react";
import { invoke } from "@tauri-apps/api/core";
import { usageQuota } from "@/api/usage";
import { cn } from "@/lib/utils";
import { fmt } from "@/pages/memory/paths";

export interface SessionContext {
  used: number;
  window: number;
  model: string | null;
}

export function sessionContext(tool: "claude" | "codex", path: string): Promise<SessionContext | null> {
  return invoke<SessionContext | null>("session_context", { tool, path });
}

function short(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(n % 1_000_000 === 0 ? 0 : 1)}M`;
  if (n >= 1000) return `${Math.round(n / 1000)}k`;
  return String(n);
}

/** green < 60 % ≤ amber < 85 % ≤ red */
function tone(pct: number): string {
  return pct >= 85 ? "bg-red-500" : pct >= 60 ? "bg-amber-500" : "bg-emerald-500";
}

function Meter({ label, pct, tip }: { label: string; pct: number; tip: string }) {
  const p = Math.max(0, Math.min(100, pct));
  return (
    <span title={tip} className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md px-1.5 text-[11px] text-muted-foreground tabular-nums">
      {label}
      <span aria-hidden="true" className="relative h-1.5 w-12 overflow-hidden rounded-full bg-muted">
        <span className={cn("absolute inset-y-0 left-0 rounded-full", tone(p))} style={{ width: `${p}%` }} />
      </span>
      <span className={cn(p >= 85 && "text-red-600 dark:text-red-400", p >= 60 && p < 85 && "text-amber-700 dark:text-amber-300")}>{Math.round(p)}%</span>
    </span>
  );
}

export interface ContextMeterText {
  context: string;
  contextHint: string;
  weekly: string;
  weeklyHint: string;
}

/**
 * Context window fill of the conversation (from its session file, refreshed whenever `refreshKey`
 * changes, e.g. after each turn) and the tool's weekly quota (from the usage page's cached quota).
 */
export function ContextMeters({
  tool,
  path,
  refreshKey,
  t,
}: {
  tool: "claude" | "codex";
  path: string | null;
  refreshKey: unknown;
  t: ContextMeterText;
}) {
  const [ctx, setCtx] = React.useState<SessionContext | null>(null);
  const [weekly, setWeekly] = React.useState<number | null>(null);
  React.useEffect(() => {
    if (!path) {
      setCtx(null);
      return;
    }
    let alive = true;
    sessionContext(tool, path)
      .then((c) => alive && setCtx(c))
      .catch(() => alive && setCtx(null));
    return () => {
      alive = false;
    };
  }, [tool, path, refreshKey]);
  React.useEffect(() => {
    let alive = true;
    usageQuota()
      .then((q) => {
        const w = q[tool]?.weekly;
        if (alive) setWeekly(q[tool]?.status === "ok" && w ? w.used_percent : null);
      })
      .catch(() => alive && setWeekly(null));
    return () => {
      alive = false;
    };
  }, [tool, refreshKey]);
  if (!ctx && weekly === null) return null;
  return (
    <>
      {ctx ? (
        <Meter
          label={t.context}
          pct={(ctx.used * 100) / ctx.window}
          tip={fmt(t.contextHint, { used: short(ctx.used), window: short(ctx.window), model: ctx.model ?? "" })}
        />
      ) : null}
      {weekly !== null ? <Meter label={t.weekly} pct={weekly} tip={t.weeklyHint} /> : null}
    </>
  );
}
