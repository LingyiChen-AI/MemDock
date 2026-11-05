import * as React from "react";
import { ChevronRight, TriangleAlert } from "lucide-react";
import type { UsageReport, UsageTotals } from "@/api/types";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { ToolTag } from "@/pages/memory/ToolTag";
import { fmt } from "@/pages/memory/paths";
import { fmtCost, fmtTokens } from "./usageUtils";

function CostCell({ t: totals }: { t: UsageTotals }) {
  const { t } = useI18n();
  if (totals.unpriced_tokens > 0 && totals.cost === 0) {
    return <span className="text-warning">{t.usage.table.unknown}</span>;
  }
  return (
    <span className="inline-flex items-center justify-end gap-1">
      {totals.unpriced_tokens > 0 ? (
        <TriangleAlert className="size-3.5 text-warning" aria-label={t.usage.table.unknown} />
      ) : null}
      {fmtCost(totals.cost)}
    </span>
  );
}

function Nums({ t: x }: { t: UsageTotals }) {
  return (
    <>
      <td className="px-3 text-right tabular-nums">{fmtTokens(x.input)}</td>
      <td className="px-3 text-right tabular-nums">{fmtTokens(x.output)}</td>
      <td className="px-3 text-right tabular-nums">{fmtTokens(x.cache_read)}</td>
      <td className="px-3 text-right tabular-nums">{fmtTokens(x.cache_write)}</td>
      <td className="px-3 text-right tabular-nums">{fmtTokens(x.total_tokens)}</td>
      <td className="px-3 text-right tabular-nums">
        <CostCell t={x} />
      </td>
    </>
  );
}

/** One row per period (newest first, empty periods hidden), expandable into tool x model. */
export function UsageTable({ report, focusKey }: { report: UsageReport; focusKey: string | null }) {
  const { t } = useI18n();
  const tt = t.usage.table;
  const [open, setOpen] = React.useState<Set<string>>(new Set());
  const rowRefs = React.useRef(new Map<string, HTMLTableRowElement>());

  React.useEffect(() => {
    if (!focusKey) return;
    setOpen((s) => new Set(s).add(focusKey));
    rowRefs.current.get(focusKey)?.scrollIntoView?.({ block: "center", behavior: "smooth" });
  }, [focusKey]);

  const rows = report.rows.filter((r) => r.totals.total_tokens > 0).reverse();
  const head = report.view === "today" ? tt.hour : report.view === "monthly" ? tt.month : tt.date;
  const label = (key: string) => (report.view === "today" ? `${key}:00` : key);
  const toggle = (key: string) =>
    setOpen((s) => {
      const n = new Set(s);
      if (n.has(key)) n.delete(key);
      else n.add(key);
      return n;
    });

  if (rows.length === 0) {
    return <p className="rounded-lg border border-border bg-card px-4 py-8 text-center text-sm text-muted-foreground">{t.usage.empty}</p>;
  }
  return (
    <div className="overflow-x-auto rounded-lg border border-border bg-card">
      <table className="w-full min-w-[720px] text-sm">
        <thead>
          <tr className="h-9 border-b border-border text-xs text-muted-foreground">
            <th className="px-3 text-left font-medium">{head}</th>
            <th className="px-3 text-right font-medium">{tt.input}</th>
            <th className="px-3 text-right font-medium">{tt.output}</th>
            <th className="px-3 text-right font-medium">{tt.cacheRead}</th>
            <th className="px-3 text-right font-medium">{tt.cacheWrite}</th>
            <th className="px-3 text-right font-medium">{tt.total}</th>
            <th className="px-3 text-right font-medium">{tt.cost}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => {
            const expanded = open.has(r.key);
            return (
              <React.Fragment key={r.key}>
                <tr
                  ref={(el) => {
                    if (el) rowRefs.current.set(r.key, el);
                    else rowRefs.current.delete(r.key);
                  }}
                  className={cn("h-9 border-b border-border last:border-b-0", focusKey === r.key && "bg-muted/60")}
                >
                  <td className="px-3">
                    <button
                      type="button"
                      aria-expanded={expanded}
                      aria-label={expanded ? tt.collapse : fmt(tt.expand, { key: label(r.key) })}
                      onClick={() => toggle(r.key)}
                      className="inline-flex cursor-pointer items-center gap-1 rounded-sm font-medium tabular-nums outline-none hover:text-accent-text focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <ChevronRight
                        className={cn("size-3.5 text-muted-foreground transition-transform duration-150 motion-reduce:transition-none", expanded && "rotate-90")}
                        aria-hidden="true"
                      />
                      {label(r.key)}
                    </button>
                  </td>
                  <Nums t={r.totals} />
                </tr>
                {expanded
                  ? r.breakdown.map((b) => (
                      <tr key={`${r.key}-${b.tool}-${b.model}`} className="h-8 border-b border-border bg-muted/30 text-xs">
                        <td className="py-1 pr-3 pl-8">
                          <span className="flex min-w-0 items-center gap-2">
                            <ToolTag tool={b.tool} />
                            <span className="truncate font-mono" title={b.model}>
                              {b.model}
                            </span>
                          </span>
                        </td>
                        <Nums t={b.totals} />
                      </tr>
                    ))
                  : null}
              </React.Fragment>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
