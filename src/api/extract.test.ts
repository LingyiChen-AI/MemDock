import { describe, it, expect, vi, beforeEach } from "vitest";

const invoke = vi.fn();
const listen = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...a: unknown[]) => listen(...a) }));

import {
  applySuggestion,
  listSuggestions,
  onExtractFinished,
  onExtractProgress,
  onSuggestionsChanged,
  setExtractConfig,
  startExtraction,
} from "./extract";
import type { ExtractConfig, RunRecord } from "./types";

beforeEach(() => {
  invoke.mockReset();
  listen.mockReset();
});

const running: RunRecord = {
  id: "r1",
  started_at: 1,
  finished_at: null,
  trigger: "manual",
  tool: "claude",
  status: "running",
  sessions_read: 0,
  projects: 0,
  suggestions_new: 0,
  warnings: [],
  error: null,
};

describe("extract api", () => {
  it("startExtraction returns the running record", async () => {
    invoke.mockResolvedValue(running);
    const r = await startExtraction();
    expect(invoke).toHaveBeenCalledWith("start_extraction");
    expect(r.status).toBe("running");
  });

  it("startExtraction rethrows already_running", async () => {
    const err = { code: "already_running", message: "x" };
    invoke.mockRejectedValue(err);
    await expect(startExtraction()).rejects.toEqual(err);
  });

  it("applySuggestion passes confirmChanged and editedNewText", async () => {
    invoke.mockResolvedValue({ kind: "applied", hash: "h", saved: true });
    await applySuggestion("s1", "edited", true);
    expect(invoke).toHaveBeenCalledWith("apply_suggestion", {
      id: "s1",
      editedNewText: "edited",
      confirmChanged: true,
    });
    await applySuggestion("s2", null, false);
    expect(invoke).toHaveBeenLastCalledWith("apply_suggestion", {
      id: "s2",
      editedNewText: null,
      confirmChanged: false,
    });
  });

  it("listSuggestions sends a null status when omitted", async () => {
    invoke.mockResolvedValue([]);
    await listSuggestions();
    expect(invoke).toHaveBeenCalledWith("list_suggestions", { status: null });
    await listSuggestions("pending");
    expect(invoke).toHaveBeenLastCalledWith("list_suggestions", { status: "pending" });
  });

  it("setExtractConfig sends the config object", async () => {
    const config: ExtractConfig = {
      tool: null,
      schedule: { kind: "weekly", weekday: 0, hh: 9, mm: 30 },
      projects: { kind: "selected", ids: ["p"] },
      include_global: true,
      lookback_days: 7,
      lang: null,
    };
    invoke.mockResolvedValue(config);
    await setExtractConfig(config);
    expect(invoke).toHaveBeenCalledWith("set_extract_config", { config });
  });

  it("event helpers listen on the right channels and unwrap payloads", async () => {
    listen.mockResolvedValue(vi.fn());
    const seen: unknown[] = [];
    await onExtractProgress((p) => seen.push(p));
    await onExtractFinished((r) => seen.push(r));
    await onSuggestionsChanged((s) => seen.push(s));
    expect(listen.mock.calls.map((c) => c[0])).toEqual([
      "extract-progress",
      "extract-finished",
      "suggestions-changed",
    ]);
    listen.mock.calls[2][1]({ payload: { total: 2, by_project: {} } });
    expect(seen).toEqual([{ total: 2, by_project: {} }]);
  });
});
