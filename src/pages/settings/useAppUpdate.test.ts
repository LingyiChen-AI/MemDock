import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { useAppUpdate } from "./useAppUpdate";

vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn().mockResolvedValue("0.1.0") }));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));

describe("desktop updates", () => {
  beforeEach(() => {
    vi.stubEnv("DEV", false);
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllEnvs();
  });

  it("checks on startup and every 30 minutes", async () => {
    vi.mocked(check).mockResolvedValue(null);
    const { result } = renderHook(() => useAppUpdate(false));
    await act(() => vi.advanceTimersByTimeAsync(5_000));
    expect(check).toHaveBeenCalledTimes(1);
    expect(result.current.status).toBe("idle");
    const candidate = { version: "0.2.0", close: vi.fn().mockResolvedValue(undefined) } as unknown as Update;
    vi.mocked(check).mockResolvedValueOnce(candidate);
    await act(() => vi.advanceTimersByTimeAsync(30 * 60 * 1000));
    expect(check).toHaveBeenCalledTimes(2);
    expect(result.current.update).toBe(candidate);
    await act(() => vi.advanceTimersByTimeAsync(30 * 60 * 1000));
    expect(check).toHaveBeenCalledTimes(3);
    expect(candidate.close).toHaveBeenCalledOnce();
  });

  it("installs a discovered update on demand and lets the user restart", async () => {
    const downloadAndInstall = vi.fn().mockResolvedValue(undefined);
    vi.mocked(check).mockResolvedValue({ version: "0.2.0", downloadAndInstall } as unknown as Update);
    const { result, rerender } = renderHook(({ blocked }) => useAppUpdate(blocked), {
      initialProps: { blocked: true },
    });
    await act(() => result.current.checkNow());
    expect(result.current.status).toBe("available");
    await act(() => result.current.install());
    expect(downloadAndInstall).not.toHaveBeenCalled();

    rerender({ blocked: false });
    downloadAndInstall.mockRejectedValueOnce(new Error("interrupted"));
    await act(() => result.current.install());
    expect(result.current.status).toBe("error");
    await act(() => result.current.install());
    expect(result.current.status).toBe("installed");
    expect(relaunch).not.toHaveBeenCalled();
    await act(() => result.current.restart());
    expect(relaunch).toHaveBeenCalledOnce();
  });
});
