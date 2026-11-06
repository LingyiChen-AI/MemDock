import * as React from "react";
import { liveStop, type LiveAgent, type LiveSessionInfo } from "@/api/live";
import { onPtyData, onPtyExit, ptyClose, ptyOpen, ptyResize, ptyWrite, type PtyOpenOptions } from "@/api/pty";
import { errorMessage } from "@/pages/skills/skillUtils";
import { isActive, loadChatPrefs } from "./chatPrefs";

/**
 * Terminal mode store: at most one pseudo terminal per conversation, kept for the
 * whole app so leaving the page / conversation and coming back re-attaches (the
 * output so far is replayed from a capped scrollback buffer).
 */

export type ChatMode = "gui" | "terminal";

const MODE_KEY = "memdock.chat.mode";

/** Last chosen view; conversations open in it. */
export function loadChatMode(): ChatMode {
  try {
    return localStorage.getItem(MODE_KEY) === "terminal" ? "terminal" : "gui";
  } catch {
    return "gui";
  }
}

export function saveChatMode(mode: ChatMode): void {
  try {
    localStorage.setItem(MODE_KEY, mode);
  } catch {
    // storage unavailable: the choice is just not remembered
  }
}

export type TermStatus = "starting" | "running" | "exited" | "error";

export interface TermEntry {
  key: string;
  ptyId: string | null;
  command: string;
  status: TermStatus;
  code: number | null;
  error: string | null;
  /** The user typed something: kept alive when the view is left. */
  touched: boolean;
  opts: PtyOpenOptions;
}

/** Scrollback kept per terminal for re-attaching. */
export const BUFFER_CAP = 2 * 1024 * 1024;

let entries: Record<string, TermEntry> = {};
const buffers = new Map<string, string>();
const keyByPty = new Map<string, string>();
/** Output that arrived before `ptyOpen` answered with its id. */
const early = new Map<string, string[]>();
const earlyExit = new Map<string, number | null>();
const sinks = new Map<string, (data: string) => void>();
const closeTimers = new Map<string, ReturnType<typeof setTimeout>>();
const listeners = new Set<() => void>();
let listening: Promise<void> | null = null;
let unlisteners: (() => void)[] = [];

function emit() {
  listeners.forEach((l) => l());
}

function patch(key: string, p: Partial<TermEntry>) {
  const cur = entries[key];
  if (!cur) return;
  entries = { ...entries, [key]: { ...cur, ...p } };
  emit();
}

export function getTerminal(key: string): TermEntry | undefined {
  return entries[key];
}

export function getTerminalBuffer(key: string): string {
  return buffers.get(key) ?? "";
}

export function useTerminal(key: string): TermEntry | undefined {
  const sub = React.useCallback((l: () => void) => {
    listeners.add(l);
    return () => {
      listeners.delete(l);
    };
  }, []);
  const get = React.useCallback(() => entries[key], [key]);
  return React.useSyncExternalStore(sub, get, get);
}

/** Appends to the capped buffer, cutting at a line start when trimming. */
export function appendCapped(buf: string, data: string, cap = BUFFER_CAP): string {
  const next = buf + data;
  if (next.length <= cap) return next;
  const cut = next.length - cap;
  const nl = next.indexOf("\n", cut);
  return nl >= 0 && nl - cut < 4096 ? next.slice(nl + 1) : next.slice(cut);
}

function deliver(key: string, data: string) {
  buffers.set(key, appendCapped(buffers.get(key) ?? "", data));
  sinks.get(key)?.(data);
}

function exited(key: string, code: number | null) {
  patch(key, { status: "exited", code });
}

function ensureListening(): Promise<void> {
  if (!listening) {
    listening = Promise.all([
      onPtyData(({ pty_id, data }) => {
        const key = keyByPty.get(pty_id);
        if (key) deliver(key, data);
        else early.set(pty_id, [...(early.get(pty_id) ?? []), data]);
      }),
      onPtyExit(({ pty_id, code }) => {
        const key = keyByPty.get(pty_id);
        if (key) exited(key, code);
        else earlyExit.set(pty_id, code);
      }),
    ])
      .then((uns) => {
        unlisteners = uns;
      })
      .catch(() => {
        listening = null;
      });
  }
  return listening;
}

/** Starts the CLI for `key` unless one is already starting or running there. */
export async function openTerminal(key: string, opts: PtyOpenOptions): Promise<void> {
  const cur = entries[key];
  if (cur && (cur.status === "starting" || cur.status === "running")) return;
  entries = {
    ...entries,
    [key]: { key, ptyId: null, command: "", status: "starting", code: null, error: null, touched: false, opts },
  };
  buffers.set(key, "");
  emit();
  try {
    await ensureListening();
    const { pty_id, command } = await ptyOpen(opts);
    if (entries[key]?.status !== "starting" || entries[key]?.ptyId) {
      // Closed (or replaced) while starting.
      void ptyClose(pty_id).catch(() => {});
      return;
    }
    keyByPty.set(pty_id, key);
    patch(key, { ptyId: pty_id, command, status: "running" });
    for (const d of early.get(pty_id) ?? []) deliver(key, d);
    early.delete(pty_id);
    if (earlyExit.has(pty_id)) {
      exited(key, earlyExit.get(pty_id) ?? null);
      earlyExit.delete(pty_id);
    }
  } catch (e) {
    patch(key, { status: "error", error: errorMessage(e) });
  }
}

