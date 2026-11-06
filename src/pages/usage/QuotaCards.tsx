import type * as React from "react";
import { Gauge, Info, TriangleAlert } from "lucide-react";
import type { QuotaWindow, Tool, ToolQuota } from "@/api/types";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { htmlLang } from "@/i18n";
import { useI18n } from "@/i18n/I18nProvider";
import { ToolTag } from "@/pages/memory/ToolTag";
import { fmt } from "@/pages/memory/paths";
import { fmtDuration } from "./usageUtils";

/** Friendly text for a quota failure code; unknown codes go through the generic message. */
export function quotaReasonText(reason: string | undefined, reasons: Record<string, string>, generic: string): string {
  if (!reason) return generic;
  return reasons[reason] ?? fmt(generic, { reason });
}

function WindowBar({ label, w, tool, now }: { label: string; w: QuotaWindow | null; tool: Tool; now: number }) {
  const { t } = useI18n();
  const q = t.usage.quota;
  if (!w) return null;
  const pct = Math.max(0, Math.min(100, w.used_percent));
  const tone = pct >= 95 ? "bg-destructive" : pct >= 80 ? "bg-warning" : tool === "claude" ? "bg-tool-claude" : "bg-tool-codex";
  const reset =
    w.resets_at == null
      ? q.resetUnknown
      : w.resets_at > now
        ? fmt(q.resetsIn, { time: fmtDuration(w.resets_at - now, t.usage.duration) })
        : q.resetDone;
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-baseline justify-between gap-2 text-xs">
        <span className="text-muted-foreground">{label}</span>
        <span className="font-semibold tabular-nums">{fmt(q.used, { pct: pct.toFixed(pct < 10 ? 1 : 0) })}</span>
      </div>
      <div
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(pct)}
        className="h-1.5 w-full overflow-hidden rounded-full bg-muted"
      >
        <div className={cn("h-full rounded-full", tone)} style={{ width: `${pct}%` }} />
      </div>
      <span className="text-xs text-muted-foreground tabular-nums">{reset}</span>
    </div>
  );
}

function Note({ warn, children }: { warn?: boolean; children: React.ReactNode }) {
  const Icon = warn ? TriangleAlert : Info;
  return (
    <p className={cn("flex gap-1.5 text-xs leading-4", warn ? "text-warning" : "text-muted-foreground")}>
      <Icon className="mt-px size-3.5 shrink-0" aria-hidden="true" />
      <span>{children}</span>
    </p>
  );
}

function QuotaCard({
  tool,
  quota,
  now,
  onEnable,
  enabling,
}: {
  tool: Tool;
  quota: ToolQuota | null;
  now: number;
  /** Claude: turn on the lookup. Codex: turn on the live lookup. */
  onEnable?: () => void;
  enabling?: boolean;
}) {
  const { t, language } = useI18n();
  const q = t.usage.quota;
  const reasonText = (r: string | undefined) => quotaReasonText(r, q.reasons, q.unavailableReason);
  const shortDate = (ms: number) => new Intl.DateTimeFormat(htmlLang(language), { month: "numeric", day: "numeric" }).format(new Date(ms));
  const enableBtn = (label: string) =>
    onEnable ? (
      <div>
        <Button type="button" variant="outline" size="sm" disabled={enabling} onClick={onEnable} className="cursor-pointer">
          {label}
        </Button>
      </div>
    ) : null;
  // Codex without the live lookup: the log snapshot (or nothing) plus an offer to turn it on.
  const codexLiveOff = tool === "codex" && quota?.source !== "live" && !quota?.live_error && quota?.status !== "unavailable";

  let body: React.ReactNode;
  if (!quota) {
    body = <div className="h-16 animate-pulse rounded-md bg-muted motion-reduce:animate-none" />;
  } else if (quota.status === "disabled") {
    body = (
      <div className="flex flex-col gap-2">
        <p className="text-sm font-medium">{q.disabledTitle}</p>
        <Note>{q.explain}</Note>
        {enableBtn(q.enable)}
      </div>
    );
  } else if (quota.status === "unavailable") {
    body = (
      <div className="flex flex-col gap-2">
        <p className="text-sm text-muted-foreground">{quota.reason ? reasonText(quota.reason) : q.unavailable}</p>
      </div>
    );
  } else if (quota.status === "no_data" || (!quota.five_hour && !quota.weekly)) {
    body = (
      <div className="flex flex-col gap-2">
        <p className="text-sm text-muted-foreground">{q.noData}</p>
        {codexLiveOff ? enableBtn(q.codexEnableLive) : null}
      </div>
    );
  } else {
    const snapshot = tool === "codex" && quota.source === "log";
    body = (
      <div className="flex flex-col gap-3">
        <div className="grid grid-cols-2 gap-4">
          <WindowBar label={q.fiveHour} w={quota.five_hour} tool={tool} now={now} />
          <WindowBar label={q.weekly} w={quota.weekly} tool={tool} now={now} />
        </div>
        {snapshot && quota.updated_at ? (
          quota.live_error ? (
            <Note warn>{fmt(q.codexLiveFailed, { reason: reasonText(quota.live_error), date: shortDate(quota.updated_at) })}</Note>
          ) : (
            <div className="flex flex-wrap items-center justify-between gap-2">
              <Note>{fmt(q.codexLogSnapshot, { date: shortDate(quota.updated_at) })}</Note>
              {enableBtn(q.codexEnableLive)}
            </div>
          )
        ) : null}
      </div>
    );
  }
  return (
    <section className="flex min-w-0 flex-col gap-3 rounded-lg border border-border bg-card p-4">
      <header className="flex items-center justify-between gap-2">
        <span className="flex items-center gap-2">
          <Gauge className="size-4 text-muted-foreground" aria-hidden="true" />
          <ToolTag tool={tool} />
          {quota?.plan ? <span className="rounded-sm bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">{fmt(q.plan, { plan: quota.plan })}</span> : null}
        </span>
        {quota?.status === "ok" && quota.updated_at ? (
          <span className="text-xs text-muted-foreground tabular-nums">
            {fmt(q.updated, { time: new Date(quota.updated_at).toLocaleString() })}
          </span>
        ) : null}
      </header>
      {body}
    </section>
  );
}

export function QuotaCards({
  claude,
  codex,
  now,
  onEnableClaude,
  onEnableCodex,
  enabling,
}: {
  claude: ToolQuota | null;
  codex: ToolQuota | null;
  now: number;
  onEnableClaude: () => void;
  onEnableCodex: () => void;
  enabling: boolean;
}) {
  return (
    <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
      <QuotaCard tool="claude" quota={claude} now={now} onEnable={onEnableClaude} enabling={enabling} />
      <QuotaCard tool="codex" quota={codex} now={now} onEnable={onEnableCodex} enabling={enabling} />
    </div>
  );
}
