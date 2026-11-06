import * as React from "react";
import { invoke } from "@tauri-apps/api/core";
import { usageQuota } from "@/api/usage";
import { cn } from "@/lib/utils";
import { fmt } from "@/pages/memory/paths";

export interface SessionContext {
  used: number;
  window: number;
  model: string | null;
  /** Who made the last request: `cli` (a terminal) or `sdk-cli` (MemDock). Claude only. */
  source?: string | null;
  /** When it was made (epoch ms). */
  at?: number | null;
}

export function sessionContext(tool: "claude" | "codex", path: string): Promise<SessionContext | null> {
  return invoke<SessionContext | null>("session_context", { tool, path });
}

/** For a conversation known only by its session / thread id (a live chat not on the list yet). */
export function sessionContextById(tool: "claude" | "codex", id: string): Promise<SessionContext | null> {
  return invoke<SessionContext | null>("session_context_by_id", { tool, id });
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
  sourceTerminal: string;
  sourceMemdock: string;
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
  nativeId = null,
  refreshKey,
  t,
  onContext,
}: {
  tool: "claude" | "codex";
  path: string | null;
  /** Used when there's no path yet. */
  nativeId?: string | null;
  refreshKey: unknown;
  t: ContextMeterText;
  /** Gets the context read (or null), e.g. to warn when the session is used elsewhere. */
  onContext?: (ctx: SessionContext | null) => void;
}) {
  const report = React.useRef(onContext);
  report.current = onContext;
  const [ctx, setCtx] = React.useState<SessionContext | null>(null);
  const [weekly, setWeekly] = React.useState<number | null>(null);
  React.useEffect(() => {
    if (!path && !nativeId) {
      setCtx(null);
      return;
    }
    let alive = true;
    (path ? sessionContext(tool, path) : sessionContextById(tool, nativeId!))
      .then((c) => {
        if (!alive) return;
        setCtx(c);
        report.current?.(c);
      })
      .catch(() => alive && setCtx(null));
    return () => {
      alive = false;
    };
  }, [tool, path, nativeId, refreshKey]);
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
          tip={
            fmt(t.contextHint, { used: short(ctx.used), window: short(ctx.window), model: ctx.model ?? "" }) +
            (ctx.source === "cli" ? ` · ${t.sourceTerminal}` : ctx.source === "sdk-cli" ? ` · ${t.sourceMemdock}` : "")
          }
        />
      ) : null}
      {weekly !== null ? <Meter label={t.weekly} pct={weekly} tip={t.weeklyHint} /> : null}
    </>
  );
}
