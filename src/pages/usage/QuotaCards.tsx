import type * as React from "react";
import { Gauge, Info } from "lucide-react";
import type { QuotaWindow, Tool, ToolQuota } from "@/api/types";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { ToolTag } from "@/pages/memory/ToolTag";
import { fmt } from "@/pages/memory/paths";
import { fmtDuration } from "./usageUtils";

function WindowBar({ label, w, tool, now }: { label: string; w: QuotaWindow | null; tool: Tool; now: number }) {
  const { t } = useI18n();
  const q = t.usage.quota;
  if (!w) return null;
  const pct = Math.max(0, Math.min(100, w.used_percent));
  const tone = pct >= 95 ? "bg-destructive" : pct >= 80 ? "bg-warning" : tool === "claude" ? "bg-tool-claude" : "bg-tool-codex";
  const reset = w.resets_at && w.resets_at > now ? fmt(q.resetsIn, { time: fmtDuration(w.resets_at - now, t.usage.duration) }) : q.resetUnknown;
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
  onEnable?: () => void;
  enabling?: boolean;
}) {
  const { t } = useI18n();
  const q = t.usage.quota;
  let body: React.ReactNode;
  if (!quota) {
    body = <div className="h-16 animate-pulse rounded-md bg-muted motion-reduce:animate-none" />;
  } else if (quota.status === "disabled") {
    body = (
      <div className="flex flex-col gap-2">
        <p className="text-sm font-medium">{q.disabledTitle}</p>
        <p className="flex gap-1.5 text-xs leading-4 text-muted-foreground">
          <Info className="mt-px size-3.5 shrink-0" aria-hidden="true" />
          <span>{q.explain}</span>
        </p>
        {onEnable ? (
          <div>
            <Button type="button" variant="outline" size="sm" disabled={enabling} onClick={onEnable} className="cursor-pointer">
              {q.enable}
            </Button>
          </div>
        ) : null}
      </div>
    );
  } else if (quota.status === "unavailable") {
    body = (
      <p className="text-sm text-muted-foreground">
        {quota.reason ? fmt(q.unavailableReason, { reason: quota.reason }) : q.unavailable}
      </p>
    );
  } else if (quota.status === "no_data" || (!quota.five_hour && !quota.weekly)) {
    body = <p className="text-sm text-muted-foreground">{q.noData}</p>;
  } else {
    body = (
      <div className="grid grid-cols-2 gap-4">
        <WindowBar label={q.fiveHour} w={quota.five_hour} tool={tool} now={now} />
        <WindowBar label={q.weekly} w={quota.weekly} tool={tool} now={now} />
      </div>
    );
  }
  return (
    <section className="flex min-w-0 flex-col gap-3 rounded-lg border border-border bg-card p-4">
      <header className="flex items-center justify-between gap-2">
        <span className="flex items-center gap-2">
          <Gauge className="size-4 text-muted-foreground" aria-hidden="true" />
          <ToolTag tool={tool} />
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
  enabling,
}: {
  claude: ToolQuota | null;
  codex: ToolQuota | null;
  now: number;
  onEnableClaude: () => void;
  enabling: boolean;
}) {
  return (
    <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
      <QuotaCard tool="claude" quota={claude} now={now} onEnable={onEnableClaude} enabling={enabling} />
      <QuotaCard tool="codex" quota={codex} now={now} />
    </div>
  );
}
