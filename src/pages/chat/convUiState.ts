import type { AgentSettings } from "./chatPrefs";

/**
 * UI state kept per conversation (module-level, keyed by the conversation key: `draft:<uuid>`,
 * `live:<live_id>` or `<agent>:<native id>`), so switching conversations and back restores it.
 */
export interface ConvUi {
  /** Unsent composer text and attached images. */
  text: string;
  images: string[];
  /** Last scroll position of the message list; null = never scrolled (start at the bottom). */
  scrollTop: number | null;
  /** The list was at the bottom (keeps following new messages). */
  stuck: boolean;
  /** Composer settings chosen in this conversation (null: global prefs / the attached process). */
  settings: Partial<AgentSettings> | null;
}

const EMPTY: ConvUi = { text: "", images: [], scrollTop: null, stuck: true, settings: null };

const map = new Map<string, ConvUi>();

export function getConvUi(key: string | null | undefined): ConvUi {
  return (key && map.get(key)) || EMPTY;
}

export function patchConvUi(key: string | null | undefined, patch: Partial<ConvUi>): void {
  if (!key) return;
  map.set(key, { ...getConvUi(key), ...patch });
}

/** The conversation got a new key (draft started, native id known): carry its state over. */
export function moveConvUi(from: string, to: string): void {
  if (from === to) return;
  const cur = map.get(from);
  if (!cur) return;
  map.delete(from);
  if (!map.has(to)) map.set(to, cur);
}

export function hasConvUi(key: string): boolean {
  return map.has(key);
}

/** Tests. */
export function resetConvUi(): void {
  map.clear();
}

/** A fresh, unique draft key. */
export function newDraftKey(): string {
  const id =
    typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
      ? crypto.randomUUID()
      : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
  return `draft:${id}`;
}
