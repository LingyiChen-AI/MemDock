import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { DragDropEvent } from "@tauri-apps/api/webview";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@/api/live", () => ({
  readImageDataUri: vi.fn(),
  liveModels: vi.fn(),
  liveCommands: vi.fn(),
  projectFiles: vi.fn(),
}));
let dropHandler: ((e: { payload: DragDropEvent }) => void) | null = null;
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: (cb: (e: { payload: DragDropEvent }) => void) => {
      dropHandler = cb;
      return Promise.resolve(() => {});
    },
  }),
}));

import { open } from "@tauri-apps/plugin-dialog";
import { liveCommands, liveModels, projectFiles, readImageDataUri } from "@/api/live";
import { useI18n } from "@/i18n/I18nProvider";
import { renderApp } from "@/test/fixtures";
import { DEFAULT_PREFS, loadChatPrefs, saveChatPrefs } from "./chatPrefs";
import { ChatComposer, resetComposerCaches, type ComposerProps } from "./Composer";

const MODELS = [
  { value: "default", display_name: "Default (recommended)", description: "Opus 5.5", is_default: true, efforts: ["low", "medium", "high", "max"], default_effort: null },
  { value: "opus", display_name: "Opus 5.5", description: "Complex work", is_default: false, efforts: ["low", "medium", "high", "max"], default_effort: null },
  { value: "sonnet", display_name: "Sonnet 5.5", description: "Efficient", is_default: false, efforts: ["low", "high"], default_effort: null },
];
const COMMANDS = [
  { name: "review", description: "Review the current diff", argument_hint: "[pr]", kind: "command" },
  { name: "fireworks-tech-graph", description: "Draw diagrams", argument_hint: "", kind: "skill" },
];

function Harness(props: Partial<ComposerProps>) {
  const { t } = useI18n();
  return <ChatComposer c={t.chat} agent="claude" cwd="C:\work\demo" onSend={vi.fn()} onError={vi.fn()} {...props} />;
}

function pngFile(name = "shot.png", bytes = 3) {
  return new File([new Uint8Array(bytes)], name, { type: "image/png" });
}

const box = () => screen.getByRole("textbox", { name: "消息" });
const status = () => screen.getByLabelText("当前设置");

beforeEach(() => {
  vi.clearAllMocks();
  resetComposerCaches();
  dropHandler = null;
  (liveModels as Mock).mockResolvedValue(MODELS);
  (liveCommands as Mock).mockResolvedValue(COMMANDS);
  (projectFiles as Mock).mockResolvedValue([]);
});

const sent = (over: object) => ({ text: "", images: [], permission: "ask", model: "", effort: "", args: [], ...over });

