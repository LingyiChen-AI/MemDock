import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { LiveEnvelope, LiveEvent, LiveSessionInfo } from "@/api/live";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  listMemories: vi.fn().mockResolvedValue({
    global: [],
    projects: [{ project: { id: "p", path: "/work/demo", name: "demo", exists: true, sources: [] }, files: [] }],
  }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@/api/live", () => ({
  liveStart: vi.fn(),
  liveSend: vi.fn().mockResolvedValue(undefined),
  liveModels: vi.fn().mockResolvedValue([]),
  liveRespond: vi.fn().mockResolvedValue(undefined),
  liveInterrupt: vi.fn().mockResolvedValue(undefined),
  liveStop: vi.fn().mockResolvedValue(undefined),
  liveClose: vi.fn().mockResolvedValue(undefined),
  liveList: vi.fn(),
  liveTranscript: vi.fn(),
  onLiveEvent: vi.fn(),
}));

import { liveList, liveRespond, liveSend, liveStart, liveTranscript, onLiveEvent } from "@/api/live";
import { renderApp } from "@/test/fixtures";
import { selectOption } from "@/test/selectOption";
import { ChatPage } from "./ChatPage";
import { initLiveStore, resetLiveStore } from "./liveStore";

function info(over: Partial<LiveSessionInfo> = {}): LiveSessionInfo {
  return {
    live_id: "L1",
    agent: "claude",
    cwd: "/work/demo",
    title: "Fix the build",
    state: "waiting_approval",
    native_id: "sess-1",
    model: "opus",
    permission: "ask",
    created_at: Date.now() - 60_000,
    updated_at: Date.now(),
    ...over,
  };
}

function envs(events: LiveEvent[], from = 1): LiveEnvelope[] {
  return events.map((event, i) => ({ live_id: "L1", seq: from + i, event }));
}

let emit: (e: LiveEnvelope) => void = () => {};
let stop: () => void = () => {};

beforeEach(() => {
  vi.clearAllMocks();
  resetLiveStore();
  (onLiveEvent as Mock).mockImplementation(async (cb: (e: LiveEnvelope) => void) => {
    emit = cb;
    return () => {};
  });
});

afterEach(() => stop());

function setup(ui = <ChatPage newChatOpen={false} onNewChatOpenChange={() => {}} />) {
  stop = initLiveStore();
  return renderApp(ui);
}

describe("ChatPage", () => {
  it("shows the empty state with a call to action", async () => {
    (liveList as Mock).mockResolvedValue([]);
    const onOpen = vi.fn();
    setup(<ChatPage newChatOpen={false} onNewChatOpenChange={onOpen} />);
    expect(await screen.findByText("开始一段新对话")).toBeInTheDocument();
    await userEvent.click(within(screen.getByRole("main")).getByRole("button", { name: "新对话" }));
    expect(onOpen).toHaveBeenCalledWith(true);
  });

  it("renders the transcript, answers an approval and streams new events", async () => {
    (liveList as Mock).mockResolvedValue([info()]);
    (liveTranscript as Mock).mockResolvedValue(
      envs([
        { type: "started", native_id: "sess-1", model: "opus" },
        { type: "user_message", id: "u1", text: "Fix the build" },
        { type: "tool_started", id: "t1", name: "Bash", kind: "shell", summary: "cargo build", input: "{}" },
        {
          type: "approval_request",
          request_id: "r1",
          tool_name: "Bash",
          kind: "shell",
          summary: "rm -rf target",
          detail: "rm -rf target",
          can_allow_session: true,
        },
      ]),
    );
    setup();
    const card = await screen.findByRole("listitem", { name: "Bash 请求批准" });
    expect(within(card).getByRole("button", { name: "本次会话都允许" })).toBeInTheDocument();
    await userEvent.click(within(card).getByRole("button", { name: "允许" }));
    expect(liveRespond).toHaveBeenCalledWith("L1", "r1", "allow");

    await act(async () => {
      for (const e of envs(
        [
          { type: "approval_resolved", request_id: "r1", decision: "allow" },
          { type: "status", state: "running" },
          { type: "assistant_delta", id: "a1", text: "Build " },
          { type: "assistant_delta", id: "a1", text: "fixed" },
        ],
        5,
      ))
        emit(e);
    });
    expect(within(card).getByText("已允许")).toBeInTheDocument();
    expect(await screen.findByText(/Build fixed/)).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "停止" }).length).toBeGreaterThan(0);

    await act(async () => emit(envs([{ type: "assistant_message", id: "a1", text: "**Done**" }], 9)[0]));
    expect(await screen.findByText("Done")).toBeInTheDocument();
  });

  it("sends on Enter but not on Shift+Enter", async () => {
    (liveList as Mock).mockResolvedValue([info({ state: "idle" })]);
    (liveTranscript as Mock).mockResolvedValue([]);
    setup();
    const box = await screen.findByRole("textbox", { name: "消息" });
    await userEvent.type(box, "hello{Shift>}{Enter}{/Shift}world");
    expect(liveSend).not.toHaveBeenCalled();
    await userEvent.type(box, "{Enter}");
    expect(liveSend).toHaveBeenCalledWith("L1", "hello\nworld", []);
    await waitFor(() => expect(box).toHaveValue(""));
  });

  it("starts a new chat from the dialog with the chosen options", async () => {
    (liveList as Mock).mockResolvedValue([]);
    (liveTranscript as Mock).mockResolvedValue([]);
    (liveStart as Mock).mockResolvedValue(info({ live_id: "L2", state: "starting", agent: "codex", title: "" }));
    const onOpen = vi.fn();
    setup(<ChatPage newChatOpen onNewChatOpenChange={onOpen} />);
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("radio", { name: /Codex/ }));
    await selectOption(within(dialog).getByRole("combobox", { name: "项目目录" }), "/work/demo");
    await selectOption(within(dialog).getByRole("combobox", { name: "权限模式" }), "bypass");
    expect(within(dialog).getByRole("alert")).toHaveTextContent("关闭沙箱");
    await userEvent.click(within(dialog).getByRole("button", { name: "高级选项" }));
    await userEvent.type(within(dialog).getByLabelText("附加参数"), "a=1{Enter}b=2");
    await userEvent.type(within(dialog).getByLabelText("第一条消息（可选）"), "hi");
    await userEvent.click(within(dialog).getByRole("button", { name: "开始对话" }));
    expect(liveStart).toHaveBeenCalledWith({
      agent: "codex",
      cwd: "/work/demo",
      model: null,
      permission: "bypass",
      extra_args: ["a=1", "b=2"],
      prompt: "hi",
    });
    await waitFor(() => expect(onOpen).toHaveBeenCalledWith(false));
    expect(JSON.parse(localStorage.getItem("memdock.chat.prefs")!)).toMatchObject({ agent: "codex", permission: { codex: "bypass" } });
  });
});
