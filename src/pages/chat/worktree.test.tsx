import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { SessionPreview, SessionSummary } from "@/api/types";
import type { LiveSessionInfo } from "@/api/live";
import type { WorktreeInfo, WorktreeStatus } from "@/api/worktree";

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
vi.mock("@/api/live", async () => {
  const real = await vi.importActual<typeof import("@/api/live")>("@/api/live");
  return {
    parseBusy: real.parseBusy,
    storedImagePath: real.storedImagePath,
    nativeOwner: vi.fn(),
    liveList: vi.fn(),
    liveStop: vi.fn(),
    liveStart: vi.fn(),
    liveSend: vi.fn(),
    liveRespond: vi.fn(),
    liveInterrupt: vi.fn(),
    liveClose: vi.fn(),
    liveTranscript: vi.fn().mockResolvedValue([]),
    onLiveEvent: vi.fn().mockResolvedValue(() => {}),
    readImageDataUri: vi.fn(),
    liveModels: vi.fn().mockResolvedValue([]),
    liveCommands: vi.fn().mockResolvedValue([]),
    projectFiles: vi.fn().mockResolvedValue([]),
  };
});
vi.mock("@/api/worktree", () => ({
  worktreeCheck: vi.fn(),
  worktreeCreate: vi.fn(),
  worktreeList: vi.fn(),
  worktreeStatus: vi.fn(),
  worktreeDiff: vi.fn(),
  worktreeMerge: vi.fn(),
  worktreeRemove: vi.fn(),
}));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn().mockResolvedValue("/home/u") }));
vi.mock("@/api/pty", () => ({
  ptyOpen: vi.fn(),
  ptyWrite: vi.fn().mockResolvedValue(undefined),
  ptyResize: vi.fn().mockResolvedValue(undefined),
  ptyClose: vi.fn().mockResolvedValue(undefined),
  saveTempImage: vi.fn(),
  onPtyData: vi.fn(async () => () => {}),
  onPtyExit: vi.fn(async () => () => {}),
}));
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    loadAddon() {}
    open() {}
    focus() {}
    reset() {}
    write() {}
    onData() {
      return { dispose() {} };
    }
    dispose() {}
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));
vi.mock("@xterm/addon-web-links", () => ({ WebLinksAddon: class {} }));
vi.mock("@xterm/xterm/css/xterm.css", () => ({}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn(), openPath: vi.fn().mockResolvedValue(undefined) }));

import { homeDir } from "@tauri-apps/api/path";
import { openPath } from "@tauri-apps/plugin-opener";
import { liveList, liveStart, liveStop, nativeOwner } from "@/api/live";
import { ptyOpen } from "@/api/pty";
import { listSessions, sessionPreview } from "@/api/sessions";
import { worktreeCheck, worktreeCreate, worktreeDiff, worktreeList, worktreeMerge, worktreeRemove, worktreeStatus } from "@/api/worktree";
import { renderApp } from "@/test/fixtures";
import { SessionsPage } from "@/pages/sessions/SessionsPage";
import { refreshLiveList, resetLiveStore } from "./liveStore";
import { resetTerminalStore } from "./terminalStore";
import { DEFAULT_PREFS, saveChatPrefs } from "./chatPrefs";
import { resetConvUi } from "./convUiState";
import { resetComposerCaches } from "./Composer";
import { parseWorktreePath, resetWorktreeStore } from "./worktreeStore";

const WT = "/work/demo/.memdock/worktrees/fix-login";

const info: WorktreeInfo = {
  path: WT,
  branch: "memdock/fix-login",
  slug: "fix-login",
  repo_root: "/work/demo",
  base_branch: "main",
  base_commit: "abc123",
  created_at: 1,
};

const status: WorktreeStatus = {
  files: [
    { path: "src/Login.tsx", status: "M", added: 30, deleted: 6 },
    { path: "src/button.css", status: "M", added: 10, deleted: 2 },
    { path: "src/Login.test.tsx", status: "A", added: 2, deleted: 0 },
  ],
  ahead: 2,
  dirty: true,
};

function mk(over: Partial<SessionSummary> & { id: string }): SessionSummary {
  return {
    tool: "claude",
    path: `/home/u/.claude/projects/x/${over.id}.jsonl`,
    cwd: "/work/demo",
    project_id: "/work/demo",
    project_name: "demo",
    title: null,
    first_prompt: "hello",
    continued: false,
    message_count: 12,
    tool_calls: 0,
    started_at: 1_700_000_000_000,
    updated_at: 1_700_000_100_000,
    model: null,
    converted_from: null,
    converted_to: [],
    ...over,
  };
}

