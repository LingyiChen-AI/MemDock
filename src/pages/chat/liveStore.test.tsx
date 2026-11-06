import { act, render } from "@testing-library/react";
import { vi, type Mock } from "vitest";
import type { LiveEnvelope, LiveEvent, LiveSessionInfo } from "@/api/live";

vi.mock("@/api/live", () => ({
  liveList: vi.fn(),
  liveStart: vi.fn(),
  liveClose: vi.fn(),
  liveTranscript: vi.fn(),
  onLiveEvent: vi.fn().mockResolvedValue(() => {}),
}));

import { liveList, liveStart, liveTranscript } from "@/api/live";
import { EMPTY_CONV, applyLiveEvent, applyTranscript, hasGap } from "./liveModel";
import {
  GAP_REFETCH_MS,
  RECENT_START_MS,
  getLiveState,
  handleLiveEnvelope,
  refreshLiveList,
  resetLiveStore,
  selectActiveCount,
  selectListSlice,
  shallowObjectEqual,
  startLive,
  useLiveConv,
  useLiveStore,
} from "./liveStore";

function info(over: Partial<LiveSessionInfo> = {}): LiveSessionInfo {
  return {
    live_id: "L1",
    agent: "claude",
    cwd: "/w",
    title: "t",
    state: "idle",
    native_id: "n1",
    model: null,
    permission: "ask",
    created_at: 1,
    updated_at: 1,
    ...over,
  };
}

const env = (seq: number, event: LiveEvent, live_id = "L1"): LiveEnvelope => ({ live_id, seq, event });
const delta = (seq: number, text: string, live_id = "L1") => env(seq, { type: "assistant_delta", id: "a1", text }, live_id);

const flush = () => act(async () => {
  await Promise.resolve();
  await Promise.resolve();
});

describe("liveModel gaps", () => {
  it("holds an envelope ahead of a missing seq and applies it once the gap is filled", () => {
    let s = applyLiveEvent(EMPTY_CONV, env(1, { type: "user_message", id: "u1", text: "hi" }));
    s = applyLiveEvent(s, delta(3, "lo"));
    expect(hasGap(s)).toBe(true);
    expect(s.lastSeq).toBe(1);
    s = applyLiveEvent(s, delta(2, "Hel"));
    expect(hasGap(s)).toBe(false);
    expect(s.lastSeq).toBe(3);
    expect(s.items[1]).toMatchObject({ type: "assistant", text: "Hello" });
  });

  it("a transcript with compaction gaps is authoritative", () => {
    const s = applyTranscript(EMPTY_CONV, [
      env(1, { type: "user_message", id: "u1", text: "hi" }),
      env(5, { type: "assistant_message", id: "a1", text: "Hello" }),
      env(9, { type: "turn_completed", usage: { input_tokens: 1, output_tokens: 1, cost_usd: null, duration_ms: null }, is_error: false }),
    ]);
    expect(s.lastSeq).toBe(9);
    expect(hasGap(s)).toBe(false);
    expect(s.items).toHaveLength(2);
  });
});

