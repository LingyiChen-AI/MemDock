import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  listMemories: vi.fn(),
  readMemoryFile: vi.fn(),
  writeMemoryFile: vi.fn(),
  readCodexDb: vi.fn(),
  addManualProject: vi.fn(),
  removeManualProject: vi.fn(),
  onMemoryChanged: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@uiw/react-codemirror", () => ({
  default: ({ value, onChange }: { value: string; onChange?: (v: string) => void }) => (
    <textarea aria-label="editor" value={value} onChange={(e) => onChange?.(e.target.value)} />
  ),
}));

import { listMemories, onMemoryChanged, readCodexDb, readMemoryFile, removeManualProject } from "@/api/memory";
import type { MemoryIndex } from "@/api/types";
import { renderApp, sampleIndex, globalClaude, projClaude, mkProject } from "@/test/fixtures";
import { MemoryPage } from "./MemoryPage";

const listeners: ((p: { paths: string[] }) => void)[] = [];
const emit = () => listeners.forEach((cb) => cb({ paths: [] }));

beforeEach(() => {
  localStorage.clear();
  listeners.length = 0;
  (onMemoryChanged as Mock).mockImplementation(async (cb: (p: { paths: string[] }) => void) => {
    listeners.push(cb);
    return () => {};
  });
  (listMemories as Mock).mockResolvedValue(sampleIndex);
  (readCodexDb as Mock).mockResolvedValue({ status: "unavailable", reason: "x" });
  (readMemoryFile as Mock).mockImplementation(async (path: string) => ({
    text: `text of ${path}`,
    hash: "h",
    has_bom: false,
    crlf: false,
    tokens: 10,
    imports: [],
  }));
});

describe("MemoryPage", () => {
  it("opens the first global file and asks before discarding edits", async () => {
    renderApp(<MemoryPage />);
    const box = await screen.findByDisplayValue("text of /home/u/.claude/CLAUDE.md");
    await userEvent.type(box, "!");
    await userEvent.click(screen.getByRole("treeitem", { name: /记忆库/ }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("有未保存的修改");
    await userEvent.click(screen.getByRole("button", { name: "继续编辑" }));
    expect(screen.getByLabelText("editor")).toHaveValue("text of /home/u/.claude/CLAUDE.md!");
    await userEvent.click(screen.getByRole("treeitem", { name: /记忆库/ }));
    await userEvent.click(await screen.findByRole("button", { name: "放弃修改并切换" }));
    await waitFor(() => expect(screen.queryByLabelText("editor")).not.toBeInTheDocument());
  });

  it("tree width is clamped and persisted from the keyboard", async () => {
    renderApp(<MemoryPage />);
    const sep = await screen.findByRole("separator");
    expect(sep).toHaveAttribute("aria-valuenow", "280");
    sep.focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(sep).toHaveAttribute("aria-valuenow", "296");
    expect(localStorage.getItem("memdock.treeWidth")).toBe("296");
    for (let i = 0; i < 20; i++) await userEvent.keyboard("{ArrowRight}");
    expect(sep).toHaveAttribute("aria-valuenow", "420");
  });

  it("keeps a dirty editor with a banner when its file leaves the index", async () => {
    renderApp(<MemoryPage />);
    const box = await screen.findByDisplayValue("text of /home/u/.claude/CLAUDE.md");
    await userEvent.type(box, "!");
    const without: MemoryIndex = { ...sampleIndex, global: sampleIndex.global.filter((f) => f !== globalClaude) };
    (listMemories as Mock).mockResolvedValue(without);
    await act(async () => emit());
    expect(await screen.findByText("文件已在外部删除或移除")).toBeInTheDocument();
    expect(screen.getByLabelText("editor")).toHaveValue("text of /home/u/.claude/CLAUDE.md!");
  });

  it("clean editor falls back to the empty state when its file leaves the index", async () => {
    renderApp(<MemoryPage />);
    await screen.findByDisplayValue("text of /home/u/.claude/CLAUDE.md");
    const without: MemoryIndex = { ...sampleIndex, global: sampleIndex.global.filter((f) => f !== globalClaude) };
    (listMemories as Mock).mockResolvedValue(without);
    await act(async () => emit());
    await waitFor(() => expect(screen.queryByLabelText("editor")).not.toBeInTheDocument());
  });

  it("removing a project whose file has unsaved edits asks first", async () => {
    const manual = mkProject({ id: "/work/jev-rpa", name: "jev-rpa", path: "/work/jev-rpa", sources: ["manual"] });
    const idx: MemoryIndex = { global: [globalClaude], projects: [{ project: manual, files: [projClaude] }] };
    (listMemories as Mock).mockResolvedValue(idx);
    (removeManualProject as Mock).mockResolvedValue({ global: [globalClaude], projects: [] });
    renderApp(<MemoryPage />);
    await screen.findByDisplayValue("text of /home/u/.claude/CLAUDE.md");
    await userEvent.click(screen.getByRole("treeitem", { name: /jev-rpa/ }));
    await userEvent.click(screen.getByRole("treeitem", { name: /CLAUDE\.md/, selected: false }));
    // dirty is false right after opening, so this switch has no prompt
    const box = await screen.findByDisplayValue("text of /work/jev-rpa/CLAUDE.md");
    await userEvent.type(box, "!");
    await userEvent.pointer({ keys: "[MouseRight]", target: screen.getByRole("treeitem", { name: /jev-rpa/ }) });
    await userEvent.click(await screen.findByRole("menuitem", { name: "移除项目" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("移除项目会关闭当前文件");
    expect(removeManualProject).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "放弃修改并移除" }));
    await waitFor(() => expect(removeManualProject).toHaveBeenCalledWith("/work/jev-rpa"));
  });
});
