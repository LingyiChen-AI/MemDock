import type { Messages } from "@/i18n";
import { fmt } from "@/pages/memory/paths";
import { asCommandError } from "@/pages/skills/skillUtils";

/** Localized text for any sync command failure (the raw message fills `{msg}`). */
export function syncErrorText(t: Messages, e: unknown): string {
  const ce = asCommandError(e);
  const raw = ce?.message ?? (e instanceof Error ? e.message : String(e));
  const map = t.syncErrors as Record<string, string>;
  const template = ce && map[ce.code] ? map[ce.code] : map.other;
  return fmt(template, { msg: raw });
}
