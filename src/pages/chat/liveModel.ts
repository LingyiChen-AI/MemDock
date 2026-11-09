import type { Decision, LiveEnvelope, LiveState, LiveToolKind, TurnUsage } from "@/api/live";

/**
 * View model of one live conversation, built by folding `live://event` envelopes
 * (transcript first, then the stream) through `applyLiveEvent`. Pure: no I/O.
 */

export interface LiveToolCall {
  id: string;
  name: string;
  kind: LiveToolKind;
  summary: string;
  input: string;
  /** Streamed output so far, replaced by the final output when the tool finishes. */
  output: string;
  finished: boolean;
  is_error: boolean;
}

export interface LiveApproval {
  tool_id?: string;
  request_id: string;
  tool_name: string;
  kind: LiveToolKind;
  summary: string;
  detail: string;
  can_allow_session: boolean;
  /** null while the agent is waiting for the user. */
  decision: Decision | null;
  /** Withdrawn without a decision (turn interrupted or ended, process gone). */
  expired: boolean;
}

export type ConvItem =
  | { type: "user"; id: string; text: string; images?: string[] }
  | { type: "assistant"; id: string; text: string; streaming: boolean }
  | { type: "thinking"; id: string; text: string }
  | { type: "tool"; call: LiveToolCall }
  | { type: "approval"; approval: LiveApproval }
  | { type: "error"; message: string; fatal: boolean }
  | { type: "exited"; code: number | null }
  | { type: "notice"; id: string; kind: string; text: string; before: number | null; after: number | null };

export interface ConvState {
  /** Highest seq applied; envelopes at or below it are duplicates. */
  lastSeq: number;
  items: ConvItem[];
  state: LiveState;
  native_id: string | null;
  model: string | null;
  lastUsage: TurnUsage | null;
  /**
   * Envelopes that arrived ahead of a missing seq (sorted, all > lastSeq + 1). Non-empty means a gap:
   * the store refetches the transcript to fill it. Never dropped silently.
   */
  pending: LiveEnvelope[];
}

export const EMPTY_CONV: ConvState = { lastSeq: 0, items: [], state: "starting", native_id: null, model: null, lastUsage: null, pending: [] };

function findIndex(items: ConvItem[], pred: (it: ConvItem) => boolean): number {
  for (let i = items.length - 1; i >= 0; i--) if (pred(items[i])) return i;
  return -1;
}

function replaceAt(items: ConvItem[], i: number, it: ConvItem): ConvItem[] {
  const next = items.slice();
  next[i] = it;
  return next;
}

/** Marks still-pending approvals as void: the turn or process they belonged to is over. */
function expire(items: ConvItem[]): ConvItem[] {
  if (!items.some((it) => it.type === "approval" && isPendingApproval(it.approval))) return items;
  return items.map((it) =>
    it.type === "approval" && isPendingApproval(it.approval) ? { ...it, approval: { ...it.approval, expired: true } } : it,
  );
}

/** Ends every streaming assistant message (turn over, interrupted or exited). */
function settle(items: ConvItem[]): ConvItem[] {
  if (!items.some((it) => it.type === "assistant" && it.streaming)) return items;
  return items.map((it) => (it.type === "assistant" && it.streaming ? { ...it, streaming: false } : it));
}

/**
 * Applies one stream envelope in seq order: duplicates are skipped, an envelope ahead of a
 * missing seq waits in `pending` (see `hasGap`) and is applied once the gap is filled.
 */
export function applyLiveEvent(state: ConvState, env: LiveEnvelope): ConvState {
  if (env.seq <= state.lastSeq) return state;
  if (env.seq > state.lastSeq + 1) {
    if (state.pending.some((p) => p.seq === env.seq)) return state;
    return { ...state, pending: [...state.pending, env].sort((a, b) => a.seq - b.seq) };
  }
  return drain(fold(state, env));
}

/** Applies pending envelopes that are now contiguous. */
function drain(state: ConvState): ConvState {
  let s = state;
  while (s.pending.length && s.pending[0].seq <= s.lastSeq + 1) {
    const [head, ...rest] = s.pending;
    s = { ...s, pending: rest };
    if (head.seq > s.lastSeq) s = fold(s, head);
  }
  return s;
}

/** True while envelopes wait behind a missing seq. */
export function hasGap(state: ConvState): boolean {
  return state.pending.length > 0;
}

/** Gives up on a gap: applies the pending envelopes as they are (after repeated refetches did not fill it). */
export function forceDrain(state: ConvState): ConvState {
  return state.pending.reduce((s, env) => (env.seq > s.lastSeq ? fold(s, env) : s), { ...state, pending: [] as LiveEnvelope[] });
}

/**
 * Folds a transcript: authoritative and possibly with gaps (the backend compacts streamed
 * deltas), so every envelope above `lastSeq` is applied in seq order regardless of gaps.
 */
