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
vi.mock("@/api/memoryGraph", () => ({ extractMemoryGraph: vi.fn(), getMemoryGraph: vi.fn() }));
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
import { extractMemoryGraph, getMemoryGraph, type MemoryGraph, type SavedMemoryGraph } from "@/api/memoryGraph";
import type { MemoryIndex } from "@/api/types";
import { renderApp, sampleIndex, globalClaude, projClaude, mkProject } from "@/test/fixtures";
import { MemoryPage } from "./MemoryPage";

const listeners: ((p: { paths: string[] }) => void)[] = [];
const emit = () => listeners.forEach((cb) => cb({ paths: [] }));

const semanticGraph: MemoryGraph = {
  nodes: [
    { id: "n1", label: "Project workflow", kind: "process", sourceFileIds: [projClaude.id] },
    { id: "n2", label: "Codex", kind: "tool", sourceFileIds: [projClaude.id] },
  ],
  relationships: [{ fromNodeId: "n1", toNodeId: "n2", label: "uses", sourceFileIds: [projClaude.id] }],
  sources: [{ fileId: projClaude.id, title: "Project source", line: 2 }],
  unread: 3,
};

beforeEach(() => {
  vi.mocked(extractMemoryGraph).mockReset().mockResolvedValue(semanticGraph);
  vi.mocked(getMemoryGraph).mockReset().mockResolvedValue(null);
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

  it("opens the graph through the project row after confirming dirty edits, then reveals a saved source", async () => {
    (readMemoryFile as Mock).mockClear();
    (readMemoryFile as Mock).mockImplementation(async (path: string) => ({
      text: path === projClaude.path ? "\n# Project source" : "Other file",
      imports: [],
      hash: "h",
    }));
    fakeView.dispatch.mockClear();
    renderApp(<MemoryPage />);
    const editor = await screen.findByLabelText("editor");
    await userEvent.type(editor, "!");
    const action = screen.getByRole("button", { name: "jev-rpa · 记忆图谱" });
    expect(action.nextElementSibling).toHaveAccessibleName(/隐藏/);
    action.focus();
    await userEvent.keyboard("{Enter}");
    await userEvent.click(await screen.findByRole("button", { name: "继续编辑" }));
    expect(editor).toHaveValue("Other file!");
    await userEvent.click(action);
    await userEvent.click(await screen.findByRole("button", { name: "放弃修改并切换" }));
    expect(screen.queryByLabelText("editor")).not.toBeInTheDocument();
    expect(readMemoryFile).not.toHaveBeenCalledWith(projClaude.path);
    await userEvent.click(screen.getByRole("button", { name: "生成图谱" }));
    await userEvent.click(await screen.findByRole("button", { name: "Project workflow" }));
    await userEvent.click(screen.getByRole("button", { name: "查看原文" }));
    expect(await screen.findByDisplayValue(/Project source/)).toBeInTheDocument();
    await waitFor(() => expect(fakeView.dispatch).toHaveBeenCalledWith(expect.objectContaining({ selection: { anchor: 200 } })));
    await userEvent.click(action);
    expect(screen.getByRole("button", { name: "刷新图谱" })).toBeInTheDocument();
  });

  it("extracts a semantic relationship using the selected CLI and retains the graph after a failed refresh", async () => {
    renderApp(<MemoryPage />);
    await screen.findByLabelText("editor");
    await userEvent.click(screen.getByRole("button", { name: "jev-rpa · 记忆图谱" }));
    expect(extractMemoryGraph).not.toHaveBeenCalled();
    const toolPicker = screen.getByRole("combobox", { name: "提取工具" });
    toolPicker.focus();
    await userEvent.keyboard("{Enter}{End}{Enter}");
    let finish!: (result: MemoryGraph) => void;
    vi.mocked(extractMemoryGraph).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    await userEvent.click(screen.getByRole("button", { name: "生成图谱" }));
    expect(extractMemoryGraph).toHaveBeenCalledWith("/work/jev-rpa", "codex", "zh");
    const loading = screen.getByRole("status");
    expect(loading).toHaveTextContent("正在生成知识图谱");
    expect(loading).toHaveTextContent("Codex");
    await act(async () => finish(semanticGraph));
    expect(await screen.findByText("未纳入提取的记忆文件 3")).toBeInTheDocument();
    const edge = screen.getByRole("button", { name: "Project workflow → uses → Codex" });
    edge.focus();
    await userEvent.keyboard("{Enter}");
    expect(edge).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("list", { name: "记忆来源" })).toHaveTextContent("Project source");
    vi.mocked(extractMemoryGraph).mockRejectedValueOnce(new Error("CLI unavailable"));
    await userEvent.click(screen.getByRole("button", { name: "刷新图谱" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("CLI unavailable");
    expect(edge).toBeInTheDocument();
  });

  it("restores a saved graph on a fresh page without calling the agent", async () => {
    vi.mocked(getMemoryGraph).mockResolvedValue({ graph: semanticGraph, tool: "codex" });
    renderApp(<MemoryPage />);
    await screen.findByLabelText("editor");
    await userEvent.click(screen.getByRole("button", { name: "jev-rpa · 记忆图谱" }));
    expect(await screen.findByRole("button", { name: "Project workflow" })).toBeInTheDocument();
    expect(getMemoryGraph).toHaveBeenCalledWith("/work/jev-rpa");
    expect(extractMemoryGraph).not.toHaveBeenCalled();
    expect(screen.getByRole("combobox", { name: "提取工具" })).toHaveTextContent("Codex");
    const node = screen.getByRole("button", { name: "Project workflow" });
    expect(screen.queryByRole("complementary", { name: "图谱详情" })).not.toBeInTheDocument();
    await userEvent.click(node);
    expect(screen.getByRole("heading", { name: "Project workflow" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "关闭图谱详情" }));
    expect(screen.queryByRole("complementary", { name: "图谱详情" })).not.toBeInTheDocument();
    expect(await screen.findByRole("group", { name: "记忆图谱" })).toBeInTheDocument();
  });

  it("keeps a newly generated graph when a pending restore completes later", async () => {
    let restore!: (saved: SavedMemoryGraph | null) => void;
    vi.mocked(getMemoryGraph).mockImplementation(() => new Promise((resolve) => { restore = resolve; }));
    renderApp(<MemoryPage />);
    await screen.findByLabelText("editor");
    await userEvent.click(screen.getByRole("button", { name: "jev-rpa · 记忆图谱" }));
    await userEvent.click(screen.getByRole("button", { name: "生成图谱" }));
    expect(await screen.findByRole("button", { name: "Project workflow" })).toBeInTheDocument();
    await act(async () => restore({ graph: { ...semanticGraph, nodes: [], relationships: [] }, tool: "codex" }));
    expect(screen.getByRole("button", { name: "Project workflow" })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "提取工具" })).toHaveTextContent("Claude Code");
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
