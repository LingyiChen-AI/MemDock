import type { EnvReport } from "@/api/types";
import type { LiveAgent } from "@/api/live";
import { loadNewChatAgent, resolveNewChatAgent, saveNewChatAgent } from "./newChatAgent";
import { rememberLast } from "./chatPrefs";

function report(...agents: LiveAgent[]): EnvReport {
  return {
    tools: agents.map((name) => ({
      name,
      installed: true,
      path: null,
      version: null,
      logged_in: null,
      desktop_app: null,
    })),
    data_dir: "",
    installing: null,
  };
}

describe("new chat default", () => {
  it("has no default without a detected CLI, including with a saved preference", () => {
    expect(resolveNewChatAgent(null, "claude")).toBeNull();
    expect(resolveNewChatAgent(report(), "codex")).toBeNull();
  });

  it("uses the only installed CLI even when the other one was preferred", () => {
    expect(resolveNewChatAgent(report("codex"), "claude")).toBe("codex");
    expect(resolveNewChatAgent(report("claude"), "codex")).toBe("claude");
  });

  it("defaults to Claude Code with both installed and keeps an explicit choice separate from recent usage", () => {
    const both = report("codex", "claude");
    expect(resolveNewChatAgent(both, null)).toBe("claude");
    saveNewChatAgent("codex");
    rememberLast({ agent: "claude" });
    expect(resolveNewChatAgent(both, loadNewChatAgent())).toBe("codex");
  });
});
