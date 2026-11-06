import { useEffect } from "react";
import { act, configure, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { PendingSummary, RunRecord, Suggestion } from "@/api/types";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  listMemories: vi.fn(),
  onMemoryChanged: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("@/api/extract", () => ({
  listSuggestions: vi.fn(),
  listExtractRuns: vi.fn(),
  applySuggestion: vi.fn(),
  rejectSuggestion: vi.fn(),
  acceptSuggestions: vi.fn(),
  suggestionContext: vi.fn(),
  onSuggestionsChanged: vi.fn(),
}));
vi.mock("@uiw/react-codemirror", () => ({
  default: ({ value, onChange }: { value: string; onChange?: (v: string) => void }) => {
    useEffect(() => {}, []);
    return <textarea aria-label="editor" value={value} onChange={(e) => onChange?.(e.target.value)} />;
  },
  EditorView: { theme: () => [], lineWrapping: [] },
}));

import {
  acceptSuggestions,
  applySuggestion,
  listExtractRuns,
  listSuggestions,
  onSuggestionsChanged,
  rejectSuggestion,
  suggestionContext,
} from "@/api/extract";
import { listMemories } from "@/api/memory";
import { mkProject, renderApp, sampleIndex } from "@/test/fixtures";
import { ReviewPage } from "./ReviewPage";

const ev = (quote = "把从库延迟调小到 1 秒") => ({ tool: "claude" as const, session_id: "sess-0123456789abcdef", ts: 1_730_000_000_000, quote });

const sug = (over: Partial<Suggestion> & { id: string }): Suggestion => ({
  fingerprint: `fp-${over.id}`,
  kind: "modify",
  file_id: "/work/jev-rpa/claude.md",
  path: "/work/jev-rpa/CLAUDE.md",
  tool_of_file: "claude",
  old_text: "从库延迟 2 秒",
  new_text: "从库延迟 1 秒",
  reason: "用户要求缩短测试耗时",
  evidence: [ev()],
  source_tool: "claude",
  created_at: 1_730_000_000_000,
  base_hash: "h",
  status: "pending",
  ...over,
});

const modify = sug({ id: "s-mod" });
const add = sug({ id: "s-add", kind: "add", old_text: null, new_text: "新增的一条规则文本", file_id: "/home/u/.claude/claude.md", path: "/home/u/.claude/CLAUDE.md" });
const obsolete = sug({ id: "s-obs", kind: "obsolete", old_text: "已经过时的旧规则", new_text: null });

const run = (over: Partial<RunRecord> = {}): RunRecord => ({
  id: "r1",
  started_at: new Date(2026, 10, 5, 9, 30).getTime(),
  finished_at: new Date(2026, 10, 5, 9, 41).getTime(),
  trigger: "manual",
  tool: "claude",
  status: "succeeded",
  sessions_read: 3,
  projects: 1,
  suggestions_new: 3,
  warnings: [],
  error: null,
  ...over,
});

// Full-page renders with Radix / DiffView are slow under load: generous async timeouts.
configure({ asyncUtilTimeout: 4000 });
vi.setConfig({ testTimeout: 15000 });

let changedCb: (s: PendingSummary) => void = () => {};

function setup(list: Suggestion[] = [modify, add, obsolete]) {
  const onNavigate = vi.fn();
  (listSuggestions as Mock).mockResolvedValue(list);
  renderApp(<ReviewPage onNavigate={onNavigate} />);
  return { onNavigate };
}

beforeEach(() => {
  vi.clearAllMocks();
  for (const m of [applySuggestion, rejectSuggestion, acceptSuggestions, suggestionContext]) (m as Mock).mockReset();
  (listExtractRuns as Mock).mockResolvedValue([run()]);
  (listMemories as Mock).mockResolvedValue({
    ...sampleIndex,
    projects: [{ project: mkProject({ id: "/work/jev-rpa", name: "jev-rpa" }), files: [{ ...sampleIndex.projects[0].files[0], id: "/work/jev-rpa/claude.md" }] }],
    global: [{ ...sampleIndex.global[0], id: "/home/u/.claude/claude.md" }],
  });
  (onSuggestionsChanged as Mock).mockImplementation(async (cb) => {
    changedCb = cb;
    return () => {};
  });
  (suggestionContext as Mock).mockResolvedValue([]);
});

const cards = () => screen.findAllByRole("article");

describe("ReviewPage", () => {
  it("cards render diff per kind", async () => {
    setup();
    const list = await cards();
    expect(list).toHaveLength(3);
    const byKind = (k: string) => list.find((c) => c.getAttribute("data-kind") === k)!;
    // modify: old -> new
    expect(within(byKind("modify")).getByText("修改")).toBeInTheDocument();
    expect(byKind("modify")).toHaveTextContent("从库延迟");
    expect(within(byKind("modify")).getByText("建议改为")).toBeInTheDocument();
    // add: empty -> new
    expect(within(byKind("add")).getByText("新增")).toBeInTheDocument();
    expect(within(byKind("add")).getByText("新增的一条规则文本")).toBeInTheDocument();
    // obsolete: old -> empty
    expect(within(byKind("obsolete")).getByText("过时")).toBeInTheDocument();
    expect(within(byKind("obsolete")).getByText("已经过时的旧规则")).toBeInTheDocument();
    expect(within(byKind("obsolete")).getByText("建议删除")).toBeInTheDocument();
    // header: count + latest run time
    expect(screen.getByText("3 条建议 · 来自 11-05 09:30")).toBeInTheDocument();
    // project / file name and reason / evidence
    expect(within(byKind("modify")).getByText("jev-rpa / CLAUDE.md")).toBeInTheDocument();
    expect(within(byKind("add")).getByText("全局 / CLAUDE.md")).toBeInTheDocument();
    expect(within(byKind("modify")).getByText("用户要求缩短测试耗时")).toBeInTheDocument();
    expect(within(byKind("modify")).getByText(/把从库延迟调小到 1 秒/)).toBeInTheDocument();
  });

  it("accept calls apply then removes card", async () => {
    setup();
    (applySuggestion as Mock).mockResolvedValue({ kind: "applied", hash: "x", saved: true });
    const list = await cards();
    await userEvent.click(within(list[0]).getByRole("button", { name: "通过" }));
    expect(applySuggestion).toHaveBeenCalledWith("s-mod", null, false);
    await waitFor(() => expect(screen.getAllByRole("article")).toHaveLength(2));
    expect(screen.queryByRole("article", { name: /^修改/ })).not.toBeInTheDocument();
  });

  it("applied with saved=false warns", async () => {
    setup([modify]);
    (applySuggestion as Mock).mockResolvedValue({ kind: "applied", hash: "x", saved: false });
    await userEvent.click(within((await cards())[0]).getByRole("button", { name: "通过" }));
    expect(await screen.findByText("已写入，但状态保存失败；重启后这条建议可能再次出现")).toBeInTheDocument();
  });

  it("already applied removes card with info toast", async () => {
    setup([modify]);
    (applySuggestion as Mock).mockResolvedValue({ kind: "already_applied" });
    await userEvent.click(within((await cards())[0]).getByRole("button", { name: "通过" }));
    expect(await screen.findByText("这条建议已经生效（文件里已有相同内容）")).toBeInTheDocument();
    expect(screen.queryByRole("article")).not.toBeInTheDocument();
  });

  it("needs confirm then retry with flag", async () => {
    setup([modify]);
    (applySuggestion as Mock)
      .mockResolvedValueOnce({ kind: "needs_confirm", reason: "target_changed" })
      .mockResolvedValueOnce({ kind: "applied", hash: "x", saved: true });
    await userEvent.click(within((await cards())[0]).getByRole("button", { name: "通过" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText("建议生成后文件已被修改，原文仍能找到，确认仍要应用？")).toBeInTheDocument();
    expect(applySuggestion).toHaveBeenCalledTimes(1);
    // the card stays while waiting for the answer
    expect(screen.getByRole("article", { hidden: true })).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "确认应用" }));
    await waitFor(() => expect(applySuggestion).toHaveBeenLastCalledWith("s-mod", null, true));
    await waitFor(() => expect(screen.queryByRole("article")).not.toBeInTheDocument());
  });

  it("needs confirm for disk_changed shows its own text", async () => {
    setup([modify]);
    (applySuggestion as Mock).mockResolvedValueOnce({ kind: "needs_confirm", reason: "disk_changed" });
    await userEvent.click(within((await cards())[0]).getByRole("button", { name: "通过" }));
    expect(await screen.findByText("文件刚被其它程序修改，请确认后重试")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.getByRole("article")).toBeInTheDocument();
    expect(applySuggestion).toHaveBeenCalledTimes(1);
  });

  it("stale card shows invalid state", async () => {
    setup([modify]);
    (applySuggestion as Mock).mockResolvedValue({ kind: "stale" });
    await userEvent.click(within((await cards())[0]).getByRole("button", { name: "通过" }));
    const card = await screen.findByRole("article");
    expect(within(card).getByText("目标内容已变化，建议已失效")).toBeInTheDocument();
    expect(within(card).queryByRole("button", { name: "通过" })).not.toBeInTheDocument();
    expect(within(card).queryByRole("button", { name: "编辑后通过" })).not.toBeInTheDocument();
    // a reload (suggestions-changed) must not make the greyed card vanish
    let resolveReload: (v: Suggestion[]) => void = () => {};
    (listSuggestions as Mock).mockReturnValue(new Promise<Suggestion[]>((r) => (resolveReload = r)));
    const before = (listSuggestions as Mock).mock.calls.length;
    act(() => changedCb({ total: 0, by_project: {} }));
    await waitFor(() => expect(listSuggestions).toHaveBeenCalledTimes(before + 1));
    // the reload returns only a different item: once it shows, the reload has been applied
    await act(async () => resolveReload([add]));
    expect(await screen.findByRole("article", { name: /^新增/ })).toBeInTheDocument();
    const stale = screen.getByRole("article", { name: /^修改/ });
    expect(within(stale).getByText("目标内容已变化，建议已失效")).toBeInTheDocument();
    (rejectSuggestion as Mock).mockResolvedValue(undefined);
    await userEvent.click(within(screen.getByRole("article", { name: /^修改/ })).getByRole("button", { name: "驳回" }));
    await waitFor(() => expect(screen.queryByRole("article", { name: /^修改/ })).not.toBeInTheDocument());
    expect(rejectSuggestion).toHaveBeenCalledWith("s-mod");
  });

  it("edit then accept sends edited text", async () => {
    setup([modify]);
    (applySuggestion as Mock).mockResolvedValue({ kind: "applied", hash: "x", saved: true });
    const card = (await cards())[0];
    await userEvent.click(within(card).getByRole("button", { name: "编辑后通过" }));
    const editor = within(card).getByLabelText("editor");
    expect(editor).toHaveValue("从库延迟 1 秒");
    await userEvent.clear(editor);
    await userEvent.type(editor, "改过的文字");
    await userEvent.click(within(card).getByRole("button", { name: "确认通过" }));
    expect(applySuggestion).toHaveBeenCalledWith("s-mod", "改过的文字", false);
    await waitFor(() => expect(screen.queryByRole("article")).not.toBeInTheDocument());
  });

  it("edit with invalid text shows inline error and keeps the card", async () => {
    setup([modify]);
    (applySuggestion as Mock).mockRejectedValue({ code: "invalid", message: "new text is empty" });
    const card = (await cards())[0];
    await userEvent.click(within(card).getByRole("button", { name: "编辑后通过" }));
    await userEvent.clear(within(card).getByLabelText("editor"));
    await userEvent.click(within(card).getByRole("button", { name: "确认通过" }));
    expect(await within(card).findByRole("alert")).toHaveTextContent("内容无效：new text is empty");
    expect(screen.getByRole("article")).toBeInTheDocument();
  });

  it("reject removes card", async () => {
    setup();
    (rejectSuggestion as Mock).mockResolvedValue(undefined);
    const list = await cards();
    await userEvent.click(within(list[1]).getByRole("button", { name: "驳回" }));
    expect(rejectSuggestion).toHaveBeenCalledWith("s-add");
    await waitFor(() => expect(screen.getAllByRole("article")).toHaveLength(2));
  });

  it("buttons are disabled while a request is in flight", async () => {
    setup([modify]);
    let resolve: (v: unknown) => void = () => {};
    (applySuggestion as Mock).mockReturnValue(new Promise((r) => (resolve = r)));
    const card = (await cards())[0];
    await userEvent.click(within(card).getByRole("button", { name: "通过" }));
    expect(within(card).getByRole("button", { name: "通过" })).toBeDisabled();
    expect(within(card).getByRole("button", { name: "驳回" })).toBeDisabled();
    await act(async () => resolve({ kind: "applied", hash: "x", saved: true }));
  });

  it("reject all confirms, rejects every listed suggestion and clears the list", async () => {
    (rejectSuggestion as Mock).mockResolvedValue(undefined);
    setup();
    await cards();
    await userEvent.click(screen.getByRole("button", { name: "全部驳回" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText(/3 条待审核建议/)).toBeInTheDocument();
    expect(rejectSuggestion).not.toHaveBeenCalled();
    await userEvent.click(within(dialog).getByRole("button", { name: "驳回 3 条" }));
    await waitFor(() => expect(rejectSuggestion).toHaveBeenCalledTimes(3));
    expect(await screen.findByText("已驳回 3 条建议")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryAllByRole("article")).toHaveLength(0));
  });

  it("a run filter shows only that run's suggestions until cleared", async () => {
    const onClear = vi.fn();
    (listSuggestions as Mock).mockResolvedValue([{ ...modify, run_id: "r1" }, { ...add, run_id: "r2" }, { ...obsolete, run_id: "r1" }]);
    renderApp(<ReviewPage runFilter={{ id: "r1", started_at: 0, finished_at: 1 }} onClearRunFilter={onClear} />);
    await waitFor(() => expect(screen.getAllByRole("article")).toHaveLength(2));
    expect(screen.getByText(/这次提取的建议/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "显示全部" }));
    expect(onClear).toHaveBeenCalled();
  });

  it("accept all confirms count", async () => {
    setup();
    (acceptSuggestions as Mock).mockResolvedValue([
      { id: "s-mod", outcome: { kind: "applied", hash: "x", saved: true } },
      { id: "s-add", outcome: { kind: "needs_confirm", reason: "target_changed" } },
      { id: "s-obs", outcome: { kind: "stale" } },
    ]);
    await cards();
    await userEvent.click(screen.getByRole("button", { name: "全部通过" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText(/3 条待审核建议/)).toBeInTheDocument();
    expect(acceptSuggestions).not.toHaveBeenCalled();
    await userEvent.click(within(dialog).getByRole("button", { name: "通过 3 条" }));
    expect(acceptSuggestions).toHaveBeenCalledWith(["s-mod", "s-add", "s-obs"]);
    expect(await screen.findByText(/已通过 1 条/)).toHaveTextContent("1 条需要确认（仍在列表中）");
    expect(screen.getByText(/1 条已失效/)).toBeInTheDocument();
    // applied one is gone; needs-confirm one stays; stale one is greyed
    await waitFor(() => expect(screen.getAllByRole("article")).toHaveLength(2));
    expect(screen.getByText("目标内容已变化，建议已失效")).toBeInTheDocument();
  });

  it("accept all uses only the filtered pending ids", async () => {
    setup();
    (acceptSuggestions as Mock).mockResolvedValue([{ id: "s-add", outcome: { kind: "applied", hash: "x", saved: true } }]);
    await cards();
    await userEvent.click(screen.getByRole("radio", { name: "新增" }));
    expect(screen.getAllByRole("article")).toHaveLength(1);
    await userEvent.click(screen.getByRole("button", { name: "全部通过" }));
    await userEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "通过 1 条" }));
    expect(acceptSuggestions).toHaveBeenCalledWith(["s-add"]);
  });

  it("empty state links to the extraction page", async () => {
    const { onNavigate } = setup([]);
    expect(await screen.findByText("没有待审核的建议")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "去提取任务" }));
    expect(onNavigate).toHaveBeenCalledWith("extract");
    expect(screen.queryByRole("button", { name: "全部通过" })).not.toBeInTheDocument();
  });

  it("evidence drawer shows the highlighted quote with tool, session and time", async () => {
    setup([modify]);
    (suggestionContext as Mock).mockResolvedValue([
      {
        evidence: ev(),
        turns: [
          { tool: "claude", session_id: "sess-0123456789abcdef", project_id: null, ts: 1_730_000_000_000, role: "assistant", text: "好的，需要调整吗" },
          { tool: "claude", session_id: "sess-0123456789abcdef", project_id: null, ts: 1_730_000_001_000, role: "user", text: "请把从库延迟调小到 1 秒，缩短测试耗时" },
        ],
      },
    ]);
    const scrollSpy = vi.spyOn(Element.prototype, "scrollIntoView");
    await userEvent.click(within((await cards())[0]).getByRole("button", { name: "查看原对话" }));
    const drawer = await screen.findByRole("dialog");
    expect(suggestionContext).toHaveBeenCalledWith("s-mod");
    // The quoted message is marked (the passage itself is painted with the CSS Highlight API).
    const hit = await waitFor(() => {
      const li = drawer.querySelector('li[data-hit="true"]');
      if (!li) throw new Error("no hit row");
      return li as HTMLElement;
    });
    expect(hit).toHaveTextContent("请把从库延迟调小到 1 秒，缩短测试耗时");
    await waitFor(() => expect(scrollSpy.mock.contexts).toContain(hit));
    expect(within(drawer).getByText("好的，需要调整吗")).toBeInTheDocument();
    expect(within(drawer).getAllByText("Claude Code").length).toBeGreaterThan(0);
    expect(within(drawer).getByText(/sess-012/)).toBeInTheDocument();
    expect(within(drawer).queryByText(/sess-0123456789abcdef/)).not.toBeInTheDocument();
  });
  it("needs confirm retry keeps the edited text", async () => {
    setup([modify]);
    (applySuggestion as Mock)
      .mockResolvedValueOnce({ kind: "needs_confirm", reason: "disk_changed" })
      .mockResolvedValueOnce({ kind: "applied", hash: "x", saved: true });
    const card = (await cards())[0];
    await userEvent.click(within(card).getByRole("button", { name: "编辑后通过" }));
    const editor = within(card).getByLabelText("editor");
    await userEvent.clear(editor);
    await userEvent.type(editor, "改过的文字");
    await userEvent.click(within(card).getByRole("button", { name: "确认通过" }));
    expect(applySuggestion).toHaveBeenNthCalledWith(1, "s-mod", "改过的文字", false);
    await userEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "确认应用" }));
    await waitFor(() => expect(applySuggestion).toHaveBeenNthCalledWith(2, "s-mod", "改过的文字", true));
  });

  it("not_found and not_pending self-heal by removing the card", async () => {
    setup([modify, add]);
    (applySuggestion as Mock)
      .mockRejectedValueOnce({ code: "not_found", message: "gone" })
      .mockRejectedValueOnce({ code: "not_pending", message: "done" });
    let list = await cards();
    await userEvent.click(within(list[0]).getByRole("button", { name: "通过" }));
    expect(await screen.findByText("这条建议已不存在")).toBeInTheDocument();
    await waitFor(() => expect(screen.getAllByRole("article")).toHaveLength(1));
    list = screen.getAllByRole("article");
    await userEvent.click(within(list[0]).getByRole("button", { name: "通过" }));
    expect(await screen.findByText("这条建议已被处理")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("article")).not.toBeInTheDocument());
  });

  it("bulk result with saved=false shows the warning", async () => {
    setup([modify]);
    (acceptSuggestions as Mock).mockResolvedValue([{ id: "s-mod", outcome: { kind: "applied", hash: "x", saved: false } }]);
    await cards();
    await userEvent.click(screen.getByRole("button", { name: "全部通过" }));
    await userEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "通过 1 条" }));
    expect(await screen.findByText("已写入，但状态保存失败；重启后这条建议可能再次出现")).toBeInTheDocument();
  });

  it("accept all is disabled while a single card request is in flight", async () => {
    setup([modify, add]);
    let resolve: (v: unknown) => void = () => {};
    (applySuggestion as Mock).mockReturnValue(new Promise((r) => (resolve = r)));
    const list = await cards();
    await userEvent.click(within(list[0]).getByRole("button", { name: "通过" }));
    expect(screen.getByRole("button", { name: "全部通过" })).toBeDisabled();
    await act(async () => resolve({ kind: "applied", hash: "x", saved: true }));
    await waitFor(() => expect(screen.getByRole("button", { name: "全部通过" })).toBeEnabled());
  });

  it("header time uses the most recent succeeded run", async () => {
    (listExtractRuns as Mock).mockResolvedValue([
      run({ id: "r3", status: "failed", started_at: new Date(2026, 10, 5, 12, 0).getTime() }),
      run({ id: "r1", started_at: new Date(2026, 10, 4, 8, 15).getTime() }),
      run({ id: "r2", started_at: new Date(2026, 10, 5, 9, 30).getTime() }),
    ]);
    setup();
    expect(await screen.findByText("3 条建议 · 来自 11-05 09:30")).toBeInTheDocument();
  });

  it("evidence prefers the user turn, normalizes whitespace, and notes a missing quote", async () => {
    setup([modify]);
    const turn = (role: "user" | "assistant", text: string, ts: number) => ({ tool: "claude" as const, session_id: "sess-0123456789abcdef", project_id: null, ts, role, text });
    (suggestionContext as Mock).mockResolvedValue([
      {
        evidence: ev("把从库延迟   调小到 1 秒"),
        turns: [turn("assistant", "我会把从库延迟 调小到 1 秒", 1), turn("user", "请把从库延迟\n调小到 1 秒，谢谢", 2)],
      },
    ]);
    await userEvent.click(within((await cards())[0]).getByRole("button", { name: "查看原对话" }));
    const drawer = await screen.findByRole("dialog");
    const hit = await waitFor(() => {
      const li = drawer.querySelector('li[data-hit="true"]');
      if (!li) throw new Error("no hit row");
      return li as HTMLElement;
    });
    expect(hit).toHaveTextContent("请把从库延迟");
    expect(within(drawer).queryByText("未在上下文中找到原话")).not.toBeInTheDocument();

    (suggestionContext as Mock).mockResolvedValue([{ evidence: ev("完全不存在的话"), turns: [turn("user", "别的内容", 2)] }]);
    await userEvent.keyboard("{Escape}");
    await userEvent.click(within((await cards())[0]).getByRole("button", { name: "查看原对话" }));
    expect(await screen.findByText("未在上下文中找到原话")).toBeInTheDocument();
  });

  it("shows the GitHub origin of imported suggestions and the import button", async () => {
    const imported = sug({
      id: "s-gh",
      kind: "add",
      old_text: null,
      new_text: "导入的规则",
      evidence: [],
      origin: { type: "github", repo: "octo/mem", path: "CLAUDE.md", git_ref: "main", commit: "abcdef123", url: "https://github.com/octo/mem/blob/abcdef123/CLAUDE.md" },
    });
    setup([imported]);
    const [card] = await cards();
    expect(within(card).getByTestId("suggestion-origin")).toHaveTextContent("来源：GitHub 导入：octo/mem · CLAUDE.md@abcdef1");
    expect(within(card).queryByRole("button", { name: "查看原对话" })).toBeNull();
    expect(screen.getByRole("button", { name: "从 GitHub 导入" })).toBeInTheDocument();
  });
});
