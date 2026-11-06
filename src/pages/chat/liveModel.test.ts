import type { LiveEnvelope, LiveEvent } from "@/api/live";
import { EMPTY_CONV, applyAll, applyLiveEvent, pendingApprovals, toConvBlocks, type ConvState } from "./liveModel";

function envs(...events: LiveEvent[]): LiveEnvelope[] {
  return events.map((event, i) => ({ live_id: "L", seq: i + 1, event }));
}

function run(...events: LiveEvent[]): ConvState {
  return applyAll(EMPTY_CONV, envs(...events));
}

describe("applyLiveEvent", () => {
  it("records started and status", () => {
    const s = run({ type: "started", native_id: "abc", model: "opus" }, { type: "status", state: "idle" });
    expect(s.native_id).toBe("abc");
    expect(s.model).toBe("opus");
    expect(s.state).toBe("idle");
    expect(s.lastSeq).toBe(2);
  });

  it("keeps the images of a user message for the conversation", () => {
    const s = run({ type: "user_message", id: "u1", text: "", images: ["data:image/png;base64,AAAA"] }, { type: "user_message", id: "u2", text: "hi", images: [] });
    expect(toConvBlocks(s.items)).toEqual([
      { kind: "user", key: "u-u1-0", text: "", images: ["data:image/png;base64,AAAA"] },
      { kind: "user", key: "u-u2-1", text: "hi", images: undefined },
    ]);
  });

  it("skips duplicate and stale seqs", () => {
    const [a, b] = envs({ type: "user_message", id: "u1", text: "hi" }, { type: "user_message", id: "u2", text: "again" });
    let s = applyLiveEvent(EMPTY_CONV, a);
    s = applyLiveEvent(s, b);
    const same = applyLiveEvent(s, a);
    expect(same).toBe(s);
    expect(applyLiveEvent(s, b)).toBe(s);
    expect(s.items).toHaveLength(2);
  });

  it("applyAll sorts by seq so transcript + buffered stream merge cleanly", () => {
    const list = envs(
      { type: "user_message", id: "u1", text: "hi" },
      { type: "assistant_delta", id: "a1", text: "Hel" },
      { type: "assistant_delta", id: "a1", text: "lo" },
    );
    const s = applyAll(EMPTY_CONV, [list[2], list[0], list[1], list[1]]);
    expect(s.items).toEqual([
      { type: "user", id: "u1", text: "hi" },
      { type: "assistant", id: "a1", text: "Hello", streaming: true },
    ]);
  });

  it("appends deltas, then replaces them with the final message", () => {
    const s = run(
      { type: "assistant_delta", id: "a1", text: "Hel" },
      { type: "assistant_delta", id: "a1", text: "lo" },
      { type: "assistant_message", id: "a1", text: "Hello!" },
    );
    expect(s.items).toEqual([{ type: "assistant", id: "a1", text: "Hello!", streaming: false }]);
  });

  it("adds a final message that had no deltas", () => {
    const s = run({ type: "assistant_message", id: "a2", text: "Done" });
    expect(s.items).toEqual([{ type: "assistant", id: "a2", text: "Done", streaming: false }]);
  });

  it("merges tool start, streamed output and finish by id", () => {
    const s = run(
      { type: "tool_started", id: "t1", name: "Bash", kind: "shell", summary: "ls", input: '{"command":"ls"}' },
      { type: "tool_started", id: "t2", name: "Read", kind: "read", summary: "a.ts", input: "{}" },
      { type: "tool_output", id: "t1", text: "a\n" },
      { type: "tool_output", id: "t1", text: "b\n" },
      { type: "tool_finished", id: "t2", output: "content", is_error: false },
    );
    const tools = s.items.flatMap((it) => (it.type === "tool" ? [it.call] : []));
    expect(tools[0]).toMatchObject({ id: "t1", output: "a\nb\n", finished: false });
    expect(tools[1]).toMatchObject({ id: "t2", output: "content", finished: true, is_error: false });

    const done = applyLiveEvent(s, { live_id: "L", seq: 6, event: { type: "tool_finished", id: "t1", output: null, is_error: true } });
    const t1 = done.items.find((it) => it.type === "tool" && it.call.id === "t1");
    expect(t1).toMatchObject({ call: { output: "a\nb\n", finished: true, is_error: true } });
  });

  it("tracks an approval from request to resolution", () => {
    const req = run(
      { type: "approval_request", request_id: "r1", tool_name: "Bash", kind: "shell", summary: "rm -rf x", detail: "rm -rf x", can_allow_session: true },
      { type: "status", state: "waiting_approval" },
    );
    expect(pendingApprovals(req)).toHaveLength(1);
    const res = applyLiveEvent(req, { live_id: "L", seq: 3, event: { type: "approval_resolved", request_id: "r1", decision: "deny" } });
    expect(pendingApprovals(res)).toHaveLength(0);
    expect(res.items[0]).toMatchObject({ type: "approval", approval: { request_id: "r1", decision: "deny" } });
  });

  it("voids approvals still pending when the turn ends or the process exits", () => {
    const req: LiveEvent = { type: "approval_request", request_id: "r1", tool_name: "Bash", kind: "shell", summary: "x", detail: "x", can_allow_session: false };
    const ended = run(req, { type: "turn_completed", usage: { input_tokens: null, output_tokens: null, cost_usd: null, duration_ms: null }, is_error: false });
    expect(pendingApprovals(ended)).toHaveLength(0);
    expect(ended.items[0]).toMatchObject({ approval: { decision: null, expired: true } });
    expect(run(req, { type: "exited", code: 0 }).items[0]).toMatchObject({ approval: { expired: true } });
    expect(run(req, { type: "error", message: "x", fatal: true }).items[0]).toMatchObject({ approval: { expired: true } });
    expect(run(req, { type: "error", message: "x", fatal: false }).items[0]).toMatchObject({ approval: { expired: false } });
  });

  it("updates the model when started repeats, and renders one exit marker", () => {
    const s = run(
      { type: "started", native_id: "n", model: "opus" },
      { type: "started", native_id: "n", model: "claude-opus-5-5" },
      { type: "exited", code: 0 },
      { type: "exited", code: 0 },
    );
    expect(s.model).toBe("claude-opus-5-5");
    expect(s.items.filter((it) => it.type === "exited")).toHaveLength(1);
  });

  it("settles streaming text when the turn completes and stores usage", () => {
    const usage = { input_tokens: 10, output_tokens: 5, cost_usd: 0.01, duration_ms: 1200 };
    const s = run({ type: "assistant_delta", id: "a1", text: "partial" }, { type: "turn_completed", usage, is_error: false });
    expect(s.items[0]).toMatchObject({ streaming: false });
    expect(s.lastUsage).toEqual(usage);
  });

  it("keeps errors inline and marks exit", () => {
    const s = run(
      { type: "error", message: "rate limited", fatal: false },
      { type: "error", message: "crashed", fatal: true },
      { type: "exited", code: 1 },
    );
    expect(s.state).toBe("exited");
    expect(s.items).toEqual([
      { type: "error", message: "rate limited", fatal: false },
      { type: "error", message: "crashed", fatal: true },
      { type: "exited", code: 1 },
    ]);
  });

  it("replaces a thinking block with the same id", () => {
    const s = run({ type: "thinking", id: "th", text: "a" }, { type: "thinking", id: "th", text: "ab" });
    expect(s.items).toEqual([{ type: "thinking", id: "th", text: "ab" }]);
  });
});

