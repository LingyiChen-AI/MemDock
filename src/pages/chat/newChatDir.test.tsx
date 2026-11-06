import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";

vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn().mockResolvedValue("C:\\Users\\me") }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn().mockResolvedValue("D:\\work\\scratch") }));
vi.mock("@/api/live", () => ({ memdockWorkspaceDir: vi.fn().mockResolvedValue("C:\\Users\\me\\memdock") }));
vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  listMemories: vi.fn(),
}));

import { renderApp } from "@/test/fixtures";
import { NewChatDirSettings } from "@/pages/settings/NewChatDirSection";
import { loadNewChatDir, resolveNewChatDir, saveNewChatDir } from "./newChatDir";

describe("new chat folder", () => {
  beforeEach(() => localStorage.clear());

  it("follows the open conversation (a worktree maps to its repository), else the home folder", async () => {
    expect(loadNewChatDir().mode).toBe("follow");
    expect(await resolveNewChatDir("D:/proj/app")).toBe("D:/proj/app");
    expect(await resolveNewChatDir("D:/proj/app/.memdock/worktrees/fix-x")).toBe("D:/proj/app");
    expect(await resolveNewChatDir(null)).toBe("C:\\Users\\me");
  });

  it("uses ~/memdock or a fixed folder when configured", async () => {
    saveNewChatDir({ mode: "memdock", fixed: "" });
    expect(await resolveNewChatDir("D:/proj/app")).toBe("C:\\Users\\me\\memdock");
    saveNewChatDir({ mode: "fixed", fixed: "E:/play" });
    expect(await resolveNewChatDir("D:/proj/app")).toBe("E:/play");
    saveNewChatDir({ mode: "fixed", fixed: "" });
    expect(await resolveNewChatDir("D:/proj/app")).toBe("C:\\Users\\me");
  });

  it("the settings card switches modes and picks a fixed folder", async () => {
    renderApp(<NewChatDirSettings />);
    expect(await screen.findByText("C:\\Users\\me\\memdock（不存在时自动创建），适合不属于任何项目的随手对话")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("radio", { name: /主目录下的 memdock 文件夹/ }));
    expect(loadNewChatDir().mode).toBe("memdock");
    await userEvent.click(screen.getByRole("button", { name: "选择文件夹" }));
    expect(loadNewChatDir()).toEqual({ mode: "fixed", fixed: "D:\\work\\scratch" });
    expect(screen.getByText("D:\\work\\scratch")).toBeInTheDocument();
  });
});
