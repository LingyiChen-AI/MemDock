// Terminal mode: the interactive CLI in a pseudo terminal. Mirrors `src-tauri/src/pty/mod.rs`.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { LiveAgent, PermissionMode } from "./live";

/** What runs in the terminal: an agent CLI or the plain system shell. */
export type PtyTool = LiveAgent | "shell";

export interface PtyOpenOptions {
  tool: PtyTool;
  cwd: string;
  /** Claude session id / Codex thread id; null starts a new conversation. */
  resume_id: string | null;
  permission: PermissionMode;
  model: string | null;
  /** Thinking effort for the agent CLI; ignored for the shell. */
  effort?: string | null;
  cols: number;
  rows: number;
}

/**
 * Fails with `busy_live:<live_id>` when a GUI live process drives the same session; when another
 * terminal already has it, that terminal's `PtyOpened` comes back (re-attach, no new process).
 */
export interface PtyOpened {
  pty_id: string;
  /** Shortened command line for the status bar, e.g. `claude --resume 1234abcd…`. */
  command: string;
}

export interface PtyData {
  pty_id: string;
  data: string;
}

export interface PtyExit {
  pty_id: string;
  code: number | null;
}

export function ptyOpen(opts: PtyOpenOptions): Promise<PtyOpened> {
  return invoke<PtyOpened>("pty_open", { opts });
}

export function ptyWrite(ptyId: string, data: string): Promise<void> {
  return invoke<void>("pty_write", { ptyId, data });
}

export function ptyResize(ptyId: string, cols: number, rows: number): Promise<void> {
  return invoke<void>("pty_resize", { ptyId, cols, rows });
}

/** Kills the process tree; `pty://exit` follows. */
export function ptyClose(ptyId: string): Promise<void> {
  return invoke<void>("pty_close", { ptyId });
}

export function onPtyData(cb: (d: PtyData) => void): Promise<UnlistenFn> {
  return listen<PtyData>("pty://data", (e) => cb(e.payload));
}

export function onPtyExit(cb: (d: PtyExit) => void): Promise<UnlistenFn> {
  return listen<PtyExit>("pty://exit", (e) => cb(e.payload));
}

/** Saves a pasted image (`data:image/...;base64,` URI) to a temp file; returns its absolute path. */
export function saveTempImage(dataUri: string): Promise<string> {
  return invoke<string>("save_temp_image", { dataUri });
}