export function applyTranscript(state: ConvState, envs: LiveEnvelope[]): ConvState {
  let s = state;
  for (const env of [...envs].sort((a, b) => a.seq - b.seq)) if (env.seq > s.lastSeq) s = fold(s, env);
  return drain({ ...s, pending: s.pending.filter((p) => p.seq > s.lastSeq) });
}

function fold(state: ConvState, env: LiveEnvelope): ConvState {
  const s: ConvState = { ...state, lastSeq: env.seq };
  const ev = env.event;
  const items = s.items;
  switch (ev.type) {
    case "started":
      // Sent again once the CLI resolves the full model name: only the model changes.
      return { ...s, native_id: ev.native_id, model: ev.model ?? s.model };
    case "status":
      return {
        ...s,
        state: ev.state,
        items: ev.state === "exited" ? expire(settle(items)) : ev.state === "running" || ev.state === "waiting_approval" ? items : settle(items),
      };
    case "user_message":
      return { ...s, items: [...items, { type: "user", id: ev.id, text: ev.text, images: ev.images?.length ? ev.images : undefined }] };
    case "assistant_delta": {
      const i = findIndex(items, (it) => it.type === "assistant" && it.id === ev.id);
      if (i < 0) return { ...s, items: [...items, { type: "assistant", id: ev.id, text: ev.text, streaming: true }] };
      const cur = items[i] as Extract<ConvItem, { type: "assistant" }>;
      return { ...s, items: replaceAt(items, i, { ...cur, text: cur.text + ev.text, streaming: true }) };
    }
    case "assistant_message": {
      const i = findIndex(items, (it) => it.type === "assistant" && it.id === ev.id);
      const it: ConvItem = { type: "assistant", id: ev.id, text: ev.text, streaming: false };
      return { ...s, items: i < 0 ? [...items, it] : replaceAt(items, i, it) };
    }
    case "thinking": {
      // A thinking block arrives whole; a repeat with the same id replaces it.
      const i = findIndex(items, (it) => it.type === "thinking" && it.id === ev.id);
      const it: ConvItem = { type: "thinking", id: ev.id, text: ev.text };
      return { ...s, items: i < 0 ? [...items, it] : replaceAt(items, i, it) };
    }
    case "tool_started": {
      const call: LiveToolCall = {
        id: ev.id,
        name: ev.name,
        kind: ev.kind,
        summary: ev.summary,
        input: ev.input,
        output: "",
        finished: false,
        is_error: false,
      };
      const i = findIndex(items, (it) => it.type === "tool" && it.call.id === ev.id);
      if (i >= 0) {
        const prev = (items[i] as Extract<ConvItem, { type: "tool" }>).call;
        return { ...s, items: replaceAt(items, i, { type: "tool", call: { ...prev, ...call, output: prev.output, finished: prev.finished, is_error: prev.is_error } }) };
      }
      return { ...s, items: [...items, { type: "tool", call }] };
    }
    case "tool_output": {
      const i = findIndex(items, (it) => it.type === "tool" && it.call.id === ev.id);
      if (i < 0) return s;
      const call = (items[i] as Extract<ConvItem, { type: "tool" }>).call;
      return { ...s, items: replaceAt(items, i, { type: "tool", call: { ...call, output: call.output + ev.text } }) };
    }
    case "tool_finished": {
      const i = findIndex(items, (it) => it.type === "tool" && it.call.id === ev.id);
      if (i < 0) {
        const call: LiveToolCall = { id: ev.id, name: "tool", kind: "other", summary: "", input: "", output: ev.output ?? "", finished: true, is_error: ev.is_error };
        return { ...s, items: [...items, { type: "tool", call }] };
      }
      const call = (items[i] as Extract<ConvItem, { type: "tool" }>).call;
      return {
        ...s,
        items: replaceAt(items, i, { type: "tool", call: { ...call, output: ev.output ?? call.output, finished: true, is_error: ev.is_error } }),
      };
    }
    case "approval_request": {
      const approval: LiveApproval = {
        request_id: ev.request_id,
        tool_id: ev.tool_id,
        tool_name: ev.tool_name,
        kind: ev.kind,
        summary: ev.summary,
        detail: ev.detail,
        can_allow_session: ev.can_allow_session,
        decision: null,
        expired: false,
      };
      return { ...s, items: [...items, { type: "approval", approval }] };
    }
    case "approval_resolved": {
      const i = findIndex(items, (it) => it.type === "approval" && it.approval.request_id === ev.request_id);
      if (i < 0) return s;
      const a = (items[i] as Extract<ConvItem, { type: "approval" }>).approval;
      return { ...s, items: replaceAt(items, i, { type: "approval", approval: { ...a, decision: ev.decision, expired: false } }) };
    }
    case "turn_completed":
      return {
        ...s,
        lastUsage: ev.usage,
        // A "compacting" notice can't outlive its turn.
        items: expire(settle(items)).filter((it) => !(it.type === "notice" && it.kind === "compacting")),
      };
    case "error":
      return { ...s, items: [...(ev.fatal ? expire(settle(items)) : items), { type: "error", message: ev.message, fatal: ev.fatal }] };
    case "notice": {
      const n = { type: "notice" as const, id: ev.id, kind: ev.kind, text: ev.text ?? "", before: ev.before ?? null, after: ev.after ?? null };
      const at = items.findIndex((it) => it.type === "notice" && it.id === ev.id);
      let next = at >= 0 ? replaceAt(items, at, n) : [...items, n];
      // A finished compaction settles any other "compacting" notice still on screen.
      if (ev.kind === "compacted") next = next.filter((it) => !(it.type === "notice" && it.kind === "compacting" && it.id !== ev.id));
      return { ...s, items: next };
    }
    case "exited":
      // The manager dedupes exits; render one marker even if a repeat slips through.
      if (s.state === "exited" && items[items.length - 1]?.type === "exited") return s;
      return { ...s, state: "exited", items: [...expire(settle(items)), { type: "exited", code: ev.code }] };
    default:
      return s;
  }
}

