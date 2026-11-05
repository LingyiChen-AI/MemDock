import { useEffect } from "react";
import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  listMemories: vi.fn(),
  readMemoryFile: vi.fn(),
  writeMemoryFile: vi.fn(),
  readCodexDb: vi.fn(),
  addManualProject: vi.fn(),
  removeManualProject: vi.fn(),
  onMemoryChanged: vi.fn(),
  searchMemories: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
const fakeView = vi.hoisted(() => ({
  dom: { isConnected: true },
  state: { doc: { lines: 10, line: (n: number) => ({ from: n * 100 }) } },
  dispatch: vi.fn(),
  focus: vi.fn(),
}));
vi.mock("@uiw/react-codemirror", () => ({
  default: ({ value, onChange, onCreateEditor }: { value: string; onChange?: (v: string) => void; onCreateEditor?: (v: unknown) => void }) => {
    useEffect(() => onCreateEditor?.(fakeView), [onCreateEditor]);
    return <textarea aria-label="editor" value={value} onChange={(e) => onChange?.(e.target.value)} />;
  },
  EditorView: { theme: () => [], lineWrapping: [] },
}));

import { listMemories, onMemoryChanged, readCodexDb, readMemoryFile, removeManualProject, searchMemories } from "@/api/memory";
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

  it("Ctrl+K opens full-text search and a hit opens its file", async () => {
    (searchMemories as Mock).mockResolvedValue([{ file_id: projClaude.id, line: 7, preview: "hit in project" }]);
    renderApp(<MemoryPage />);
    await screen.findByDisplayValue("text of /home/u/.claude/CLAUDE.md");
    await userEvent.keyboard("{Control>}k{/Control}");
    const box = await screen.findByRole("searchbox");
    expect(box).toHaveFocus();
    await userEvent.type(box, "hit");
    await userEvent.click(await screen.findByRole("button", { name: /hit in project/ }));
    expect(await screen.findByDisplayValue("text of /work/jev-rpa/CLAUDE.md")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "关闭搜索" }));
    expect(screen.getByRole("tree")).toBeInTheDocument();
  });

  it("a search jump is consumed: A -> B -> A does not scroll again", async () => {
    fakeView.dispatch.mockClear();
    (searchMemories as Mock).mockResolvedValue([{ file_id: projClaude.id, line: 7, preview: "hit in project" }]);
    renderApp(<MemoryPage />);
    await screen.findByDisplayValue("text of /home/u/.claude/CLAUDE.md");
    await userEvent.keyboard("{Control>}k{/Control}");
    await userEvent.type(await screen.findByRole("searchbox"), "hit");
    await userEvent.click(await screen.findByRole("button", { name: /hit in project/ }));
    await screen.findByDisplayValue("text of /work/jev-rpa/CLAUDE.md");
    await waitFor(() => expect(fakeView.dispatch).toHaveBeenCalledTimes(1));
    await userEvent.click(screen.getByRole("button", { name: "关闭搜索" }));
    await userEvent.click(screen.getAllByRole("treeitem", { name: /CLAUDE/ })[0]);
    await screen.findByDisplayValue("text of /home/u/.claude/CLAUDE.md");
    const back = await screen.findAllByRole("treeitem", { name: /CLAUDE/ });
    await userEvent.click(back[back.length - 1]);
    await screen.findByDisplayValue("text of /work/jev-rpa/CLAUDE.md");
    await new Promise((r) => setTimeout(r, 50));
    expect(fakeView.dispatch).toHaveBeenCalledTimes(1);
  });
});
