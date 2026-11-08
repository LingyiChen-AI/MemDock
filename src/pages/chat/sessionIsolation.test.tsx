import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { SessionPreview, SessionSummary } from "@/api/types";
import type { LiveSessionInfo } from "@/api/live";

vi.mock("@/api/env", () => ({
  envDetect: vi.fn().mockResolvedValue({ tools: [{ name: "claude", installed: true }] }),
}));

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
vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn().mockResolvedValue("/home/u") }));

const pty = vi.hoisted(() => ({
  data: null as null | ((d: { pty_id: string; data: string }) => void),
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
  onPtyExit: vi.fn(async () => () => {}),
}));

const xterm = vi.hoisted(() => ({ written: [] as string[], input: null as null | ((d: string) => void) }));
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    loadAddon() {}
    open() {}
    focus() {}
    reset() {}
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

import { homeDir } from "@tauri-apps/api/path";
import { liveList, liveStart, liveStop, nativeOwner } from "@/api/live";
import { ptyClose, ptyOpen } from "@/api/pty";
import { listSessions, sessionPreview } from "@/api/sessions";
import { renderApp } from "@/test/fixtures";
import { SessionsPage } from "@/pages/sessions/SessionsPage";
import { refreshLiveList, resetLiveStore } from "./liveStore";
import { getTerminal, getTerminalBuffer, migrateTerminal, openTerminal, prepareTerminal, resetTerminalStore, terminalKey } from "./terminalStore";
import { DEFAULT_PREFS, saveChatPrefs } from "./chatPrefs";
import { resetConvUi } from "./convUiState";

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
  total_messages: 1,
  stats: { user_messages: 1, assistant_messages: 0, tool_calls: 0, mapped_calls: 0, degraded_calls: 0 },
  timeline: [{ type: "user", ts: "2026-11-01T10:00:00.000Z", text: "Fix the build" }],
  counts: { user: 1, assistant: 0, tools: 0, shell: 0, files: 0, errors: 0 },
  timeline_total: 1,
};

function liveInfo(over: Partial<LiveSessionInfo> = {}): LiveSessionInfo {
  return {
    live_id: "L1",
    agent: "claude",
    cwd: "/work/demo",
    title: "Build fix",
    state: "idle",
    native_id: "c1",
    model: null,
    permission: "ask",
    created_at: 1,
    updated_at: 2,
    ...over,
  };
}

const opts = { tool: "claude" as const, cwd: "/w", resume_id: null, permission: "ask" as const, model: null, cols: 80, rows: 24 };

async function openSession() {
  renderApp(<SessionsPage />);
  const list = await screen.findByRole("region", { name: "demo" });
  await userEvent.click(within(list).getByRole("button", { name: /Build fix/ }));
  return list;
}

beforeEach(() => {
  resetTerminalStore();
  resetLiveStore();
  resetConvUi();
  vi.clearAllMocks();
  localStorage.clear();
  xterm.written = [];
  saveChatPrefs({ ...DEFAULT_PREFS, cwd: "/work/demo" });
  (listSessions as Mock).mockResolvedValue([session]);
  (sessionPreview as Mock).mockResolvedValue(preview);
  (liveList as Mock).mockResolvedValue([]);
  (liveStop as Mock).mockResolvedValue(undefined);
  (nativeOwner as Mock).mockResolvedValue(null);
  (ptyOpen as Mock).mockReset();
});

