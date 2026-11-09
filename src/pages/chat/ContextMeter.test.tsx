import { render, screen } from "@testing-library/react";
import { vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
const usageQuota = vi.fn();
vi.mock("@/api/usage", () => ({ usageQuota: () => usageQuota() }));

import { ContextMeters } from "./ContextMeter";

const t = { context: "上下文", contextHint: "当前上下文 {used} / {window}（{model}）", sourceTerminal: "来自外部终端", sourceMemdock: "来自 MemDock", fiveHour: "5 小时", fiveHourHint: "5 小时额度", weekly: "本周", weeklyHint: "本周额度" };

describe("ContextMeters", () => {
  it("shows the context fill and the weekly quota", async () => {
    invoke.mockResolvedValue({ used: 771_724, window: 1_000_000, model: "claude-opus-5-5" });
    usageQuota.mockResolvedValue({ claude: { status: "ok", weekly: { used_percent: 78 }, five_hour: null, updated_at: 1 }, codex: { status: "disabled" } });
    render(<ContextMeters tool="claude" path="/s.jsonl" refreshKey="1" t={t} />);
    expect(await screen.findByText("77%")).toBeInTheDocument();
    expect(screen.getByTitle("当前上下文 772k / 1M（claude-opus-5-5）")).toBeInTheDocument();
    expect(await screen.findByText("78%")).toBeInTheDocument();
    expect(screen.queryByText("5 小时")).not.toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith("session_context", { tool: "claude", path: "/s.jsonl" });
  });

  it.each(["claude", "codex"] as const)("shows a reported 5-hour quota for %s", async (tool) => {
    usageQuota.mockResolvedValue({
      [tool]: { status: "ok", five_hour: { used_percent: 0 }, weekly: null },
    });
    render(<ContextMeters tool={tool} path={null} refreshKey="1" t={t} />);
    expect(await screen.findByTitle("5 小时额度")).toHaveTextContent("0%");
  });

  it("renders nothing without a session file or quota", async () => {
    usageQuota.mockRejectedValue(new Error("off"));
    const { container } = render(<ContextMeters tool="codex" path={null} refreshKey="1" t={t} />);
    await new Promise((r) => setTimeout(r, 0));
    expect(container).toBeEmptyDOMElement();
  });
});

import { prettyModel } from "./Composer";

describe("model names", () => {
  it("turns Claude model ids into display names", () => {
    expect(prettyModel("claude-opus-5-5")).toBe("Opus 5.5");
    expect(prettyModel("claude-haiku-4-5-20251001")).toBe("Haiku 4.5");
    expect(prettyModel("claude-fable-5-1")).toBe("Fable 5.1");
    expect(prettyModel("claude-sonnet-4-6[1m]")).toBe("Sonnet 4.6 (1M)");
    expect(prettyModel("gpt-6.1-sol")).toBe("gpt-6.1-sol");
  });

  it("asks by session id when the file path is unknown", async () => {
    invoke.mockReset();
    invoke.mockResolvedValue({ used: 17_073, window: 475_000, model: "gpt-6.1-sol" });
    usageQuota.mockRejectedValue(new Error("off"));
    render(<ContextMeters tool="codex" path={null} nativeId="thread-1" refreshKey="1" t={t} />);
    expect(await screen.findByText("4%")).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith("session_context_by_id", { tool: "codex", id: "thread-1" });
  });
});
