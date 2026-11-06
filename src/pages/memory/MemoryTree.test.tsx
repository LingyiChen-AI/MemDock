import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  addManualProject: vi.fn(),
  removeManualProject: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { addManualProject, removeManualProject } from "@/api/memory";
import { open } from "@tauri-apps/plugin-dialog";
import { renderApp, sampleIndex, globalClaude, projClaude } from "@/test/fixtures";
import { fileKey } from "./paths";
import { MemoryTree } from "./MemoryTree";

function setup(over: Partial<React.ComponentProps<typeof MemoryTree>> = {}) {
  const onSelect = vi.fn();
  const onIndexChange = vi.fn();
  renderApp(<MemoryTree index={sampleIndex} onSelect={onSelect} onIndexChange={onIndexChange} {...over} />);
  return { onSelect, onIndexChange };
}

describe("MemoryTree", () => {
  beforeEach(() => localStorage.clear());

  it("tree shows global and projects", () => {
    localStorage.setItem("memdock.showEmptyProjects", "1");
    setup();
    expect(screen.getByText("全局")).toBeInTheDocument();
    expect(screen.getByText("CLAUDE.md")).toBeInTheDocument();
    expect(screen.getByText("AGENTS.md")).toBeInTheDocument();
    expect(screen.getByText("项目 · 2")).toBeInTheDocument();
    const project = screen.getByRole("treeitem", { name: /jev-rpa/ });
    expect(project).toHaveAttribute("title", "/work/jev-rpa");
    expect(screen.getByText("Claude Code")).toBeInTheDocument();
    expect(screen.getAllByText("Codex").length).toBeGreaterThan(0);
  });

  it("missing project shows flag", () => {
    localStorage.setItem("memdock.showEmptyProjects", "1");
    setup();
    expect(screen.getByText("目录不存在")).toBeInTheDocument();
  });

  it("empty project hidden by default; toggle shows it and persists", async () => {
    setup();
    expect(screen.queryByRole("treeitem", { name: /gone/ })).not.toBeInTheDocument();
    expect(screen.getByText("项目 · 1")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("switch", { name: "显示空项目（1）" }));
    expect(screen.getByRole("treeitem", { name: /gone/ })).toBeInTheDocument();
    expect(screen.getByText("项目 · 2")).toBeInTheDocument();
    expect(localStorage.getItem("memdock.showEmptyProjects")).toBe("1");
  });

  it("not-created global file shows flag", () => {
    setup();
    expect(screen.getByText("未创建")).toBeInTheDocument();
  });

  it("auto memories grouped with count", async () => {
    setup();
    await userEvent.click(screen.getByRole("treeitem", { name: /jev-rpa/ }));
    const group = screen.getByRole("treeitem", { name: "自动记忆（3 条）" });
    expect(group).toBeInTheDocument();
    expect(screen.queryByText("m1.md")).not.toBeInTheDocument();
    await userEvent.click(group);
    expect(screen.getByText("m1.md")).toBeInTheDocument();
    expect(screen.getByText("m3.md")).toBeInTheDocument();
  });

  it("clicking a file selects it", async () => {
    const { onSelect } = setup();
    await userEvent.click(screen.getByText("CLAUDE.md"));
    expect(onSelect).toHaveBeenCalledWith(globalClaude);
  });

  it("keyboard: arrows move, right expands, enter opens", async () => {
    const { onSelect } = setup({ selectedKey: fileKey(globalClaude) });
    const first = screen.getByRole("treeitem", { name: /CLAUDE\.md/ });
    first.focus();
    await userEvent.keyboard("{ArrowDown}");
    expect(screen.getByRole("treeitem", { name: /AGENTS\.md/ })).toHaveFocus();
    await userEvent.keyboard("{ArrowDown}{ArrowDown}");
    const project = screen.getByRole("treeitem", { name: /jev-rpa/ });
    expect(project).toHaveFocus();
    await userEvent.keyboard("{ArrowRight}");
    expect(project).toHaveAttribute("aria-expanded", "true");
    await userEvent.keyboard("{ArrowDown}{Enter}");
    expect(onSelect).toHaveBeenCalledWith(projClaude);
    await userEvent.keyboard("{ArrowLeft}");
    expect(project).toHaveFocus();
    await userEvent.keyboard("{ArrowLeft}");
    expect(project).toHaveAttribute("aria-expanded", "false");
  });

  it("add project button calls addManualProject with picked dir", async () => {
    (open as Mock).mockResolvedValue("C:\\code\\new-proj");
    (addManualProject as Mock).mockResolvedValue(sampleIndex);
    const { onIndexChange } = setup();
    await userEvent.click(screen.getByRole("button", { name: "添加项目目录" }));
    expect(open).toHaveBeenCalledWith({ directory: true });
    await waitFor(() => expect(addManualProject).toHaveBeenCalledWith("C:\\code\\new-proj"));
    await waitFor(() => expect(onIndexChange).toHaveBeenCalledWith(sampleIndex));
  });

  it("cancelled picker does nothing", async () => {
    (open as Mock).mockResolvedValue(null);
    (addManualProject as Mock).mockClear();
    setup();
    await userEvent.click(screen.getByRole("button", { name: "添加项目目录" }));
    expect(addManualProject).not.toHaveBeenCalled();
  });

  it("manual-only project can be removed from the context menu", async () => {
    (removeManualProject as Mock).mockResolvedValue(sampleIndex);
    localStorage.setItem("memdock.showEmptyProjects", "1");
    const { onIndexChange } = setup();
    await userEvent.pointer({
      keys: "[MouseRight]",
      target: screen.getByRole("treeitem", { name: /gone/ }),
    });
    await userEvent.click(await screen.findByRole("menuitem", { name: "移除项目" }));
    await waitFor(() => expect(removeManualProject).toHaveBeenCalledWith("/work/gone"));
    await waitFor(() => expect(onIndexChange).toHaveBeenCalled());
  });

  it("search box opens full-text search on typing, click or Ctrl+K but not on focus", async () => {
    const onSearchOpen = vi.fn();
    setup({ onSearchOpen });
    const box = screen.getByRole("textbox", { name: "搜索全部记忆" });
    box.focus();
    expect(onSearchOpen).not.toHaveBeenCalled();
    await userEvent.click(box);
    expect(onSearchOpen).toHaveBeenCalledWith("");
    onSearchOpen.mockClear();
    await userEvent.type(box, "a");
    expect(onSearchOpen).toHaveBeenCalledWith("a");
    onSearchOpen.mockClear();
    await userEvent.keyboard("{Control>}k{/Control}");
    expect(onSearchOpen).toHaveBeenCalledWith("");
  });
  it("memory tree shows pending dot", () => {
    setup({ pendingByProject: { "/work/jev-rpa": 3, __global__: 1, "/work/other": 5 } });
    const project = screen.getByRole("treeitem", { name: /jev-rpa/ });
    const dot = within(project).getByRole("img", { name: "3 条待审核" });
    expect(dot).toHaveAttribute("title", "3 条待审核");
    expect(dot.className).toContain("bg-warning");
    // the global group heading carries its own dot
    expect(screen.getByRole("img", { name: "1 条待审核" })).toBeInTheDocument();
    expect(screen.getAllByRole("img")).toHaveLength(2);
  });

  it("no dot without pending suggestions", () => {
    setup({ pendingByProject: { "/work/jev-rpa": 0 } });
    expect(screen.queryByRole("img")).not.toBeInTheDocument();
  });
});

describe("MemoryTree skipped files", () => {
  beforeEach(() => localStorage.clear());

  it("marks files the last sync skipped", () => {
    setup({ skipped: { [globalClaude.id]: "secret" } });
    const mark = screen.getByRole("img", { name: /上次同步时因疑似密钥被跳过/ });
    expect(mark).toHaveAttribute("title", expect.stringContaining("疑似密钥"));
    expect(screen.getAllByRole("img", { name: /上次同步时/ })).toHaveLength(1);
  });

  it("explains other skip reasons", () => {
    setup({ skipped: { [globalClaude.id]: "not_utf8_local" } });
    expect(screen.getByRole("img", { name: "上次同步时被跳过：本机文件不是 UTF-8 文本" })).toBeInTheDocument();
  });
});