describe("draft terminals", () => {
  it("each draft gets its own terminal; a touched one stays listed and re-attaches", async () => {
    (homeDir as unknown as { mockResolvedValueOnce: (v: string) => void }).mockResolvedValueOnce("/work/demo"); // no conversation open: new chats start in the home folder
    (ptyOpen as Mock).mockResolvedValueOnce({ pty_id: "P1", command: "claude (A)" }).mockResolvedValueOnce({ pty_id: "P2", command: "claude (B)" });
    renderApp(<SessionsPage />);
    await screen.findByRole("region", { name: "demo" });
    const banner = screen.getByRole("banner");
    await userEvent.click(within(banner).getByRole("button", { name: "新对话" }));
    await userEvent.click(await screen.findByRole("radio", { name: /命令行/ }));
    await waitFor(() => expect(ptyOpen).toHaveBeenCalledTimes(1));
    act(() => pty.data?.({ pty_id: "P1", data: "draft A output" }));
    act(() => xterm.input?.("x"));

    // A second draft: a new process, not draft A's terminal.
    xterm.written = [];
    await userEvent.click(within(banner).getByRole("button", { name: "新对话" }));
    await waitFor(() => expect(ptyOpen).toHaveBeenCalledTimes(2));
    expect(xterm.written.join("")).not.toContain("draft A output");

    // Draft A is still reachable from the list and re-attaches with its scrollback.
    const section = screen.getByRole("region", { name: "新对话" });
    await userEvent.click(within(section).getByRole("button", { name: /claude \(A\)/ }));
    await waitFor(() => expect(xterm.written.join("")).toContain("draft A output"));
    expect(ptyOpen).toHaveBeenCalledTimes(2);
  });

  it("terminal keys never collide between drafts", () => {
    const a = terminalKey({ tool: "claude", nativeId: null, live: null, draftKey: "draft:a" });
    const b = terminalKey({ tool: "claude", nativeId: null, live: null, draftKey: "draft:b" });
    expect(a).not.toBe(b);
    expect(terminalKey({ tool: "claude", nativeId: null, live: liveInfo({ native_id: null }), draftKey: "draft:a" })).toBe("live:L1");
    expect(terminalKey({ tool: "claude", nativeId: "c1", live: null })).toBe("claude:c1");
  });
});

describe("key migration", () => {
  it("moves a running terminal to the new key with its output; no new process", async () => {
    (ptyOpen as Mock).mockResolvedValue({ pty_id: "P1", command: "claude" });
    await openTerminal("draft:a", opts);
    act(() => pty.data?.({ pty_id: "P1", data: "before" }));
    expect(migrateTerminal("draft:a", "claude:n1")).toBe(true);
    expect(getTerminal("draft:a")).toBeUndefined();
    expect(getTerminal("claude:n1")).toMatchObject({ ptyId: "P1", status: "running", key: "claude:n1" });
    act(() => pty.data?.({ pty_id: "P1", data: " after" }));
    expect(getTerminalBuffer("claude:n1")).toBe("before after");
    expect(ptyOpen).toHaveBeenCalledTimes(1);
  });

  it("a live process that learns its native id moves its terminal to the session key", async () => {
    (ptyOpen as Mock).mockResolvedValue({ pty_id: "P1", command: "claude" });
    await openTerminal("live:L1", opts);
    (liveList as Mock).mockResolvedValue([liveInfo()]);
    await refreshLiveList();
    renderApp(<SessionsPage />);
    await screen.findByRole("region", { name: "demo" });
    await waitFor(() => expect(getTerminal("claude:c1")?.ptyId).toBe("P1"));
    expect(getTerminal("live:L1")).toBeUndefined();
  });
});

