import { describe, it, expect, vi, beforeEach } from "vitest";

const invoke = vi.fn();
const listen = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...a: unknown[]) => listen(...a) }));

import {
  writeMemoryFile,
  listMemories,
  updateUiPrefs,
  onMemoryChanged,
} from "./memory";

beforeEach(() => {
  invoke.mockReset();
  listen.mockReset();
});

describe("memory api", () => {
  it("writeMemoryFile passes camelCase args", async () => {
    invoke.mockResolvedValue({ text: "t", hash: "h", tokens: 1, imports: [] });
    await writeMemoryFile("/a/CLAUDE.md", "t", "abc");
    expect(invoke).toHaveBeenCalledWith("write_memory_file", {
      path: "/a/CLAUDE.md",
      text: "t",
      expectedHash: "abc",
    });
  });

  it("writeMemoryFile rethrows CommandError", async () => {
    const err = { code: "disk_changed", message: "file changed on disk", current_hash: "zz" };
    invoke.mockRejectedValue(err);
    await expect(writeMemoryFile("/a", "t", "h")).rejects.toEqual(err);
  });

  it("listMemories and updateUiPrefs call the right commands", async () => {
    invoke.mockResolvedValue({});
    await listMemories();
    expect(invoke).toHaveBeenCalledWith("list_memories");
    await updateUiPrefs("en", undefined);
    expect(invoke).toHaveBeenLastCalledWith("update_ui_prefs", { language: "en", theme: null });
  });

  it("onMemoryChanged unwraps the payload", async () => {
    const unlisten = vi.fn();
    listen.mockImplementation(async (_name: string, handler: (e: { payload: unknown }) => void) => {
      handler({ payload: { paths: ["a"] } });
      return unlisten;
    });
    const cb = vi.fn();
    const un = await onMemoryChanged(cb);
    expect(listen).toHaveBeenCalledWith("memory-changed", expect.any(Function));
    expect(cb).toHaveBeenCalledWith({ paths: ["a"] });
    expect(un).toBe(unlisten);
  });
});
