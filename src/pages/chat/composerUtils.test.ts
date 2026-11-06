import { baseName, findMenuToken, formatInsertPath, inRect, insertAtCursor, isImagePath, replaceToken } from "./composerUtils";

describe("formatInsertPath", () => {
  it("makes paths inside the session folder relative with forward slashes", () => {
    expect(formatInsertPath("C:\\work\\demo\\src\\App.tsx", "C:\\work\\demo")).toBe("src/App.tsx");
    expect(formatInsertPath("c:/Work/Demo/src", "C:\\work\\demo\\")).toBe("src");
    expect(formatInsertPath("/home/u/demo/a/b.ts", "/home/u/demo")).toBe("a/b.ts");
    expect(formatInsertPath("C:\\work\\demo", "C:\\work\\demo")).toBe(".");
  });

  it("keeps paths outside the folder absolute", () => {
    expect(formatInsertPath("C:\\work\\demo2\\x.ts", "C:\\work\\demo")).toBe("C:\\work\\demo2\\x.ts");
    expect(formatInsertPath("/home/u/Demo/a.ts", "/home/u/demo")).toBe("/home/u/Demo/a.ts");
    expect(formatInsertPath("/tmp/a.ts", "")).toBe("/tmp/a.ts");
  });

  it("quotes paths with spaces", () => {
    expect(formatInsertPath("C:\\work\\demo\\my file.ts", "C:\\work\\demo")).toBe('"my file.ts"');
    expect(formatInsertPath("D:\\My Docs\\a.txt", "C:\\work\\demo")).toBe('"D:\\My Docs\\a.txt"');
  });
});

describe("insertAtCursor", () => {
  it("pads with spaces only where the text would touch", () => {
    expect(insertAtCursor("see", 3, 3, "a.ts")).toEqual({ text: "see a.ts", caret: 8 });
    expect(insertAtCursor("see  now", 4, 4, "a.ts")).toEqual({ text: "see a.ts now", caret: 8 });
    expect(insertAtCursor("", 0, 0, "a.ts")).toEqual({ text: "a.ts", caret: 4 });
    expect(insertAtCursor("fixXYZnow", 3, 6, "a.ts")).toEqual({ text: "fix a.ts now", caret: 8 });
  });
});

describe("isImagePath / inRect", () => {
  it("detects image extensions case-insensitively", () => {
    expect(isImagePath("C:\\shots\\A.PNG")).toBe(true);
    expect(isImagePath("/x/photo.jpeg")).toBe(true);
    expect(isImagePath("/x/notes.md")).toBe(false);
    expect(isImagePath("/x/folder.png/inner")).toBe(false);
  });

  it("converts physical drag positions with the device pixel ratio", () => {
    const rect = { left: 100, top: 100, right: 300, bottom: 200 };
    expect(inRect({ x: 400, y: 300 }, rect, 2)).toBe(true);
    expect(inRect({ x: 400, y: 300 }, rect, 1)).toBe(false);
  });
});

describe("menu tokens", () => {
  it("finds a / command only at the start and an @ word anywhere", () => {
    expect(findMenuToken("/mo", 3)).toEqual({ kind: "slash", query: "mo", start: 0, end: 3 });
    expect(findMenuToken("hi /mo", 6)).toBeNull();
    expect(findMenuToken("see @src/a", 10)).toEqual({ kind: "at", query: "src/a", start: 4, end: 10 });
    expect(findMenuToken("mail a@b", 8)).toBeNull();
    expect(findMenuToken("/model x", 8)).toBeNull();
  });

  it("replaces the word, or removes it", () => {
    const tok = findMenuToken("see @no more", 7)!;
    expect(replaceToken("see @no more", tok, "docs/x.md")).toEqual({ text: "see docs/x.md more", caret: 13 });
    expect(replaceToken("/eff", findMenuToken("/eff", 4)!, "")).toEqual({ text: "", caret: 0 });
    expect(baseName(String.raw`C:\work\demo\ `.trim())).toBe("demo");
    expect(baseName("/home/u/cc-memory/")).toBe("cc-memory");
  });
});
