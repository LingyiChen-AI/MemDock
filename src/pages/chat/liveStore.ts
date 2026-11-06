import * as React from "react";
import {
  liveClose,
  liveList,
  liveStart,
  liveTranscript,
  onLiveEvent,
  type LiveEnvelope,
  type LiveSessionInfo,
  type StartOptions,
} from "@/api/live";
import { errorMessage } from "@/pages/skills/skillUtils";
import { EMPTY_CONV, applyAll, applyLiveEvent, applyTranscript, forceDrain, hasGap, type ConvState } from "./liveModel";

/**
 * Page-independent store of live sessions: lives for the whole app so conversations
 * keep updating (and the sidebar badge stays right) while another page is open.
 */

export interface LiveConv {
  conv: ConvState;
  /** False until the transcript is merged; stream events are buffered meanwhile. */
  loaded: boolean;
  buffer: LiveEnvelope[];
  loadError: string | null;
}

export interface LiveStoreState {
  /** False until the first `liveList` answered (or failed). */
  ready: boolean;
  listError: string | null;
  sessions: LiveSessionInfo[];
  convs: Record<string, LiveConv>;
  selectedId: string | null;
}

const INITIAL: LiveStoreState = { ready: false, listError: null, sessions: [], convs: {}, selectedId: null };

/** Debounce before refetching the transcript to fill a seq gap. */
export const GAP_REFETCH_MS = 250;
/** A session started this recently survives a list that does not have it yet. */
export const RECENT_START_MS = 5000;
/** Refetches that may leave a gap open before the waiting envelopes are applied as they are. */
const MAX_GAP_TRIES = 3;

let state: LiveStoreState = INITIAL;
const listeners = new Set<() => void>();
/** Transcript fetches in flight. */
const loading = new Set<string>();
/** Stream envelopes seen while a transcript fetch was in flight (re-applied on top of it). */
const during = new Map<string, LiveEnvelope[]>();
/** When `startLive` created a session (protects it from a stale list). */
const startedAt = new Map<string, number>();
const gapTimers = new Map<string, ReturnType<typeof setTimeout>>();
const gapTries = new Map<string, number>();
let initCount = 0;
let unlisten: (() => void) | null = null;
let refreshTimer: ReturnType<typeof setTimeout> | null = null;

function set(next: LiveStoreState) {
  if (next === state) return;
  state = next;
  listeners.forEach((l) => l());
}

function update(fn: (s: LiveStoreState) => LiveStoreState) {
  set(fn(state));
}

export function getLiveState(): LiveStoreState {
  return state;
}

export function subscribeLive(l: () => void): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}

const identity = <T,>(x: T) => x;

/**
 * Subscribes to a slice of the store: the component re-renders only when `selector`'s result
 * changes (by `isEqual`, `Object.is` by default). Without a selector, the whole state.
 */
export function useLiveStore(): LiveStoreState;
export function useLiveStore<T>(selector: (s: LiveStoreState) => T, isEqual?: (a: T, b: T) => boolean): T;
export function useLiveStore<T>(selector: (s: LiveStoreState) => T = identity as (s: LiveStoreState) => T, isEqual: (a: T, b: T) => boolean = Object.is): T {
  const memo = React.useRef<{ s: LiveStoreState; sel: (s: LiveStoreState) => T; v: T } | null>(null);
  const get = () => {
    const s = state;
    const prev = memo.current;
    if (prev && prev.s === s && prev.sel === selector) return prev.v;
    const v = selector(s);
    if (prev && isEqual(prev.v, v)) {
      memo.current = { s, sel: selector, v: prev.v };
      return prev.v;
    }
    memo.current = { s, sel: selector, v };
    return v;
  };
  return React.useSyncExternalStore(subscribeLive, get, get);
}

/** One conversation's slice. */
export function useLiveConv(liveId: string | null | undefined): LiveConv | undefined {
  const sel = React.useCallback((s: LiveStoreState) => (liveId ? s.convs[liveId] : undefined), [liveId]);
  return useLiveStore(sel);
}

