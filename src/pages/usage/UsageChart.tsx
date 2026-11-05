import { Bar, BarChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import type { UsageReport } from "@/api/types";
import { useI18n } from "@/i18n/I18nProvider";
import { chartData, fmtCost, type ChartPoint } from "./usageUtils";

interface TipProps {
  active?: boolean;
  payload?: { payload: ChartPoint }[];
}

function Tip({ active, payload }: TipProps) {
  const { t } = useI18n();
  const p = payload?.[0]?.payload;
  if (!active || !p) return null;
  return (
    <div className="rounded-md border border-border bg-popover px-3 py-2 text-xs text-popover-foreground shadow-lg">
      <p className="mb-1 font-semibold">{p.key}</p>
      <p className="flex items-center gap-2">
        <span className="size-2 rounded-sm bg-tool-claude" aria-hidden="true" />
        <span className="flex-1">{t.memory.toolClaude}</span>
        <span className="tabular-nums">{fmtCost(p.claude)}</span>
      </p>
      <p className="flex items-center gap-2">
        <span className="size-2 rounded-sm bg-tool-codex" aria-hidden="true" />
        <span className="flex-1">{t.memory.toolCodex}</span>
        <span className="tabular-nums">{fmtCost(p.codex)}</span>
      </p>
      <p className="mt-1 flex justify-between gap-4 border-t border-border pt-1 font-semibold">
        <span>{t.usage.chart.total}</span>
        <span className="tabular-nums">{fmtCost(p.total)}</span>
      </p>
    </div>
  );
}

/** Cost per period stacked by tool; clicking a bar reports its row key. */
export function UsageChart({ report, onPick }: { report: UsageReport; onPick: (key: string) => void }) {
  const { t } = useI18n();
  const data = chartData(report);
  const title = report.view === "today" ? t.usage.chart.today : report.view === "monthly" ? t.usage.chart.monthly : t.usage.chart.daily;
  const pick = (d: unknown) => {
    const key = (d as { payload?: ChartPoint } | undefined)?.payload?.key;
    if (key) onPick(key);
  };
  const tick = { fill: "var(--muted-foreground)", fontSize: 12 };
  return (
    <section className="flex flex-col gap-2 rounded-lg border border-border bg-card p-4">
      <header className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-sm font-semibold">{title}</h2>
        <div className="flex items-center gap-4 text-xs text-muted-foreground">
          <span className="flex items-center gap-1.5">
            <span className="size-2.5 rounded-sm bg-tool-claude" aria-hidden="true" />
            {t.memory.toolClaude}
          </span>
          <span className="flex items-center gap-1.5">
            <span className="size-2.5 rounded-sm bg-tool-codex" aria-hidden="true" />
            {t.memory.toolCodex}
          </span>
          {report.view !== "today" ? <span>{t.usage.chart.hint}</span> : null}
        </div>
      </header>
      <div className="h-56 w-full">
        <ResponsiveContainer width="100%" height="100%">
          <BarChart data={data} margin={{ top: 8, right: 8, bottom: 0, left: 0 }}>
            <CartesianGrid vertical={false} stroke="var(--border)" strokeDasharray="3 3" />
            <XAxis dataKey="label" tick={tick} tickLine={false} axisLine={{ stroke: "var(--border)" }} minTickGap={12} />
            <YAxis tick={tick} tickLine={false} axisLine={false} width={56} tickFormatter={(v: number) => fmtCost(v)} />
            <Tooltip content={<Tip />} cursor={{ fill: "var(--muted)", opacity: 0.5 }} />
            <Bar dataKey="claude" stackId="cost" fill="var(--tool-claude)" cursor="pointer" onClick={pick} isAnimationActive={false} />
            <Bar dataKey="codex" stackId="cost" fill="var(--tool-codex)" radius={[3, 3, 0, 0]} cursor="pointer" onClick={pick} isAnimationActive={false} />
          </BarChart>
        </ResponsiveContainer>
      </div>
    </section>
  );
}
