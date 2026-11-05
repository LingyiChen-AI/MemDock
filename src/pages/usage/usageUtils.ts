import type { UsageReport, UsageView } from "@/api/types";

/** Local calendar date as `YYYY-MM-DD`. */
export function localDate(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

export type RangePreset = "last7" | "last30" | "thisMonth" | "last6m" | "last12m" | "custom";

export const PRESETS: Record<Exclude<UsageView, "today">, RangePreset[]> = {
  daily: ["last7", "last30", "thisMonth"],
  monthly: ["last6m", "last12m"],
};

/** Inclusive date range for a preset, relative to `now`. */
export function presetRange(preset: RangePreset, now: Date): { from: string; to: string } {
  const to = localDate(now);
  const back = (days: number) => {
    const d = new Date(now);
    d.setDate(d.getDate() - days);
    return localDate(d);
  };
  const monthsBack = (m: number) => localDate(new Date(now.getFullYear(), now.getMonth() - m, 1));
  switch (preset) {
    case "last7":
      return { from: back(6), to };
    case "thisMonth":
      return { from: localDate(new Date(now.getFullYear(), now.getMonth(), 1)), to };
    case "last6m":
      return { from: monthsBack(5), to };
    case "last12m":
      return { from: monthsBack(11), to };
    case "last30":
    case "custom":
    default:
      return { from: back(29), to };
  }
}

/** `$12.34`; tiny non-zero values show `<$0.01`; large values drop cents. */
export function fmtCost(n: number): string {
  if (n > 0 && n < 0.01) return "<$0.01";
  if (n >= 10_000) return `$${Math.round(n).toLocaleString("en-US")}`;
  return `$${n.toFixed(2)}`;
}

/** 999, 12.3K, 4.56M, 1.2B. */
export function fmtTokens(n: number): string {
  const units: [number, string][] = [
    [1e9, "B"],
    [1e6, "M"],
    [1e3, "K"],
  ];
  for (const [v, u] of units) {
    if (n >= v) {
      const x = n / v;
      return `${x >= 100 ? x.toFixed(0) : x >= 10 ? x.toFixed(1) : x.toFixed(2)}${u}`;
    }
  }
  return String(Math.round(n));
}

export function fmtPct(ratio: number | null): string {
  if (ratio === null || Number.isNaN(ratio)) return "—";
  return `${(ratio * 100).toFixed(1)}%`;
}

/** Splits a positive duration into days / hours / minutes for the reset countdown. */
export function splitDuration(ms: number): { d: number; h: number; m: number } {
  const totalMin = Math.max(0, Math.ceil(ms / 60_000));
  return { d: Math.floor(totalMin / 1440), h: Math.floor((totalMin % 1440) / 60), m: totalMin % 60 };
}

export function fmtDuration(ms: number, tpl: { d: string; h: string; m: string }): string {
  const { d, h, m } = splitDuration(ms);
  const fill = (s: string, v: Record<string, number>) => s.replace(/\{(\w)\}/g, (_, k: string) => String(v[k] ?? 0));
  if (d > 0) return fill(tpl.d, { d, h });
  if (h > 0) return fill(tpl.h, { h, m });
  return fill(tpl.m, { m });
}

export interface ChartPoint {
  key: string;
  label: string;
  claude: number;
  codex: number;
  total: number;
}

/** One point per row; labels are shortened (`11-05`, `14:00`, `2026-11`). */
export function chartData(r: UsageReport): ChartPoint[] {
  return r.rows.map((row) => ({
    key: row.key,
    label: r.view === "today" ? `${row.key}:00` : r.view === "daily" ? row.key.slice(5) : row.key,
    claude: round2(row.claude.cost),
    codex: round2(row.codex.cost),
    total: round2(row.totals.cost),
  }));
}

function round2(n: number): number {
  return Math.round(n * 100) / 100;
}

/** First and last day of a `YYYY-MM` month (for drilling from monthly into daily). */
export function monthRange(month: string): { from: string; to: string } {
  const [y, m] = month.split("-").map(Number);
  return { from: localDate(new Date(y, m - 1, 1)), to: localDate(new Date(y, m, 0)) };
}
