import { invoke } from "@tauri-apps/api/core";
import type {
  GhImportMemoriesResult,
  GhImportPreview,
  GhImportScan,
  GhMemoryPick,
  InstallTarget,
  SkillEntry,
} from "./types";

// Every function rejects with a `CommandError`. Import-specific codes:
// import_bad_url, import_not_found, import_rate_limited, import_too_large,
// import_expired; plus network, unauthorized, github_error, invalid, exists.

/** Reads the repository behind a GitHub link and lists what can be imported. */
export function ghImportScan(input: string): Promise<GhImportScan> {
  return invoke<GhImportScan>("gh_import_scan", { input });
}

export function ghImportPreview(sessionId: string, itemId: string): Promise<GhImportPreview> {
  return invoke<GhImportPreview>("gh_import_preview", { sessionId, itemId });
}

/** Creates pending review suggestions (nothing is written to memory files). */
export function ghImportMemories(sessionId: string, picks: GhMemoryPick[]): Promise<GhImportMemoriesResult> {
  return invoke<GhImportMemoriesResult>("gh_import_memories", { sessionId, picks });
}

/** Rejects with `exists` (carrying `dirs`) when a target folder is taken and `overwrite` is false. */
export function ghImportSkill(
  sessionId: string,
  itemId: string,
  targets: InstallTarget[],
  overwrite: boolean,
): Promise<SkillEntry[]> {
  return invoke<SkillEntry[]>("gh_import_skill", { sessionId, itemId, targets, overwrite });
}
