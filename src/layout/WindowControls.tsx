import * as React from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Copy, Minus, Square, X } from "lucide-react";
import { useI18n } from "@/i18n/I18nProvider";
import { cn } from "@/lib/utils";

export const IS_MAC = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

/** True inside the Tauri app (the window API is unavailable in plain browsers / tests). */
function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * The window's own title strip (the system title bar is turned off on Windows / Linux):
 * a 32px drag region (double-click maximizes) with minimize / maximize / close on the right.
 * Close goes through the normal close request, so it still hides MemDock to the tray.
 * On macOS the native traffic lights stay, so only the drag region is drawn.
 */
export function WindowTitleBar() {
  const { t } = useI18n();
  const [maximized, setMaximized] = React.useState(false);
  React.useEffect(() => {
    if (!inTauri()) return;
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    const sync = () => void win.isMaximized().then(setMaximized).catch(() => {});
    sync();
    void win.onResized(sync).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);
  const act = (f: (w: ReturnType<typeof getCurrentWindow>) => Promise<void>) => () => {
    if (inTauri()) void f(getCurrentWindow()).catch(() => {});
  };
  const btn =
    "grid h-8 w-11 cursor-default place-items-center text-muted-foreground outline-none transition-colors duration-100 hover:bg-muted hover:text-foreground focus-visible:bg-muted motion-reduce:transition-none [&_svg]:size-3.5";
  return (
    <div data-tauri-drag-region className="flex h-8 shrink-0 select-none items-center justify-end">
      {IS_MAC ? null : (
        <div className="flex h-8 items-stretch">
          <button type="button" aria-label={t.shell.winMinimize} title={t.shell.winMinimize} className={btn} onClick={act((w) => w.minimize())}>
            <Minus aria-hidden="true" />
          </button>
          <button
            type="button"
            aria-label={maximized ? t.shell.winRestore : t.shell.winMaximize}
            title={maximized ? t.shell.winRestore : t.shell.winMaximize}
            className={btn}
            onClick={act((w) => w.toggleMaximize())}
          >
            {maximized ? <Copy className="size-3! -scale-x-100" aria-hidden="true" /> : <Square className="size-3!" aria-hidden="true" />}
          </button>
          <button
            type="button"
            aria-label={t.shell.winClose}
            title={t.shell.winClose}
            className={cn(btn, "hover:bg-red-600 hover:text-white focus-visible:bg-red-600 focus-visible:text-white")}
            onClick={act((w) => w.close())}
          >
            <X aria-hidden="true" />
          </button>
        </div>
      )}
    </div>
  );
}