const plain = mk({ id: "c1", title: "Research" });
const wtSession = mk({ id: "w1", title: "Fix login", cwd: WT, project_id: WT, project_name: "fix-login", updated_at: 1_700_000_200_000 });
const other = mk({ id: "p1", title: "Plain stuff", cwd: "/work/plain", project_id: "/work/plain", project_name: "plain", updated_at: 1_600_000_000_000 });

const preview: SessionPreview = {
  summary: plain,
  messages: [],
  total_messages: 0,
  stats: { user_messages: 0, assistant_messages: 0, tool_calls: 0, mapped_calls: 0, degraded_calls: 0 },
  timeline: [],
  counts: { user: 0, assistant: 0, tools: 0, shell: 0, files: 0, errors: 0 },
  timeline_total: 0,
};

function liveInfo(over: Partial<LiveSessionInfo> = {}): LiveSessionInfo {
  return {
    live_id: "L1",
    agent: "claude",
    cwd: WT,
    title: "Fix login",
    state: "idle",
    native_id: "w1",
    model: null,
    permission: "ask",
    created_at: 1,
    updated_at: 2,
    ...over,
  };
}

beforeEach(() => {
  resetTerminalStore();
  resetLiveStore();
  resetConvUi();
  resetComposerCaches();
  resetWorktreeStore();
  vi.clearAllMocks();
  localStorage.clear();
  saveChatPrefs({ ...DEFAULT_PREFS, cwd: "/work/demo" });
  (listSessions as Mock).mockResolvedValue([plain, wtSession, other]);
  (sessionPreview as Mock).mockResolvedValue(preview);
  (liveList as Mock).mockResolvedValue([]);
  (liveStop as Mock).mockResolvedValue(undefined);
  (nativeOwner as Mock).mockResolvedValue(null);
  (liveStart as Mock).mockResolvedValue(liveInfo({ live_id: "L9", native_id: null }));
  (ptyOpen as Mock).mockResolvedValue({ pty_id: "S1", command: "PowerShell" });
  (worktreeCheck as Mock).mockImplementation(async (cwd: string) => {
    if (cwd.startsWith("/work/demo")) return { repo_root: "/work/demo", base_branch: "main" };
    throw "不是 git 仓库";
  });
  (worktreeList as Mock).mockResolvedValue([info]);
  (worktreeStatus as Mock).mockResolvedValue(status);
  (worktreeCreate as Mock).mockResolvedValue(info);
  (worktreeDiff as Mock).mockResolvedValue("diff --git a/src/Login.tsx b/src/Login.tsx\n@@ -1,2 +1,2 @@\n-old line\n+new line\n same\n");
  (worktreeRemove as Mock).mockResolvedValue(undefined);
});

async function openWorktreeSession() {
  renderApp(<SessionsPage />);
  const group = await screen.findByRole("region", { name: "demo" });
  await userEvent.click(await within(group).findByRole("button", { name: /Fix login/ }));
  return await screen.findByRole("region", { name: "独立工作区" });
}

describe("worktree helpers", () => {
  it("parses worktree folders", () => {
    expect(parseWorktreePath("C:\\code\\app\\.memdock\\worktrees\\fix")).toEqual({ repoRoot: "C:/code/app", slug: "fix" });
    expect(parseWorktreePath("/work/demo")).toBeNull();
  });
});

describe("/worktree in the composer", () => {
  it("is offered for a draft in a git repo and toggles the violet status item", async () => {
    (homeDir as unknown as { mockResolvedValueOnce: (v: string) => void }).mockResolvedValueOnce("/work/demo"); // no conversation open: new chats start in the home folder
    renderApp(<SessionsPage />);
    await screen.findByRole("region", { name: "demo" });
    await userEvent.click(within(screen.getByRole("banner")).getByRole("button", { name: "新对话" }));
    const box = await screen.findByRole("textbox", { name: "消息" });
    await waitFor(() => expect(worktreeCheck).toHaveBeenCalledWith("/work/demo"));
    await userEvent.type(box, "/work");
    const opt = await screen.findByRole("option", { name: /\/worktree/ });
    expect(opt).toHaveTextContent("关");
    await userEvent.click(opt);
    const item = await screen.findByRole("button", { name: "在独立工作区运行（仅新会话）: 开" });
    expect(item).toHaveTextContent("🌿 独立工作区");
    await userEvent.click(item);
    expect(screen.queryByRole("button", { name: "在独立工作区运行（仅新会话）: 开" })).toBeNull();
  });

  it("is hidden outside git repos and for started conversations", async () => {
    (homeDir as unknown as { mockResolvedValueOnce: (v: string) => void }).mockResolvedValueOnce("/work/plain"); // no conversation open: new chats start in the home folder
    saveChatPrefs({ ...DEFAULT_PREFS, cwd: "/work/plain" });
    renderApp(<SessionsPage />);
    await screen.findByRole("region", { name: "demo" });
    await userEvent.click(within(screen.getByRole("banner")).getByRole("button", { name: "新对话" }));
    const box = await screen.findByRole("textbox", { name: "消息" });
    await waitFor(() => expect(worktreeCheck).toHaveBeenCalledWith("/work/plain"));
    await act(async () => {});
    await userEvent.type(box, "/");
    await screen.findByRole("option", { name: /\/model/ });
    expect(screen.queryByRole("option", { name: /\/worktree/ })).toBeNull();

    // A started session (not a draft) never offers it.
    await userEvent.click(within(screen.getByRole("region", { name: "demo" })).getByRole("button", { name: /Research/ }));
    const box2 = await screen.findByRole("textbox", { name: "消息" });
    await userEvent.type(box2, "/");
    await screen.findByRole("option", { name: /\/model/ });
    expect(screen.queryByRole("option", { name: /\/worktree/ })).toBeNull();
  });
});

