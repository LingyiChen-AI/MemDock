import { render, screen } from "@testing-library/react";
import { vi } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  listMemories: vi.fn().mockResolvedValue({ global: [], projects: [] }),
  onMemoryChanged: vi.fn().mockResolvedValue(() => {}),
}));

import App from "./App";

describe("App", () => {
  it("renders the MemDock brand and the nav", async () => {
    render(<App />);
    expect(screen.getByText("MemDock")).toBeInTheDocument();
    expect(screen.getByAltText("MemDock")).toHaveAttribute("width", "24");
    expect(await screen.findByText("记忆总览")).toBeInTheDocument();
  });
});
