const BLOCKS = new Set(["p", "h1", "h2", "h3", "h4", "h5", "h6", "li", "blockquote", "pre", "table", "hr"]);

interface HastNode {
  type: string;
  tagName?: string;
  properties?: Record<string, unknown>;
  position?: { start?: { line?: number } };
  children?: HastNode[];
}

function walk(node: HastNode) {
  if (node.type === "element" && node.tagName && BLOCKS.has(node.tagName)) {
    const line = node.position?.start?.line;
    if (typeof line === "number") node.properties = { ...node.properties, dataLine: line };
  }
  node.children?.forEach(walk);
}

/** Stamps block-level elements with `data-line` (1-based source line) for editor/preview scroll sync. */
export function rehypeSourceLine() {
  return (tree: HastNode) => walk(tree);
}