/** Kills the terminal of `key` (if any) and forgets it. */
export async function closeTerminal(key: string): Promise<void> {
  const cur = entries[key];
  const t = closeTimers.get(key);
  if (t) clearTimeout(t);
  closeTimers.delete(key);
  if (!cur) return;
  const rest = { ...entries };
  delete rest[key];
  entries = rest;
  buffers.delete(key);
  emit();
  if (cur.ptyId) {
    keyByPty.delete(cur.ptyId);
    await ptyClose(cur.ptyId).catch(() => {});
  }
}

/** Replies xterm sends on its own (cursor position, device attributes, focus in/out): not user input. */
const AUTO_REPLY = /^(\x1b\[\d+;\d+R|\x1b\[[?>][\d;]*c|\x1b\[[IO])+$/;

export function writeTerminal(key: string, data: string): void {
  const cur = entries[key];
  if (!cur?.ptyId || cur.status !== "running") return;
  if (!cur.touched && !AUTO_REPLY.test(data)) patch(key, { touched: true });
  void ptyWrite(cur.ptyId, data).catch(() => {});
}

export function resizeTerminal(key: string, cols: number, rows: number): void {
  const cur = entries[key];
  if (!cur) return;
  if (cur.opts.cols !== cols || cur.opts.rows !== rows) {
    entries = { ...entries, [key]: { ...cur, opts: { ...cur.opts, cols, rows } } };
  }
  if (cur.ptyId && cur.status === "running") void ptyResize(cur.ptyId, cols, rows).catch(() => {});
}

/**
 * The view showing `key` receives its output. On detach a terminal the user
 * never typed into is closed after a short grace period (browsing conversations
 * in terminal mode should not leave CLIs running); touched ones stay alive.
 */
export function attachTerminal(key: string, sink: (data: string) => void): () => void {
  const t = closeTimers.get(key);
  if (t) clearTimeout(t);
  closeTimers.delete(key);
  sinks.set(key, sink);
  return () => {
    if (sinks.get(key) === sink) sinks.delete(key);
    const cur = entries[key];
    if (cur && !cur.touched) {
      closeTimers.set(
        key,
        setTimeout(() => {
          closeTimers.delete(key);
          if (!sinks.has(key) && entries[key] && !entries[key].touched) void closeTerminal(key);
        }, 1500),
      );
    }
  };
}

/** Kills every terminal (tests, app teardown). */
export function resetTerminalStore(): void {
  for (const t of closeTimers.values()) clearTimeout(t);
  closeTimers.clear();
  unlisteners.forEach((u) => u());
  unlisteners = [];
  listening = null;
  entries = {};
  buffers.clear();
  keyByPty.clear();
  early.clear();
  earlyExit.clear();
  sinks.clear();
  emit();
}

/** Stable per conversation: the native id once known, else the GUI live process. */
export function terminalKey(t: { tool: LiveAgent; nativeId: string | null; live: LiveSessionInfo | null }): string {
  return t.nativeId ? `${t.tool}:${t.nativeId}` : `live:${t.live?.live_id ?? "new"}`;
}

/** PTY options for a conversation, with the permission / model last used for its tool. */
export function terminalOptions(t: { tool: LiveAgent; cwd: string; nativeId: string | null }, cols = 100, rows = 30): PtyOpenOptions {
  const prefs = loadChatPrefs();
  return {
    tool: t.tool,
    cwd: t.cwd,
    resume_id: t.nativeId,
    permission: prefs.permission[t.tool],
    model: prefs.model[t.tool] || null,
    cols,
    rows,
  };
}

/**
 * Before the terminal takes over: stops the GUI process driving the same
 * conversation. A turn in progress needs `confirm()`; returns false when declined.
 */
export async function releaseGuiProcess(live: LiveSessionInfo | null, confirm: () => Promise<boolean>): Promise<boolean> {
  if (!live || live.state === "exited") return true;
  if (isActive(live.state) && !(await confirm())) return false;
  await liveStop(live.live_id);
  return true;
}

/** Initial view of a conversation: its open terminal, else the remembered mode unless a GUI process is attached. */
export function initialMode(key: string, live: LiveSessionInfo | null): ChatMode {
  if (entries[key]) return "terminal";
  if (live && live.state !== "exited") return "gui";
  return loadChatMode();
}
