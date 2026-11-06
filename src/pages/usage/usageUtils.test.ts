import { describe, expect, it } from "vitest";
import type { UsageReport } from "@/api/types";
import { chartData, fmtCost, fmtDuration, fmtPct, fmtTokens, localDate, monthRange, presetRange } from "./usageUtils";

const zero = { input: 0, output: 0, cache_read: 0, cache_write: 0, total_tokens: 0, requests: 0, cost: 0, unpriced_tokens: 0 };

describe("usage formatting", () => {
  it("formats cost, tokens and percentages", () => {
    expect(fmtCost(0)).toBe("$0.00");
    expect(fmtCost(0.004)).toBe("<$0.01");
    expect(fmtCost(12.345)).toBe("$12.35");
    expect(fmtCost(12345.6)).toBe("$12,346");
    expect(fmtTokens(999)).toBe("999");
    expect(fmtTokens(12_345)).toBe("12.3K");
    expect(fmtTokens(4_560_000)).toBe("4.56M");
    expect(fmtTokens(123_000_000)).toBe("123M");
    expect(fmtPct(0.4567)).toBe("45.7%");
    expect(fmtPct(null)).toBe("—");
  });

  it("formats reset countdowns", () => {
    const tpl = { d: "{d}d {h}h", h: "{h}h {m}m", m: "{m}m" };
    expect(fmtDuration(90 * 60_000, tpl)).toBe("1h 30m");
    expect(fmtDuration(3 * 86_400_000 + 2 * 3_600_000, tpl)).toBe("3d 2h");
    expect(fmtDuration(30_000, tpl)).toBe("1m");
  });
});

describe("usage ranges", () => {
  const now = new Date(2026, 10, 6, 15, 0); // 2026-11-06 local
  it("computes presets in local time", () => {
    expect(localDate(now)).toBe("2026-11-06");
    expect(presetRange("last7", now)).toEqual({ from: "2026-10-31", to: "2026-11-06" });
    expect(presetRange("last30", now)).toEqual({ from: "2026-10-08", to: "2026-11-06" });
    expect(presetRange("thisMonth", now)).toEqual({ from: "2026-11-01", to: "2026-11-06" });
    expect(presetRange("last12m", now)).toEqual({ from: "2025-12-01", to: "2026-11-06" });
    expect(monthRange("2026-02")).toEqual({ from: "2026-02-01", to: "2026-02-28" });
  });

  it("builds stacked chart points", () => {
    const r = {
      view: "daily",
      rows: [{ key: "2026-11-05", totals: { ...zero, cost: 3.333 }, claude: { ...zero, cost: 1.111 }, codex: { ...zero, cost: 2.222 }, breakdown: [] }],
    } as unknown as UsageReport;
    expect(chartData(r)).toEqual([{ key: "2026-11-05", label: "11-05", claude: 1.11, codex: 2.22, total: 3.33 }]);
    expect(chartData({ ...r, view: "today", rows: [{ ...r.rows[0], key: "09" }] })[0].label).toBe("09:00");
  });
});