/** Element-wise `Object.is` (for selectors returning fresh arrays). */
export function shallowArrayEqual<T>(a: readonly T[], b: readonly T[]): boolean {
  return a.length === b.length && a.every((x, i) => Object.is(x, b[i]));
}

/** Same keys with `Object.is` values (for selectors returning fresh objects). */
export function shallowObjectEqual<T extends object>(a: T, b: T): boolean {
  const ka = Object.keys(a) as (keyof T)[];
  return ka.length === Object.keys(b).length && ka.every((k) => Object.is(a[k], b[k]));
}

/** Everything but the conversations: what session lists need (no re-render per streamed token). */
export function selectListSlice(s: LiveStoreState): Omit<LiveStoreState, "convs"> {
  return { ready: s.ready, listError: s.listError, sessions: s.sessions, selectedId: s.selectedId };
}

/** Running or waiting sessions (the sidebar badge). */
export function selectActiveCount(s: LiveStoreState): number {
  return s.sessions.filter((x) => x.state === "running" || x.state === "waiting_approval").length;
}

/** Test helper: back to an empty, uninitialised store. */
export function resetLiveStore() {
  unlisten?.();
  unlisten = null;
  initCount = 0;
  loading.clear();
  during.clear();
  startedAt.clear();
  for (const t of gapTimers.values()) clearTimeout(t);
  gapTimers.clear();
  gapTries.clear();
  if (refreshTimer) clearTimeout(refreshTimer);
  refreshTimer = null;
  set(INITIAL);
}

function baseConv(info?: LiveSessionInfo): ConvState {
  return info ? { ...EMPTY_CONV, state: info.state, native_id: info.native_id, model: info.model } : EMPTY_CONV;
}

function emptyConv(info?: LiveSessionInfo): LiveConv {
  return { conv: baseConv(info), loaded: false, buffer: [], loadError: null };
}

/** The info after `env`; streaming fragments return it unchanged (no list re-render per token). */
function patchInfo(info: LiveSessionInfo, env: LiveEnvelope): LiveSessionInfo {
  const ev = env.event;
  const now = Date.now();
  switch (ev.type) {
    case "assistant_delta":
    case "tool_output":
    case "thinking":
      return info;
    case "status":
      return { ...info, state: ev.state, updated_at: now };
    case "started":
      return { ...info, native_id: ev.native_id, model: ev.model ?? info.model, updated_at: now };
    case "user_message": {
      const line = ev.text.split(/\r?\n/).find((l) => l.trim())?.trim() ?? "";
      return { ...info, title: info.title || line.slice(0, 80), updated_at: now };
    }
    case "exited":
      return { ...info, state: "exited", updated_at: now };
    default:
      return { ...info, updated_at: now };
  }
}

/** Kept although the backend list lacks it: just started, or events are waiting to be applied. */
function protectedId(s: LiveStoreState, id: string, now: number): boolean {
  const t = startedAt.get(id);
  if (t !== undefined && now - t < RECENT_START_MS) return true;
  const c = s.convs[id];
  return loading.has(id) || (!!c && (c.buffer.length > 0 || c.conv.pending.length > 0));
}

/** Merges a list from the backend, keeping conversations and adding empty ones for new sessions. */
function mergeList(list: LiveSessionInfo[]) {
  const now = Date.now();
  update((s) => {
    const ids = new Set(list.map((i) => i.live_id));
    const kept = s.sessions.filter((x) => !ids.has(x.live_id) && protectedId(s, x.live_id, now));
    const sessions = [...list, ...kept];
    const keep = new Set(sessions.map((x) => x.live_id));
    const convs = { ...s.convs };
    for (const info of list) if (!convs[info.live_id]) convs[info.live_id] = emptyConv(info);
    for (const id of Object.keys(convs)) if (!keep.has(id) && !protectedId(s, id, now)) delete convs[id];
    const selectedId = s.selectedId && (keep.has(s.selectedId) || protectedId(s, s.selectedId, now)) ? s.selectedId : null;
    return { ...s, ready: true, listError: null, sessions, convs, selectedId };
  });
  for (const info of list) void ensureTranscript(info.live_id);
}

