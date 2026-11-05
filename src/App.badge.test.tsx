import { act, configure, render, screen, within } from "@testing-library/react";
import { vi, type Mock } from "vitest";
import type { PendingSummary } from "@/api/types";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  listMemories: vi.fn().mockResolvedValue({ global: [], projects: [] }),
  onMemoryChanged: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("@/api/extract", () => ({
  pendingSummary: vi.fn(),
  onSuggestionsChanged: vi.fn(),
}));

import { onSuggestionsChanged, pendingSummary } from "@/api/extract";
import App from "./App";

configure({ asyncUtilTimeout: 4000 });
vi.setConfig({ testTimeout: 15000 });

describe("sidebar pending badge", () => {
  it("sidebar badge updates on event", async () => {
    let cb: (s: PendingSummary) => void = () => {};
    (pendingSummary as Mock).mockResolvedValue({ total: 2, by_project: {} });
    (onSuggestionsChanged as Mock).mockImplementation(async (f) => {
      cb = f;
      return () => {};
    });
    render(<App />);
    const nav = await screen.findByRole("button", { name: "待审核" }, { timeout: 4000 });
    const item = nav.closest("li")!;
    expect(await within(item).findByText("2", {}, { timeout: 4000 })).toBeInTheDocument();
    await act(async () => cb({ total: 7, by_project: {} }));
    expect(within(item).getByText("7")).toBeInTheDocument();
    await act(async () => cb({ total: 0, by_project: {} }));
    expect(within(item).queryByText("0")).not.toBeInTheDocument();
    expect(within(item).queryByText("7")).not.toBeInTheDocument();
  });
});
