import type { UsageRow, UsageTotals, UsageView } from "@/api/types";
import { localDate } from "./usageUtils";

/** Span of the heatmap in the `today` view (the page itself shows only one day). */
export const TODAY_SPAN_DAYS = 30;

export type HeatMetric = "tokens" | "cost";
/** Which tool's usage colors the cells. */
export type HeatTool = "all" | "claude" | "codex";

/** Cell pitch (cell + gap) in px: fills the width, but stays within these bounds. */
export const MIN_STEP = 13;
export const MAX_STEP = 30;

/** Fixed cell pitch (cell + gap) of the GitHub-style wall. */
export const WALL_STEP = 16;

/**
 * First day shown: enough whole weeks before `to` to fill `available` px at
 * `WALL_STEP`, but never later than `rangeFrom` (a long range scrolls instead).
 */
export function wallStart(rangeFrom: string, to: string, available: number): string {
  const end = parseLocal(to);
  const weekStart = addDays(end, -((end.getDay() + 6) % 7));
  const fit = Math.max(1, Math.floor(available / WALL_STEP));
  const fitted = localDate(addDays(weekStart, -7 * (fit - 1)));
  return fitted < rangeFrom ? fitted : rangeFrom;
}

/** Pitch that spreads `weeks` columns over `available` px, clamped to [MIN_STEP, MAX_STEP]. */
export function cellStep(available: number, weeks: number): number {
  if (available <= 0 || weeks <= 0) return MIN_STEP;
  return Math.max(MIN_STEP, Math.min(MAX_STEP, Math.floor(available / weeks)));
}

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
 * Days the heatmap covers: exactly the page's filter range. The `today` view
 * (a single day on the page) shows the last 30 days up to today.
 */
export function heatmapRange(view: UsageView, range: { from: string; to: string }, now: Date): { from: string; to: string } {
  if (view === "today") {
    const to = localDate(now);
    return { from: localDate(addDays(parseLocal(to), -(TODAY_SPAN_DAYS - 1))), to };
  }
  return range.from <= range.to ? { from: range.from, to: range.to } : { from: range.to, to: range.from };
}

export interface HeatStat {
  cost: number;
  tokens: number;
  requests: number;
}

export interface HeatCell {
  date: string;
  /** Both tools together. */
  total: HeatStat;
  claude: HeatStat;
  codex: HeatStat;
  /** 0 (no usage) to 4 (most), from the selected tool and metric. */
  level: 0 | 1 | 2 | 3 | 4;
  /** Before the selected range: drawn as an empty cell, no tooltip. */
  outside?: boolean;
}

const ZERO: HeatStat = { cost: 0, tokens: 0, requests: 0 };

function stat(t: UsageTotals | undefined): HeatStat {
  return t ? { cost: t.cost, tokens: t.total_tokens, requests: t.requests } : ZERO;
}

function pick(r: UsageRow | undefined, tool: HeatTool): HeatStat {
  if (!r) return ZERO;
  return stat(tool === "claude" ? r.claude : tool === "codex" ? r.codex : r.totals);
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

/**
 * Lays daily rows (`YYYY-MM-DD` keys) out in Monday-first week columns. Cells
 * are colored by `tool`'s usage (`all` = both tools); every cell still carries
 * the per-tool split for the tooltip.
 */
export function buildHeatGrid(
  rows: UsageRow[],
  from: string,
  to: string,
  metric: HeatMetric,
  tool: HeatTool = "all",
  /** Days before this are padding: empty cells, excluded from the color scale. */
  rangeFrom: string = from,
): HeatGrid {
  const byDay = new Map(rows.map((r) => [r.key, r]));
  const value = (k: string) => {
    const s = pick(byDay.get(k), tool);
    return metric === "cost" ? s.cost : s.tokens;
  };
  const start = parseLocal(from);
  const end = parseLocal(to);
  let max = 0;
  for (let d = parseLocal(rangeFrom); d <= end; d = addDays(d, 1)) max = Math.max(max, value(localDate(d)));

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
      const r = byDay.get(key);
      if (d.getDate() === 1 || (weeks.length === 0 && months.length === 0)) {
        months.push({ week: weeks.length, month: key.slice(0, 7) });
      }
      const outside = key < rangeFrom;
      col.push({
        date: key,
        total: pick(r, "all"),
        claude: pick(r, "claude"),
        codex: pick(r, "codex"),
        level: outside ? 0 : levelOf(value(key), max),
        ...(outside ? { outside: true } : {}),
      });
    }
    weeks.push(col);
    cursor = addDays(cursor, 7);
  }
  // A label right before the next one would overlap it; drop the earlier one.
  const spaced = months.filter((m, i) => i + 1 >= months.length || months[i + 1].week - m.week >= 3);
  return { weeks, months: spaced, max };
}
