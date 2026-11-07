import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { ConvertResult, SessionPreview, SessionSummary } from "@/api/types";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  listMemories: vi.fn(),
}));
vi.mock("@/api/sessions", () => ({
  listSessions: vi.fn(),
  sessionPreview: vi.fn(),
  convertTargetDir: vi.fn(),
  convertSession: vi.fn(),
  renameSession: vi.fn(),
}));

vi.mock("@/api/workspaceCode", () => ({
  codeEntries: vi.fn().mockResolvedValue([]),
  codeStatus: vi.fn().mockResolvedValue({}),
  readCode: vi.fn().mockResolvedValue({ kind: "text", text: "fn main() {}", revision: "r1", baseline: null }),
}));

import { readCode } from "@/api/workspaceCode";
import { convertSession, convertTargetDir, listSessions, renameSession, sessionPreview } from "@/api/sessions";
import { renderApp } from "@/test/fixtures";
import { SessionsPage, historyBefore } from "./SessionsPage";
import { filterSessions, groupByProject } from "./sessionsUtils";

function mk(over: Partial<SessionSummary> & { id: string }): SessionSummary {
  return {
    tool: "claude",
    path: `/home/u/.claude/projects/-work-demo/${over.id}.jsonl`,
    cwd: "/work/demo",
    project_id: "/work/demo",
    project_name: "demo",
    title: null,
    first_prompt: "Fix the build",
    continued: false,
    message_count: 4,
    tool_calls: 2,
    started_at: 1_700_000_000_000,
    updated_at: 1_700_000_100_000,
    model: null,
    converted_from: null,
    converted_to: [],
    ...over,
  };
}

const claudeSession = mk({ id: "c1", title: "Build fix" });
const codexSession = mk({
  id: "x1",
  tool: "codex",
  path: "/home/u/.codex/sessions/2026/11/01/rollout-x1.jsonl",
  cwd: "/work/api",
  project_id: "/work/api",
  project_name: "api",
  first_prompt: "List the files",
  updated_at: 1_700_000_200_000,
});

const preview: SessionPreview = {
  summary: claudeSession,
  messages: [
    { role: "user", ts: "2026-11-01T10:00:00.000Z", text: "Fix the build" },
    { role: "shell", ts: "2026-11-01T10:00:01.000Z", text: "cargo build" },
  ],
  total_messages: 2,
  stats: { user_messages: 1, assistant_messages: 1, tool_calls: 2, mapped_calls: 1, degraded_calls: 1 },
  timeline: [
    { type: "user", ts: "2026-11-01T10:00:00.000Z", text: "Fix the build" },
    { type: "tool", ts: "2026-11-01T10:00:01.000Z", name: "Bash", kind: "shell", summary: "cargo build", input: "cargo build", output: "ok", is_error: false, mapped: true },
    { type: "tool", ts: "2026-11-01T10:00:02.000Z", name: "Read", kind: "read", summary: "src/main.rs", input: "{}", output: "no such file", is_error: true, mapped: false },
    { type: "assistant", ts: "2026-11-01T10:00:03.000Z", text: "Fixed." },
  ],
  counts: { user: 1, assistant: 1, tools: 2, shell: 1, files: 1, errors: 1 },
  timeline_total: 4,
};

const result: ConvertResult = {
  tool: "codex",
  session_id: "new-id",
  path: "/home/u/.codex/sessions/2026/11/06/rollout-new-id.jsonl",
  cwd: "/work/demo",
  resume_command: 'cd "/work/demo"; codex resume new-id',
  indexed: false,
  warnings: ["state_db_missing"],
  stats: preview.stats,
};

describe("sessionsUtils", () => {
  it("filters by tool and every search word", () => {
    const list = [claudeSession, codexSession];
    expect(filterSessions(list, "codex", "")).toEqual([codexSession]);
    expect(filterSessions(list, "all", "build demo")).toEqual([claudeSession]);
    expect(filterSessions(list, "all", "API")).toEqual([codexSession]);
    expect(filterSessions(list, "claude", "files")).toEqual([]);
  });

  it("groups by project with the newest group first", () => {
    const groups = groupByProject([claudeSession, codexSession, mk({ id: "c2", updated_at: 1 })]);
    expect(groups.map((g) => g.name)).toEqual(["api", "demo"]);
    expect(groups[1].sessions.map((s) => s.id)).toEqual(["c1", "c2"]);
  });
});