/** Applies envelopes in seq order (duplicates and already-applied ones are skipped). */
export function applyAll(state: ConvState, envs: LiveEnvelope[]): ConvState {
  return [...envs].sort((a, b) => a.seq - b.seq).reduce(applyLiveEvent, state);
}

/** Approval requests still waiting for a decision. */
export function isPendingApproval(approval: LiveApproval): boolean {
  return approval.decision === null && !approval.expired;
}

export function pendingApprovals(state: ConvState): LiveApproval[] {
  return state.items.flatMap((it) => (it.type === "approval" && isPendingApproval(it.approval) ? [it.approval] : []));
}

/** Rendering groups: user messages, assistant turns (one card), notices stand alone. */
export type TurnSegment =
  | { kind: "text"; id: string; text: string; streaming: boolean }
  | { kind: "thinking"; id: string; text: string }
  | { kind: "tools"; calls: LiveToolCall[]; approvals?: Map<string, LiveApproval[]> }
  | { kind: "approval"; approval: LiveApproval }
  | { kind: "error"; message: string };

export type ConvBlock =
  | { kind: "user"; key: string; text: string; images?: string[] }
  | { kind: "turn"; key: string; segments: TurnSegment[] }
  | { kind: "fatal"; key: string; message: string }
  | { kind: "exited"; key: string; code: number | null }
  | { kind: "notice"; key: string; notice: { kind: string; text: string; before: number | null; after: number | null } };

export function toConvBlocks(items: ConvItem[]): ConvBlock[] {
  const out: ConvBlock[] = [];
  let turn: Extract<ConvBlock, { kind: "turn" }> | null = null;
  const openTurn = (key: string) => {
    if (!turn) {
      turn = { kind: "turn", key, segments: [] };
      out.push(turn);
    }
    return turn.segments;
  };
  items.forEach((it, i) => {
    const key = `${i}`;
    switch (it.type) {
      case "user":
        turn = null;
        out.push({ kind: "user", key: `u-${it.id}-${i}`, text: it.text, images: it.images });
        return;
      case "approval":
        openTurn(key).push({ kind: "approval", approval: it.approval });
        return;
      case "exited":
        turn = null;
        out.push({ kind: "exited", key: `x-${i}`, code: it.code });
        return;
      case "notice":
        turn = null;
        out.push({ kind: "notice", key: `n-${it.id}`, notice: { kind: it.kind, text: it.text, before: it.before, after: it.after } });
        return;
      case "error":
        if (it.fatal) {
          turn = null;
          out.push({ kind: "fatal", key: `f-${i}`, message: it.message });
        } else {
          openTurn(key).push({ kind: "error", message: it.message });
        }
        return;
      case "assistant":
        openTurn(key).push({ kind: "text", id: it.id, text: it.text, streaming: it.streaming });
        return;
      case "thinking":
        openTurn(key).push({ kind: "thinking", id: it.id, text: it.text });
        return;
      case "tool": {
        const segs = openTurn(key);
        const last = segs[segs.length - 1];
        if (last?.kind === "tools") last.calls.push(it.call);
        else segs.push({ kind: "tools", calls: [it.call] });
        return;
      }
    }
  });
  for (const block of out) {
    if (block.kind !== "turn") continue;
    const runs = block.segments.filter((seg) => seg.kind === "tools");
    const tools = runs.flatMap((run) => run.calls.map((call) => ({ run, call })));
    block.segments = block.segments.filter((seg) => {
      if (seg.kind !== "approval") return true;
      const a = seg.approval;
      if (isPendingApproval(a)) return true;
      const matches = tools.filter(({ call }) => a.tool_id
        ? call.id === a.tool_id
        : a.summary !== "" && call.name === a.tool_name && call.kind === a.kind && call.summary === a.summary);
      if (matches.length !== 1) return true;
      const { run, call } = matches[0];
      const approvals = run.approvals ??= new Map();
      approvals.set(call.id, [...(approvals.get(call.id) ?? []), a]);
      return false;
    });
  }
  return out;
}
