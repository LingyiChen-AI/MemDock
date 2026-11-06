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

/** Terminals by key: the first pane of each terminal (what lists and conversations see). */
let entries: Record<string, TermEntry> = {};
/** Extra split panes, keyed `<terminal key>#pane-<n>`; never listed on their own. */
let subs: Record<string, TermEntry> = {};
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

export const PANE_SEP = "#pane-";

/** The terminal a pane key belongs to (a terminal key is its own first pane). */
export function paneRoot(key: string): string {
  const i = key.indexOf(PANE_SEP);
  return i >= 0 ? key.slice(0, i) : key;
}

function isSub(key: string): boolean {
  return key.includes(PANE_SEP);
}

function getEntry(key: string): TermEntry | undefined {
  return isSub(key) ? subs[key] : entries[key];
}

function putEntry(key: string, e: TermEntry) {
  if (isSub(key)) subs = { ...subs, [key]: e };
  else entries = { ...entries, [key]: e };
}

function dropEntry(key: string) {
  if (isSub(key)) {
    const rest = { ...subs };
    delete rest[key];
    subs = rest;
  } else {
    const rest = { ...entries };
    delete rest[key];
    entries = rest;
  }
}

function patch(key: string, p: Partial<TermEntry>) {
  const cur = getEntry(key);
  if (!cur) return;
  putEntry(key, { ...cur, ...p });
  emit();
}

