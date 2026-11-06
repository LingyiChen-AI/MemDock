import { middleEllipsis, splitPath } from "./pathText";

describe("pathText", () => {
  it("splits name and directory", () => {
    expect(splitPath("projects/github.com/a/b/claude/CLAUDE.md")).toEqual({
      name: "CLAUDE.md",
      dir: "projects/github.com/a/b/claude",
    });
    expect(splitPath("C:\\x\\y.md")).toEqual({ name: "y.md", dir: "C:/x" });
    expect(splitPath("only.md")).toEqual({ name: "only.md", dir: "" });
  });

  it("cuts the middle", () => {
    expect(middleEllipsis("short", 10)).toBe("short");
    const out = middleEllipsis("abcdefghijklmnopqrstuvwxyz", 11);
    expect(out).toBe("abcde…vwxyz");
    expect(out.length).toBe(11);
  });
});
