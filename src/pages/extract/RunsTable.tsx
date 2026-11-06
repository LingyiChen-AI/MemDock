import * as React from "react";
import { Ban, CheckCircle2, ChevronDown, ChevronRight, CircleCheck, CircleDot, CircleX, Loader2, TriangleAlert, type LucideIcon } from "lucide-react";
import type { RunRecord, RunStatus } from "@/api/types";
import { cn } from "@/lib/utils";
import type { RunProgress } from "./runStats";
import { useI18n } from "@/i18n/I18nProvider";
import type { Messages } from "@/i18n";
import { ToolTag } from "@/pages/memory/ToolTag";
import { fmt } from "@/pages/memory/paths";

/** Localized text for a failed run: its `error_code` when known, else the raw detail. */
export function runErrorText(x: Messages["extract"], r: Pick<RunRecord, "error" | "error_code">): string | null {
  if (r.error_code) return x.errorCodes[r.error_code] ?? r.error;
  return r.error;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** Local time "MM-DD HH:mm". */
export function formatRunTime(ms: number): string {
  const d = new Date(ms);
  return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

const STATUS: Record<RunStatus, { icon: LucideIcon; tone: string; spin?: boolean }> = {
  running: { icon: Loader2, tone: "text-accent-text", spin: true },
  succeeded: { icon: CircleCheck, tone: "text-accent-text" },
  failed: { icon: CircleX, tone: "text-destructive" },
  cancelled: { icon: Ban, tone: "text-muted-foreground" },
  interrupted: { icon: TriangleAlert, tone: "text-warning" },
};

function StatusCell({ status }: { status: RunStatus }) {
  const { t } = useI18n();
  const x = t.extract;
  const label = {
    running: x.statusRunning,
    succeeded: x.statusSucceeded,
    failed: x.statusFailed,
    cancelled: x.statusCancelled,
    interrupted: x.statusInterrupted,
  }[status];
  const { icon: Icon, tone, spin } = STATUS[status];
  return (
    <span className={cn("inline-flex items-center gap-1.5", tone)}>
      <Icon className={cn("size-4 shrink-0", spin && "animate-spin motion-reduce:animate-none")} aria-hidden="true" />
      <span>{label}</span>
    </span>
  );
}

const TH = "px-3 py-2 text-left text-xs font-medium whitespace-nowrap text-muted-foreground";

function ProgressCell({ p, x }: { p: RunProgress | undefined; x: Messages["extract"] }) {
  if (!p || p.total === 0) return <span className="text-muted-foreground">—</span>;
  const handled = p.total - p.pending;
  const detail = fmt(x.progressDetail, { accepted: p.accepted, rejected: p.rejected, stale: p.stale, pending: p.pending });
  if (p.pending === 0)
    return (
      <span title={detail} className="inline-flex items-center gap-1.5 text-muted-foreground">
        <CheckCircle2 className="size-3.5 text-emerald-600 dark:text-emerald-400" aria-hidden="true" />
        {x.progressDone}
      </span>
    );
  if (handled === 0)
    return (
      <span title={detail} className="inline-flex items-center gap-1.5">
        <CircleDot className="size-3.5 text-warning" aria-hidden="true" />
        {x.progressNone}
      </span>
    );
  return (
    <span title={detail} className="inline-flex items-center gap-1.5">
      <CircleDot className="size-3.5 text-primary" aria-hidden="true" />
      {fmt(x.progressPartial, { done: handled, total: p.total })}
    </span>
  );
}

export function RunsTable({
  runs,
  progress,
  onOpenReview,
}: {
  runs: RunRecord[];
  /** Review progress of each run's suggestions, by run id. */
  progress?: Map<string, RunProgress>;
  onOpenReview?: (run: RunRecord) => void;
}) {
  const { t } = useI18n();
  const x = t.extract;
  const [open, setOpen] = React.useState<Set<string>>(new Set());
  const toggle = (id: string) =>
    setOpen((s) => {
      const n = new Set(s);
      if (!n.delete(id)) n.add(id);
      return n;
    });

  return (
    <div className="overflow-x-auto">
      <table className="w-full border-collapse text-sm">
        <thead>
          <tr className="border-b border-border">
            <th scope="col" className={cn(TH, "w-8")}>
              <span className="sr-only">{x.expand}</span>
            </th>
            <th scope="col" className={TH}>{x.colTime}</th>
            <th scope="col" className={TH}>{x.colTrigger}</th>
            <th scope="col" className={TH}>{x.colTool}</th>
            <th scope="col" className={cn(TH, "text-right")}>{x.colSessions}</th>
            <th scope="col" className={cn(TH, "text-right")}>{x.colProjects}</th>
            <th scope="col" className={cn(TH, "text-right")}>{x.colNew}</th>
            <th scope="col" className={TH}>{x.colProgress}</th>
            <th scope="col" className={TH}>{x.colStatus}</th>
          </tr>
        </thead>
        <tbody>
          {runs.map((r) => {
            const hasDetail = !!r.error || r.warnings.length > 0;
            const expanded = open.has(r.id);
            return (
              <React.Fragment key={r.id}>
                <tr className="border-b border-border last:border-b-0 hover:bg-muted/50">
                  <td className="px-3 py-2">
                    {hasDetail ? (
                      <button
                        type="button"
                        aria-expanded={expanded}
                        aria-label={expanded ? x.collapse : x.expand}
                        onClick={() => toggle(r.id)}
                        className="grid size-6 cursor-pointer place-items-center rounded-sm text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                      >
                        {expanded ? <ChevronDown className="size-4" aria-hidden="true" /> : <ChevronRight className="size-4" aria-hidden="true" />}
                      </button>
                    ) : null}
                  </td>
                  <td className="px-3 py-2 whitespace-nowrap tabular-nums">{formatRunTime(r.started_at)}</td>
                  <td className="px-3 py-2 whitespace-nowrap">{r.trigger === "manual" ? x.triggerManual : x.triggerScheduled}</td>
                  <td className="px-3 py-2"><ToolTag tool={r.tool} /></td>
                  <td className="px-3 py-2 text-right tabular-nums">{r.sessions_read}</td>
                  <td className="px-3 py-2 text-right tabular-nums">{r.projects}</td>
                  <td className="px-3 py-2 text-right tabular-nums">
                    {r.suggestions_new > 0 && onOpenReview && (progress?.get(r.id)?.pending ?? 0) > 0 ? (
                      <button
                        type="button"
                        title={x.openReview}
                        onClick={() => onOpenReview(r)}
                        className="cursor-pointer rounded-sm px-1 font-medium text-accent-text underline-offset-2 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
                      >
                        {r.suggestions_new}
                      </button>
                    ) : (
                      r.suggestions_new
                    )}
                  </td>
                  <td className="px-3 py-2 whitespace-nowrap">
                    <ProgressCell p={progress?.get(r.id)} x={x} />
                  </td>
                  <td className="px-3 py-2 whitespace-nowrap"><StatusCell status={r.status} /></td>
                </tr>
                {expanded ? (
                  <tr className="border-b border-border bg-muted/30 last:border-b-0">
                    <td />
                    <td colSpan={8} className="px-3 py-3">
                      {r.error ? (
                        <div className="mb-2">
                          <p className="mb-1 text-xs font-medium text-muted-foreground">{x.errorLabel}</p>
                          {r.error_code ? (
                            <>
                              <p title={r.error} className="text-xs text-destructive">{runErrorText(x, r)}</p>
                              <p className="mt-0.5 font-mono text-xs [overflow-wrap:anywhere] whitespace-pre-wrap text-muted-foreground">{r.error}</p>
                            </>
                          ) : (
                            <p className="font-mono text-xs [overflow-wrap:anywhere] whitespace-pre-wrap text-destructive">{r.error}</p>
                          )}
                        </div>
                      ) : null}
                      {r.warnings.length > 0 ? (
                        <div>
                          <p className="mb-1 text-xs font-medium text-muted-foreground">{x.warningsLabel}</p>
                          <ul className="space-y-0.5">
                            {r.warnings.map((w, i) => (
                              <li key={i} className="flex items-start gap-1.5 text-xs">
                                <TriangleAlert className="mt-0.5 size-3.5 shrink-0 text-warning" aria-hidden="true" />
                                <span className="[overflow-wrap:anywhere]">{w}</span>
                              </li>
                            ))}
                          </ul>
                        </div>
                      ) : null}
                    </td>
                  </tr>
                ) : null}
              </React.Fragment>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

