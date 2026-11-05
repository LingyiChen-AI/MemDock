import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  CodexDbResult,
  FileContentDto,
  MemoryChangedPayload,
  MemoryIndex,
  SearchHit,
  Settings,
  SnapshotInfo,
} from "./types";

export function listMemories(): Promise<MemoryIndex> {
  return invoke<MemoryIndex>("list_memories");
}

export function readMemoryFile(path: string): Promise<FileContentDto> {
  return invoke<FileContentDto>("read_memory_file", { path });
}

/** Rejects with a `CommandError` object (code `disk_changed` carries `current_hash`). */
export function writeMemoryFile(
  path: string,
  text: string,
  expectedHash: string,
): Promise<FileContentDto> {
  return invoke<FileContentDto>("write_memory_file", { path, text, expectedHash });
}

export function searchMemories(query: string): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("search_memories", { query });
}

export function listSnapshots(path: string): Promise<SnapshotInfo[]> {
  return invoke<SnapshotInfo[]>("list_snapshots", { path });
}

export function readSnapshot(path: string, id: string): Promise<string> {
  return invoke<string>("read_snapshot", { path, id });
}

export function readCodexDb(): Promise<CodexDbResult> {
  return invoke<CodexDbResult>("read_codex_db");
}

export function addManualProject(path: string): Promise<MemoryIndex> {
  return invoke<MemoryIndex>("add_manual_project", { path });
}

export function removeManualProject(path: string): Promise<MemoryIndex> {
  return invoke<MemoryIndex>("remove_manual_project", { path });
}

export function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

/** Omitted (or undefined) fields are left unchanged. */
export function updateUiPrefs(language?: string, theme?: string): Promise<Settings> {
  return invoke<Settings>("update_ui_prefs", {
    language: language ?? null,
    theme: theme ?? null,
  });
}

export function onMemoryChanged(
  cb: (payload: MemoryChangedPayload) => void,
): Promise<UnlistenFn> {
  return listen<MemoryChangedPayload>("memory-changed", (e) => cb(e.payload));
}
