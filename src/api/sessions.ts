import { invoke } from "@tauri-apps/api/core";
import type { ConvertResult, SessionPreview, SessionSummary, Tool } from "./types";

/** Claude Code and Codex sessions on this machine, newest first. */
export function listSessions(): Promise<SessionSummary[]> {
  return invoke<SessionSummary[]>("list_sessions");
}

/** The converted messages (capped) and mapping stats for one session. */
export function sessionPreview(tool: Tool, path: string): Promise<SessionPreview> {
  return invoke<SessionPreview>("session_preview", { tool, path });
}

/** Renames a session (also where Claude Code / Codex keep the name, so their history shows it). */
export function renameSession(tool: Tool, path: string, title: string): Promise<void> {
  return invoke<void>("rename_session", { tool, path, title });
}

/** The folder a conversion of this session would be written to. */
export function convertTargetDir(tool: Tool, path: string): Promise<string> {
  return invoke<string>("convert_target_dir", { tool, path });
}

/**
 * Converts the session into the other tool's format as a new session.
 * Rejects `already_converted` unless `force`.
 */
export function convertSession(tool: Tool, path: string, force: boolean): Promise<ConvertResult> {
  return invoke<ConvertResult>("convert_session", { tool, path, force, includeNote: true });
}
