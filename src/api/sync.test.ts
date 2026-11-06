import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";

const listen = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({ listen: (...a: unknown[]) => listen(...a) }));

import {
  ackPublicRepo,
  loginCancel,
  loginStart,
  logout,
  onLoginEvent,
  onSyncStatusChanged,
  openVerificationUrl,
  restoreFromHistory,
  setIncludedLocal,
  setRepoName,
  syncDiscard,
  syncFileAt,
  syncFinish,
  syncHistory,
  syncProjects,
  syncStart,
  syncStatus,
} from "./sync";

let calls: Array<[string, unknown]> = [];

beforeEach(() => {
  calls = [];
  listen.mockReset();
  mockIPC((cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === "sync_start")
      return {
        kind: "done",
        summary: { pulled: 1 },
        skipped: [{ repo_path: "global/codex/AGENTS.md", reason: "not_utf8_remote" }],
        missing_skills: [],
        removed_elsewhere: [],
      };
    if (cmd === "sync_finish") throw { code: "plan_not_found", message: "gone" };
    return null;
  });
});

afterEach(() => clearMocks());

/** Arguments of the last call, without Tauri's internal fields. */
function last(): [string, unknown] {
  const [cmd, args] = calls[calls.length - 1];
  return [cmd, args && Object.keys(args as object).length ? args : undefined];
}

describe("sync api", () => {
  it("commands without arguments use the right names", async () => {
    const cases: Array<[() => Promise<unknown>, string]> = [
      [syncStatus, "sync_status"],
      [loginStart, "login_start"],
      [loginCancel, "login_cancel"],
      [logout, "logout"],
      [ackPublicRepo, "ack_public_repo"],
      [openVerificationUrl, "open_verification_url"],
      [syncProjects, "sync_projects"],
    ];
    for (const [fn, name] of cases) {
      await fn();
      expect(last()).toEqual([name, undefined]);
    }
  });

  it("commands with arguments pass camelCase names", async () => {
    await setRepoName("MyHub");
    expect(last()).toEqual(["set_repo_name", { name: "MyHub" }]);
    await syncDiscard("p1");
    expect(last()).toEqual(["sync_discard", { planId: "p1" }]);
    await setIncludedLocal(["a", "b"]);
    expect(last()).toEqual(["set_included_local", { keys: ["a", "b"] }]);
    await syncHistory(20);
    expect(last()).toEqual(["sync_history", { limit: 20 }]);
    await syncFileAt("abc", "global/claude/CLAUDE.md");
    expect(last()).toEqual(["sync_file_at", { commit: "abc", repoPath: "global/claude/CLAUDE.md" }]);
    await restoreFromHistory("abc", "global/codex/AGENTS.md");
    expect(last()).toEqual([
      "restore_from_history",
      { commit: "abc", repoPath: "global/codex/AGENTS.md" },
    ]);
  });

  it("syncStart returns the tagged result", async () => {
    const r = await syncStart();
    expect(last()).toEqual(["sync_start", { secrets: "block" }]);
    expect(r.kind).toBe("done");
    if (r.kind === "done") expect(r.skipped[0].reason).toBe("not_utf8_remote");
  });

  it("syncFinish sends resolutions keyed by repo path and rethrows errors", async () => {
    const resolutions = {
      "global/claude/CLAUDE.md": { kind: "text" as const, text: "merged\n" },
      "projects/x/codex/AGENTS.md": { kind: "keep_local" as const },
    };
    await expect(syncFinish("plan-1", resolutions)).rejects.toEqual({
      code: "plan_not_found",
      message: "gone",
    });
    expect(last()).toEqual(["sync_finish", { planId: "plan-1", resolutions }]);
  });

  it("event helpers listen on the sync channels", async () => {
    listen.mockResolvedValue(() => {});
    const onLogin = vi.fn();
    const onStatus = vi.fn();
    await onLoginEvent(onLogin);
    await onSyncStatusChanged(onStatus);
    expect(listen).toHaveBeenCalledWith("sync://login", expect.any(Function));
    expect(listen).toHaveBeenCalledWith("sync://status-changed", expect.any(Function));
    listen.mock.calls[0][1]({ payload: { status: "ok" } });
    listen.mock.calls[1][1]({ payload: { pending: 3 } });
    expect(onLogin).toHaveBeenCalledWith({ status: "ok" });
    expect(onStatus).toHaveBeenCalledWith({ pending: 3 });
  });
});