describe("ChatComposer", () => {
  it("sends on Enter, not on Shift+Enter or while an IME is composing", async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    renderApp(<Harness onSend={onSend} />);
    await userEvent.type(box(), "hello{Shift>}{Enter}{/Shift}world");
    fireEvent.keyDown(box(), { key: "Enter", keyCode: 229 });
    fireEvent.keyDown(box(), { key: "Enter", isComposing: true });
    expect(onSend).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "发送" })).toBeEnabled();
    await userEvent.type(box(), "{Enter}");
    expect(onSend).toHaveBeenCalledWith(sent({ text: "hello\nworld" }));
    await waitFor(() => expect(box()).toHaveValue(""));
  });

  it("keeps send disabled while empty", () => {
    renderApp(<Harness />);
    expect(screen.getByRole("button", { name: "发送" })).toBeDisabled();
  });

  it("attaches pasted images, removes them, and sends images only", async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    renderApp(<Harness onSend={onSend} />);
    fireEvent.paste(box(), { clipboardData: { files: [pngFile("a.png"), pngFile("b.png")] } });
    expect(await screen.findByRole("img", { name: "图片 2" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "移除图片 1" }));
    expect(screen.getAllByRole("img")).toHaveLength(1);
    await userEvent.click(screen.getByRole("button", { name: "发送" }));
    expect(onSend).toHaveBeenCalledWith(sent({ images: ["data:image/png;base64,AAAA"] }));
    await waitFor(() => expect(screen.queryByRole("img")).not.toBeInTheDocument());
  });

  it("rejects pasted images over 5 MB", async () => {
    const onError = vi.fn();
    renderApp(<Harness onError={onError} />);
    fireEvent.paste(box(), { clipboardData: { files: [pngFile("big.png", 5 * 1024 * 1024 + 1)] } });
    await waitFor(() => expect(onError).toHaveBeenCalledWith(expect.stringContaining("big.png")));
    expect(screen.queryByRole("img")).not.toBeInTheDocument();
  });

  it("inserts picked file paths at the cursor", async () => {
    (open as Mock).mockResolvedValue(["C:\\work\\demo\\src\\App.tsx", "D:\\My Docs\\spec.md"]);
    renderApp(<Harness />);
    await userEvent.type(box(), "look at");
    await userEvent.click(screen.getByRole("button", { name: "插入文件路径" }));
    await waitFor(() => expect(box()).toHaveValue('look at src/App.tsx "D:\\My Docs\\spec.md"'));
  });

  it("dropped images become attachments and other files insert their path", async () => {
    (readImageDataUri as Mock).mockResolvedValue("data:image/png;base64,QUJD");
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue({ left: 0, top: 0, right: 800, bottom: 600, width: 800, height: 600, x: 0, y: 0, toJSON: () => ({}) });
    renderApp(<Harness />);
    await waitFor(() => expect(dropHandler).not.toBeNull());
    const position = { x: 10, y: 10 } as never;
    act(() => dropHandler!({ payload: { type: "over", position } }));
    expect(screen.getByText("松开以添加")).toBeInTheDocument();
    await act(async () => dropHandler!({ payload: { type: "drop", paths: ["C:\\work\\demo\\shot.PNG", "C:\\work\\demo\\README.md"], position } }));
    expect(screen.queryByText("松开以添加")).not.toBeInTheDocument();
    expect(readImageDataUri).toHaveBeenCalledWith("C:\\work\\demo\\shot.PNG");
    expect(await screen.findByRole("img", { name: "图片 1" })).toHaveAttribute("src", "data:image/png;base64,QUJD");
    expect(box()).toHaveValue("README.md");
    vi.restoreAllMocks();
  });

  it("shows the status line: agent, folder, effort, permission (bypass in amber) and args; the model in the card", async () => {
    saveChatPrefs({
      ...DEFAULT_PREFS,
      permission: { claude: "bypass", codex: "ask" },
      model: { claude: "sonnet", codex: "" },
      effort: { claude: "high", codex: "" },
      args: { claude: '--add-dir "D:/my dir"', codex: "" },
    });
    renderApp(<Harness />);
    const line = status();
    expect(within(line).getByText("Claude Code")).toBeInTheDocument();
    expect(within(line).getByText("demo")).toBeInTheDocument();
    expect(within(line).getByText("思考 high")).toBeInTheDocument();
    expect(within(line).getByRole("button", { name: /权限模式/ })).toHaveTextContent("跳过所有确认");
    expect(within(line).getByRole("button", { name: /权限模式/ }).className).toMatch(/amber/);
    expect(within(line).getByText("参数 2")).toBeInTheDocument();
    // The model is shown once, inside the card, with its display name.
    expect(await screen.findByRole("button", { name: "模型" })).toHaveTextContent("Sonnet 5.5");
    expect(within(line).queryByText("Sonnet 5.5")).toBeNull();
    // Agent and folder are fixed outside a draft.
    expect(within(line).queryByRole("button", { name: /智能体/ })).toBeNull();
  });

  it("marks changes made while attached to a running process as next time, and offers stop", async () => {
    const onStop = vi.fn();
    renderApp(<Harness running onStop={onStop} attached={{ permission: "bypass", model: "opus" }} />);
    expect(await screen.findByRole("button", { name: "模型" })).toHaveTextContent("Opus 5.5");
    const perm = within(status()).getByRole("button", { name: /权限模式/ });
    expect(perm).toHaveTextContent("跳过所有确认");
    expect(screen.queryByText("下次生效")).toBeNull();
    await userEvent.click(perm);
    await userEvent.click(await screen.findByRole("option", { name: /只读规划/ }));
    expect(within(status()).getByRole("button", { name: /权限模式/ })).toHaveTextContent("只读规划下次生效");
    expect(loadChatPrefs().permission.claude).toBe("plan");
    await userEvent.click(screen.getByRole("button", { name: "停止" }));
    expect(onStop).toHaveBeenCalled();
  });

  it("picks the model from the label in the card and remembers it", async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    renderApp(<Harness onSend={onSend} />);
    await userEvent.click(screen.getByRole("button", { name: "模型" }));
    await userEvent.click(await screen.findByRole("option", { name: /Sonnet 5.5/ }));
    expect(loadChatPrefs().model.claude).toBe("sonnet");
    expect(screen.getByRole("button", { name: "模型" })).toHaveTextContent("Sonnet 5.5");
    await userEvent.type(box(), "hi{Enter}");
    expect(onSend).toHaveBeenCalledWith(sent({ text: "hi", model: "sonnet" }));
  });

  it("/ menu: settings first with their values, then CLI commands; filters and navigates by keyboard", async () => {
    renderApp(<Harness />);
    await userEvent.type(box(), "/");
    const menu = await screen.findByRole("listbox", { name: "命令" });
    await within(menu).findByText("/fireworks-tech-graph");
    const names = within(menu).getAllByRole("option").map((o) => o.textContent);
    expect(names[0]).toMatch(/^\/model切换模型默认 · Opus 5\.5$/);
    expect(names.slice(0, 4).map((n) => n!.split(/[^/a-z-]/)[0])).toEqual(["/model", "/effort", "/permissions", "/args"]);
    expect(within(menu).queryByText("/agent")).toBeNull();
    expect(within(menu).getByText("每次询问")).toBeInTheDocument();
    expect(liveCommands).toHaveBeenCalledWith("claude");

    // ↓ moves the selection; Esc closes.
    expect(within(menu).getAllByRole("option")[0]).toHaveAttribute("aria-selected", "true");
    await userEvent.keyboard("{ArrowDown}");
    expect(within(menu).getAllByRole("option")[1]).toHaveAttribute("aria-selected", "true");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("listbox", { name: "命令" })).toBeNull();

    // Typing filters.
    await userEvent.type(box(), "f");
    const filtered = await screen.findByRole("listbox", { name: "命令" });
    expect(within(filtered).getAllByRole("option").map((o) => o.textContent!.split(/[^/a-z-]/)[0])).toEqual(["/effort", "/fireworks-tech-graph"]);
  });

  it("picking a setting command removes its text, opens the picker and saves the value", async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    renderApp(<Harness onSend={onSend} />);
    await userEvent.type(box(), "/eff");
    await screen.findByRole("listbox", { name: "命令" });
    await userEvent.keyboard("{Enter}");
    expect(box()).toHaveValue("");
    const picker = await screen.findByRole("listbox", { name: "思考强度" });
    // Levels of the default model, plus the CLI default.
    expect(within(picker).getAllByRole("option").map((o) => o.textContent)).toEqual(["默认", "low", "medium", "high", "max"]);
    await userEvent.click(within(picker).getByRole("option", { name: "high" }));
    expect(screen.queryByRole("listbox", { name: "思考强度" })).toBeNull();
    expect(loadChatPrefs().effort.claude).toBe("high");
    expect(within(status()).getByText("思考 high")).toBeInTheDocument();
    await userEvent.type(box(), "go{Enter}");
    expect(onSend).toHaveBeenCalledWith(sent({ text: "go", effort: "high" }));
  });

  it("/args saves extra start arguments that the send passes split", async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    renderApp(<Harness onSend={onSend} />);
    await userEvent.type(box(), "/args");
    await screen.findByRole("listbox", { name: "命令" });
    await userEvent.keyboard("{Tab}");
    const field = await screen.findByLabelText("附加启动参数");
    await userEvent.type(field, '--add-dir "D:/my dir" --verbose');
    await userEvent.click(screen.getByRole("button", { name: "保存" }));
    expect(loadChatPrefs().args.claude).toBe('--add-dir "D:/my dir" --verbose');
    await userEvent.type(box(), "x{Enter}");
    expect(onSend).toHaveBeenCalledWith(sent({ text: "x", args: ["--add-dir", "D:/my dir", "--verbose"] }));
  });

  it("picking a CLI command or skill inserts it into the message", async () => {
    renderApp(<Harness />);
    await userEvent.type(box(), "/rev");
    const menu = await screen.findByRole("listbox", { name: "命令" });
    await within(menu).findByText("/review");
    await userEvent.keyboard("{Enter}");
    expect(box()).toHaveValue("/review ");
    expect(screen.queryByRole("listbox", { name: "命令" })).toBeNull();
  });

  it("offers /agent and /cwd only in a draft, and reports the picks", async () => {
    const onAgentChange = vi.fn();
    const onCwdChange = vi.fn();
    renderApp(<Harness cwd="" draft={{ onAgentChange, onCwdChange }} projects={[{ path: "/work/api", name: "api" }]} />);
    expect(screen.getByText("先选择工作目录：输入 /cwd，或点下方的目录")).toBeInTheDocument();
    await userEvent.type(box(), "/");
    const menu = await screen.findByRole("listbox", { name: "命令" });
    expect(within(menu).getByText("/agent")).toBeInTheDocument();
    await userEvent.click(within(menu).getByText("/agent"));
    await userEvent.click(await screen.findByRole("option", { name: /Codex/ }));
    expect(onAgentChange).toHaveBeenCalledWith("codex");
    expect(loadChatPrefs().agent).toBe("codex");

    await userEvent.click(within(status()).getByRole("button", { name: /工作目录/ }));
    await userEvent.click(await screen.findByRole("option", { name: /api/ }));
    expect(onCwdChange).toHaveBeenCalledWith("/work/api");
    expect(loadChatPrefs().cwd).toBe("/work/api");
  });

  it("@ lists project files and Enter inserts the relative path (quoted with spaces)", async () => {
    (projectFiles as Mock).mockResolvedValue([
      { path: "src/App.tsx", is_dir: false },
      { path: "docs/my notes.md", is_dir: false },
    ]);
    renderApp(<Harness />);
    await userEvent.type(box(), "see @no");
    const menu = await screen.findByRole("listbox", { name: "文件" });
    await within(menu).findByText("docs/my notes.md");
    expect(projectFiles).toHaveBeenLastCalledWith("C:\\work\\demo", "no");
    await userEvent.keyboard("{ArrowDown}{Enter}");
    expect(box()).toHaveValue('see "docs/my notes.md" ');
  });
});
