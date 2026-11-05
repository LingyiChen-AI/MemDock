/** A preview block: its 1-based source line and its top offset inside the preview scroller's content. */
export interface Block {
  line: number;
  top: number;
}

/** Index of the last block whose `key` is <= value, or -1. Blocks are sorted by line and top. */
function lastAtOrBefore(blocks: Block[], key: "line" | "top", value: number): number {
  let lo = 0;
  let hi = blocks.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (blocks[mid][key] <= value) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return found;
}

/**
 * Preview scrollTop for the editor position `line + fraction` (1-based line, fraction in [0, 1)).
 * Interpolates between the block at or above the line and the next block; clamps outside the range.
 */
export function previewTopForLine(blocks: Block[], line: number, fraction: number): number {
  if (blocks.length === 0) return 0;
  const pos = line + fraction;
  const i = lastAtOrBefore(blocks, "line", pos);
  if (i < 0) return 0;
  const cur = blocks[i];
  // Several blocks can share a line (e.g. a list item and its paragraph): use the last one.
  const next = blocks[i + 1];
  if (!next) return cur.top;
  const span = next.line - cur.line;
  if (span <= 0) return cur.top;
  return cur.top + ((pos - cur.line) / span) * (next.top - cur.top);
}

/** Inverse of {@link previewTopForLine}: the (fractional) source line shown at preview scrollTop `top`. */
export function lineForPreviewTop(blocks: Block[], top: number): number {
  if (blocks.length === 0) return 1;
  const i = lastAtOrBefore(blocks, "top", top);
  if (i < 0) return 1;
  const cur = blocks[i];
  const next = blocks[i + 1];
  if (!next) return cur.line;
  const span = next.top - cur.top;
  if (span <= 0) return cur.line;
  return cur.line + ((top - cur.top) / span) * (next.line - cur.line);
}

/**
 * Appends an end marker so the last (possibly tall) block interpolates towards the bottom:
 * `endLine` is the editor position at its maximum scroll, `endTop` the preview's maximum scrollTop.
 * Blocks at or after `endLine` are dropped to keep the list sorted.
 */
export function withSentinel(blocks: Block[], endLine: number, endTop: number): Block[] {
  const kept = blocks.filter((b) => b.line < endLine);
  const lastTop = kept.length ? kept[kept.length - 1].top : 0;
  return [...kept, { line: endLine, top: Math.max(endTop, lastTop) }];
}
