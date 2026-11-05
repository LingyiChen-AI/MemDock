import { render, screen } from "@testing-library/react";
import { vi } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
}));

import App from "./App";

describe("App", () => {
  it("renders the Memdock brand and the nav", async () => {
    render(<App />);
    expect(screen.getByText("Memdock")).toBeInTheDocument();
    expect(await screen.findByText("记忆总览")).toBeInTheDocument();
  });
});
