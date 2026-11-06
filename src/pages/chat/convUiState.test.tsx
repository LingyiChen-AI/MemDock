import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@/api/live", async () => {
  const real = await vi.importActual<typeof import("@/api/live")>("@/api/live");
  return {
    storedImagePath: real.storedImagePath,
    readImageDataUri: vi.fn(),
    liveModels: vi.fn().mockResolvedValue([]),
    liveCommands: vi.fn().mockResolvedValue([]),
    projectFiles: vi.fn().mockResolvedValue([]),
  };
});
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: () => Promise.resolve(() => {}) }),
}));

import { readImageDataUri } from "@/api/live";
import { useI18n } from "@/i18n/I18nProvider";
import { renderApp } from "@/test/fixtures";
import { ChatComposer, resetComposerCaches, type ComposerProps } from "./Composer";
import { Conversation, LiveThumbs, resetImageCache } from "./Conversation";
import { getConvUi, moveConvUi, patchConvUi, resetConvUi } from "./convUiState";
import { EMPTY_CONV, type ConvState } from "./liveModel";

function Harness(props: Partial<ComposerProps>) {
  const { t } = useI18n();
  return <ChatComposer c={t.chat} agent="claude" cwd="/work/demo" onSend={vi.fn()} onError={vi.fn()} {...props} />;
}

const box = () => screen.getByRole("textbox", { name: "消息" });

beforeEach(() => {
  resetConvUi();
  resetComposerCaches();
  resetImageCache();
  vi.clearAllMocks();
});

describe("per-conversation composer state", () => {
  it("keeps unsent text and images per conversation and restores them on switch", async () => {
    const a = renderApp(<Harness stateKey="claude:a" />);
    await userEvent.type(box(), "half written");
    a.unmount();

    const b = renderApp(<Harness stateKey="claude:b" />);
    expect(box()).toHaveValue("");
    b.unmount();

    patchConvUi("claude:c", { images: ["data:image/png;base64,AAAA"] });
    const c = renderApp(<Harness stateKey="claude:c" />);
    expect(screen.getByRole("list", { name: "添加图片" })).toBeInTheDocument();
    c.unmount();

    renderApp(<Harness stateKey="claude:a" />);
    expect(box()).toHaveValue("half written");
  });

  it("a send clears the stored text first; a cancelled send (false) keeps it", async () => {
    const onSend = vi.fn().mockResolvedValue(false);
    renderApp(<Harness stateKey="draft:x" draft={{ onAgentChange: vi.fn(), onCwdChange: vi.fn() }} onSend={onSend} />);
    await userEvent.type(box(), "hi{Enter}");
    await waitFor(() => expect(onSend).toHaveBeenCalled());
    expect(box()).toHaveValue("hi");
    expect(getConvUi("draft:x").text).toBe("hi");

    // Moved to the started conversation: the composer there starts empty with the draft's settings.
    onSend.mockImplementation(async () => moveConvUi("draft:x", "claude:n1"));
    await userEvent.type(box(), "{Enter}");
    await waitFor(() => expect(box()).toHaveValue(""));
    expect(getConvUi("claude:n1")).toMatchObject({ text: "", images: [], settings: { permission: "ask", model: "" } });
  });

  it("settings chosen in a started conversation stay with it and do not touch the global prefs", async () => {
    patchConvUi("claude:a", { settings: { permission: "plan" } });
    renderApp(<Harness stateKey="claude:a" />);
    expect(screen.getByLabelText("当前设置")).toHaveTextContent("规划");
    expect(localStorage.getItem("memdock.chat.prefs")).toBeNull();
  });
});

describe("per-conversation scroll position", () => {
  const top = new WeakMap<Element, number>();
  beforeAll(() => {
    Object.defineProperty(HTMLElement.prototype, "scrollHeight", { configurable: true, get: () => 1000 });
    Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 200 });
    Object.defineProperty(HTMLElement.prototype, "scrollTop", {
      configurable: true,
      get() {
        return top.get(this) ?? 0;
      },
      set(v: number) {
        top.set(this, v);
      },
    });
  });
  afterAll(() => {
    for (const p of ["scrollHeight", "clientHeight", "scrollTop"]) delete (HTMLElement.prototype as unknown as Record<string, unknown>)[p];
  });

  const conv: ConvState = { ...EMPTY_CONV, state: "idle", items: [{ type: "user", id: "u1", text: "hi" }] };
  const view = (key: string) => <Conversation scrollKey={key} conv={conv} agent="claude" running={false} starting={false} onRespond={vi.fn()} />;

  it("restores where the user scrolled to; a new conversation starts at the bottom", () => {
    const first = renderApp(view("claude:a"));
    const el = screen.getByTestId("chat-scroll");
    expect(el.scrollTop).toBe(1000);
    el.scrollTop = 120;
    fireEvent.scroll(el);
    expect(getConvUi("claude:a")).toMatchObject({ scrollTop: 120, stuck: false });
    first.unmount();

    const other = renderApp(view("claude:b"));
    expect(screen.getByTestId("chat-scroll").scrollTop).toBe(1000);
    other.unmount();

    renderApp(view("claude:a"));
    expect(screen.getByTestId("chat-scroll").scrollTop).toBe(120);
  });
});

describe("stored image thumbnails", () => {
  it("loads memdock-image: refs through readImageDataUri (once) and keeps data URIs as they are", async () => {
    (readImageDataUri as Mock).mockResolvedValue("data:image/png;base64,QUJD");
    const ui = <LiveThumbs images={["memdock-image:C:/Temp/a.png", "data:image/png;base64,WFla"]} label="loading" />;
    const first = renderApp(ui);
    await waitFor(() => expect(document.querySelectorAll("img")[0]).toHaveAttribute("src", "data:image/png;base64,QUJD"));
    expect(document.querySelectorAll("img")[1]).toHaveAttribute("src", "data:image/png;base64,WFla");
    expect(readImageDataUri).toHaveBeenCalledWith("C:/Temp/a.png");
    first.unmount();
    renderApp(ui);
    await act(async () => {});
    expect(readImageDataUri).toHaveBeenCalledTimes(1);
  });
});
