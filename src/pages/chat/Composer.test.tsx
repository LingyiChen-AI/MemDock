import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { DragDropEvent } from "@tauri-apps/api/webview";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@/api/live", () => ({ readImageDataUri: vi.fn() }));
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
import { readImageDataUri } from "@/api/live";
import { useI18n } from "@/i18n/I18nProvider";
import { renderApp } from "@/test/fixtures";
import { ChatComposer, type ComposerProps } from "./Composer";

function Harness(props: Partial<ComposerProps>) {
  const { t } = useI18n();
  return <ChatComposer c={t.chat} agent="claude" cwd="C:\work\demo" onSend={vi.fn()} onError={vi.fn()} {...props} />;
}

function pngFile(name = "shot.png", bytes = 3) {
  return new File([new Uint8Array(bytes)], name, { type: "image/png" });
}

beforeEach(() => {
  vi.clearAllMocks();
  dropHandler = null;
});

describe("ChatComposer", () => {
  it("sends on Enter, not on Shift+Enter or while an IME is composing", async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    renderApp(<Harness onSend={onSend} />);
    const box = screen.getByRole("textbox", { name: "消息" });
    await userEvent.type(box, "hello{Shift>}{Enter}{/Shift}world");
    fireEvent.keyDown(box, { key: "Enter", keyCode: 229 });
    fireEvent.keyDown(box, { key: "Enter", isComposing: true });
    expect(onSend).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "发送" })).toBeEnabled();
    await userEvent.type(box, "{Enter}");
    expect(onSend).toHaveBeenCalledWith({ text: "hello\nworld", images: [], permission: "ask", model: "" });
    await waitFor(() => expect(box).toHaveValue(""));
  });

  it("keeps send disabled while empty", () => {
    renderApp(<Harness />);
    expect(screen.getByRole("button", { name: "发送" })).toBeDisabled();
  });

  it("attaches pasted images, removes them, and sends images only", async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    renderApp(<Harness onSend={onSend} />);
    const box = screen.getByRole("textbox", { name: "消息" });
    fireEvent.paste(box, { clipboardData: { files: [pngFile("a.png"), pngFile("b.png")] } });
    expect(await screen.findByRole("img", { name: "图片 2" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "移除图片 1" }));
    expect(screen.getAllByRole("img")).toHaveLength(1);
    await userEvent.click(screen.getByRole("button", { name: "发送" }));
    expect(onSend).toHaveBeenCalledWith({ text: "", images: ["data:image/png;base64,AAAA"], permission: "ask", model: "" });
    await waitFor(() => expect(screen.queryByRole("img")).not.toBeInTheDocument());
  });

  it("rejects pasted images over 5 MB", async () => {
    const onError = vi.fn();
    renderApp(<Harness onError={onError} />);
    fireEvent.paste(screen.getByRole("textbox", { name: "消息" }), { clipboardData: { files: [pngFile("big.png", 5 * 1024 * 1024 + 1)] } });
    await waitFor(() => expect(onError).toHaveBeenCalledWith(expect.stringContaining("big.png")));
    expect(screen.queryByRole("img")).not.toBeInTheDocument();
  });

  it("inserts picked file paths at the cursor", async () => {
    (open as Mock).mockResolvedValue(["C:\\work\\demo\\src\\App.tsx", "D:\\My Docs\\spec.md"]);
    renderApp(<Harness />);
    const box = screen.getByRole("textbox", { name: "消息" });
    await userEvent.type(box, "look at");
    await userEvent.click(screen.getByRole("button", { name: "插入文件路径" }));
    await waitFor(() => expect(box).toHaveValue('look at src/App.tsx "D:\\My Docs\\spec.md"'));
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
    expect(screen.getByRole("textbox", { name: "消息" })).toHaveValue("README.md");
  });

  it("shows the running session's settings read-only and offers stop", async () => {
    const onStop = vi.fn();
    renderApp(<Harness running onStop={onStop} attached={{ permission: "bypass", model: "opus" }} />);
    expect(screen.getByRole("combobox", { name: "权限模式" })).toBeDisabled();
    expect(screen.getByRole("combobox", { name: "权限模式" })).toHaveTextContent("跳过所有确认");
    expect(screen.getByRole("button", { name: "模型" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "模型" })).toHaveTextContent("opus");
    await userEvent.click(screen.getByRole("button", { name: "停止" }));
    expect(onStop).toHaveBeenCalled();
  });

  it("remembers the model picked for the next start", async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    renderApp(<Harness onSend={onSend} />);
    await userEvent.click(screen.getByRole("button", { name: "模型" }));
    await userEvent.click(await screen.findByRole("option", { name: "sonnet" }));
    expect(JSON.parse(localStorage.getItem("memdock.chat.prefs")!)).toMatchObject({ model: { claude: "sonnet" } });
    await userEvent.type(screen.getByRole("textbox", { name: "消息" }), "hi{Enter}");
    expect(onSend).toHaveBeenCalledWith({ text: "hi", images: [], permission: "ask", model: "sonnet" });
  });
});
