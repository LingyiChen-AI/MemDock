// Where a new chat (sidebar / header "新对话", Ctrl+N) starts. The project "＋ 新建会话" always uses its project.
import { homeDir } from "@tauri-apps/api/path";
import { memdockWorkspaceDir } from "@/api/live";
import { parseWorktreePath } from "./worktreeStore";

/** follow: the open conversation's project, else the home folder · memdock: ~/memdock · fixed: a chosen folder. */
export type NewChatDirMode = "follow" | "memdock" | "fixed";

export interface NewChatDirSetting {
  mode: NewChatDirMode;
  /** The folder for `fixed`. */
  fixed: string;
}

const KEY = "memdock.chat.newChatDir";
export const DEFAULT_NEW_CHAT_DIR: NewChatDirSetting = { mode: "follow", fixed: "" };

export function loadNewChatDir(): NewChatDirSetting {
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) ?? "null") as Partial<NewChatDirSetting> | null;
    if (!raw || typeof raw !== "object") return DEFAULT_NEW_CHAT_DIR;
    const mode: NewChatDirMode = raw.mode === "memdock" || raw.mode === "fixed" ? raw.mode : "follow";
    return { mode, fixed: typeof raw.fixed === "string" ? raw.fixed : "" };
  } catch {
    return DEFAULT_NEW_CHAT_DIR;
  }
}

export function saveNewChatDir(v: NewChatDirSetting): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(v));
  } catch {
    // storage unavailable: the choice is just not remembered
  }
}

/**
 * The folder for a new chat. `current` is the open conversation's folder (a worktree maps to its repository);
 * anything that can't be resolved falls back to the home folder, then to `fallback`.
 */
export async function resolveNewChatDir(current: string | null, fallback = ""): Promise<string> {
  const s = loadNewChatDir();
  const home = () => homeDir().catch(() => fallback);
  if (s.mode === "fixed" && s.fixed.trim()) return s.fixed.trim();
  if (s.mode === "memdock") return memdockWorkspaceDir().catch(home);
  if (s.mode === "follow" && current) return parseWorktreePath(current)?.repoRoot ?? current;
  return home();
}
