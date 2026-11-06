// Preferences sync (memdock_settings.json): which localStorage keys travel, how a cloud copy
// compares with this computer, and how its frontend part is written back.
import type { ExtractConfig, PrefsFile, Settings } from "@/api/types";

export type PrefKind = "json" | "string";

/** localStorage keys that are synced. Layout widths, window / chat / terminal state,
 *  the theme mirror keys and hidden projects (keyed by local path) stay on this computer. */
export const SYNCED_KEYS: { key: string; kind: PrefKind }[] = [
  { key: "memdock.chat.prefs", kind: "json" },
  { key: "memdock.chat.newChatDir", kind: "json" },
  { key: "memdock.chat.mode", kind: "string" },
  { key: "memdock.nav.collapsed", kind: "json" },
  { key: "memdock.diffLayout", kind: "string" },
  { key: "memdock.editorMode", kind: "string" },
  { key: "memdock.syncScroll", kind: "string" },
  { key: "memdock.showEmptyProjects", kind: "string" },
  { key: "memdock.showHiddenProjects", kind: "string" },
];

/** What each key means while it is not stored (as the pages that own them read it). */
const DEFAULTS: Record<string, unknown> = {
  "memdock.chat.prefs": {
    agent: "claude",
    permission: { claude: "ask", codex: "ask" },
    model: { claude: "", codex: "" },
    effort: { claude: "", codex: "" },
    args: { claude: "", codex: "" },
  },
  "memdock.chat.newChatDir": { mode: "follow", fixed: "" },
  "memdock.chat.mode": "gui",
  "memdock.nav.collapsed": [],
  "memdock.diffLayout": "split",
  "memdock.editorMode": "split",
  "memdock.syncScroll": "1",
  "memdock.showEmptyProjects": "0",
  "memdock.showHiddenProjects": "0",
};

/** "1" / "0" flags shown as on / off. */
export const FLAG_KEYS = new Set(["memdock.syncScroll", "memdock.showEmptyProjects", "memdock.showHiddenProjects"]);

type Obj = Record<string, unknown>;

function isObj(v: unknown): v is Obj {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function readRaw(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

/** Parsed localStorage value; `undefined` when absent or unreadable. */
function readValue(key: string, kind: PrefKind): unknown {
  const raw = readRaw(key);
  if (raw === null) return undefined;
  if (kind === "string") return raw;
  try {
    return JSON.parse(raw) as unknown;
  } catch {
    return undefined;
  }
}

/** Drops the fields that hold local paths (the backend strips them again). */
function portable(key: string, v: unknown): unknown {
  if (key === "memdock.chat.prefs" && isObj(v)) {
    const { cwd: _cwd, ...rest } = v;
    return rest;
  }
  if (key === "memdock.chat.newChatDir" && isObj(v)) return { mode: v.mode };
  return v;
}

/** This computer's synced localStorage part (sent to the backend on upload). */
export function collectFrontendPrefs(): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const { key, kind } of SYNCED_KEYS) {
    const v = readValue(key, kind);
    if (v !== undefined && v !== null) out[key] = portable(key, v);
  }
  return out;
}

/** The stored value, or what the app uses while none is stored. */
function localOrDefault(key: string, kind: PrefKind): unknown {
  return readValue(key, kind) ?? DEFAULTS[key];
}

/** The value `key` gets on this computer when the cloud value `remote` is applied. */
function nextValue(key: string, kind: PrefKind, remote: unknown): unknown {
  const local = localOrDefault(key, kind);
  if (key === "memdock.chat.prefs" && isObj(remote)) {
    const base = isObj(local) ? local : {};
    const { cwd: _cwd, ...rest } = remote;
    return { ...base, ...rest, ...(typeof base.cwd === "string" ? { cwd: base.cwd } : {}) };
  }
  if (key === "memdock.chat.newChatDir" && isObj(remote)) {
    const fixed = isObj(local) && typeof local.fixed === "string" ? local.fixed : "";
    let mode = remote.mode === "memdock" || remote.mode === "fixed" ? remote.mode : "follow";
    // A fixed folder is a local path: without one here, follow the conversation instead.
    if (mode === "fixed" && !fixed.trim()) mode = "follow";
    return { mode, fixed };
  }
  if (kind === "string" && typeof remote !== "string") return local;
  return remote;
}

