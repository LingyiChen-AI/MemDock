import type { LiveAgent, LiveState, PermissionMode } from "@/api/live";

/** The composer remembers the last agent / folder, and per agent the permission mode, model, effort and args. */
export interface ChatPrefs {
  agent: LiveAgent;
  permission: Record<LiveAgent, PermissionMode>;
  model: Record<LiveAgent, string>;
  /** Thinking effort; "" is the CLI / model default. */
  effort: Record<LiveAgent, string>;
  /** Extra start arguments as typed (Claude CLI args, Codex `key=value`). */
  args: Record<LiveAgent, string>;
  /** Last project folder a chat was started in. */
  cwd: string;
}

const KEY = "memdock.chat.prefs";

export const DEFAULT_PREFS: ChatPrefs = {
  agent: "claude",
  permission: { claude: "ask", codex: "ask" },
  model: { claude: "", codex: "" },
  effort: { claude: "", codex: "" },
  args: { claude: "", codex: "" },
  cwd: "",
};

const MODES: PermissionMode[] = ["ask", "auto_edit", "plan", "bypass"];

export const MODEL_SUGGESTIONS: Record<LiveAgent, string[]> = {
  claude: ["opus", "sonnet", "haiku"],
  codex: ["gpt-5.2-codex", "gpt-5-codex", "gpt-5"],
};

export function loadChatPrefs(): ChatPrefs {
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) ?? "null") as Partial<ChatPrefs> | null;
    if (!raw || typeof raw !== "object") return DEFAULT_PREFS;
    const agent: LiveAgent = raw.agent === "codex" ? "codex" : "claude";
    const perm = (v: unknown): PermissionMode => (MODES.includes(v as PermissionMode) ? (v as PermissionMode) : "ask");
    const str = (v: unknown) => (typeof v === "string" ? v : "");
    return {
      agent,
      permission: { claude: perm(raw.permission?.claude), codex: perm(raw.permission?.codex) },
      model: { claude: str(raw.model?.claude), codex: str(raw.model?.codex) },
      effort: { claude: str(raw.effort?.claude), codex: str(raw.effort?.codex) },
      args: { claude: str(raw.args?.claude), codex: str(raw.args?.codex) },
      cwd: str(raw.cwd),
    };
  } catch {
    return DEFAULT_PREFS;
  }
}

export function saveChatPrefs(p: ChatPrefs): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(p));
  } catch {
    // storage unavailable: the choice is just not remembered
  }
}

/** Remembers what was used for `agent` (called after a successful start). */
export function rememberChoice(agent: LiveAgent, permission: PermissionMode, model: string, cwd: string): void {
  const p = loadChatPrefs();
  saveChatPrefs({
    ...p,
    agent,
    permission: { ...p.permission, [agent]: permission },
    model: { ...p.model, [agent]: model },
    cwd,
  });
}

/** One agent's settings as the composer edits them. */
export interface AgentSettings {
  permission: PermissionMode;
  model: string;
  effort: string;
  args: string;
}

export function agentSettings(p: ChatPrefs, agent: LiveAgent): AgentSettings {
  return { permission: p.permission[agent], model: p.model[agent], effort: p.effort[agent], args: p.args[agent] };
}

/** Remembers part of `agent`'s settings, keeping the other prefs; returns the new prefs. */
export function rememberAgentSettings(agent: LiveAgent, patch: Partial<AgentSettings>): ChatPrefs {
  const p = loadChatPrefs();
  const next: ChatPrefs = { ...p };
  if (patch.permission !== undefined) next.permission = { ...p.permission, [agent]: patch.permission };
  if (patch.model !== undefined) next.model = { ...p.model, [agent]: patch.model };
  if (patch.effort !== undefined) next.effort = { ...p.effort, [agent]: patch.effort };
  if (patch.args !== undefined) next.args = { ...p.args, [agent]: patch.args };
  saveChatPrefs(next);
  return next;
}

/** Remembers the last agent and / or project folder. */
export function rememberLast(patch: { agent?: LiveAgent; cwd?: string }): ChatPrefs {
  const next = { ...loadChatPrefs(), ...patch };
  saveChatPrefs(next);
  return next;
}

/**
 * Splits a command line into arguments on whitespace; single or double quotes group
 * (and are removed), a backslash escapes the next character inside double quotes or outside quotes.
 */
export function splitArgs(line: string): string[] {
  const out: string[] = [];
  let cur = "";
  let has = false;
  let quote: '"' | "'" | null = null;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (quote) {
      if (c === quote) quote = null;
      else if (c === "\\" && quote === '"' && (line[i + 1] === '"' || line[i + 1] === "\\")) cur += line[++i];
      else cur += c;
    } else if (c === '"' || c === "'") {
      quote = c;
      has = true;
    } else if (/\s/.test(c)) {
      if (has || cur) out.push(cur);
      cur = "";
      has = false;
    } else if (c === "\\" && (line[i + 1] === '"' || line[i + 1] === "'" || line[i + 1] === " ")) {
      cur += line[++i];
    } else {
      cur += c;
    }
  }
  if (has || cur) out.push(cur);
  return out;
}

/** Extra args for the chosen agent: Claude CLI arguments, or Codex `key=value` overrides (lines or spaces). */
export function parseExtraArgs(agent: LiveAgent, text: string): string[] {
  if (!text.trim()) return [];
  if (agent === "claude") return splitArgs(text);
  return text.split(/\r?\n/).flatMap((l) => splitArgs(l));
}

/** Running or waiting: what the sidebar badge counts. */
export function isActive(state: LiveState): boolean {
  return state === "running" || state === "waiting_approval";
}

/** "just now" / "N min ago" / "N h ago" within a day, else the sessions page format. */
export function relativeTime(
  ms: number,
  labels: { justNow: string; minutes: string; hours: string },
  absolute: (ms: number) => string,
  now: number = Date.now(),
): string {
  const diff = Math.max(0, now - ms);
  if (diff < 60_000) return labels.justNow;
  if (diff < 3_600_000) return labels.minutes.replace("{n}", String(Math.floor(diff / 60_000)));
  if (diff < 86_400_000) return labels.hours.replace("{n}", String(Math.floor(diff / 3_600_000)));
  return absolute(ms);
}
