import { useCallback, useEffect, useRef, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { isTauri } from "@tauri-apps/api/core";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";

const CHECK_INTERVAL = 6 * 60 * 60 * 1000;

type Status = "idle" | "checking" | "current" | "available" | "downloading" | "installing" | "installed" | "error";

export function useAppUpdate(blocked: boolean) {
  const enabled = !import.meta.env.DEV && isTauri();
  const [version, setVersion] = useState("");
  const [update, setUpdate] = useState<Update | null>(null);
  const [status, setStatus] = useState<Status>("idle");
  const [progress, setProgress] = useState<number | undefined>();
  const [restartFailed, setRestartFailed] = useState(false);
  const operation = useRef(false);

  useEffect(() => {
    if (isTauri()) void getVersion().then(setVersion).catch(() => {});
  }, []);

  const checkNow = useCallback(async (manual = true) => {
    if (!enabled || operation.current || update) return;
    operation.current = true;
    if (manual) setStatus("checking");
    try {
      const candidate = await check({ timeout: 15_000 });
      setUpdate(candidate);
      setStatus(candidate ? "available" : manual ? "current" : "idle");
    } catch {
      if (manual) setStatus("error");
    } finally {
      operation.current = false;
    }
  }, [enabled, update]);

  useEffect(() => {
    if (!enabled || update) return;
    const startup = window.setTimeout(() => void checkNow(false), 5_000);
    const interval = window.setInterval(() => void checkNow(false), CHECK_INTERVAL);
    return () => {
      window.clearTimeout(startup);
      window.clearInterval(interval);
    };
  }, [checkNow, enabled, update]);

  const install = async () => {
    if (!update || blocked || operation.current || status === "installed") return;
    operation.current = true;
    setProgress(undefined);
    setStatus("downloading");
    let received = 0;
    let total: number | undefined;
    try {
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength;
            break;
          case "Progress":
            received += event.data.chunkLength;
            if (total) setProgress(Math.min(100, Math.floor(received / total * 100)));
            break;
          case "Finished":
            setStatus("installing");
            break;
        }
      });
      setStatus("installed");
    } catch {
      setStatus("error");
    } finally {
      operation.current = false;
    }
  };

  const restart = async () => {
    if (blocked || operation.current || status !== "installed") return;
    operation.current = true;
    setRestartFailed(false);
    try {
      await relaunch();
    } catch {
      setRestartFailed(true);
    } finally {
      operation.current = false;
    }
  };

  return { enabled, version, update, status, progress, blocked, restartFailed, checkNow, install, restart };
}

export type AppUpdate = ReturnType<typeof useAppUpdate>;