/** The terminal (or split pane) of `key`. */
export function getTerminal(key: string): TermEntry | undefined {
  return getEntry(key);
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
  const get = React.useCallback(() => getEntry(key), [key]);
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
  const cur = getEntry(key);
  if (cur && (cur.status === "starting" || cur.status === "running")) return;
  putEntry(key, { key, ptyId: null, command: "", status: "starting", code: null, error: null, busy: null, touched: keep, opts });
  buffers.set(key, "");
  emit();
  try {
    await ensureListening();
    const { pty_id, command } = await ptyOpen(opts);
    if (getEntry(key)?.status !== "starting" || getEntry(key)?.ptyId) {
      // Closed (or replaced) while starting.
      if (!keyByPty.has(pty_id)) void ptyClose(pty_id).catch(() => {});
      return;
    }
    const other = keyByPty.get(pty_id);
    if (other && other !== key && getEntry(other) && !isSub(key)) {
      // The backend re-attached a terminal we already show under another key: move it here.
      dropEntry(key);
      migrateTerminal(other, key);
      return;
    }
    keyByPty.set(pty_id, key);
    patch(key, { ptyId: pty_id, command, status: "running" });
    // The view was laid out (or resized) while the process started.
    const size = getEntry(key)?.opts;
    if (size && (size.cols !== opts.cols || size.rows !== opts.rows)) void ptyResize(pty_id, size.cols, size.rows).catch(() => {});
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
  const cur = getEntry(from);
  if (from === to || !cur || isSub(to)) return false;
  const there = entries[to];
  if (there && (there.status === "starting" || there.status === "running")) return false;
  if (isSub(from)) {
    // A split pane becomes a terminal of its own.
    detachPane(from);
  } else {
    // Its split panes move along.
    const lay = layouts[from];
    if (lay) {
      const ren = (k: string) => (k === from ? to : to + k.slice(from.length));
      for (const k of leaves(lay.tree)) if (k !== from) moveKey(k, ren(k));
      const next = { ...layouts };
      delete next[from];
      layouts = { ...next, [to]: { ...lay, tree: mapLeaves(lay.tree, ren), focus: ren(lay.focus) } };
    }
  }
  moveKey(from, to);
  emit();
  return true;
}

/** Moves one pane's entry, scrollback and pty mapping to another key (no emit). */
function moveKey(from: string, to: string) {
  const cur = getEntry(from);
  if (!cur) return;
  dropEntry(from);
  putEntry(to, { ...cur, key: to });
  buffers.set(to, buffers.get(from) ?? "");
  buffers.delete(from);
  if (cur.ptyId) keyByPty.set(cur.ptyId, to);
  const t = closeTimers.get(from);
  if (t) clearTimeout(t);
  closeTimers.delete(from);
  sinks.delete(from);
}

/** The key showing terminal `ptyId`, if any. */
export function terminalKeyForPty(ptyId: string): string | undefined {
  return keyByPty.get(ptyId);
}

// ---------------------------------------------------------------------------
// Split panes: each terminal's layout is a binary tree whose leaves are pane keys.
// ---------------------------------------------------------------------------

export type SplitDir = "row" | "column";
export type PaneNode = { type: "leaf"; pane: string } | { type: "split"; id: string; dir: SplitDir; ratio: number; a: PaneNode; b: PaneNode };
export interface PaneLayout {
  tree: PaneNode;
  focus: string;
  seq: number;
}

/** Most panes one terminal can be split into. */
export const MAX_PANES = 6;

/** Layouts of split terminals by terminal key; a terminal without one is a single pane. */
let layouts: Record<string, PaneLayout> = {};
let splitSeq = 0;

export function leaves(n: PaneNode): string[] {
  return n.type === "leaf" ? [n.pane] : [...leaves(n.a), ...leaves(n.b)];
}

function mapLeaves(n: PaneNode, f: (k: string) => string): PaneNode {
  return n.type === "leaf" ? { type: "leaf", pane: f(n.pane) } : { ...n, a: mapLeaves(n.a, f), b: mapLeaves(n.b, f) };
}

function removeLeaf(n: PaneNode, key: string): PaneNode | null {
  if (n.type === "leaf") return n.pane === key ? null : n;
  const a = removeLeaf(n.a, key);
  const b = removeLeaf(n.b, key);
  if (!a) return b;
  if (!b) return a;
  return a === n.a && b === n.b ? n : { ...n, a, b };
}

function replaceLeaf(n: PaneNode, key: string, by: PaneNode): PaneNode {
  if (n.type === "leaf") return n.pane === key ? by : n;
  return { ...n, a: replaceLeaf(n.a, key, by), b: replaceLeaf(n.b, key, by) };
}

function single(key: string): PaneLayout {
  return { tree: { type: "leaf", pane: key }, focus: key, seq: 1 };
}

export function getLayout(key: string): PaneLayout {
  return layouts[key] ?? single(key);
}

const singles = new Map<string, PaneLayout>();
/** Bumped when another pane takes over a terminal's first pane: its view must re-attach. */
const heirGens = new Map<string, number>();

export function paneGen(key: string): number {
  return heirGens.get(key) ?? 0;
}

/** The pane layout of terminal `key` (a single pane until it is split). */
export function useLayout(key: string): PaneLayout {
  const sub = React.useCallback((l: () => void) => {
    listeners.add(l);
    return () => {
      listeners.delete(l);
    };
  }, []);
  const get = React.useCallback(() => {
    const lay = layouts[key];
    if (lay) return lay;
    let s = singles.get(key);
    if (!s) {
      s = single(key);
      singles.set(key, s);
    }
    return s;
  }, [key]);
  return React.useSyncExternalStore(sub, get, get);
}

function setLayout(root: string, lay: PaneLayout | null) {
  const next = { ...layouts };
  if (lay && lay.tree.type === "split") next[root] = lay;
  else delete next[root];
  layouts = next;
}

/** Takes a split pane out of its terminal's layout (its process is left alone; no emit). */
function detachPane(key: string) {
  const root = paneRoot(key);
  const lay = layouts[root];
  if (!lay) return;
  const tree = removeLeaf(lay.tree, key) ?? { type: "leaf", pane: root };
  const ks = leaves(tree);
  setLayout(root, { ...lay, tree, focus: ks.includes(lay.focus) ? lay.focus : ks[0] });
}

/**
 * Splits pane `pane` of terminal `root`: the new pane goes right (`row`) or below (`column`),
 * gets the focus and starts `opts` (a shell unless told otherwise). Null at the pane limit.
 */
export function splitPane(root: string, pane: string, dir: SplitDir, opts: PtyOpenOptions): string | null {
  const lay = getLayout(root);
  if (leaves(lay.tree).length >= MAX_PANES || !leaves(lay.tree).includes(pane)) return null;
  const key = `${root}${PANE_SEP}${lay.seq + 1}`;
  splitSeq += 1;
  const tree = replaceLeaf(lay.tree, pane, { type: "split", id: `s${splitSeq}`, dir, ratio: 0.5, a: { type: "leaf", pane }, b: { type: "leaf", pane: key } });
  setLayout(root, { tree, focus: key, seq: lay.seq + 1 });
  // A split terminal is in use: it stays when its view is left.
  const head = entries[root];
  if (head && !head.touched) entries = { ...entries, [root]: { ...head, touched: true } };
  emit();
  void openTerminal(key, opts, true);
  return key;
}

/**
 * Closes one pane, killing only its process. Closing the first pane hands its place (and key)
 * to the next one. Returns false when it was the last pane: the caller closes the whole terminal.
 */
export function closePane(root: string, pane: string): boolean {
  const lay = layouts[root];
  if (!lay || !leaves(lay.tree).includes(pane)) return false;
  const tree = removeLeaf(lay.tree, pane);
  if (!tree) return false;
  let next: PaneLayout = { ...lay, tree, focus: lay.focus === pane ? leaves(tree)[0] : lay.focus };
  void killKey(pane);
  if (pane === root) {
    const heir = leaves(tree)[0];
    moveKey(heir, root);
    heirGens.set(root, paneGen(root) + 1);
    const ren = (k: string) => (k === heir ? root : k);
    next = { ...next, tree: mapLeaves(next.tree, ren), focus: ren(next.focus) };
  }
  setLayout(root, next);
  emit();
  return true;
}

export function setPaneFocus(root: string, pane: string): void {
  const lay = layouts[root];
  if (!lay || lay.focus === pane || !leaves(lay.tree).includes(pane)) return;
  layouts = { ...layouts, [root]: { ...lay, focus: pane } };
  emit();
}

function withRatio(n: PaneNode, id: string, ratio: number): PaneNode {
  if (n.type === "leaf") return n;
  if (n.id === id) return { ...n, ratio };
  return { ...n, a: withRatio(n.a, id, ratio), b: withRatio(n.b, id, ratio) };
}

export function setSplitRatio(root: string, id: string, ratio: number): void {
  const lay = layouts[root];
  if (!lay) return;
  layouts = { ...layouts, [root]: { ...lay, tree: withRatio(lay.tree, id, Math.min(0.95, Math.max(0.05, ratio))) } };
  emit();
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface SplitBox {
  id: string;
  dir: SplitDir;
  ratio: number;
  rect: Rect;
}

/** Pane and divider rectangles in the unit square. */
export function paneRects(tree: PaneNode): { panes: Record<string, Rect>; splits: SplitBox[] } {
  const panes: Record<string, Rect> = {};
  const splits: SplitBox[] = [];
  const walk = (n: PaneNode, r: Rect) => {
    if (n.type === "leaf") {
      panes[n.pane] = r;
      return;
    }
    splits.push({ id: n.id, dir: n.dir, ratio: n.ratio, rect: r });
    if (n.dir === "row") {
      walk(n.a, { ...r, w: r.w * n.ratio });
      walk(n.b, { ...r, x: r.x + r.w * n.ratio, w: r.w * (1 - n.ratio) });
    } else {
      walk(n.a, { ...r, h: r.h * n.ratio });
      walk(n.b, { ...r, y: r.y + r.h * n.ratio, h: r.h * (1 - n.ratio) });
    }
  };
  walk(tree, { x: 0, y: 0, w: 1, h: 1 });
  return { panes, splits };
}

export type FocusDir = "left" | "right" | "up" | "down";

/** The pane next to `from` in direction `dir` (the closest, then the most overlapping), if any. */
export function neighborPane(tree: PaneNode, from: string, dir: FocusDir): string | null {
  const { panes } = paneRects(tree);
  const cur = panes[from];
  if (!cur) return null;
  const eps = 1e-6;
  let best: { key: string; dist: number; overlap: number } | null = null;
  for (const [key, r] of Object.entries(panes)) {
    if (key === from) continue;
    let dist: number;
    let overlap: number;
    if (dir === "left" || dir === "right") {
      dist = dir === "right" ? r.x - (cur.x + cur.w) : cur.x - (r.x + r.w);
      overlap = Math.min(cur.y + cur.h, r.y + r.h) - Math.max(cur.y, r.y);
    } else {
      dist = dir === "down" ? r.y - (cur.y + cur.h) : cur.y - (r.y + r.h);
      overlap = Math.min(cur.x + cur.w, r.x + r.w) - Math.max(cur.x, r.x);
    }
    if (dist < -eps || overlap <= eps) continue;
    if (!best || dist < best.dist - eps || (Math.abs(dist - best.dist) <= eps && overlap > best.overlap)) best = { key, dist, overlap };
  }
  return best?.key ?? null;
}

/** Moves the focus of terminal `root` to the neighbouring pane; returns the newly focused pane. */
export function movePaneFocus(root: string, dir: FocusDir): string | null {
  const lay = layouts[root];
  if (!lay) return null;
  const to = neighborPane(lay.tree, lay.focus, dir);
  if (to) setPaneFocus(root, to);
  return to;
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

/** Kills the pty of one pane and forgets it (no emit). */
function killKey(key: string): Promise<void> {
  const cur = getEntry(key);
  const t = closeTimers.get(key);
  if (t) clearTimeout(t);
  closeTimers.delete(key);
  if (!cur) return Promise.resolve();
  dropEntry(key);
  buffers.delete(key);
  if (!cur.ptyId) return Promise.resolve();
  keyByPty.delete(cur.ptyId);
  return ptyClose(cur.ptyId).catch(() => {});
}

/**
 * Kills the terminal of `key` (if any), with all its split panes, and forgets it.
 * A split pane's key closes just that pane.
 */
export async function closeTerminal(key: string): Promise<void> {
  if (isSub(key)) {
    detachPane(key);
    const p = killKey(key);
    emit();
    await p;
    return;
  }
  const lay = layouts[key];
  const kills = lay ? leaves(lay.tree).filter((k) => k !== key).map(killKey) : [];
  if (lay) {
    const next = { ...layouts };
    delete next[key];
    layouts = next;
  }
  const hadEntry = !!entries[key];
  kills.push(killKey(key));
  if (hadEntry || lay) emit();
  await Promise.all(kills);
}

/** Replies xterm sends on its own (cursor position, device attributes, focus in/out): not user input. */
const AUTO_REPLY = /^(\x1b\[\d+;\d+R|\x1b\[[?>][\d;]*c|\x1b\[[IO])+$/;

export function writeTerminal(key: string, data: string): void {
  const cur = getEntry(key);
  if (!cur?.ptyId || cur.status !== "running") return;
  if (!cur.touched && !AUTO_REPLY.test(data)) patch(key, { touched: true });
  void ptyWrite(cur.ptyId, data).catch(() => {});
}

export function resizeTerminal(key: string, cols: number, rows: number): void {
  const cur = getEntry(key);
  if (!cur) return;
  if (cur.opts.cols !== cols || cur.opts.rows !== rows) {
    putEntry(key, { ...cur, opts: { ...cur.opts, cols, rows } });
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
    // Split panes live and die with their terminal.
    const cur = isSub(key) ? undefined : entries[key];
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
  subs = {};
  layouts = {};
  singles.clear();
  heirGens.clear();
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
