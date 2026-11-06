import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { SessionPreview, SessionSummary } from "@/api/types";
import type { LiveSessionInfo } from "@/api/live";

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
vi.mock("@/api/live", () => ({
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
}));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn().mockResolvedValue("/home/u") }));

// Captured pty event callbacks, to play backend output.
const pty = vi.hoisted(() => ({
  data: null as null | ((d: { pty_id: string; data: string }) => void),
  exit: null as null | ((d: { pty_id: string; code: number | null }) => void),
}));
vi.mock("@/api/pty", () => ({
  ptyOpen: vi.fn(),
  ptyWrite: vi.fn().mockResolvedValue(undefined),
  ptyResize: vi.fn().mockResolvedValue(undefined),
  ptyClose: vi.fn().mockResolvedValue(undefined),
  saveTempImage: vi.fn(),
  onPtyData: vi.fn(async (cb: (d: { pty_id: string; data: string }) => void) => {
    pty.data = cb;
    return () => {};
  }),
  onPtyExit: vi.fn(async (cb: (d: { pty_id: string; code: number | null }) => void) => {
    pty.exit = cb;
    return () => {};
  }),
}));

// xterm needs a real canvas / layout: a recording stand-in.
const xterm = vi.hoisted(() => ({ written: [] as string[], input: null as null | ((d: string) => void) }));
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    loadAddon() {}
    open() {}
    focus() {}
    reset() {
      xterm.written = [];
    }
    write(d: string) {
      xterm.written.push(d);
    }
    onData(cb: (d: string) => void) {
      xterm.input = cb;
      return { dispose() {} };
    }
    dispose() {}
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));
vi.mock("@xterm/addon-web-links", () => ({ WebLinksAddon: class {} }));
vi.mock("@xterm/xterm/css/xterm.css", () => ({}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

import { liveList, liveStart, liveStop } from "@/api/live";
import { ptyClose, ptyOpen, ptyWrite, saveTempImage } from "@/api/pty";
import { listSessions, sessionPreview } from "@/api/sessions";
import { renderApp } from "@/test/fixtures";
import { SessionsPage } from "@/pages/sessions/SessionsPage";
import { refreshLiveList, resetLiveStore } from "./liveStore";

import { appendCapped, getShells, getTerminal, loadChatMode, resetTerminalStore, writeTerminal } from "./terminalStore";
import { saveChatPrefs, loadChatPrefs, DEFAULT_PREFS } from "./chatPrefs";

const session: SessionSummary = {
  id: "c1",
  tool: "claude",
  path: "/home/u/.claude/projects/-work-demo/c1.jsonl",
  cwd: "/work/demo",
  project_id: "/work/demo",
  project_name: "demo",
  title: "Build fix",
  first_prompt: "Fix the build",
  continued: false,
  message_count: 2,
  tool_calls: 0,
  started_at: 1_700_000_000_000,
  updated_at: 1_700_000_100_000,
  model: null,
  converted_from: null,
  converted_to: [],
};

const preview: SessionPreview = {
  summary: session,
  messages: [],
  total_messages: 2,
  stats: { user_messages: 1, assistant_messages: 1, tool_calls: 0, mapped_calls: 0, degraded_calls: 0 },
  timeline: [
    { type: "user", ts: "2026-11-01T10:00:00.000Z", text: "Fix the build" },
    { type: "assistant", ts: "2026-11-01T10:00:03.000Z", text: "Fixed." },
  ],
  counts: { user: 1, assistant: 1, tools: 0, shell: 0, files: 0, errors: 0 },
  timeline_total: 2,
};

function liveInfo(state: LiveSessionInfo["state"]): LiveSessionInfo {
  return {
    live_id: "L1",
    agent: "claude",
    cwd: "/work/demo",
    title: "Build fix",
    state,
    native_id: "c1",
    model: null,
    permission: "ask",
    created_at: 1,
    updated_at: 2,
  };
}

async function openSession() {
  renderApp(<SessionsPage />);
  const list = await screen.findByRole("region", { name: "demo" });
  await userEvent.click(within(list).getByRole("button", { name: /Build fix/ }));
  await screen.findByRole("radio", { name: /终端/ });
}

describe("terminal mode", () => {
  beforeEach(() => {
    resetTerminalStore();
    resetLiveStore();
    xterm.written = [];
    (listSessions as Mock).mockResolvedValue([session]);
    (sessionPreview as Mock).mockResolvedValue(preview);
    (liveList as Mock).mockResolvedValue([]);
    (liveStop as Mock).mockResolvedValue(undefined);
    (ptyOpen as Mock).mockResolvedValue({ pty_id: "P1", command: "claude --resume c1" });
    (ptyOpen as Mock).mockClear();
    (ptyClose as Mock).mockClear();
    (ptyWrite as Mock).mockClear();
    (liveStop as Mock).mockClear();
  });

  it("switches to the terminal resuming the session, and back to the GUI with a fresh preview", async () => {
    saveChatPrefs({ ...DEFAULT_PREFS, permission: { claude: "auto_edit", codex: "ask" }, model: { claude: "opus", codex: "" } });
    await openSession();
    await screen.findByText("Fixed.");
    expect(screen.getByRole("radio", { name: /聊天/ })).toHaveAttribute("aria-checked", "true");

    await userEvent.click(screen.getByRole("radio", { name: /终端/ }));
    await waitFor(() =>
      expect(ptyOpen).toHaveBeenCalledWith({
        tool: "claude",
        cwd: "/work/demo",
        resume_id: "c1",
        permission: "auto_edit",
        model: "opus",
        effort: null,
        cols: 80,
        rows: 24,
      }),
    );
    expect(await screen.findByText("终端 · claude --resume c1")).toBeInTheDocument();
    // The GUI composer is gone, the choice is remembered.
    expect(screen.queryByText("Fixed.")).toBeNull();
    expect(loadChatMode()).toBe("terminal");

    // Output reaches xterm, typing reaches the pty.
    act(() => pty.data?.({ pty_id: "P1", data: "hello 终端" }));
    expect(xterm.written.join("")).toContain("hello 终端");
    act(() => xterm.input?.("hi\r"));
    expect(ptyWrite).toHaveBeenCalledWith("P1", "hi\r");

    const previews = (sessionPreview as Mock).mock.calls.length;
    await userEvent.click(screen.getByRole("radio", { name: /聊天/ }));
    await waitFor(() => expect(ptyClose).toHaveBeenCalledWith("P1"));
    await waitFor(() => expect((sessionPreview as Mock).mock.calls.length).toBe(previews + 1));
    expect(await screen.findByText("Fixed.")).toBeInTheDocument();
    expect(loadChatMode()).toBe("gui");
  });

  it("asks before stopping a GUI process that is replying, and stays in the GUI when cancelled", async () => {
    (liveList as Mock).mockResolvedValue([liveInfo("running")]);
    await refreshLiveList();
    await openSession();
    await userEvent.click(screen.getByRole("radio", { name: /终端/ }));
    const dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(liveStop).not.toHaveBeenCalled();
    expect(ptyOpen).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("radio", { name: /终端/ }));
    await userEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "停止并切换" }));
    await waitFor(() => expect(liveStop).toHaveBeenCalledWith("L1"));
    await waitFor(() => expect(ptyOpen).toHaveBeenCalled());
  });

  it("stops an idle GUI process without asking", async () => {
    (liveList as Mock).mockResolvedValue([liveInfo("idle")]);
    await refreshLiveList();
    await openSession();
    await userEvent.click(screen.getByRole("radio", { name: /终端/ }));
    await waitFor(() => expect(liveStop).toHaveBeenCalledWith("L1"));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    await waitFor(() => expect(ptyOpen).toHaveBeenCalled());
  });

  it("opens conversations in the remembered mode and re-attaches with the scrollback", async () => {
    localStorage.setItem("memdock.chat.mode", "terminal");
    const first = renderApp(<SessionsPage />);
    await userEvent.click(await screen.findByRole("button", { name: /Build fix/ }));
    await waitFor(() => expect(ptyOpen).toHaveBeenCalledTimes(1));
    act(() => pty.data?.({ pty_id: "P1", data: "earlier output" }));
    writeTerminal("claude:c1", "typed");
    first.unmount();
    xterm.written = [];

    renderApp(<SessionsPage />);
    await userEvent.click(await screen.findByRole("button", { name: /Build fix/ }));
    await waitFor(() => expect(xterm.written.join("")).toContain("earlier output"));
    expect(ptyOpen).toHaveBeenCalledTimes(1);
    expect(getTerminal("claude:c1")?.status).toBe("running");
  });

  it("shows the exit and restarts", async () => {
    localStorage.setItem("memdock.chat.mode", "terminal");
    await (async () => {
      renderApp(<SessionsPage />);
      await userEvent.click(await screen.findByRole("button", { name: /Build fix/ }));
    })();
    await waitFor(() => expect(ptyOpen).toHaveBeenCalledTimes(1));
    act(() => pty.exit?.({ pty_id: "P1", code: 0 }));
    expect(await screen.findByText("已退出（0）")).toBeInTheDocument();
    (ptyOpen as Mock).mockResolvedValue({ pty_id: "P2", command: "claude --resume c1" });
    await userEvent.click(screen.getByRole("button", { name: /重新启动/ }));
    await waitFor(() => expect(ptyOpen).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(getTerminal("claude:c1")?.ptyId).toBe("P2"));
  });
});