describe("SessionsPage", () => {
  beforeEach(() => {
    (listSessions as Mock).mockResolvedValue([codexSession, claudeSession]);
    (sessionPreview as Mock).mockResolvedValue(preview);
    (convertTargetDir as Mock).mockResolvedValue("/home/u/.codex/sessions");
    (convertSession as Mock).mockResolvedValue(result);
  });

  it("lists sessions grouped by project and filters them", async () => {
    renderApp(<SessionsPage />);
    expect(await screen.findByRole("region", { name: "api" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "demo" })).toBeInTheDocument();
    await userEvent.type(screen.getByRole("searchbox", { name: "搜索会话内容、项目或 ID" }), "build");
    expect(screen.queryByRole("region", { name: "api" })).toBeNull();
    await userEvent.click(screen.getByRole("radio", { name: "Codex" }));
    expect(screen.getByText("没有匹配的会话")).toBeInTheDocument();
  });

  it("each assistant turn is one card with tool chips that open a dialog", async () => {
    renderApp(<SessionsPage />);
    await userEvent.click(await screen.findByRole("button", { name: /Build fix/ }));
    expect(await screen.findByText("Fixed.")).toBeInTheDocument();
    expect(screen.getByText("cargo build")).toBeInTheDocument();
    expect(screen.getByText("src/main.rs")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /Bash.*cargo build/ }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("ok")).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    await userEvent.click(screen.getByRole("radio", { name: /出错/ }));
    expect(screen.queryByText("cargo build")).not.toBeInTheDocument();
    expect(screen.getByText("src/main.rs")).toBeInTheDocument();
    expect(screen.queryByText("Fixed.")).not.toBeInTheDocument();
  });

  it("opens a history file in the session's right-hand code panel", async () => {
    vi.mocked(sessionPreview).mockResolvedValue({
      ...preview,
      timeline: preview.timeline.map((item) => item.type === "tool" && item.name === "Read"
        ? { ...item, input: '{"file_path":"/work/demo/src/main.rs"}', is_error: false }
        : item),
    });
    renderApp(<SessionsPage />);
    await userEvent.click(await screen.findByRole("button", { name: /Build fix/ }));
    await userEvent.click(await screen.findByRole("button", { name: /Read.*src\/main.rs/ }));
    const codeButton = screen.getByRole("button", { name: "代码", expanded: true });
    const panel = await screen.findByRole("complementary", { name: "代码" }, { timeout: 5000 });
    expect(await within(panel).findByRole("button", { name: "main.rs", pressed: true })).toBeInTheDocument();
    expect(readCode).toHaveBeenCalledWith("/work/demo", "src/main.rs", null);
    expect(codeButton.previousElementSibling).toHaveAttribute("role", "radiogroup");
    expect(codeButton.previousElementSibling).toHaveClass("h-8");
    expect(codeButton).toHaveClass("h-8", "rounded-none");
    expect(within(codeButton).getByText("📂")).toHaveClass("text-lg");
    expect(panel.parentElement).not.toContainElement(codeButton);
    expect(panel.parentElement?.previousElementSibling).toContainElement(codeButton);
    await userEvent.click(screen.getByRole("button", { name: "代码", expanded: true }));
    expect(panel).not.toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "代码", expanded: false }));
    expect(within(panel).getByRole("button", { name: "main.rs", pressed: true })).toBeInTheDocument();
  }, 10000);

  it("folds long runs of tool chips behind +N", async () => {
    const many = Array.from({ length: 11 }, (_, i) => ({
      type: "tool" as const, ts: "2026-11-01T10:00:01.000Z", name: "Read", kind: "read" as const,
      summary: `file-${i}.rs`, input: "{}", output: "x", is_error: false, mapped: false,
    }));
    (sessionPreview as Mock).mockResolvedValue({ ...preview, timeline: [preview.timeline[0], ...many] });
    renderApp(<SessionsPage />);
    await userEvent.click(await screen.findByRole("button", { name: /Build fix/ }));
    expect(await screen.findByText("file-7.rs")).toBeInTheDocument();
    expect(screen.queryByText("file-8.rs")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "+3 个" }));
    expect(screen.getByText("file-10.rs")).toBeInTheDocument();
  });

  it("previews, confirms and shows the resume command", async () => {
    renderApp(<SessionsPage />);
    await userEvent.click(await screen.findByRole("button", { name: /Build fix/ }));
    expect(await screen.findByText("cargo build")).toBeInTheDocument();
    expect(screen.getByText("1 个 shell 命令保留为工具调用")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "转换为 Codex 会话" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText(/\/home\/u\/\.codex\/sessions/)).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "转换" }));

    await waitFor(() => expect(convertSession).toHaveBeenCalledWith("claude", claudeSession.path, false));
    expect(await screen.findByText('cd "/work/demo"; codex resume new-id')).toBeInTheDocument();
    expect(screen.getByText(/没有找到 Codex 状态库/)).toBeInTheDocument();
    expect(listSessions).toHaveBeenCalledTimes(2);
  });

  it("asks again as a repeat conversion when the backend reports already_converted", async () => {
    (convertSession as Mock)
      .mockRejectedValueOnce({ code: "already_converted", message: "already converted to old-id" })
      .mockResolvedValueOnce(result);
    renderApp(<SessionsPage />);
    await userEvent.click(await screen.findByRole("button", { name: /Build fix/ }));
    await screen.findByText("cargo build");
    await userEvent.click(screen.getByRole("button", { name: "转换为 Codex 会话" }));
    await userEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "转换" }));
    const again = await screen.findByRole("alertdialog");
    expect(within(again).getByText(/之前已经转换过/)).toBeInTheDocument();
    await userEvent.click(within(again).getByRole("button", { name: "转换" }));
    await waitFor(() => expect(convertSession).toHaveBeenLastCalledWith("claude", claudeSession.path, true));
  });

  it("refresh and clicking the open session again re-read its history", async () => {
    renderApp(<SessionsPage />);
    await userEvent.click(await screen.findByRole("button", { name: /Build fix/ }));
    await waitFor(() => expect(sessionPreview).toHaveBeenCalledTimes(1));
    await userEvent.click(screen.getByRole("button", { name: "刷新" }));
    await waitFor(() => expect(sessionPreview).toHaveBeenCalledTimes(2));
    await userEvent.click(screen.getByRole("button", { name: /Build fix/ }));
    await waitFor(() => expect(sessionPreview).toHaveBeenCalledTimes(3));
  });

  it("loads earlier history entries on demand", async () => {
    (sessionPreview as Mock).mockResolvedValue({ ...preview, timeline_total: 900 });
    renderApp(<SessionsPage />);
    await userEvent.click(await screen.findByRole("button", { name: /Build fix/ }));
    const more = await screen.findByRole("button", { name: "查看更早的 400 条" });
    expect(sessionPreview).toHaveBeenLastCalledWith("claude", claudeSession.path, 400);
    await userEvent.click(more);
    await waitFor(() => expect(sessionPreview).toHaveBeenLastCalledWith("claude", claudeSession.path, 800));
  });

  it("renames the open session in place", async () => {
    (renameSession as Mock).mockResolvedValue(undefined);
    renderApp(<SessionsPage />);
    await userEvent.click(await screen.findByRole("button", { name: /Build fix/ }));
    await userEvent.click(await screen.findByRole("button", { name: "重命名" }));
    const box = screen.getByRole("textbox", { name: "会话名称" });
    await userEvent.clear(box);
    await userEvent.type(box, "登录页修复{Enter}");
    await waitFor(() => expect(renameSession).toHaveBeenCalledWith("claude", claudeSession.path, "登录页修复"));
    await waitFor(() => expect(listSessions).toHaveBeenCalledTimes(2));
  });
});

