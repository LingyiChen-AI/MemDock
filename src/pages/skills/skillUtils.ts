import type { CommandError, SkillEntry } from "@/api/types";
import type { Messages } from "@/i18n";

export const entryKey = (e: SkillEntry): string =>
  `${e.scope.type === "global" ? "global" : `project:${e.scope.project_id}`}:${e.id}`;

export function asCommandError(e: unknown): CommandError | null {
  return typeof e === "object" && e !== null && "code" in e && "message" in e ? (e as CommandError) : null;
}

export function errorMessage(e: unknown): string {
  return asCommandError(e)?.message ?? (e instanceof Error ? e.message : String(e));
}

function host(url: string): string {
  try {
    return new URL(url).hostname.toLowerCase();
  } catch {
    const m = /^(?:[\w+.-]+:\/\/)?(?:[^@/]+@)?([^/:]+)/.exec(url);
    return (m?.[1] ?? "").toLowerCase();
  }
}

/** Where the skill comes from, as shown in the source tag. */
export function sourceLabel(e: SkillEntry, s: Messages["skills"]): string {
  if (e.kind === "plugin") return e.plugin ?? s.srcLocal;
  if (e.kind === "codex_system") return s.srcSystem;
  if (e.kind === "claude_synced") return s.srcSynced;
  if (!e.source) return s.srcLocal;
  return host(e.source.url) === "github.com" ? s.srcGithub : s.srcGit;
}

/** Writable first, then by name (case-insensitive), then by path for a stable order. */
export function sortEntries(list: SkillEntry[]): SkillEntry[] {
  return [...list].sort(
    (a, b) =>
      Number(a.readonly) - Number(b.readonly) ||
      a.name.localeCompare(b.name, undefined, { sensitivity: "base" }) ||
      a.dir.localeCompare(b.dir),
  );
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export function shortCommit(c: string | null | undefined): string {
  return c ? c.slice(0, 7) : "";
}