describe("new chats and terminals", () => {
  beforeEach(() => {
    resetTerminalStore();
    resetLiveStore();
    xterm.written = [];
    (listSessions as Mock).mockResolvedValue([session]);
    (sessionPreview as Mock).mockResolvedValue(preview);
    (liveList as Mock).mockResolvedValue([]);
    (ptyOpen as Mock).mockReset().mockResolvedValue({ pty_id: "S1", command: "PowerShell" });
    (ptyClose as Mock).mockClear();
    (ptyWrite as Mock).mockClear();
    (liveStart as Mock).mockReset();
  });

  it("新对话 opens a draft with the last agent and folder; the first send starts it with effort and args", async () => {
    saveChatPrefs({ ...DEFAULT_PREFS, agent: "codex", cwd: "/work/demo", effort: { claude: "", codex: "high" }, args: { claude: "", codex: "a=1 b=2" } });
    (liveStart as Mock).mockResolvedValue({ ...liveInfo("starting"), live_id: "L9", agent: "codex", native_id: null });
    renderApp(<SessionsPage />);
    await screen.findByRole("region", { name: "demo" });
    await userEvent.click(within(screen.getByRole("banner")).getByRole("button", { name: "新对话" }));
    const status = await screen.findByLabelText("当前设置");
    expect(within(status).getByText("Codex")).toBeInTheDocument();
    expect(within(status).getByText("demo")).toBeInTheDocument();
    await userEvent.type(screen.getByRole("textbox", { name: "消息" }), "hi{Enter}");
    await waitFor(() =>
      expect(liveStart).toHaveBeenCalledWith({
        extra_args: ["a=1", "b=2"],
        agent: "codex",
        cwd: "/work/demo",
        resume_id: null,
        permission: "ask",
        model: null,
        effort: "high",
        prompt: "hi",
        prompt_images: [],
      }),
    );
    expect(loadChatPrefs()).toMatchObject({ agent: "codex", cwd: "/work/demo" });
  });

  it("the project header opens a draft in that folder and a shell terminal at the top of the group", async () => {
    saveChatPrefs({ ...DEFAULT_PREFS, cwd: "" });
    renderApp(<SessionsPage />);
    const group = await screen.findByRole("region", { name: "demo" });
    await projectMenu(group, /^新建会话/);
    expect(within(await screen.findByLabelText("当前设置")).getByText("demo")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /新对话/ })).toBeInTheDocument();

    await projectMenu(group, /^新建终端/);
    await waitFor(() => expect(ptyOpen).toHaveBeenCalledWith(expect.objectContaining({ tool: "shell", cwd: "/work/demo" })));
    expect(screen.queryByRole("radiogroup", { name: "视图" })).toBeNull();
    expect(screen.queryByRole("textbox", { name: "消息" })).toBeNull();
    const rows = within(group).getAllByRole("listitem");
    expect(rows[0]).toHaveTextContent("终端");
    expect(rows[0]).toHaveTextContent("PowerShell");
    expect(rows[0]).toHaveTextContent("运行中");

    // Leaving it does not kill it; closing does and removes the row.
    await userEvent.click(within(group).getByRole("button", { name: /Build fix/ }));
    await new Promise((r) => setTimeout(r, 1700));
    expect(ptyClose).not.toHaveBeenCalled();
    await userEvent.click(within(rows[0]).getByRole("button", { name: "关闭终端" }));
    await waitFor(() => expect(ptyClose).toHaveBeenCalledWith("S1"));
    expect(getShells()).toEqual([]);
  });

  it("新建终端 in the header opens a shell in the home folder under a 终端 group", async () => {
    renderApp(<SessionsPage />);
    await screen.findByRole("region", { name: "demo" });
    await userEvent.click(within(screen.getByRole("banner")).getByRole("button", { name: "新建终端" }));
    await waitFor(() => expect(ptyOpen).toHaveBeenCalledWith(expect.objectContaining({ tool: "shell", cwd: "/home/u" })));
    expect(within(screen.getByRole("region", { name: "终端" })).getByText("/home/u")).toBeInTheDocument();
  });

  it("pasting a screenshot into a terminal saves it and types the quoted path", async () => {
    (saveTempImage as Mock).mockResolvedValue("C:\\Temp\\memdock images\\a.png");
    renderApp(<SessionsPage />);
    await projectMenu(await screen.findByRole("region", { name: "demo" }), /^新建终端/);
    await waitFor(() => expect(getTerminal(getShells()[0].key)?.status).toBe("running"));
    const host = screen.getByTestId("terminal-host");
    fireEvent.paste(host, { clipboardData: { files: [new File([new Uint8Array(3)], "s.png", { type: "image/png" })] } });
    await waitFor(() => expect(ptyWrite).toHaveBeenCalledWith("S1", '"C:\\Temp\\memdock images\\a.png"'));
    expect(saveTempImage).toHaveBeenCalledWith("data:image/png;base64,AAAA");
    expect(await screen.findByText("已把截图保存为文件，并把路径填进了命令行")).toBeInTheDocument();

    // Text paste is left to xterm.
    (ptyWrite as Mock).mockClear();
    fireEvent.paste(host, { clipboardData: { files: [], getData: () => "ls" } });
    expect(saveTempImage).toHaveBeenCalledTimes(1);
  });
});

/** The project group's "＋" menu. */
async function projectMenu(group: HTMLElement, item: RegExp) {
  await userEvent.click(within(group).getByRole("button", { name: "新建" }));
  await userEvent.click(await screen.findByRole("menuitem", { name: item }));
}

describe("appendCapped", () => {
  it("keeps the tail, cut at a line start", () => {
    expect(appendCapped("abc", "def", 10)).toBe("abcdef");
    expect(appendCapped("line1\nline2\n", "line3", 12)).toBe("line2\nline3");
  });
});