/** Leaf values by dotted path (arrays are leaves). */
function flatten(v: unknown, prefix = ""): [string, unknown][] {
  if (isObj(v)) return Object.keys(v).flatMap((k) => flatten(v[k], prefix ? `${prefix}.${k}` : k));
  return [[prefix, v]];
}

const same = (a: unknown, b: unknown) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);

export type ChangeGroup = "backend" | "extract" | "frontend";

export interface PrefChange {
  group: ChangeGroup;
  /** Settings field, extract field, or localStorage key. */
  key: string;
  /** Sub-path inside a localStorage object value ("" for the whole value). */
  path: string;
  from: unknown;
  to: unknown;
}

export interface LocalPrefs {
  settings: Settings;
  extract: ExtractConfig | null;
}

const BACKEND_FIELDS = ["language", "theme", "accent", "claude_quota_enabled", "codex_quota_enabled"] as const;

/** What applying `file` would change on this computer. */
export function diffPrefs(file: PrefsFile, local: LocalPrefs): PrefChange[] {
  const out: PrefChange[] = [];
  const be = file.backend ?? ({} as PrefsFile["backend"]);
  for (const f of BACKEND_FIELDS) {
    if (!(f in be)) continue;
    const from = f.endsWith("_enabled") ? Boolean(local.settings[f]) : (local.settings[f] ?? null);
    const to = f.endsWith("_enabled") ? Boolean(be[f]) : (be[f] ?? null);
    if (!same(from, to)) out.push({ group: "backend", key: f, path: "", from, to });
  }
  if (be.extract && local.extract) {
    for (const [k, to] of Object.entries(be.extract)) {
      if (k === "projects") continue;
      const from = (local.extract as unknown as Obj)[k];
      if (!same(from, to)) out.push({ group: "extract", key: k, path: "", from: from ?? null, to: to ?? null });
    }
  }
  for (const { key, kind } of SYNCED_KEYS) {
    if (!(key in (file.frontend ?? {}))) continue;
    const before = portable(key, localOrDefault(key, kind));
    const after = portable(key, nextValue(key, kind, file.frontend[key]));
    const a = new Map(flatten(before));
    for (const [path, to] of flatten(after)) {
      const from = a.get(path);
      if (!same(from, to)) out.push({ group: "frontend", key, path, from: from ?? null, to });
    }
  }
  return out;
}

/** Writes the cloud frontend values to localStorage; returns whether anything changed. */
export function applyFrontendPrefs(frontend: Record<string, unknown>): boolean {
  let changed = false;
  for (const { key, kind } of SYNCED_KEYS) {
    if (!(key in frontend)) continue;
    const v = nextValue(key, kind, frontend[key]);
    if (v === undefined || v === null) continue;
    const text = kind === "string" ? String(v) : JSON.stringify(v);
    if (readRaw(key) === text) continue;
    // Not stored and the cloud value is the default: leave it unset.
    if (readRaw(key) === null && same(v, DEFAULTS[key])) continue;
    try {
      localStorage.setItem(key, text);
      changed = true;
    } catch {
      // storage unavailable: nothing to apply
    }
  }
  return changed;
}

/** The local extraction config with the cloud's portable fields (project selection stays local). */
export function mergeExtract(local: ExtractConfig, remote: PrefsFile["backend"]["extract"]): ExtractConfig {
  if (!remote) return local;
  const { projects: _p, ...rest } = remote as Obj;
  return { ...local, ...(rest as Partial<ExtractConfig>), projects: local.projects };
}