export async function refreshLiveList(): Promise<void> {
  try {
    mergeList(await liveList());
  } catch (e) {
    update((s) => ({ ...s, ready: true, listError: errorMessage(e) }));
  }
}

function scheduleRefresh() {
  if (refreshTimer) return;
  refreshTimer = setTimeout(() => {
    refreshTimer = null;
    void refreshLiveList();
  }, 150);
}

/**
 * Fetches the transcript and rebuilds the conversation from it (it is authoritative), then
 * re-applies what the stream delivered meanwhile. Used for the first load and to fill gaps.
 */
async function loadTranscript(liveId: string): Promise<void> {
  if (loading.has(liveId)) return;
  loading.add(liveId);
  during.set(liveId, []);
  let envs: LiveEnvelope[] = [];
  let loadError: string | null = null;
  try {
    envs = await liveTranscript(liveId);
  } catch (e) {
    loadError = errorMessage(e);
  }
  loading.delete(liveId);
  const seen = during.get(liveId) ?? [];
  during.delete(liveId);
  update((s) => {
    const cur = s.convs[liveId];
    if (!cur) return s;
    let conv: ConvState;
    if (loadError !== null) {
      conv = cur.loaded ? cur.conv : applyAll(cur.conv, cur.buffer);
    } else {
      conv = applyTranscript(baseConv(s.sessions.find((x) => x.live_id === liveId)), envs);
      for (const env of [...cur.conv.pending, ...cur.buffer, ...seen]) conv = applyLiveEvent(conv, env);
    }
    return { ...s, convs: { ...s.convs, [liveId]: { conv, loaded: true, buffer: [], loadError } } };
  });
  checkGap(liveId);
}

/** Loads the transcript once and replays buffered stream events on top. */
export async function ensureTranscript(liveId: string): Promise<void> {
  const c = state.convs[liveId];
  if ((c && c.loaded) || loading.has(liveId)) return;
  await loadTranscript(liveId);
}

/** A loaded conversation with envelopes behind a missing seq: refetch the transcript (debounced). */
function checkGap(liveId: string) {
  const c = state.convs[liveId];
  if (!c?.loaded || !hasGap(c.conv)) {
    // Filled by a late envelope: no refetch needed.
    const t = gapTimers.get(liveId);
    if (t) clearTimeout(t);
    gapTimers.delete(liveId);
    gapTries.delete(liveId);
    return;
  }
  if (gapTimers.has(liveId) || loading.has(liveId)) return;
  gapTimers.set(
    liveId,
    setTimeout(() => {
      gapTimers.delete(liveId);
      const tries = (gapTries.get(liveId) ?? 0) + 1;
      if (tries > MAX_GAP_TRIES) {
        gapTries.delete(liveId);
        update((s) => {
          const cur = s.convs[liveId];
          return cur ? { ...s, convs: { ...s.convs, [liveId]: { ...cur, conv: forceDrain(cur.conv) } } } : s;
        });
        return;
      }
      gapTries.set(liveId, tries);
      void loadTranscript(liveId);
    }, GAP_REFETCH_MS),
  );
}

export function handleLiveEnvelope(env: LiveEnvelope) {
  const id = env.live_id;
  const known = state.sessions.some((x) => x.live_id === id);
  during.get(id)?.push(env);
  update((s) => {
    const cur = s.convs[id] ?? emptyConv();
    const next: LiveConv = cur.loaded ? { ...cur, conv: applyLiveEvent(cur.conv, env) } : { ...cur, buffer: [...cur.buffer, env] };
    let sessionsChanged = false;
    const sessions = s.sessions.map((x) => {
      if (x.live_id !== id) return x;
      const p = patchInfo(x, env);
      if (p !== x) sessionsChanged = true;
      return p;
    });
    const convChanged = !s.convs[id] || next.conv !== cur.conv || next.buffer !== cur.buffer;
    if (!convChanged && !sessionsChanged) return s;
    return {
      ...s,
      sessions: sessionsChanged ? sessions : s.sessions,
      convs: convChanged ? { ...s.convs, [id]: next } : s.convs,
    };
  });
  if (!known) scheduleRefresh();
  else if (!state.convs[id]?.loaded) void ensureTranscript(id);
  checkGap(id);
}

