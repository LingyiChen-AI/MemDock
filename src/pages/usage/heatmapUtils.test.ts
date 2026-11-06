import { describe, expect, it } from "vitest";
import type { UsageRow, UsageTotals } from "@/api/types";
import { buildHeatGrid, fmtCostFull, fmtInt, heatmapRange, levelOf } from "./heatmapUtils";

const totals = (tokens: number, cost: number, requests: number): UsageTotals => ({
  input: tokens,
  output: 0,
  cache_read: 0,
  cache_write: 0,
  total_tokens: tokens,
  requests,
  cost,
  unpriced_tokens: 0,
});
const row = (key: string, tokens: number, cost: number, requests = 1): UsageRow => {
  const t = totals(tokens, cost, requests);
  return { key, totals: t, claude: t, codex: totals(0, 0, 0), breakdown: [] };
};

describe("KPI formatting", () => {
  it("formats full cost and integers", () => {
    expect(fmtCostFull(2308.53)).toBe("$2,308.53");
    expect(fmtCostFull(0)).toBe("$0.00");
    expect(fmtCostFull(0.004)).toBe("<$0.01");
    expect(fmtInt(1234567)).toBe("1,234,567");
  });
});

describe("heatmap", () => {
  it("covers at least 26 weeks, or the longer filter range", () => {
    const now = new Date(2026, 10, 6);
    expect(heatmapRange("today", { from: "2026-11-01", to: "2026-11-01" }, now)).toEqual({ from: "2026-05-09", to: "2026-11-06" });
    expect(heatmapRange("daily", { from: "2026-10-08", to: "2026-11-06" }, now)).toEqual({ from: "2026-05-09", to: "2026-11-06" });
    expect(heatmapRange("monthly", { from: "2025-12-01", to: "2026-11-06" }, now)).toEqual({ from: "2025-12-01", to: "2026-11-06" });
  });

  it("assigns quartile levels", () => {
    expect(levelOf(0, 100)).toBe(0);
    expect(levelOf(1, 100)).toBe(1);
    expect(levelOf(26, 100)).toBe(2);
    expect(levelOf(75, 100)).toBe(3);
    expect(levelOf(100, 100)).toBe(4);
    expect(levelOf(5, 0)).toBe(0);
  });

  it("lays days out Monday-first with padding and month labels", () => {
    // 2026-10-28 is a Wednesday, 2026-11-08 a Sunday.
    const rows = [row("2026-10-28", 100, 1), row("2026-11-02", 400, 0.5, 3)];
    const g = buildHeatGrid(rows, "2026-10-28", "2026-11-08", "tokens");
    expect(g.weeks).toHaveLength(2);
    expect(g.weeks[0][0]).toBeNull();
    expect(g.weeks[0][1]).toBeNull();
    expect(g.weeks[0][2]?.date).toBe("2026-10-28");
    expect(g.weeks[0][2]?.level).toBe(1);
    expect(g.weeks[1][0]).toMatchObject({ date: "2026-11-02", tokens: 400, requests: 3, level: 4 });
    expect(g.weeks[1][6]?.date).toBe("2026-11-08");
    expect(g.max).toBe(400);
    // November starts in week 0 (Sunday the 1st) and the earlier October label is too close.
    expect(g.months).toEqual([{ week: 0, month: "2026-11" }]);

    const byCost = buildHeatGrid(rows, "2026-10-28", "2026-11-08", "cost");
    expect(byCost.weeks[0][2]?.level).toBe(4);
    expect(byCost.weeks[1][0]?.level).toBe(2);
  });
});
