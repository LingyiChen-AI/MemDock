import { Coins, Database, Hash, Zap } from "lucide-react";
import type * as React from "react";
import type { UsageReport, UsageView } from "@/api/types";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";
import { fmtCostFull, fmtInt } from "./heatmapUtils";
import { fmtCost, fmtPct, fmtTokens } from "./usageUtils";

function Kpi({ icon, label, value, sub }: { icon: React.ReactNode; label: string; value: string; sub?: string }) {
  return (
    <div className="flex min-w-0 flex-col gap-1 rounded-lg border border-border bg-card px-4 py-3">
      <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
        {icon}
        {label}
      </span>
      <span className="truncate text-2xl leading-8 font-semibold tabular-nums">{value}</span>
      <span className="truncate text-xs text-muted-foreground tabular-nums">{sub ?? " "}</span>
    </div>
  );
}

/** Total cost, requests, tokens and cache hit rate for the current filter. */
export function UsageKpis({ report, view }: { report: UsageReport | null; view: UsageView }) {
  const { t } = useI18n();
  const k = t.usage.kpi;
  const s = report?.summary;
  const tt = s?.totals;
  const perDay = view !== "today" && s && tt && s.days > 0;
  const iconCls = "size-3.5";
  return (
    <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
      <Kpi
        icon={<Coins className={iconCls} aria-hidden="true" />}
        label={k.cost}
        value={tt ? fmtCostFull(tt.cost) : "—"}
        sub={perDay ? fmt(k.dailyAvg, { value: fmtCost(s.daily_avg_cost) }) : undefined}
      />
      <Kpi
        icon={<Hash className={iconCls} aria-hidden="true" />}
        label={k.requests}
        value={tt ? fmtInt(tt.requests) : "—"}
        sub={perDay ? fmt(k.dailyAvg, { value: fmtInt(tt.requests / s.days) }) : undefined}
      />
      <Kpi
        icon={<Zap className={iconCls} aria-hidden="true" />}
        label={k.tokens}
        value={tt ? fmtTokens(tt.total_tokens) : "—"}
        sub={tt ? fmt(k.tokensSub, { input: fmtTokens(tt.input + tt.cache_read + tt.cache_write), output: fmtTokens(tt.output) }) : undefined}
      />
      <Kpi
        icon={<Database className={iconCls} aria-hidden="true" />}
        label={k.hitRate}
        value={s ? fmtPct(s.cache_hit_rate) : "—"}
        sub={k.hitRateSub}
      />
    </div>
  );
}
