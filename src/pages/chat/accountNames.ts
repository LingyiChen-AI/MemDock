// The user's display name per tool, from the account each CLI is logged in with (read once, cached).
import * as React from "react";
import { invoke } from "@tauri-apps/api/core";

export interface AccountNames {
  claude: string | null;
  codex: string | null;
}

let cache: AccountNames | null = null;
let pending: Promise<AccountNames> | null = null;
const listeners = new Set<() => void>();

function load(): Promise<AccountNames> {
  pending ??= invoke<AccountNames>("account_names")
    .catch(() => ({ claude: null, codex: null }))
    .then((v) => {
      cache = v;
      listeners.forEach((l) => l());
      return v;
    });
  return pending;
}

/** The logged-in name for `tool`, or `fallback` while loading / when unknown. */
export function useAccountName(tool: "claude" | "codex", fallback: string): string {
  const [, bump] = React.useReducer((x: number) => x + 1, 0);
  React.useEffect(() => {
    listeners.add(bump);
    if (!cache) void load();
    return () => {
      listeners.delete(bump);
    };
  }, []);
  return cache?.[tool] || fallback;
}

/** Tests: forget the cached names. */
export function resetAccountNames() {
  cache = null;
  pending = null;
}
