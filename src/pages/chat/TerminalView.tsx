import * as React from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import "@xterm/xterm/css/xterm.css";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ImageIcon, Loader2, RotateCw, SquareTerminal } from "lucide-react";
import { saveTempImage, type PtyOpenOptions } from "@/api/pty";
import { Button } from "@/components/ui/button";
import type { Messages } from "@/i18n";
import { fmt } from "@/pages/memory/paths";
import { errorMessage } from "@/pages/skills/skillUtils";
import { fileToDataUri, quotePath } from "./composerUtils";
import { attachTerminal, getTerminal, getTerminalBuffer, openTerminal, resizeTerminal, useTerminal, writeTerminal } from "./terminalStore";
import { TERMINAL_CHROME, TERMINAL_FONT, TERMINAL_THEME } from "./terminalTheme";

type C = Messages["chat"];

/**
 * The real CLI in xterm.js. The pseudo terminal lives in `terminalStore`, keyed by
 * conversation: mounting attaches (replaying the scrollback) or starts it.
 * `keepAlive`: the process stays when the view is left even if nothing was typed.
 */
export function TerminalView({ termKey, opts, c, keepAlive = false }: { termKey: string; opts: PtyOpenOptions; c: C; keepAlive?: boolean }) {
  const entry = useTerminal(termKey);
  const [toast, setToast] = React.useState<{ text: string; error: boolean } | null>(null);
  const toastTimer = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const showToast = React.useCallback((text: string, error = false) => {
    if (toastTimer.current) clearTimeout(toastTimer.current);
    setToast({ text, error });
    toastTimer.current = setTimeout(() => setToast(null), 3500);
  }, []);
  React.useEffect(
    () => () => {
      if (toastTimer.current) clearTimeout(toastTimer.current);
    },
    [],
  );
  const pasteRef = React.useRef<(e: ClipboardEvent) => void>(() => {});
  // A pasted screenshot: saved as a temp file and its path typed at the cursor (both CLIs read image paths).
  pasteRef.current = (e: ClipboardEvent) => {
    const file = Array.from(e.clipboardData?.files ?? []).find((f) => f.type.startsWith("image/"));
    if (!file) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    void (async () => {
      try {
        const path = await saveTempImage(await fileToDataUri(file));
        writeTerminal(termKey, quotePath(path));
        showToast(c.terminalImagePasted);
      } catch (err) {
        showToast(fmt(c.terminalImageFailed, { error: errorMessage(err) }), true);
      }
      term.current?.focus();
    })();
  };
  const host = React.useRef<HTMLDivElement>(null);
  const term = React.useRef<Terminal | null>(null);
  const optsRef = React.useRef(opts);
  optsRef.current = opts;
  const keepRef = React.useRef(keepAlive);
  keepRef.current = keepAlive;

  React.useEffect(() => {
    const el = host.current;
    if (!el) return;
    const t = new Terminal({
      theme: TERMINAL_THEME,
      fontFamily: TERMINAL_FONT,
      fontSize: 13,
      lineHeight: 1.15,
      cursorBlink: true,
      scrollback: 10_000,
      macOptionIsMeta: true,
    });
    const fit = new FitAddon();
    t.loadAddon(fit);
    t.loadAddon(new WebLinksAddon((_e, uri) => void openUrl(uri).catch(() => {})));
    t.open(el);
    term.current = t;
    try {
      fit.fit();
    } catch {
      // not laid out yet
    }
    const replay = getTerminalBuffer(termKey);
    if (replay) t.write(replay);
    const detach = attachTerminal(termKey, (d) => t.write(d));
    const input = t.onData((d) => writeTerminal(termKey, d));
    if (!getTerminal(termKey)) void openTerminal(termKey, { ...optsRef.current, cols: t.cols, rows: t.rows }, keepRef.current);
    // Capture phase: runs before xterm's own paste handler on its hidden textarea.
    const onPaste = (e: ClipboardEvent) => pasteRef.current(e);
    el.addEventListener("paste", onPaste, true);
    let last = `${t.cols}x${t.rows}`;
    const ro = new ResizeObserver(() => {
      try {
        fit.fit();
      } catch {
        return;
      }
      const size = `${t.cols}x${t.rows}`;
      if (size !== last) {
        last = size;
        resizeTerminal(termKey, t.cols, t.rows);
      }
    });
    ro.observe(el);
    t.focus();
    return () => {
      el.removeEventListener("paste", onPaste, true);
      ro.disconnect();
      input.dispose();
      detach();
      term.current = null;
      t.dispose();
    };
  }, [termKey]);

  const restart = () => {
    const t = term.current;
    t?.reset();
    void openTerminal(termKey, { ...optsRef.current, cols: t?.cols ?? opts.cols, rows: t?.rows ?? opts.rows }, keepAlive);
    t?.focus();
  };

  const status = entry?.status ?? "starting";
  return (
    <div className="flex min-h-0 flex-1 flex-col" style={{ background: TERMINAL_CHROME.background }}>
      <div
        className="flex h-8 shrink-0 items-center gap-2 border-b px-3 text-xs"
        style={{ borderColor: TERMINAL_CHROME.border, color: TERMINAL_CHROME.text }}
      >
        <SquareTerminal className="size-3.5 shrink-0" aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate font-mono" title={entry?.command}>
          {fmt(c.terminalStatus, { command: entry?.command || opts.tool })}
        </span>
        {status === "starting" ? (
          <span className="flex shrink-0 items-center gap-1">
            <Loader2 className="size-3 animate-spin motion-reduce:animate-none" aria-hidden="true" />
            {c.terminalStarting}
          </span>
        ) : status === "exited" ? (
          <span className="shrink-0">{entry?.code != null ? fmt(c.terminalExited, { code: entry.code }) : c.terminalExitedNoCode}</span>
        ) : status === "error" ? (
          <span role="alert" className="min-w-0 shrink truncate" style={{ color: TERMINAL_CHROME.error }} title={entry?.error ?? ""}>
            {entry?.busy?.kind === "live" ? c.busyLive : fmt(c.terminalFailed, { error: entry?.error ?? "" })}
          </span>
        ) : null}
        {status === "exited" || status === "error" ? (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className="h-6 shrink-0 cursor-pointer gap-1 px-2 text-xs hover:bg-white/10"
            style={{ color: TERMINAL_CHROME.strong }}
            onClick={restart}
          >
            <RotateCw className="size-3" aria-hidden="true" />
            {c.terminalRestart}
          </Button>
        ) : null}
      </div>
      <div className="relative flex min-h-0 flex-1 flex-col">
        <div ref={host} data-testid="terminal-host" className="min-h-0 flex-1 overflow-hidden py-1 pl-2" />
        {toast ? (
          <div
            role={toast.error ? "alert" : "status"}
            className="pointer-events-none absolute right-3 bottom-3 flex max-w-[80%] items-center gap-2 rounded-md border px-3 py-1.5 text-xs shadow-lg"
            style={{ background: TERMINAL_CHROME.background, borderColor: TERMINAL_CHROME.border, color: toast.error ? TERMINAL_CHROME.error : TERMINAL_CHROME.strong }}
          >
            <ImageIcon className="size-3.5 shrink-0" aria-hidden="true" />
            <span className="min-w-0 truncate">{toast.text}</span>
          </div>
        ) : null}
      </div>
    </div>
  );
}