/** Every few seconds while something runs, the backend's state is checked against ours. */
export const RESYNC_MS = 5000;
let resyncTimer: ReturnType<typeof setInterval> | null = null;

const BUSY: ReadonlySet<string> = new Set(["starting", "running", "waiting_approval"]);

/**
 * Safety net for missed stream events (a dropped listener, a hot reload in development...):
 * when the backend reports a different state for a session we show as busy, rebuild that
 * conversation from its transcript.
 */
export async function resyncLive(): Promise<void> {
  if (!state.sessions.some((x) => BUSY.has(x.state))) return;
  let list: LiveSessionInfo[];
  try {
    list = await liveList();
  } catch {
    return;
  }
  const stale = list
    .filter((info) => {
      const mine = state.sessions.find((x) => x.live_id === info.live_id);
      return mine && mine.state !== info.state;
    })
    .map((info) => info.live_id);
  if (!stale.length) return;
  mergeList(list);
  // Overwrite the stale state with the backend's, then rebuild from the transcript.
  update((s) => ({
    ...s,
    sessions: s.sessions.map((x) => list.find((i) => i.live_id === x.live_id) ?? x),
  }));
  for (const id of stale) void loadTranscript(id);
}

/** Starts listening (ref-counted); returns the matching stop. Safe without Tauri (errors are kept). */
export function initLiveStore(): () => void {
  initCount++;
  if (initCount === 1) {
    void refreshLiveList();
    onLiveEvent(handleLiveEnvelope)
      .then((un) => {
        if (initCount > 0) unlisten = un;
        else un();
      })
      .catch(() => {});
    resyncTimer = setInterval(() => void resyncLive(), RESYNC_MS);
  }
  return () => {
    initCount--;
    if (initCount === 0) {
      unlisten?.();
      unlisten = null;
      if (resyncTimer) clearInterval(resyncTimer);
      resyncTimer = null;
    }
  };
}

// Development hot reload replaces this module (and its state): listen again right away.
if (import.meta.hot) {
  import.meta.hot.dispose((data: { initCount?: number }) => {
    data.initCount = initCount;
    unlisten?.();
    unlisten = null;
    if (resyncTimer) clearInterval(resyncTimer);
    resyncTimer = null;
  });
  if ((import.meta.hot.data as { initCount?: number }).initCount) initLiveStore();
}

export function selectLive(liveId: string | null) {
  update((s) => (s.selectedId === liveId ? s : { ...s, selectedId: liveId }));
  if (liveId) void ensureTranscript(liveId);
}

/**
 * Starts a session, adds it to the list and selects it. With `resume_id` already driven by a GUI
 * process the backend returns that one (no second process); a terminal owner fails with `busy_pty:`.
 */
export async function startLive(opts: StartOptions, onStarted?: (info: LiveSessionInfo) => void): Promise<LiveSessionInfo> {
  const info = await liveStart(opts);
  startedAt.set(info.live_id, Date.now());
  // Before the selection moves: e.g. carry a draft's UI state over to the new key.
  onStarted?.(info);
  update((s) => ({
    ...s,
    sessions: s.sessions.some((x) => x.live_id === info.live_id) ? s.sessions : [info, ...s.sessions],
    convs: s.convs[info.live_id] ? s.convs : { ...s.convs, [info.live_id]: emptyConv(info) },
    selectedId: info.live_id,
  }));
  void ensureTranscript(info.live_id);
  return info;
}

/** Ends the process and drops the session from the list. */
export async function closeLive(liveId: string): Promise<void> {
  await liveClose(liveId);
  startedAt.delete(liveId);
  update((s) => {
    const convs = { ...s.convs };
    delete convs[liveId];
    const sessions = s.sessions.filter((x) => x.live_id !== liveId);
    return { ...s, sessions, convs, selectedId: s.selectedId === liveId ? (sessions[0]?.live_id ?? null) : s.selectedId };
  });
}

/** Test helper: how many transcript fetches are in flight. */
export function transcriptsLoading(): number {
  return loading.size;
}