describe("project ＋ menu", () => {
  it("starts a Codex draft from a Codex project even when Claude was last used", async () => {
    saveChatPrefs({ ...DEFAULT_PREFS, agent: "claude", cwd: "/work/demo" });
    (listSessions as Mock).mockResolvedValue([mk({ id: "codex-1", tool: "codex" })]);
    renderApp(<SessionsPage />);
    await userEvent.click(screen.getByRole("radio", { name: "Codex" }));
    const project = await screen.findByRole("region", { name: "demo" });
    await userEvent.click(within(project).getByRole("button", { name: "新建" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: /新建会话 · Codex/ }));
    await userEvent.type(screen.getByRole("textbox"), "hello Codex{Enter}");
    await waitFor(() => expect(liveStart).toHaveBeenCalledWith(expect.objectContaining({
      agent: "codex",
      cwd: "/work/demo",
      prompt: "hello Codex",
    })));
  });

  it("offers three actions; the worktree one is disabled outside git repos", async () => {
    renderApp(<SessionsPage />);
    const demo = await screen.findByRole("region", { name: "demo" });
    await userEvent.click(within(demo).getByRole("button", { name: "新建" }));
    const menu = await screen.findByRole("menu", { name: "demo" });
    const items = within(menu).getAllByRole("menuitem");
    expect(items.map((x) => x.textContent)).toEqual([
      expect.stringContaining("新建会话"),
      expect.stringContaining("在独立工作区新建会话"),
      expect.stringContaining("新建终端"),
    ]);
    expect(items[0]).toHaveFocus();
    await waitFor(() => expect(items[1]).toBeEnabled());
    expect(items[1]).toHaveTextContent("单独的目录和分支，做完再合并");
    await userEvent.keyboard("{Escape}");

    const plainGroup = screen.getByRole("region", { name: "plain" });
    await userEvent.click(within(plainGroup).getByRole("button", { name: "新建" }));
    const menu2 = await screen.findByRole("menu", { name: "plain" });
    await waitFor(() => expect(within(menu2).getAllByRole("menuitem")[1]).toHaveTextContent("不是 git 仓库"));
    expect(within(menu2).getAllByRole("menuitem")[1]).toBeDisabled();
  });
});

describe("starting a worktree draft", () => {
  async function worktreeDraft() {
    renderApp(<SessionsPage />);
    const demo = await screen.findByRole("region", { name: "demo" });
    await userEvent.click(within(demo).getByRole("button", { name: "新建" }));
    const item = (await screen.findByRole("menuitem", { name: /在独立工作区新建会话/ })) as HTMLButtonElement;
    await waitFor(() => expect(item).toBeEnabled());
    await userEvent.click(item);
  }

  it("first send creates the worktree, then starts the live session in it", async () => {
    await worktreeDraft();
    await screen.findByRole("button", { name: "在独立工作区运行（仅新会话）: 开" });
    await userEvent.type(screen.getByRole("textbox", { name: "消息" }), "Fix login{Enter}");
    await waitFor(() => expect(liveStart).toHaveBeenCalledWith(expect.objectContaining({ cwd: WT, prompt: "Fix login", resume_id: null })));
    expect(worktreeCreate).toHaveBeenCalledWith("/work/demo", "Fix login");
    expect((worktreeCreate as Mock).mock.invocationCallOrder[0]).toBeLessThan((liveStart as Mock).mock.invocationCallOrder[0]);
  });

  it("a failed create shows the error and keeps the text", async () => {
    (worktreeCreate as Mock).mockRejectedValue("分支 memdock/fix-login 已存在");
    await worktreeDraft();
    await screen.findByRole("button", { name: "在独立工作区运行（仅新会话）: 开" });
    await userEvent.type(screen.getByRole("textbox", { name: "消息" }), "Fix login{Enter}");
    expect(await screen.findByText(/分支 memdock\/fix-login 已存在/)).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "消息" })).toHaveValue("Fix login");
    expect(liveStart).not.toHaveBeenCalled();
  });

  it("switching the draft to terminal mode creates the worktree first and opens the CLI there", async () => {
    await worktreeDraft();
    await userEvent.click(await screen.findByRole("radio", { name: /命令行/ }));
    await waitFor(() => expect(ptyOpen).toHaveBeenCalledWith(expect.objectContaining({ tool: "claude", cwd: WT })));
    expect(worktreeCreate).toHaveBeenCalledWith("/work/demo", "terminal");
  });
});

