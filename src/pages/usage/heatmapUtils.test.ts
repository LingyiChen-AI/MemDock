import { describe, expect, it } from "vitest";
import type { UsageRow, UsageTotals } from "@/api/types";
import { buildHeatGrid, cellStep, fmtCostFull, fmtInt, heatmapRange, levelOf, MAX_STEP, MIN_STEP, wallStart } from "./heatmapUtils";

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
/** A day with both tools: totals are the sum of the two. */
const mixed = (key: string, claude: [number, number, number], codex: [number, number, number]): UsageRow => ({
  key,
  claude: totals(...claude),
  codex: totals(...codex),
  totals: totals(claude[0] + codex[0], claude[1] + codex[1], claude[2] + codex[2]),
  breakdown: [],
});

describe("KPI formatting", () => {
  it("formats full cost and integers", () => {
    expect(fmtCostFull(2308.53)).toBe("$2,308.53");
    expect(fmtCostFull(0)).toBe("$0.00");
    expect(fmtCostFull(0.004)).toBe("<$0.01");
    expect(fmtInt(1234567)).toBe("1,234,567");
  });
});

describe("heatmap", () => {
  it("covers exactly the page range; the today view shows the last 30 days", () => {
    const now = new Date(2026, 10, 6);
    expect(heatmapRange("today", { from: "2026-11-01", to: "2026-11-01" }, now)).toEqual({ from: "2026-10-08", to: "2026-11-06" });
    expect(heatmapRange("daily", { from: "2026-10-08", to: "2026-11-06" }, now)).toEqual({ from: "2026-10-08", to: "2026-11-06" });
    expect(heatmapRange("daily", { from: "2026-10-31", to: "2026-11-06" }, now)).toEqual({ from: "2026-10-31", to: "2026-11-06" });
    expect(heatmapRange("monthly", { from: "2025-12-01", to: "2026-11-06" }, now)).toEqual({ from: "2025-12-01", to: "2026-11-06" });
    // A reversed custom range is put in order.
    expect(heatmapRange("daily", { from: "2026-11-06", to: "2026-11-01" }, now)).toEqual({ from: "2026-11-01", to: "2026-11-06" });
  });

  it("grows cells for short ranges within bounds", () => {
    expect(cellStep(1000, 5)).toBe(MAX_STEP);
    expect(cellStep(1000, 53)).toBe(18);
    expect(cellStep(300, 53)).toBe(MIN_STEP);
    expect(cellStep(0, 5)).toBe(MIN_STEP);
  });

  it("splits cells by tool and colors by the selected tool", () => {
    // 2026-11-02 (Mon) Claude heavy, 2026-11-03 Codex heavy.
    const rows = [mixed("2026-11-02", [400, 4, 4], [100, 1, 1]), mixed("2026-11-03", [0, 0, 0], [300, 2, 5])];
    const all = buildHeatGrid(rows, "2026-11-02", "2026-11-03", "tokens");
    const mon = all.weeks[0][0]!;
    expect(mon.total).toEqual({ tokens: 500, cost: 5, requests: 5 });
    expect(mon.claude).toEqual({ tokens: 400, cost: 4, requests: 4 });
    expect(mon.codex).toEqual({ tokens: 100, cost: 1, requests: 1 });
    expect(mon.level).toBe(4);
    expect(all.weeks[0][1]?.level).toBe(3);

    const codex = buildHeatGrid(rows, "2026-11-02", "2026-11-03", "tokens", "codex");
    expect(codex.max).toBe(300);
    expect(codex.weeks[0][0]?.level).toBe(2);
    expect(codex.weeks[0][1]?.level).toBe(4);
    // The split stays complete whatever the coloring.
    expect(codex.weeks[0][0]?.claude.tokens).toBe(400);

    const claude = buildHeatGrid(rows, "2026-11-02", "2026-11-03", "cost", "claude");
    expect(claude.weeks[0][1]?.level).toBe(0);
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
    expect(g.weeks[1][0]).toMatchObject({ date: "2026-11-02", total: { tokens: 400, requests: 3 }, level: 4 });
    expect(g.weeks[1][6]?.date).toBe("2026-11-08");
    expect(g.max).toBe(400);
    // November starts in week 0 (Sunday the 1st) and the earlier October label is too close.
    expect(g.months).toEqual([{ week: 0, month: "2026-11" }]);

    const byCost = buildHeatGrid(rows, "2026-10-28", "2026-11-08", "cost");
    expect(byCost.weeks[0][2]?.level).toBe(4);
    expect(byCost.weeks[1][0]?.level).toBe(2);
  });
});

describe("wallStart", () => {
  it("fills the width with whole weeks ending at the range end", () => {
    // 2026-11-06 is a Friday; its week starts Monday 2026-11-02. 5 weeks fit in 80px.
    expect(wallStart("2026-11-01", "2026-11-06", 80)).toBe("2026-10-05");
  });
  it("never starts after the range start (long ranges scroll)", () => {
    expect(wallStart("2026-01-01", "2026-11-06", 80)).toBe("2026-01-01");
  });
});

describe("buildHeatGrid padding", () => {
  it("marks days before the range as outside with level 0", () => {
    const g = buildHeatGrid([], "2026-10-05", "2026-11-06", "tokens", "all", "2026-11-01");
    const cells = g.weeks.flat().filter((c) => c !== null);
    expect(cells.find((c) => c!.date === "2026-10-20")!.outside).toBe(true);
    expect(cells.find((c) => c!.date === "2026-11-03")!.outside).toBeUndefined();
  });
});
