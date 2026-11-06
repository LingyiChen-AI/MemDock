import { render, screen } from "@testing-library/react";
import { vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
const usageQuota = vi.fn();
vi.mock("@/api/usage", () => ({ usageQuota: () => usageQuota() }));

import { ContextMeters } from "./ContextMeter";

const t = { context: "上下文", contextHint: "当前上下文 {used} / {window}（{model}）", weekly: "本周", weeklyHint: "本周额度" };

describe("ContextMeters", () => {
  it("shows the context fill and the weekly quota", async () => {
    invoke.mockResolvedValue({ used: 771_724, window: 1_000_000, model: "claude-opus-5-5" });
    usageQuota.mockResolvedValue({ claude: { status: "ok", weekly: { used_percent: 78 }, five_hour: null, updated_at: 1 }, codex: { status: "disabled" } });
    render(<ContextMeters tool="claude" path="/s.jsonl" refreshKey="1" t={t} />);
    expect(await screen.findByText("77%")).toBeInTheDocument();
    expect(screen.getByTitle("当前上下文 772k / 1M（claude-opus-5-5）")).toBeInTheDocument();
    expect(await screen.findByText("78%")).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith("session_context", { tool: "claude", path: "/s.jsonl" });
  });

  it("renders nothing without a session file or quota", async () => {
    usageQuota.mockRejectedValue(new Error("off"));
    const { container } = render(<ContextMeters tool="codex" path={null} refreshKey="1" t={t} />);
    await new Promise((r) => setTimeout(r, 0));
    expect(container).toBeEmptyDOMElement();
  });
});
