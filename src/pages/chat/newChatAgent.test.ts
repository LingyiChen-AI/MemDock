import type { EnvReport } from "@/api/types";
import type { LiveAgent } from "@/api/live";
import { loadProjectChatAgent, resolveProjectChatAgent, saveProjectChatAgent } from "./newChatAgent";
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

describe("project chat default", () => {
  it("has no default without a detected CLI, including with a saved preference", () => {
    expect(resolveProjectChatAgent(null, "claude")).toBeNull();
    expect(resolveProjectChatAgent(report(), "codex")).toBeNull();
  });

  it("uses the only installed CLI even when the other one was preferred", () => {
    expect(resolveProjectChatAgent(report("codex"), "claude")).toBe("codex");
    expect(resolveProjectChatAgent(report("claude"), "codex")).toBe("claude");
  });

  it("defaults to Claude Code with both installed and keeps an explicit choice separate from recent usage", () => {
    const both = report("codex", "claude");
    expect(resolveProjectChatAgent(both, null)).toBe("claude");
    saveProjectChatAgent("codex");
    rememberLast({ agent: "claude" });
    expect(resolveProjectChatAgent(both, loadProjectChatAgent())).toBe("codex");
  });
});
