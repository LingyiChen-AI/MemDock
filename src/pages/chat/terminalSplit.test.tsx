import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";

vi.mock("@/api/live", () => ({
  liveStop: vi.fn(),
  nativeOwner: vi.fn(),
  parseBusy: vi.fn(() => null),
}));

const pty = vi.hoisted(() => ({
  data: null as null | ((d: { pty_id: string; data: string }) => void),
  exit: null as null | ((d: { pty_id: string; code: number | null }) => void),
  seq: 0,
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

interface FakeTerm {
  written: string[];
  input: null | ((d: string) => void);
  selection: string;
  focused: number;
  disposed: boolean;
}
const xterm = vi.hoisted(() => ({ terms: [] as FakeTerm[] }));
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    s: FakeTerm = { written: [], input: null, selection: "", focused: 0, disposed: false };
    constructor() {
      xterm.terms.push(this.s);
    }
    loadAddon() {}
    open() {}
    focus() {
      this.s.focused += 1;
    }
    reset() {
      this.s.written = [];
    }
    write(d: string) {
      this.s.written.push(d);
    }
    paste(d: string) {
      this.s.input?.(d);
    }
    getSelection() {
      return this.s.selection;
    }
    clearSelection() {
      this.s.selection = "";
    }
    onData(cb: (d: string) => void) {
      this.s.input = cb;
      return { dispose() {} };
    }
    dispose() {
      this.s.disposed = true;
    }
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));
vi.mock("@xterm/addon-web-links", () => ({ WebLinksAddon: class {} }));
vi.mock("@xterm/xterm/css/xterm.css", () => ({}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

import { ptyClose, ptyOpen, ptyWrite, type PtyOpenOptions } from "@/api/pty";
import { getMessages } from "@/i18n";
import { DEFAULT_PREFS, saveChatPrefs } from "./chatPrefs";
import { TerminalView } from "./TerminalView";
import {
  addShell,
  getLayout,
  getShells,
  getTerminal,
  getTerminalBuffer,
  leaves,
  migrateTerminal,
  neighborPane,
  openTerminal,
  resetTerminalStore,
  shellOptions,
  splitPane,
  writeTerminal,
} from "./terminalStore";

const c = getMessages("zh").chat;
const KEY = "claude:c1";
const cli: PtyOpenOptions = { tool: "claude", cwd: "/work/demo", resume_id: "c1", permission: "ask", model: null, effort: null, cols: 100, rows: 30 };
const clipboard = { writeText: vi.fn().mockResolvedValue(undefined), readText: vi.fn() };

function view(key = KEY, opts = cli) {
  return render(<TerminalView termKey={key} opts={opts} c={c} />);
}

const panes = () => screen.getAllByRole("group", { name: /^分屏 \d$/ });
const hosts = () => screen.getAllByTestId("terminal-host");
const focusedPane = () => panes().find((p) => p.dataset.focused === "true")?.dataset.pane;

async function running(key: string) {
  await waitFor(() => expect(getTerminal(key)?.status).toBe("running"));
}

async function splitRight() {
  await userEvent.click(screen.getByRole("button", { name: "向右分屏" }));
}

describe("terminal split panes", () => {
  beforeEach(() => {
    resetTerminalStore();
    xterm.terms = [];
    pty.seq = 0;
    (ptyOpen as Mock).mockReset().mockImplementation(async (o: PtyOpenOptions) => {
      pty.seq += 1;
      return { pty_id: `P${pty.seq}`, command: o.tool === "shell" ? "PowerShell" : `${o.tool} cli` };
    });
    (ptyClose as Mock).mockClear();
    (ptyWrite as Mock).mockClear();
    clipboard.writeText.mockClear();
    clipboard.readText.mockReset();
    Object.defineProperty(navigator, "clipboard", { value: clipboard, configurable: true });
  });

  it("split right opens a shell in the same folder beside the focused pane, which takes the focus", async () => {
    view();
    await running(KEY);
    expect(screen.queryAllByRole("group", { name: /^分屏 \d$/ })).toHaveLength(0 + 1);
    await splitRight();
    await waitFor(() => expect(ptyOpen).toHaveBeenCalledTimes(2));
    expect((ptyOpen as Mock).mock.calls[1][0]).toMatchObject({ tool: "shell", cwd: "/work/demo", resume_id: null });
    const sub = `${KEY}#pane-2`;
    await running(sub);
    expect(getLayout(KEY).tree).toMatchObject({ type: "split", dir: "row", a: { pane: KEY }, b: { pane: sub } });
    expect(panes()).toHaveLength(2);
    expect(focusedPane()).toBe(sub);
    // Each pane has a header with its index and command.
    expect(within(panes()[0]).getByText("1 · claude cli")).toBeInTheDocument();
    expect(within(panes()[1]).getByText("2 · PowerShell")).toBeInTheDocument();
    expect(panes()[1]).toHaveStyle({ left: "50%", width: "50%" });
    // Typing in the new pane reaches its own pty.
    act(() => xterm.terms[1].input?.("dir\r"));
    expect(ptyWrite).toHaveBeenCalledWith("P2", "dir\r");
  });

  it("Alt+Shift+- splits down; the tree lays the panes out", async () => {
    view();
    await running(KEY);
    await splitRight();
    await running(`${KEY}#pane-2`);
    fireEvent.keyDown(hosts()[1], { key: "_", code: "Minus", altKey: true, shiftKey: true });
    await running(`${KEY}#pane-3`);
    expect((ptyOpen as Mock).mock.calls[2][0]).toMatchObject({ tool: "shell", cwd: "/work/demo" });
    expect(getLayout(KEY).tree).toMatchObject({
      type: "split",
      dir: "row",
      a: { pane: KEY },
      b: { type: "split", dir: "column", a: { pane: `${KEY}#pane-2` }, b: { pane: `${KEY}#pane-3` } },
    });
    const [p1, p2, p3] = panes();
    expect(p1).toHaveStyle({ left: "0%", top: "0%", width: "50%", height: "100%" });
    expect(p2).toHaveStyle({ left: "50%", top: "0%", width: "50%", height: "50%" });
    expect(p3).toHaveStyle({ left: "50%", top: "50%", width: "50%", height: "50%" });
    expect(screen.getAllByRole("separator", { name: c.terminalDivider })).toHaveLength(2);
    // Alt+Shift+= splits right.
    fireEvent.keyDown(hosts()[2], { key: "+", code: "Equal", altKey: true, shiftKey: true });
    await running(`${KEY}#pane-4`);
    expect(panes()).toHaveLength(4);
  });

  it("double-clicking a divider splits evenly again", async () => {
    view();
    await running(KEY);
    await splitRight();
    const { setSplitRatio } = await import("./terminalStore");
    const tree = getLayout(KEY).tree;
    act(() => setSplitRatio(KEY, tree.type === "split" ? tree.id : "", 0.3));
    expect(panes()[0]).toHaveStyle({ width: "30%" });
    fireEvent.doubleClick(screen.getByRole("separator", { name: c.terminalDivider }));
    expect(panes()[0]).toHaveStyle({ width: "50%" });
  });

  it("Ctrl+Shift+W closes the focused pane: only its pty is killed and the tree collapses", async () => {
    view();
    await running(KEY);
    await splitRight();
    await running(`${KEY}#pane-2`);
    fireEvent.keyDown(hosts()[1], { key: "W", code: "KeyW", ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(ptyClose).toHaveBeenCalledWith("P2"));
    expect(ptyClose).toHaveBeenCalledTimes(1);
    expect(getLayout(KEY).tree).toEqual({ type: "leaf", pane: KEY });
    expect(getTerminal(`${KEY}#pane-2`)).toBeUndefined();
    expect(getTerminal(KEY)?.status).toBe("running");
    expect(screen.queryByText(/^1 · /)).toBeNull();
  });

  it("closing the first pane hands its place to the next one", async () => {
    view();
    await running(KEY);
    await splitRight();
    await running(`${KEY}#pane-2`);
    act(() => pty.data?.({ pty_id: "P2", data: "shell output" }));
    await userEvent.click(screen.getByRole("button", { name: "分屏 1 · 关闭分屏" }));
    await waitFor(() => expect(ptyClose).toHaveBeenCalledWith("P1"));
    expect(getLayout(KEY).tree).toEqual({ type: "leaf", pane: KEY });
    expect(getTerminal(KEY)?.ptyId).toBe("P2");
    expect(getTerminalBuffer(KEY)).toContain("shell output");
    await waitFor(() => expect(xterm.terms[xterm.terms.length - 1]?.written.join("")).toContain("shell output"));
    writeTerminal(KEY, "x");
    expect(ptyWrite).toHaveBeenCalledWith("P2", "x");
  });

  it("closing the last pane of a shell removes the shell", async () => {
    const key = addShell("/work/demo");
    view(key, shellOptions("/work/demo"));
    await running(key);
    await userEvent.click(screen.getByRole("button", { name: "关闭当前分屏" }));
    await waitFor(() => expect(ptyClose).toHaveBeenCalledWith("P1"));
    expect(getShells()).toEqual([]);
  });

  it("closing the last pane of a conversation closes its terminal, which can be restarted", async () => {
    view();
    await running(KEY);
    fireEvent.keyDown(hosts()[0], { key: "W", code: "KeyW", ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(ptyClose).toHaveBeenCalledWith("P1"));
    expect(getTerminal(KEY)).toBeUndefined();
    await userEvent.click(await screen.findByRole("button", { name: /重新启动/ }));
    await running(KEY);
    expect((ptyOpen as Mock).mock.calls[1][0]).toMatchObject({ tool: "claude", resume_id: "c1" });
  });

  it("an onClose prop replaces the default close", async () => {
    const onClose = vi.fn();
    render(<TerminalView termKey={KEY} opts={cli} c={c} onClose={onClose} />);
    await running(KEY);
    await userEvent.click(screen.getByRole("button", { name: "关闭当前分屏" }));
    expect(onClose).toHaveBeenCalled();
  });

  it("Alt+Arrow moves the focus between panes", async () => {
    view();
    await running(KEY);
    await splitRight();
    await running(`${KEY}#pane-2`);
    expect(focusedPane()).toBe(`${KEY}#pane-2`);
    const before = xterm.terms[0].focused;
    fireEvent.keyDown(hosts()[1], { key: "ArrowLeft", altKey: true });
    expect(focusedPane()).toBe(KEY);
    expect(xterm.terms[0].focused).toBeGreaterThan(before);
    // Nothing further left: stays.
    fireEvent.keyDown(hosts()[0], { key: "ArrowLeft", altKey: true });
    expect(focusedPane()).toBe(KEY);
    fireEvent.keyDown(hosts()[0], { key: "ArrowRight", altKey: true });
    expect(focusedPane()).toBe(`${KEY}#pane-2`);
  });

  it("neighborPane picks the adjacent pane in a direction", () => {
    const tree = {
      type: "split" as const,
      id: "s",
      dir: "row" as const,
      ratio: 0.5,
      a: { type: "leaf" as const, pane: "a" },
      b: { type: "split" as const, id: "t", dir: "column" as const, ratio: 0.5, a: { type: "leaf" as const, pane: "b" }, b: { type: "leaf" as const, pane: "c" } },
    };
    expect(neighborPane(tree, "a", "right")).toBe("b");
    expect(neighborPane(tree, "c", "left")).toBe("a");
    expect(neighborPane(tree, "b", "down")).toBe("c");
    expect(neighborPane(tree, "c", "up")).toBe("b");
    expect(neighborPane(tree, "a", "up")).toBeNull();
  });

  it("copy shortcuts: Ctrl+Shift+C copies, Ctrl+C copies a selection and is the interrupt otherwise", async () => {
    view();
    await running(KEY);
    const host = hosts()[0];
    xterm.terms[0].selection = "picked";
    expect(fireEvent.keyDown(host, { key: "C", code: "KeyC", ctrlKey: true, shiftKey: true })).toBe(false);
    expect(clipboard.writeText).toHaveBeenCalledWith("picked");
    // No selection: Ctrl+C goes on to xterm (not prevented).
    clipboard.writeText.mockClear();
    expect(fireEvent.keyDown(host, { key: "c", code: "KeyC", ctrlKey: true })).toBe(true);
    expect(clipboard.writeText).not.toHaveBeenCalled();
    xterm.terms[0].selection = "again";
    expect(fireEvent.keyDown(host, { key: "c", code: "KeyC", ctrlKey: true })).toBe(false);
    expect(clipboard.writeText).toHaveBeenCalledWith("again");
    expect(xterm.terms[0].selection).toBe("");
  });

  it("the context menu splits, starts Claude / Codex in a new pane, copies, pastes and closes", async () => {
    saveChatPrefs({ ...DEFAULT_PREFS, permission: { claude: "auto_edit", codex: "plan" }, model: { claude: "opus", codex: "gpt-5" } });
    view();
    await running(KEY);
    const menu = async (i: number) => {
      fireEvent.contextMenu(panes()[i], { clientX: 10, clientY: 10 });
      return screen.findByRole("menu", { name: "分屏菜单" });
    };

    await userEvent.click(within(await menu(0)).getByRole("menuitem", { name: /在新分屏运行 Claude Code/ }));
    await running(`${KEY}#pane-2`);
    expect((ptyOpen as Mock).mock.calls[1][0]).toMatchObject({ tool: "claude", cwd: "/work/demo", resume_id: null, permission: "auto_edit", model: "opus" });
    expect(screen.queryByRole("menu")).toBeNull();

    await userEvent.click(within(await menu(1)).getByRole("menuitem", { name: /在新分屏运行 Codex/ }));
    await running(`${KEY}#pane-3`);
    expect((ptyOpen as Mock).mock.calls[2][0]).toMatchObject({ tool: "codex", cwd: "/work/demo", resume_id: null, permission: "plan", model: "gpt-5" });

    await userEvent.click(within(await menu(0)).getByRole("menuitem", { name: /向下分屏/ }));
    await running(`${KEY}#pane-4`);
    expect((ptyOpen as Mock).mock.calls[3][0]).toMatchObject({ tool: "shell", cwd: "/work/demo" });

    xterm.terms[0].selection = "sel";
    await userEvent.click(within(await menu(0)).getByRole("menuitem", { name: /复制/ }));
    expect(clipboard.writeText).toHaveBeenCalledWith("sel");

    clipboard.readText.mockResolvedValue("echo hi");
    await userEvent.click(within(await menu(0)).getByRole("menuitem", { name: /粘贴/ }));
    await waitFor(() => expect(ptyWrite).toHaveBeenCalledWith("P1", "echo hi"));

    await userEvent.click(within(await menu(1)).getByRole("menuitem", { name: /关闭分屏/ }));
    await waitFor(() => expect(ptyClose).toHaveBeenCalledTimes(1));
    expect(panes()).toHaveLength(3);
  });

  it("allows at most 6 panes", async () => {
    view();
    await running(KEY);
    for (let i = 0; i < 5; i += 1) await splitRight();
    await waitFor(() => expect(ptyOpen).toHaveBeenCalledTimes(6));
    expect(panes()).toHaveLength(6);
    expect(screen.getByRole("button", { name: "向右分屏" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "向下分屏" })).toBeDisabled();
    fireEvent.keyDown(hosts()[5], { key: "+", code: "Equal", altKey: true, shiftKey: true });
    expect(await screen.findByText("每个终端最多 6 个分屏")).toBeInTheDocument();
    expect(splitPane(KEY, KEY, "row", shellOptions("/work/demo"))).toBeNull();
    expect(ptyOpen).toHaveBeenCalledTimes(6);
  });

  it("keeps the layout and each pane's output when the view is left and opened again", async () => {
    const first = view();
    await running(KEY);
    await splitRight();
    await running(`${KEY}#pane-2`);
    act(() => {
      pty.data?.({ pty_id: "P1", data: "cli out" });
      pty.data?.({ pty_id: "P2", data: "shell out" });
    });
    first.unmount();
    await new Promise((r) => setTimeout(r, 1700));
    expect(ptyClose).not.toHaveBeenCalled();
    xterm.terms = [];

    view();
    expect(panes()).toHaveLength(2);
    expect(xterm.terms[0].written.join("")).toContain("cli out");
    expect(xterm.terms[1].written.join("")).toContain("shell out");
    expect(focusedPane()).toBe(`${KEY}#pane-2`);
    expect(ptyOpen).toHaveBeenCalledTimes(2);
  });

  it("migrateTerminal moves every pane to the new key", async () => {
    await openTerminal("draft:a", cli);
    const sub = splitPane("draft:a", "draft:a", "column", shellOptions("/work/demo"));
    expect(sub).toBe("draft:a#pane-2");
    await running("draft:a#pane-2");
    act(() => pty.data?.({ pty_id: "P2", data: "two" }));

    expect(migrateTerminal("draft:a", "claude:n1")).toBe(true);
    expect(getTerminal("draft:a")).toBeUndefined();
    expect(getTerminal("draft:a#pane-2")).toBeUndefined();
    expect(leaves(getLayout("claude:n1").tree)).toEqual(["claude:n1", "claude:n1#pane-2"]);
    expect(getLayout("draft:a").tree).toEqual({ type: "leaf", pane: "draft:a" });
    expect(getTerminalBuffer("claude:n1#pane-2")).toBe("two");
    act(() => pty.data?.({ pty_id: "P2", data: "+more" }));
    expect(getTerminalBuffer("claude:n1#pane-2")).toBe("two+more");
    writeTerminal("claude:n1#pane-2", "ls");
    expect(ptyWrite).toHaveBeenCalledWith("P2", "ls");
  });
});
