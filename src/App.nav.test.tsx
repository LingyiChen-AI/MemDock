import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  listMemories: vi.fn().mockResolvedValue({ global: [], projects: [] }),
  onMemoryChanged: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("@/pages/skills/SkillsPage", () => ({
  SkillsPage: ({ onDirtyChange }: { onDirtyChange?: (d: boolean) => void }) => (
    <button type="button" onClick={() => onDirtyChange?.(true)}>
      make-dirty
    </button>
  ),
}));

import App from "./App";

describe("App navigation guard", () => {
  it("asks before leaving a page with unsaved edits", async () => {
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Skill 管理" }));
    await userEvent.click(screen.getByRole("button", { name: "make-dirty" }));
    await userEvent.click(screen.getByRole("button", { name: "记忆总览" }));
    expect(await screen.findByText("放弃未保存的修改？")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "make-dirty", hidden: true })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "放弃修改并离开" }));
    expect(screen.queryByRole("button", { name: "make-dirty" })).toBeNull();
  });

  it("navigates freely when nothing is dirty", async () => {
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Skill 管理" }));
    await userEvent.click(screen.getByRole("button", { name: "记忆总览" }));
    expect(screen.queryByText("放弃未保存的修改？")).toBeNull();
  });
});