describe("list", () => {
  it("groups worktree sessions under the parent project with a badge and changed files", async () => {
    renderApp(<SessionsPage />);
    const demo = await screen.findByRole("region", { name: "demo" });
    const row = await within(demo).findByRole("button", { name: /Fix login/ });
    expect(within(row).getByText("fix-login")).toBeInTheDocument();
    expect(row).toHaveTextContent("🌿 fix-login");
    expect(await within(row).findByText("改了 3 个文件")).toBeInTheDocument();
    expect(within(demo).getByRole("button", { name: /Research/ })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "fix-login" })).toBeNull();
  });
});

describe("worktree panel", () => {
  it("shows the branch, path and change summary", async () => {
    const panel = await openWorktreeSession();
    expect(panel).toHaveTextContent("🌿 memdock/fix-login ← main");
    expect(within(panel).getByText(WT)).toBeInTheDocument();
    await waitFor(() => expect(within(panel).getByTestId("worktree-summary")).toHaveTextContent("3 个文件 · +42 −8 · 2 个提交"));
    await userEvent.click(within(panel).getByRole("button", { name: /打开文件夹/ }));
    expect(openPath).toHaveBeenCalledWith(WT);
  });

  it("says when nothing changed yet", async () => {
    (worktreeStatus as Mock).mockResolvedValue({ files: [], ahead: 0, dirty: false });
    const panel = await openWorktreeSession();
    await waitFor(() => expect(within(panel).getByTestId("worktree-summary")).toHaveTextContent("还没有改动"));
  });

  it("查看改动 lists the files and shows the selected file's diff", async () => {
    const panel = await openWorktreeSession();
    await waitFor(() => expect(within(panel).getByTestId("worktree-summary")).toHaveTextContent("3 个文件"));
    await userEvent.click(within(panel).getByRole("button", { name: /查看改动/ }));
    const dialog = await screen.findByRole("dialog");
    const files = within(dialog).getByRole("list", { name: "改动的文件" });
    expect(within(files).getAllByRole("button")).toHaveLength(3);
    await waitFor(() => expect(worktreeDiff).toHaveBeenCalledWith(WT, "src/Login.tsx"));
    expect(await within(dialog).findByText("+new line")).toHaveAttribute("data-kind", "add");
    expect(within(dialog).getByText("-old line")).toHaveAttribute("data-kind", "del");
    await userEvent.click(within(files).getByRole("button", { name: /src\/button.css/ }));
    await waitFor(() => expect(worktreeDiff).toHaveBeenCalledWith(WT, "src/button.css"));
  });

  it("merges (squash, title as message, delete after) and then shows the deleted state", async () => {
    (worktreeMerge as Mock).mockResolvedValue({ merged: true, conflicts: [], auto_committed: true, commit: "deadbeefcafe" });
    const panel = await openWorktreeSession();
    await waitFor(() => expect(within(panel).getByTestId("worktree-summary")).toHaveTextContent("3 个文件"));
    await userEvent.click(within(panel).getByRole("button", { name: /合并到 main/ }));
    const dialog = await screen.findByRole("dialog", { name: "合并 memdock/fix-login 到 main" });
    expect(within(dialog).getByRole("radio", { name: "压缩成一个提交合并" })).toBeChecked();
    expect(within(dialog).getByRole("textbox", { name: "提交信息" })).toHaveValue("Fix login");
    expect(within(dialog).getByRole("checkbox", { name: "合并后删除工作区和分支" })).toBeChecked();
    expect(dialog).toHaveTextContent("合并前会先自动提交");
    await userEvent.click(within(dialog).getByRole("radio", { name: "只保留分支，不合并" }));
    expect(within(dialog).getByRole("checkbox", { name: "删除工作区（保留分支）" })).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("radio", { name: "压缩成一个提交合并" }));
    await userEvent.click(within(dialog).getByRole("button", { name: "合并" }));
    await waitFor(() => expect(worktreeMerge).toHaveBeenCalledWith(WT, "squash", "Fix login", false));
    await waitFor(() => expect(worktreeRemove).toHaveBeenCalledWith(WT, true));
    const notice = await screen.findByText(/已合并到 main/);
    expect(notice).toHaveTextContent("合并前已自动提交");
    expect(screen.queryByRole("region", { name: "独立工作区" })).toBeNull();
    expect(screen.getByRole("textbox", { name: "消息" })).toBeDisabled();
    expect(screen.getAllByText(/工作区已删除/).length).toBeGreaterThan(0);
  });

  it("merge is disabled while the session is replying", async () => {
    (liveList as Mock).mockResolvedValue([liveInfo({ state: "running" })]);
    await refreshLiveList();
    const panel = await openWorktreeSession();
    expect(within(panel).getByRole("button", { name: /合并到 main/ })).toBeDisabled();
  });

  async function conflicted() {
    (worktreeMerge as Mock).mockResolvedValue({ merged: false, conflicts: ["a.ts", "b.ts"], auto_committed: false, commit: null });
    const panel = await openWorktreeSession();
    await userEvent.click(within(panel).getByRole("button", { name: /合并到 main/ }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("checkbox"));
    await userEvent.click(within(dialog).getByRole("radio", { name: "保留全部提交合并" }));
    await userEvent.click(within(dialog).getByRole("button", { name: "合并" }));
    await waitFor(() => expect(worktreeMerge).toHaveBeenCalledWith(WT, "merge", "Fix login", false));
    const alert = await within(panel).findByRole("alert");
    expect(alert).toHaveTextContent("合并有冲突");
    expect(within(alert).getByText("a.ts")).toBeInTheDocument();
    return alert;
  }

  it("conflicts: 在终端处理 opens a shell in the worktree", async () => {
    const alert = await conflicted();
    await userEvent.click(within(alert).getByRole("button", { name: /在终端处理/ }));
    await waitFor(() => expect(ptyOpen).toHaveBeenCalledWith(expect.objectContaining({ tool: "shell", cwd: WT })));
  });

  it("conflicts: 让 AI 解决冲突 sends the session a rebase request", async () => {
    const alert = await conflicted();
    await userEvent.click(within(alert).getByRole("button", { name: /让 AI 解决冲突/ }));
    await waitFor(() =>
      expect(liveStart).toHaveBeenCalledWith(
        expect.objectContaining({ cwd: WT, resume_id: "w1", prompt: "请把当前分支 rebase 到 main 的最新提交，并解决冲突：a.ts、b.ts" }),
      ),
    );
  });

  it("放弃并删除 stops the process, removes the worktree and branch", async () => {
    (liveList as Mock).mockResolvedValue([liveInfo({ state: "idle" })]);
    await refreshLiveList();
    const panel = await openWorktreeSession();
    await userEvent.click(within(panel).getByRole("button", { name: "更多操作" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: /放弃并删除工作区/ }));
    const confirm = await screen.findByRole("alertdialog");
    await userEvent.click(within(confirm).getByRole("button", { name: "放弃并删除" }));
    await waitFor(() => expect(worktreeRemove).toHaveBeenCalledWith(WT, true));
    expect(liveStop).toHaveBeenCalledWith("L1");
    expect((liveStop as Mock).mock.invocationCallOrder[0]).toBeLessThan((worktreeRemove as Mock).mock.invocationCallOrder[0]);
    await waitFor(() => expect(screen.queryByRole("region", { name: "独立工作区" })).toBeNull());
  });
});

describe("deleted worktree", () => {
  it("stays in history with a muted badge and cannot be continued", async () => {
    (worktreeList as Mock).mockResolvedValue([]);
    renderApp(<SessionsPage />);
    const demo = await screen.findByRole("region", { name: "demo" });
    const row = await within(demo).findByRole("button", { name: /Fix login/ });
    await waitFor(() => expect(row).toHaveTextContent("工作区已删除"));
    await userEvent.click(row);
    const box = await screen.findByRole("textbox", { name: "消息" });
    expect(box).toBeDisabled();
    expect(screen.getAllByText("这个会话的独立工作区已删除，不能继续对话；记录仍然保留").length).toBeGreaterThan(0);
    expect(screen.queryByRole("region", { name: "独立工作区" })).toBeNull();
  });
});
