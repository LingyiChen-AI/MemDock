import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { QuotaDto, Settings, UsageQuery, UsageReport } from "./types";

export function usageSummary(q: UsageQuery): Promise<UsageReport> {
  return invoke<UsageReport>("usage_summary", { ...q });
}

/** Claude and live Codex only when their opt-in switches are on; Codex falls back to local logs. */
/** Cached for 30 minutes; `force` asks the services again right away. */
export function usageQuota(force = false): Promise<QuotaDto> {
  return invoke<QuotaDto>("usage_quota", { force });
}

export function usageExportCsv(q: UsageQuery, path: string): Promise<void> {
  return invoke<void>("usage_export_csv", { ...q, path });
}

export function setClaudeQuotaEnabled(enabled: boolean): Promise<Settings> {
  return invoke<Settings>("set_claude_quota_enabled", { enabled });
}

/** Opt-in live Codex quota lookup (reads `~/.codex/auth.json`, never writes it). */
export function setCodexQuotaEnabled(enabled: boolean): Promise<Settings> {
  return invoke<Settings>("set_codex_quota_enabled", { enabled });
}

export function usageRescan(): Promise<boolean> {
  return invoke<boolean>("usage_rescan");
}

/** Fired after the first log scan and whenever logs changed. */
export function onUsageUpdated(cb: () => void): Promise<UnlistenFn> {
  return listen("usage://updated", () => cb());
}