describe("historyBefore", () => {
  it("keeps only what was on disk before the live process started", () => {
    const start = Date.parse("2026-11-01T10:00:01.500Z");
    const cut = historyBefore(preview, start);
    expect(cut.timeline.map((t) => t.ts)).toEqual(["2026-11-01T10:00:00.000Z", "2026-11-01T10:00:01.000Z"]);
    expect(cut.timeline_total).toBe(2);
    expect(historyBefore(preview, Date.parse("2026-11-01T09:00:00.000Z")).timeline).toEqual([]);
    expect(historyBefore(preview, Date.parse("2026-12-01T00:00:00.000Z"))).toBe(preview);
  });
});

import { linkPaneIds } from "./SessionsPage";

describe("pane identity", () => {
  it("keeps one key while a conversation goes draft → live → listed, and separates others", () => {
    const k1 = linkPaneIds(["d:draft-a"]);
    // first send: the draft is linked to its live process and session id
    expect(linkPaneIds(["d:draft-a", "l:L1", "n:claude:S1"])).toBe(k1);
    // the session file shows up on the list: selection switches to it
    expect(linkPaneIds(["s:claude:/p/S1.jsonl", "n:claude:S1", "l:L1"])).toBe(k1);
    // a different conversation gets its own key
    expect(linkPaneIds(["s:claude:/p/S2.jsonl", "n:claude:S2"])).not.toBe(k1);
  });
});
