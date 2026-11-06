import { render } from "@testing-library/react";
import { ChatText } from "./ChatMessage";

class FakeHighlight {
  ranges: Range[];
  constructor(...ranges: Range[]) {
    this.ranges = ranges;
  }
  add(r: Range) {
    this.ranges.push(r);
  }
  delete(r: Range) {
    this.ranges = this.ranges.filter((x) => x !== r);
  }
}

describe("ChatText", () => {
  const g = globalThis as unknown as { CSS?: { highlights?: Map<string, FakeHighlight> }; Highlight?: unknown };
  const saved = { css: g.CSS, hl: g.Highlight };
  beforeEach(() => {
    g.CSS = { ...(saved.css ?? {}), highlights: new Map() };
    g.Highlight = FakeHighlight;
  });
  afterEach(() => {
    g.CSS = saved.css;
    g.Highlight = saved.hl;
  });

  it("highlights a quote across markdown formatting and line breaks", () => {
    render(<ChatText text={"请把**从库延迟**\n调小到 1 秒，谢谢"} highlight="把从库延迟 调小到 1 秒" expandLabel="展开" collapseLabel="收起" />);
    const h = g.CSS!.highlights!.get("chat-quote")!;
    expect(h.ranges).toHaveLength(1);
    expect(h.ranges[0].toString().replace(/\s+/g, "")).toBe("把从库延迟调小到1秒");
  });

  it("does nothing when the quote is absent", () => {
    render(<ChatText text="完全无关" highlight="找不到" expandLabel="展开" collapseLabel="收起" />);
    expect(g.CSS!.highlights!.get("chat-quote")).toBeUndefined();
  });
});