describe("toConvBlocks", () => {
  it("groups an assistant turn into one card with tool runs, approvals stand alone", () => {
    const s = run(
      { type: "user_message", id: "u1", text: "fix it" },
      { type: "thinking", id: "th", text: "hmm" },
      { type: "assistant_message", id: "a1", text: "Looking" },
      { type: "tool_started", id: "t1", name: "Read", kind: "read", summary: "a", input: "" },
      { type: "tool_started", id: "t2", name: "Grep", kind: "search", summary: "b", input: "" },
      { type: "approval_request", request_id: "r1", tool_name: "Edit", kind: "edit", summary: "a.ts", detail: "diff", can_allow_session: false },
      { type: "approval_resolved", request_id: "r1", decision: "allow" },
      { type: "error", message: "warn", fatal: false },
      { type: "assistant_message", id: "a2", text: "Done" },
      { type: "user_message", id: "u2", text: "thanks" },
    );
    const blocks = toConvBlocks(s.items);
    expect(blocks.map((b) => b.kind)).toEqual(["user", "turn", "approval", "turn", "user"]);
    const first = blocks[1];
    expect(first.kind === "turn" && first.segments.map((x) => x.kind)).toEqual(["thinking", "text", "tools"]);
    expect(first.kind === "turn" && first.segments[2].kind === "tools" && first.segments[2].calls.map((c) => c.id)).toEqual(["t1", "t2"]);
    const second = blocks[3];
    expect(second.kind === "turn" && second.segments.map((x) => x.kind)).toEqual(["error", "text"]);
  });

  it("puts fatal errors and exit outside of turns", () => {
    const blocks = toConvBlocks(run({ type: "error", message: "boom", fatal: true }, { type: "exited", code: null }).items);
    expect(blocks.map((b) => b.kind)).toEqual(["fatal", "exited"]);
  });
});

describe("notices", () => {
  it("a compaction notice is replaced in place; command output is shown", () => {
    const s = run(
      { type: "user_message", id: "u1", text: "/compact" },
      { type: "notice", id: "compact-1", kind: "compacting" },
      { type: "notice", id: "compact-1", kind: "compacted", before: 45993, after: 9575 },
      { type: "notice", id: "cmd-1", kind: "command_output", text: "Compacted" },
      { type: "turn_completed", usage: { input_tokens: 0, output_tokens: 0, cost_usd: null, duration_ms: null }, is_error: false },
    );
    const blocks = toConvBlocks(s.items);
    expect(blocks.map((b) => b.kind)).toEqual(["user", "notice", "notice"]);
    expect(blocks[1]).toMatchObject({ notice: { kind: "compacted", before: 45993, after: 9575 } });
    expect(blocks[2]).toMatchObject({ notice: { kind: "command_output", text: "Compacted" } });
  });
});
