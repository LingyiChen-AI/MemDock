import * as React from "react";
import { liveStop, nativeOwner, parseBusy, type LiveAgent, type LiveSessionInfo, type NativeOwner } from "@/api/live";
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
  /** `pty_open` refused: a GUI live process drives this session (`busy_live:`). */
  busy: NativeOwner | null;
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

/**
 * Starts the CLI for `key` unless one is already starting or running there.
 * `keep`: never closed automatically when its view is left (plain shell terminals).
 */
export async function openTerminal(key: string, opts: PtyOpenOptions, keep = false): Promise<void> {
  const cur = entries[key];
  if (cur && (cur.status === "starting" || cur.status === "running")) return;
  entries = {
    ...entries,
    [key]: { key, ptyId: null, command: "", status: "starting", code: null, error: null, busy: null, touched: keep, opts },
  };
  buffers.set(key, "");
  emit();
  try {
    await ensureListening();
    const { pty_id, command } = await ptyOpen(opts);
    if (entries[key]?.status !== "starting" || entries[key]?.ptyId) {
      // Closed (or replaced) while starting.
      if (!keyByPty.has(pty_id)) void ptyClose(pty_id).catch(() => {});
      return;
    }
    const other = keyByPty.get(pty_id);
    if (other && other !== key && entries[other]) {
      // The backend re-attached a terminal we already show under another key: move it here.
      const rest = { ...entries };
      delete rest[key];
      entries = rest;
      migrateTerminal(other, key);
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
    patch(key, { status: "error", error: errorMessage(e), busy: parseBusy(e) });
  }
}

/**
 * The conversation of `from` got a new key (a draft started, its native id became known):
 * moves the terminal with its scrollback; the process keeps running. False when nothing moved.
 */
export function migrateTerminal(from: string, to: string): boolean {
  const cur = entries[from];
  if (from === to || !cur) return false;
  const there = entries[to];
  if (there && (there.status === "starting" || there.status === "running")) return false;
  const rest = { ...entries };
  delete rest[from];
  entries = { ...rest, [to]: { ...cur, key: to } };
  buffers.set(to, buffers.get(from) ?? "");
  buffers.delete(from);
  if (cur.ptyId) keyByPty.set(cur.ptyId, to);
  const t = closeTimers.get(from);
  if (t) clearTimeout(t);
  closeTimers.delete(from);
  sinks.delete(from);
  emit();
  return true;
}

/** The key showing terminal `ptyId`, if any. */
export function terminalKeyForPty(ptyId: string): string | undefined {
  return keyByPty.get(ptyId);
}

/** Every terminal entry (re-rendered on any change). */
export function useTerminalEntries(): Record<string, TermEntry> {
  const sub = React.useCallback((l: () => void) => {
    listeners.add(l);
    return () => {
      listeners.delete(l);
    };
  }, []);
  const get = React.useCallback(() => entries, []);
  return React.useSyncExternalStore(sub, get, get);
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
  shells = [];
  buffers.clear();
  keyByPty.clear();
  early.clear();
  earlyExit.clear();
  sinks.clear();
  emit();
}

/**
 * Stable per conversation: the native id once known, else the GUI live process, else the
 * draft's own `draft:<uuid>` key (never shared between drafts).
 */
export function terminalKey(t: { tool: LiveAgent; nativeId: string | null; live: LiveSessionInfo | null; draftKey?: string | null }): string {
  if (t.nativeId) return `${t.tool}:${t.nativeId}`;
  if (t.live) return `live:${t.live.live_id}`;
  return t.draftKey ?? "draft:none";
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
    effort: prefs.effort[t.tool] || null,
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

/**
 * Before a terminal opens for a conversation: no other process may drive its native session.
 * Stops the conversation's own GUI process and any other GUI process the backend reports as the
 * owner (asking first when one is replying); a terminal we already show elsewhere is moved to `key`.
 * Returns false when the user declined.
 */
export async function prepareTerminal(
  t: { tool: LiveAgent; nativeId: string | null; live: LiveSessionInfo | null },
  key: string,
  lives: LiveSessionInfo[],
  confirm: () => Promise<boolean>,
): Promise<boolean> {
  if (!(await releaseGuiProcess(t.live, confirm))) return false;
  if (!t.nativeId) return true;
  let owner: NativeOwner | null = null;
  try {
    owner = await nativeOwner(t.tool, t.nativeId);
  } catch {
    return true;
  }
  if (owner?.kind === "live" && owner.id !== t.live?.live_id) {
    const info = lives.find((x) => x.live_id === owner.id);
    if (info && isActive(info.state) && !(await confirm())) return false;
    await liveStop(owner.id);
  } else if (owner?.kind === "pty") {
    const other = keyByPty.get(owner.id);
    if (other && other !== key) migrateTerminal(other, key);
  }
  return true;
}

/** Initial view of a conversation: its open terminal, else the remembered mode unless a GUI process is attached. */
export function initialMode(key: string, live: LiveSessionInfo | null): ChatMode {
  if (entries[key]) return "terminal";
  if (live && live.state !== "exited") return "gui";
  return loadChatMode();
}

/** A plain shell terminal opened from the session list (not persisted). */
export interface ShellTerm {
  key: string;
  cwd: string;
  created_at: number;
}

let shells: ShellTerm[] = [];
let shellSeq = 0;

function useShellSnapshot(): ShellTerm[] {
  const sub = React.useCallback((l: () => void) => {
    listeners.add(l);
    return () => {
      listeners.delete(l);
    };
  }, []);
  const get = React.useCallback(() => shells, []);
  return React.useSyncExternalStore(sub, get, get);
}

/** Shell terminals, newest first. */
export function useShells(): ShellTerm[] {
  return useShellSnapshot();
}

export function getShells(): ShellTerm[] {
  return shells;
}

/** Adds a shell terminal in `cwd`; its view starts the process. Returns its key. */
export function addShell(cwd: string): string {
  shellSeq += 1;
  const key = `shell:${Date.now().toString(36)}-${shellSeq}`;
  shells = [{ key, cwd, created_at: Date.now() }, ...shells];
  emit();
  return key;
}

/** Kills a shell terminal and removes it from the list. */
export async function removeShell(key: string): Promise<void> {
  shells = shells.filter((x) => x.key !== key);
  emit();
  await closeTerminal(key);
}

/** PTY options of a plain shell in `cwd`. */
export function shellOptions(cwd: string, cols = 100, rows = 30): PtyOpenOptions {
  return { tool: "shell", cwd, resume_id: null, permission: "ask", model: null, effort: null, cols, rows };
}
