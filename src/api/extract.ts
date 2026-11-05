import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AcceptResult,
  ApplyOutcome,
  ExtractConfig,
  ExtractProgress,
  PendingSummary,
  RunRecord,
  RunnerInfo,
  Suggestion,
  SuggestionContext,
  SuggestionStatus,
} from "./types";

export function getExtractConfig(): Promise<ExtractConfig> {
  return invoke<ExtractConfig>("get_extract_config");
}

/** Rejects with `CommandError` code `invalid` for out-of-range values. */
export function setExtractConfig(config: ExtractConfig): Promise<ExtractConfig> {
  return invoke<ExtractConfig>("set_extract_config", { config });
}

export function extractRunners(): Promise<RunnerInfo[]> {
  return invoke<RunnerInfo[]>("extract_runners");
}

/** Resolves with the Running record; rejects `already_running` / `not_installed`. */
export function startExtraction(): Promise<RunRecord> {
  return invoke<RunRecord>("start_extraction");
}

export function cancelExtraction(): Promise<void> {
  return invoke<void>("cancel_extraction");
}

/** Newest first. */
export function listExtractRuns(): Promise<RunRecord[]> {
  return invoke<RunRecord[]>("list_extract_runs");
}

/** Newest first; all statuses when `status` is omitted. */
export function listSuggestions(status?: SuggestionStatus): Promise<Suggestion[]> {
  return invoke<Suggestion[]>("list_suggestions", { status: status ?? null });
}

export function pendingSummary(): Promise<PendingSummary> {
  return invoke<PendingSummary>("pending_summary");
}

/** `needs_confirm` is not applied: ask the user, then call again with `confirmChanged: true`. */
export function applySuggestion(
  id: string,
  editedNewText: string | null,
  confirmChanged: boolean,
): Promise<ApplyOutcome> {
  return invoke<ApplyOutcome>("apply_suggestion", { id, editedNewText, confirmChanged });
}

export function rejectSuggestion(id: string): Promise<void> {
  return invoke<void>("reject_suggestion", { id });
}

export function acceptSuggestions(ids: string[]): Promise<AcceptResult[]> {
  return invoke<AcceptResult[]>("accept_suggestions", { ids });
}

export function suggestionContext(id: string): Promise<SuggestionContext[]> {
  return invoke<SuggestionContext[]>("suggestion_context", { id });
}

export function onExtractProgress(cb: (p: ExtractProgress) => void): Promise<UnlistenFn> {
  return listen<ExtractProgress>("extract-progress", (e) => cb(e.payload));
}

export function onExtractFinished(cb: (r: RunRecord) => void): Promise<UnlistenFn> {
  return listen<RunRecord>("extract-finished", (e) => cb(e.payload));
}

export function onSuggestionsChanged(cb: (s: PendingSummary) => void): Promise<UnlistenFn> {
  return listen<PendingSummary>("suggestions-changed", (e) => cb(e.payload));
}
