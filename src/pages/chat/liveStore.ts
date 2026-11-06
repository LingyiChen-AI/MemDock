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
import { EMPTY_CONV, applyAll, applyLiveEvent, type ConvState } from "./liveModel";

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

let state: LiveStoreState = INITIAL;
const listeners = new Set<() => void>();
const loading = new Set<string>();
let initCount = 0;
let unlisten: (() => void) | null = null;
let refreshTimer: ReturnType<typeof setTimeout> | null = null;

function set(next: LiveStoreState) {
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

export function useLiveStore(): LiveStoreState {
  return React.useSyncExternalStore(subscribeLive, getLiveState, getLiveState);
}

/** Test helper: back to an empty, uninitialised store. */
export function resetLiveStore() {
  unlisten?.();
  unlisten = null;
  initCount = 0;
  loading.clear();
  if (refreshTimer) clearTimeout(refreshTimer);
  refreshTimer = null;
  set(INITIAL);
}

function emptyConv(info?: LiveSessionInfo): LiveConv {
  return {
    conv: info ? { ...EMPTY_CONV, state: info.state, native_id: info.native_id, model: info.model } : EMPTY_CONV,
    loaded: false,
    buffer: [],
    loadError: null,
  };
}

function patchInfo(info: LiveSessionInfo, env: LiveEnvelope): LiveSessionInfo {
  const ev = env.event;
  const now = Date.now();
  switch (ev.type) {
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

/** Merges a list from the backend, keeping conversations and adding empty ones for new sessions. */
function mergeList(list: LiveSessionInfo[]) {
  update((s) => {
    const convs = { ...s.convs };
    for (const info of list) if (!convs[info.live_id]) convs[info.live_id] = emptyConv(info);
    const ids = new Set(list.map((i) => i.live_id));
    for (const id of Object.keys(convs)) if (!ids.has(id)) delete convs[id];
    const selectedId = s.selectedId && ids.has(s.selectedId) ? s.selectedId : null;
    return { ...s, ready: true, listError: null, sessions: list, convs, selectedId };
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

/** Loads the transcript once and replays buffered stream events on top. */
export async function ensureTranscript(liveId: string): Promise<void> {
  const c = state.convs[liveId];
  if ((c && c.loaded) || loading.has(liveId)) return;
  loading.add(liveId);
  let envs: LiveEnvelope[] = [];
  let loadError: string | null = null;
  try {
    envs = await liveTranscript(liveId);
  } catch (e) {
    loadError = errorMessage(e);
  } finally {
    loading.delete(liveId);
  }
  update((s) => {
    const cur = s.convs[liveId];
    if (!cur) return s;
    const conv = applyAll(cur.conv, [...envs, ...cur.buffer]);
    return { ...s, convs: { ...s.convs, [liveId]: { conv, loaded: true, buffer: [], loadError } } };
  });
}

export function handleLiveEnvelope(env: LiveEnvelope) {
  const known = state.sessions.some((x) => x.live_id === env.live_id);
  update((s) => {
    const cur = s.convs[env.live_id] ?? emptyConv();
    const next: LiveConv = cur.loaded ? { ...cur, conv: applyLiveEvent(cur.conv, env) } : { ...cur, buffer: [...cur.buffer, env] };
    return {
      ...s,
      sessions: s.sessions.map((x) => (x.live_id === env.live_id ? patchInfo(x, env) : x)),
      convs: { ...s.convs, [env.live_id]: next },
    };
  });
  if (!known) scheduleRefresh();
  else if (!state.convs[env.live_id]?.loaded) void ensureTranscript(env.live_id);
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
  }
  return () => {
    initCount--;
    if (initCount === 0) {
      unlisten?.();
      unlisten = null;
    }
  };
}

export function selectLive(liveId: string | null) {
  update((s) => ({ ...s, selectedId: liveId }));
  if (liveId) void ensureTranscript(liveId);
}

/** Starts a session, adds it to the list and selects it. */
export async function startLive(opts: StartOptions): Promise<LiveSessionInfo> {
  const info = await liveStart(opts);
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
  update((s) => {
    const convs = { ...s.convs };
    delete convs[liveId];
    const sessions = s.sessions.filter((x) => x.live_id !== liveId);
    return { ...s, sessions, convs, selectedId: s.selectedId === liveId ? (sessions[0]?.live_id ?? null) : s.selectedId };
  });
}
