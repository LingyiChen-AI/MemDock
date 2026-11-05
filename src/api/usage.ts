import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { QuotaDto, Settings, UsageQuery, UsageReport } from "./types";

export function usageSummary(q: UsageQuery): Promise<UsageReport> {
  return invoke<UsageReport>("usage_summary", { ...q });
}

/** Codex from local logs; Claude only when the opt-in switch is on. */
export function usageQuota(): Promise<QuotaDto> {
  return invoke<QuotaDto>("usage_quota");
}

export function usageExportCsv(q: UsageQuery, path: string): Promise<void> {
  return invoke<void>("usage_export_csv", { ...q, path });
}

export function setClaudeQuotaEnabled(enabled: boolean): Promise<Settings> {
  return invoke<Settings>("set_claude_quota_enabled", { enabled });
}

export function usageRescan(): Promise<boolean> {
  return invoke<boolean>("usage_rescan");
}

/** Fired after the first log scan and whenever logs changed. */
export function onUsageUpdated(cb: () => void): Promise<UnlistenFn> {
  return listen("usage://updated", () => cb());
}