describe("GUI / terminal handoff", () => {
  it("before a terminal opens, stops another GUI process that owns the session (asking when it replies)", async () => {
    (nativeOwner as Mock).mockResolvedValue({ kind: "live", id: "L5" });
    const confirm = vi.fn().mockResolvedValue(true);
    const ok = await prepareTerminal({ tool: "claude", nativeId: "c1", live: null }, "claude:c1", [liveInfo({ live_id: "L5", state: "running" })], confirm);
    expect(ok).toBe(true);
    expect(confirm).toHaveBeenCalled();
    expect(liveStop).toHaveBeenCalledWith("L5");

    confirm.mockResolvedValue(false);
    (liveStop as Mock).mockClear();
    expect(await prepareTerminal({ tool: "claude", nativeId: "c1", live: null }, "claude:c1", [liveInfo({ live_id: "L5", state: "running" })], confirm)).toBe(false);
    expect(liveStop).not.toHaveBeenCalled();
  });

  it("a terminal we already show under another key is moved instead of opening a second one", async () => {
    (ptyOpen as Mock).mockResolvedValue({ pty_id: "P1", command: "claude" });
    await openTerminal("live:L1", opts);
    (nativeOwner as Mock).mockResolvedValue({ kind: "pty", id: "P1" });
    await prepareTerminal({ tool: "claude", nativeId: "c1", live: null }, "claude:c1", [], vi.fn());
    expect(getTerminal("claude:c1")?.ptyId).toBe("P1");
  });

  it("pty_open refused with busy_live shows a clear message", async () => {
    (ptyOpen as Mock).mockRejectedValue("busy_live:L3");
    await openSession();
    await userEvent.click(await screen.findByRole("radio", { name: /命令行/ }));
    expect(await screen.findByText("这个会话正在聊天视图里运行，先在聊天视图里停止它，再切到命令行")).toBeInTheDocument();
    expect(getTerminal("claude:c1")?.busy).toEqual({ kind: "live", id: "L3" });
  });

  it("sending while a terminal owns the session asks; closing it sends here", async () => {
    (nativeOwner as Mock).mockResolvedValue({ kind: "pty", id: "P7" });
    (liveStart as Mock).mockResolvedValue(liveInfo({ live_id: "L2", state: "starting" }));
    await openSession();
    await userEvent.type(await screen.findByRole("textbox", { name: "消息" }), "go on{Enter}");
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText("会话正在命令行里运行")).toBeInTheDocument();
    expect(liveStart).not.toHaveBeenCalled();
    await userEvent.click(within(dialog).getByRole("button", { name: "关闭命令行并发送" }));
    await waitFor(() => expect(liveStart).toHaveBeenCalledWith(expect.objectContaining({ resume_id: "c1", prompt: "go on" })));
    expect(ptyClose).toHaveBeenCalledWith("P7");
  });

  it("…or switches to the terminal and keeps the text", async () => {
    (nativeOwner as Mock).mockResolvedValue({ kind: "pty", id: "P7" });
    (ptyOpen as Mock).mockResolvedValue({ pty_id: "P7", command: "claude --resume c1" });
    await openSession();
    await userEvent.type(await screen.findByRole("textbox", { name: "消息" }), "go on{Enter}");
    await userEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "切到命令行查看" }));
    expect(await screen.findByText("命令行 · claude --resume c1")).toBeInTheDocument();
    expect(liveStart).not.toHaveBeenCalled();
    expect(ptyClose).not.toHaveBeenCalled();
  });

  it("live_start refused with busy_pty (terminal opened after the check): same choice, then one retry", async () => {
    (liveStart as Mock).mockRejectedValueOnce({ code: "busy", message: "busy_pty:P8" }).mockResolvedValueOnce(liveInfo({ live_id: "L2", state: "starting" }));
    await openSession();
    await userEvent.type(await screen.findByRole("textbox", { name: "消息" }), "go on{Enter}");
    await userEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "关闭命令行并发送" }));
    await waitFor(() => expect(liveStart).toHaveBeenCalledTimes(2));
    expect(ptyClose).toHaveBeenCalledWith("P8");
  });
});

describe("several processes on one session", () => {
  it("lists every running process with a warning badge", async () => {
    (liveList as Mock).mockResolvedValue([
      liveInfo({ live_id: "L1", state: "idle", created_at: 1 }),
      liveInfo({ live_id: "L2", state: "running", created_at: 2 }),
    ]);
    await refreshLiveList();
    renderApp(<SessionsPage />);
    const list = await screen.findByRole("region", { name: "demo" });
    expect(within(list).getByText("同一会话有两个进程")).toBeInTheDocument();
    expect(within(list).getByRole("button", { name: /进程 L1/ })).toBeInTheDocument();
    expect(within(list).getByRole("button", { name: /进程 L2/ })).toBeInTheDocument();
    await userEvent.click(within(list).getByRole("button", { name: /进程 L1/ }));
    expect(within(list).getByRole("button", { name: /进程 L1/ })).toHaveAttribute("aria-current", "true");
  });
});
