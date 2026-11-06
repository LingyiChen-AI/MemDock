import type { UsageRow, UsageView } from "@/api/types";
import { localDate } from "./usageUtils";

/** Default heatmap span: 26 weeks. */
export const HEATMAP_DAYS = 26 * 7;

export type HeatMetric = "tokens" | "cost";

/** `$2,308.53`: always two decimals with thousands separators (KPI card). */
export function fmtCostFull(n: number): string {
  if (n > 0 && n < 0.01) return "<$0.01";
  return `$${n.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

/** `12,345`. */
export function fmtInt(n: number): string {
  return Math.round(n).toLocaleString("en-US");
}

function parseLocal(s: string): Date {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(y, m - 1, d);
}

function addDays(d: Date, n: number): Date {
  const x = new Date(d);
  x.setDate(x.getDate() + n);
  return x;
}

/**
 * Days the heatmap covers: the current filter range, extended back so at least
 * 26 weeks are shown. The `today` view shows the last 26 weeks.
 */
export function heatmapRange(view: UsageView, range: { from: string; to: string }, now: Date): { from: string; to: string } {
  const to = view === "today" ? localDate(now) : range.to;
  const minFrom = localDate(addDays(parseLocal(to), -(HEATMAP_DAYS - 1)));
  const from = view === "today" || range.from > minFrom ? minFrom : range.from;
  return { from, to };
}

export interface HeatCell {
  date: string;
  cost: number;
  tokens: number;
  requests: number;
  /** 0 (no usage) to 4 (most). */
  level: 0 | 1 | 2 | 3 | 4;
}

export interface HeatGrid {
  /** Columns; each has 7 slots Monday..Sunday, `null` outside the range. */
  weeks: (HeatCell | null)[][];
  /** Month label at the week column where the month starts (`YYYY-MM`). */
  months: { week: number; month: string }[];
  max: number;
}

/** Level of `v` relative to `max`: 0 for nothing, then quartiles 1..4. */
export function levelOf(v: number, max: number): HeatCell["level"] {
  if (v <= 0 || max <= 0) return 0;
  return Math.min(4, Math.max(1, Math.ceil((v / max) * 4))) as HeatCell["level"];
}

/** Lays daily rows (`YYYY-MM-DD` keys) out in Monday-first week columns. */
export function buildHeatGrid(rows: UsageRow[], from: string, to: string, metric: HeatMetric): HeatGrid {
  const byDay = new Map(rows.map((r) => [r.key, r.totals]));
  const value = (k: string) => {
    const t = byDay.get(k);
    if (!t) return 0;
    return metric === "cost" ? t.cost : t.total_tokens;
  };
  const start = parseLocal(from);
  const end = parseLocal(to);
  let max = 0;
  for (let d = start; d <= end; d = addDays(d, 1)) max = Math.max(max, value(localDate(d)));

  const weeks: (HeatCell | null)[][] = [];
  const months: HeatGrid["months"] = [];
  // Monday on or before `from` (getDay: 0 = Sunday).
  let cursor = addDays(start, -((start.getDay() + 6) % 7));
  while (cursor <= end) {
    const col: (HeatCell | null)[] = [];
    for (let i = 0; i < 7; i++) {
      const d = addDays(cursor, i);
      if (d < start || d > end) {
        col.push(null);
        continue;
      }
      const key = localDate(d);
      const t = byDay.get(key);
      if (d.getDate() === 1 || (weeks.length === 0 && months.length === 0)) {
        months.push({ week: weeks.length, month: key.slice(0, 7) });
      }
      col.push({
        date: key,
        cost: t?.cost ?? 0,
        tokens: t?.total_tokens ?? 0,
        requests: t?.requests ?? 0,
        level: levelOf(value(key), max),
      });
    }
    weeks.push(col);
    cursor = addDays(cursor, 7);
  }
  // A label right before the next one would overlap it; drop the earlier one.
  const spaced = months.filter((m, i) => i + 1 >= months.length || months[i + 1].week - m.week >= 3);
  return { weeks, months: spaced, max };
}
