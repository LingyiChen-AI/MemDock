import { Coins, Database, Hash, Zap } from "lucide-react";
import type * as React from "react";
import type { UsageReport, UsageRow, UsageTotals, UsageView } from "@/api/types";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";
import { fmtCostFull, fmtInt } from "./heatmapUtils";
import { fmtCost, fmtPct, fmtTokens } from "./usageUtils";

interface Split {
  claude: number;
  codex: number;
  format: (n: number) => string;
  /** Share of the bar; defaults to the values themselves (not meaningful for a rate). */
  weights?: [number, number];
}

function Kpi({ icon, label, value, sub, split }: { icon: React.ReactNode; label: string; value: string; sub?: string; split?: Split }) {
  const { t } = useI18n();
  const [wc, wx] = split?.weights ?? [split?.claude ?? 0, split?.codex ?? 0];
  const sum = wc + wx;
  return (
    <div className="flex min-w-0 flex-col gap-1 rounded-lg border border-border bg-card px-4 py-3">
      <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
        {icon}
        {label}
      </span>
      <span className="truncate text-2xl leading-8 font-semibold tabular-nums">{value}</span>
      <span className="truncate text-xs text-muted-foreground tabular-nums">{sub ?? " "}</span>
      {split ? (
        <div className="mt-1.5 flex flex-col gap-1.5 border-t border-border/70 pt-2">
          <div className="flex h-1.5 w-full overflow-hidden rounded-full bg-muted" aria-hidden="true">
            {sum > 0 ? (
              <>
                <span className="h-full bg-tool-claude" style={{ width: `${(wc / sum) * 100}%` }} />
                <span className="h-full bg-tool-codex" style={{ width: `${(wx / sum) * 100}%` }} />
              </>
            ) : null}
          </div>
          <div className="flex items-center justify-between gap-2 text-[11px] text-muted-foreground tabular-nums">
            <span className="flex min-w-0 items-center gap-1 truncate">
              <span className="size-2 shrink-0 rounded-full bg-tool-claude" aria-hidden="true" />
              {t.memory.toolClaude} {split.format(split.claude)}
            </span>
            <span className="flex min-w-0 items-center gap-1 truncate">
              <span className="size-2 shrink-0 rounded-full bg-tool-codex" aria-hidden="true" />
              {t.memory.toolCodex} {split.format(split.codex)}
            </span>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function sumTool(rows: UsageRow[], tool: "claude" | "codex"): UsageTotals {
  const z: UsageTotals = { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0, requests: 0, cost: 0, unpriced_tokens: 0 };
  for (const r of rows) {
    const x = r[tool];
    z.input += x.input;
    z.output += x.output;
    z.cache_read += x.cache_read;
    z.cache_write += x.cache_write;
    z.total_tokens += x.total_tokens;
    z.requests += x.requests;
    z.cost += x.cost;
    z.unpriced_tokens += x.unpriced_tokens;
  }
  return z;
}

function hitRate(x: UsageTotals): number | null {
  const all = x.input + x.cache_read + x.cache_write;
  return all > 0 ? x.cache_read / all : null;
}

/** Total cost, requests, tokens and cache hit rate for the current filter. */
export function UsageKpis({ report, view }: { report: UsageReport | null; view: UsageView }) {
  const { t } = useI18n();
  const k = t.usage.kpi;
  const s = report?.summary;
  const tt = s?.totals;
  const perDay = view !== "today" && s && tt && s.days > 0;
  const iconCls = "size-3.5";
  const rows = report?.rows ?? [];
  const c = sumTool(rows, "claude");
  const x = sumTool(rows, "codex");
  const both = c.total_tokens > 0 && x.total_tokens > 0;
  const pct = (n: number | null) => fmtPct(n);
  return (
    <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
      <Kpi
        icon={<Coins className={iconCls} aria-hidden="true" />}
        label={k.cost}
        value={tt ? fmtCostFull(tt.cost) : "—"}
        sub={perDay ? fmt(k.dailyAvg, { value: fmtCost(s.daily_avg_cost) }) : undefined}
        split={both ? { claude: c.cost, codex: x.cost, format: fmtCost } : undefined}
      />
      <Kpi
        icon={<Hash className={iconCls} aria-hidden="true" />}
        label={k.requests}
        value={tt ? fmtInt(tt.requests) : "—"}
        sub={perDay ? fmt(k.dailyAvg, { value: fmtInt(tt.requests / s.days) }) : undefined}
        split={both ? { claude: c.requests, codex: x.requests, format: fmtInt } : undefined}
      />
      <Kpi
        icon={<Zap className={iconCls} aria-hidden="true" />}
        label={k.tokens}
        value={tt ? fmtTokens(tt.total_tokens) : "—"}
        sub={tt ? fmt(k.tokensSub, { input: fmtTokens(tt.input + tt.cache_read + tt.cache_write), output: fmtTokens(tt.output) }) : undefined}
        split={both ? { claude: c.total_tokens, codex: x.total_tokens, format: fmtTokens } : undefined}
      />
      <Kpi
        icon={<Database className={iconCls} aria-hidden="true" />}
        label={k.hitRate}
        value={s ? fmtPct(s.cache_hit_rate) : "—"}
        sub={k.hitRateSub}
        split={both ? { claude: hitRate(c) ?? 0, codex: hitRate(x) ?? 0, format: (n) => pct(n), weights: [c.input + c.cache_read + c.cache_write, x.input + x.cache_read + x.cache_write] } : undefined}
      />
    </div>
  );
}
