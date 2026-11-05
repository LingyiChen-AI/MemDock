import { hasMarkers, parse, render } from "./conflictBlocks";

const text = [
  "# Title\n",
  "<<<<<<< local\n",
  "mine\n",
  "||||||| base\n",
  "orig\n",
  "=======\n",
  "theirs\n",
  ">>>>>>> remote\n",
  "middle\n",
  "<<<<<<< local\n",
  "a\n",
  "=======\n",
  "b\n",
  ">>>>>>> remote\n",
].join("");

describe("conflictBlocks", () => {
  it("parse_and_render_blocks", () => {
    expect(parse("plain\ntext")).toEqual([{ kind: "text", text: "plain\ntext" }]);
    const blocks = parse(text);
    expect(blocks).toEqual([
      { kind: "text", text: "# Title\n" },
      { kind: "conflict", local: "mine\n", base: "orig\n", remote: "theirs\n" },
      { kind: "text", text: "middle\n" },
      { kind: "conflict", local: "a\n", base: "", remote: "b\n" },
    ]);
    expect(render(blocks, { 0: "local" })).toBeNull();
    expect(render(blocks, { 0: "local", 1: "remote" })).toBe("# Title\nmine\nmiddle\nb\n");
    expect(render(blocks, { 0: "both", 1: { custom: "c\n" } })).toBe("# Title\nmine\ntheirs\nmiddle\nc\n");
    expect(render(parse("x\n"), {})).toBe("x\n");
  });

  it("detects markers", () => {
    expect(hasMarkers(text)).toBe(true);
    expect(hasMarkers("a\n=======\n")).toBe(false);
  });
});
