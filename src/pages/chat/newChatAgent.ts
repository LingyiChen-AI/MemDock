import type { LiveAgent } from "@/api/live";
import type { EnvReport } from "@/api/types";

const KEY = "memdock.chat.projectDefaultAgent";
export const NEW_CHAT_AGENTS = ["claude", "codex"] as const;

export function loadNewChatAgent(): LiveAgent | null {
  try {
    const value = localStorage.getItem(KEY);
    return value === "claude" || value === "codex" ? value : null;
  } catch {
    return null;
  }
}

export function saveNewChatAgent(agent: LiveAgent): void {
  localStorage.setItem(KEY, agent);
}

export function isChatAgentInstalled(env: EnvReport | null, agent: LiveAgent): boolean {
  return env?.tools.some((tool) => tool.name === agent && tool.installed) ?? false;
}

export function resolveNewChatAgent(env: EnvReport | null, preferred: LiveAgent | null): LiveAgent | null {
  if (preferred && isChatAgentInstalled(env, preferred)) return preferred;
  return NEW_CHAT_AGENTS.find((agent) => isChatAgentInstalled(env, agent)) ?? null;
}
