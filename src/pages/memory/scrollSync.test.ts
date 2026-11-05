import { lineForPreviewTop, previewTopForLine, type Block } from "./scrollSync";

const blocks: Block[] = [
  { line: 1, top: 20 },
  { line: 3, top: 80 },
  { line: 7, top: 200 },
  { line: 20, top: 600 },
];

describe("previewTopForLine", () => {
  it("returns the block top exactly at a block line", () => {
    expect(previewTopForLine(blocks, 3, 0)).toBe(80);
    expect(previewTopForLine(blocks, 7, 0)).toBe(200);
  });

  it("interpolates between blocks by source line", () => {
    // line 5 is halfway between 3 and 7
    expect(previewTopForLine(blocks, 5, 0)).toBe(140);
    // fraction of a line counts too: 5.5 -> 5/8 of the way
    expect(previewTopForLine(blocks, 5, 0.5)).toBeCloseTo(80 + (2.5 / 4) * 120);
  });

  it("clamps before the first block and after the last", () => {
    expect(previewTopForLine(blocks, 0, 0)).toBe(0);
    expect(previewTopForLine(blocks, 50, 0.5)).toBe(600);
  });

  it("handles no blocks", () => {
    expect(previewTopForLine([], 5, 0)).toBe(0);
  });

  it("is monotonic non-decreasing", () => {
    let prev = -1;
    for (let l = 0; l <= 25; l += 0.25) {
      const v = previewTopForLine(blocks, Math.floor(l), l - Math.floor(l));
      expect(v).toBeGreaterThanOrEqual(prev);
      prev = v;
    }
  });

  it("tolerates several blocks on the same line", () => {
    const dup: Block[] = [
      { line: 2, top: 10 },
      { line: 2, top: 30 },
      { line: 4, top: 70 },
    ];
    expect(Number.isFinite(previewTopForLine(dup, 2, 0))).toBe(true);
    expect(previewTopForLine(dup, 3, 0)).toBeGreaterThanOrEqual(30);
  });
});

describe("lineForPreviewTop", () => {
  it("returns the block line exactly at a block top", () => {
    expect(lineForPreviewTop(blocks, 80)).toBe(3);
    expect(lineForPreviewTop(blocks, 200)).toBe(7);
  });

  it("interpolates between blocks by pixel offset", () => {
    expect(lineForPreviewTop(blocks, 140)).toBe(5);
  });

  it("clamps before the first block and after the last", () => {
    expect(lineForPreviewTop(blocks, 0)).toBe(1);
    expect(lineForPreviewTop(blocks, 5000)).toBe(20);
  });

  it("handles no blocks", () => {
    expect(lineForPreviewTop([], 100)).toBe(1);
  });

  it("is monotonic non-decreasing", () => {
    let prev = 0;
    for (let top = 0; top <= 700; top += 5) {
      const v = lineForPreviewTop(blocks, top);
      expect(v).toBeGreaterThanOrEqual(prev);
      prev = v;
    }
  });

  it("round-trips with previewTopForLine inside the range", () => {
    for (const line of [1, 2.5, 4, 6.2, 12, 19.9]) {
      const top = previewTopForLine(blocks, Math.floor(line), line - Math.floor(line));
      expect(lineForPreviewTop(blocks, top)).toBeCloseTo(line, 5);
    }
  });
});
