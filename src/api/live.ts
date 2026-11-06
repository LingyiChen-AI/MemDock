// Live chat with a real Claude Code / Codex process. Mirrors `src-tauri/src/live/mod.rs`.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type LiveAgent = "claude" | "codex";

/** ask: confirm every tool call · auto_edit: file edits run without asking ·
 *  plan: read-only planning · bypass: never ask (Claude `--dangerously-skip-permissions`,
 *  Codex `never` + `danger-full-access`). */
export type PermissionMode = "ask" | "auto_edit" | "plan" | "bypass";

export type LiveState = "starting" | "idle" | "running" | "waiting_approval" | "exited";

export type Decision = "allow" | "allow_session" | "deny";

export type LiveToolKind = "shell" | "read" | "edit" | "write" | "search" | "web" | "agent" | "todo" | "other";

export interface StartOptions {
  agent: LiveAgent;
  cwd: string;
  model?: string | null;
  permission: PermissionMode;
  /** Claude session id / Codex thread id to continue. */
  resume_id?: string | null;
  /** Claude: extra CLI args (already split). Codex: `key=value` config overrides. */
  extra_args?: string[];
  /** Sent right after the process starts. */
  prompt?: string | null;
  /** Images of the first message (`data:image/...;base64,` URIs). */
  prompt_images?: string[];
}

export interface TurnUsage {
  input_tokens: number | null;
  output_tokens: number | null;
  cost_usd: number | null;
  duration_ms: number | null;
}

export type LiveEvent =
  | { type: "started"; native_id: string; model: string | null }
  | { type: "status"; state: LiveState }
  | { type: "user_message"; id: string; text: string; images?: string[] }
  | { type: "assistant_delta"; id: string; text: string }
  | { type: "assistant_message"; id: string; text: string }
  | { type: "thinking"; id: string; text: string }
  | { type: "tool_started"; id: string; name: string; kind: LiveToolKind; summary: string; input: string }
  | { type: "tool_output"; id: string; text: string }
  | { type: "tool_finished"; id: string; output: string | null; is_error: boolean }
  | {
      type: "approval_request";
      request_id: string;
      tool_name: string;
      kind: LiveToolKind;
      summary: string;
      detail: string;
      can_allow_session: boolean;
    }
  | { type: "approval_resolved"; request_id: string; decision: Decision }
  | { type: "turn_completed"; usage: TurnUsage; is_error: boolean }
  | { type: "error"; message: string; fatal: boolean }
  | { type: "exited"; code: number | null };

export interface LiveEnvelope {
  live_id: string;
  seq: number;
  event: LiveEvent;
}

export interface LiveSessionInfo {
  live_id: string;
  agent: LiveAgent;
  cwd: string;
  title: string;
  state: LiveState;
  native_id: string | null;
  model: string | null;
  permission: PermissionMode;
  created_at: number;
  updated_at: number;
}

export function liveStart(opts: StartOptions): Promise<LiveSessionInfo> {
  return invoke<LiveSessionInfo>("live_start", { opts: { extra_args: [], ...opts } });
}

/** `images` are `data:image/...;base64,` URIs (pasted or dropped screenshots). */
export function liveSend(liveId: string, text: string, images: string[] = []): Promise<void> {
  return invoke<void>("live_send", { liveId, text, images });
}

/** Reads a png/jpg/gif/webp file (max 5 MB) as a `data:image/...;base64,` URI. */
export function readImageDataUri(path: string): Promise<string> {
  return invoke<string>("read_image_data_uri", { path });
}

export function liveRespond(liveId: string, requestId: string, decision: Decision): Promise<void> {
  return invoke<void>("live_respond", { liveId, requestId, decision });
}

/** Stops the current turn; the session stays open. */
export function liveInterrupt(liveId: string): Promise<void> {
  return invoke<void>("live_interrupt", { liveId });
}

/** Ends the process; the conversation stays listed (and resumable from history). */
export function liveStop(liveId: string): Promise<void> {
  return invoke<void>("live_stop", { liveId });
}

/** Ends the process and removes it from the live list. */
export function liveClose(liveId: string): Promise<void> {
  return invoke<void>("live_close", { liveId });
}

export function liveList(): Promise<LiveSessionInfo[]> {
  return invoke<LiveSessionInfo[]>("live_list");
}

/** All events so far, to rebuild a conversation after re-opening it. */
export function liveTranscript(liveId: string): Promise<LiveEnvelope[]> {
  return invoke<LiveEnvelope[]>("live_transcript", { liveId });
}

export function onLiveEvent(cb: (env: LiveEnvelope) => void): Promise<UnlistenFn> {
  return listen<LiveEnvelope>("live://event", (e) => cb(e.payload));
}
