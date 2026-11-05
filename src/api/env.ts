import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { EnvReport, InstallEvent } from "./types";

export function envDetect(): Promise<EnvReport> {
  return invoke<EnvReport>("env_detect");
}

/** Starts `npm install -g` in the background; rejects `npm_missing` / `busy`. */
export function envInstall(tool: "claude" | "codex"): Promise<void> {
  return invoke<void>("env_install", { tool });
}

/** Opens the data folder in the file manager and resolves with its path. */
export function openDataDir(): Promise<string> {
  return invoke<string>("open_data_dir");
}

export function onInstallLog(cb: (e: Extract<InstallEvent, { kind: "log" }>) => void): Promise<UnlistenFn> {
  return listen<Extract<InstallEvent, { kind: "log" }>>("env://install-log", (e) => cb(e.payload));
}

export function onInstallDone(cb: (e: Extract<InstallEvent, { kind: "done" }>) => void): Promise<UnlistenFn> {
  return listen<Extract<InstallEvent, { kind: "done" }>>("env://install-done", (e) => cb(e.payload));
}
