import { DEFAULT_PREFS, loadChatPrefs, parseExtraArgs, relativeTime, rememberChoice, splitArgs } from "./chatPrefs";

describe("splitArgs", () => {
  it("splits on whitespace and respects quotes", () => {
    expect(splitArgs(`--add-dir "D:/my dir" --max-turns 20`)).toEqual(["--add-dir", "D:/my dir", "--max-turns", "20"]);
    expect(splitArgs(`--x 'a b' ""`)).toEqual(["--x", "a b", ""]);
    expect(splitArgs("  ")).toEqual([]);
  });

  it("keeps Windows backslashes", () => {
    expect(splitArgs(String.raw`--add-dir C:\work\x`)).toEqual(["--add-dir", String.raw`C:\work\x`]);
  });
});

describe("parseExtraArgs", () => {
  it("reads Codex overrides one per line or space separated", () => {
    expect(parseExtraArgs("codex", "a=1\nb=2 c=3\n\n")).toEqual(["a=1", "b=2", "c=3"]);
    expect(parseExtraArgs("claude", "--verbose")).toEqual(["--verbose"]);
    expect(parseExtraArgs("claude", "")).toEqual([]);
  });
});

describe("chat prefs", () => {
  it("defaults, then remembers per-agent choices", () => {
    expect(loadChatPrefs()).toEqual(DEFAULT_PREFS);
    rememberChoice("codex", "bypass", "gpt-5", "D:/p");
    rememberChoice("claude", "plan", "", "D:/q");
    const p = loadChatPrefs();
    expect(p.agent).toBe("claude");
    expect(p.permission).toEqual({ claude: "plan", codex: "bypass" });
    expect(p.model).toEqual({ claude: "", codex: "gpt-5" });
    expect(p.cwd).toBe("D:/q");
  });

  it("ignores junk in storage", () => {
    localStorage.setItem("memdock.chat.prefs", JSON.stringify({ agent: "x", permission: { claude: "nope" } }));
    expect(loadChatPrefs().permission.claude).toBe("ask");
    localStorage.setItem("memdock.chat.prefs", "{bad");
    expect(loadChatPrefs()).toEqual(DEFAULT_PREFS);
  });
});

describe("relativeTime", () => {
  const labels = { justNow: "刚刚", minutes: "{n} 分钟前", hours: "{n} 小时前" };
  it("formats recent times relatively", () => {
    const now = 10_000_000_000;
    expect(relativeTime(now - 5_000, labels, () => "abs", now)).toBe("刚刚");
    expect(relativeTime(now - 5 * 60_000, labels, () => "abs", now)).toBe("5 分钟前");
    expect(relativeTime(now - 3 * 3_600_000, labels, () => "abs", now)).toBe("3 小时前");
    expect(relativeTime(now - 2 * 86_400_000, labels, () => "abs", now)).toBe("abs");
  });
});