describe("liveStore", () => {
  beforeEach(() => {
    vi.useRealTimers();
    resetLiveStore();
    vi.clearAllMocks();
  });

  it("out-of-order delivery is reordered; a real gap refetches the transcript and nothing is lost", async () => {
    (liveList as Mock).mockResolvedValue([info()]);
    (liveTranscript as Mock).mockResolvedValueOnce([env(1, { type: "user_message", id: "u1", text: "hi" })]);
    await refreshLiveList();
    await flush();
    expect(getLiveState().convs.L1.loaded).toBe(true);

    // 3 overtakes 2: buffered, then both applied in order.
    handleLiveEnvelope(delta(3, "lo"));
    expect(getLiveState().convs.L1.conv.lastSeq).toBe(1);
    handleLiveEnvelope(delta(2, "Hel"));
    expect(getLiveState().convs.L1.conv.items[1]).toMatchObject({ text: "Hello" });

    // 4 never arrives: 5 waits, the transcript is refetched (debounced, once) and fills it.
    vi.useFakeTimers();
    (liveTranscript as Mock).mockResolvedValueOnce([
      env(1, { type: "user_message", id: "u1", text: "hi" }),
      env(4, { type: "assistant_message", id: "a1", text: "Hello!" }),
      env(5, { type: "tool_started", id: "t1", name: "Bash", kind: "shell", summary: "ls", input: "{}" }),
    ]);
    handleLiveEnvelope(env(5, { type: "tool_started", id: "t1", name: "Bash", kind: "shell", summary: "ls", input: "{}" }));
    handleLiveEnvelope(env(6, { type: "tool_output", id: "t1", text: "a.txt" }));
    expect(getLiveState().convs.L1.conv.pending).toHaveLength(2);
    expect(liveTranscript).toHaveBeenCalledTimes(1);
    await act(async () => {
      vi.advanceTimersByTime(GAP_REFETCH_MS);
    });
    vi.useRealTimers();
    await flush();
    expect(liveTranscript).toHaveBeenCalledTimes(2);
    const conv = getLiveState().convs.L1.conv;
    expect(conv.pending).toEqual([]);
    expect(conv.lastSeq).toBe(6);
    expect(conv.items.map((it) => it.type)).toEqual(["user", "assistant", "tool"]);
    expect(conv.items[1]).toMatchObject({ text: "Hello!" });
    // 6 arrived after the transcript snapshot: re-applied on top of it.
    expect(conv.items[2]).toMatchObject({ call: { output: "a.txt" } });
  });

  it("a stale list right after startLive keeps the new session and its selection", async () => {
    (liveStart as Mock).mockResolvedValue(info({ live_id: "L9", native_id: null }));
    (liveTranscript as Mock).mockResolvedValue([]);
    await startLive({ agent: "claude", cwd: "/w", permission: "ask" });
    expect(getLiveState().selectedId).toBe("L9");
    (liveList as Mock).mockResolvedValue([]);
    await refreshLiveList();
    expect(getLiveState().selectedId).toBe("L9");
    expect(getLiveState().sessions.map((x) => x.live_id)).toEqual(["L9"]);
    expect(getLiveState().convs.L9).toBeDefined();

    // Long after the start, a list without it does drop it.
    const now = Date.now();
    vi.spyOn(Date, "now").mockReturnValue(now + RECENT_START_MS + 1);
    await refreshLiveList();
    vi.restoreAllMocks();
    expect(getLiveState().selectedId).toBeNull();
    expect(getLiveState().convs.L9).toBeUndefined();
  });

  it("a stale list keeps a conversation with buffered events", async () => {
    (liveList as Mock).mockResolvedValue([]);
    await refreshLiveList();
    handleLiveEnvelope(delta(1, "x", "L7"));
    expect(getLiveState().convs.L7.buffer).toHaveLength(1);
    await refreshLiveList();
    expect(getLiveState().convs.L7).toBeDefined();
  });

  it("selectors: a streamed token re-renders only the component that shows that conversation", async () => {
    (liveList as Mock).mockResolvedValue([info({ state: "running" }), info({ live_id: "L2", native_id: "n2", state: "running" })]);
    (liveTranscript as Mock).mockResolvedValue([]);
    await refreshLiveList();
    await flush();
    const renders = { badge: 0, list: 0, conv1: 0, conv2: 0 };
    function Badge() {
      renders.badge++;
      return <span>{useLiveStore(selectActiveCount)}</span>;
    }
    function List() {
      renders.list++;
      const s = useLiveStore(selectListSlice, shallowObjectEqual);
      return <span>{s.sessions.length}</span>;
    }
    function Conv({ id, k }: { id: string; k: "conv1" | "conv2" }) {
      renders[k]++;
      const lc = useLiveConv(id);
      return <span>{lc?.conv.items.length}</span>;
    }
    render(
      <>
        <Badge />
        <List />
        <Conv id="L1" k="conv1" />
        <Conv id="L2" k="conv2" />
      </>,
    );
    const before = { ...renders };
    act(() => {
      handleLiveEnvelope(delta(1, "a"));
      handleLiveEnvelope(delta(2, "b"));
    });
    expect(renders.conv1).toBeGreaterThan(before.conv1);
    expect(renders.conv2).toBe(before.conv2);
    expect(renders.badge).toBe(before.badge);
    expect(renders.list).toBe(before.list);

    // A state change reaches the badge and the list.
    act(() => handleLiveEnvelope(env(3, { type: "status", state: "idle" })));
    expect(renders.badge).toBe(before.badge + 1);
    expect(renders.list).toBe(before.list + 1);
  });
});
