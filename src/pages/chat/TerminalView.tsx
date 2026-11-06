import * as React from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import "@xterm/xterm/css/xterm.css";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Loader2, RotateCw, SquareTerminal } from "lucide-react";
import type { PtyOpenOptions } from "@/api/pty";
import { Button } from "@/components/ui/button";
import type { Messages } from "@/i18n";
import { fmt } from "@/pages/memory/paths";
import { attachTerminal, getTerminal, getTerminalBuffer, openTerminal, resizeTerminal, useTerminal, writeTerminal } from "./terminalStore";
import { TERMINAL_CHROME, TERMINAL_FONT, TERMINAL_THEME } from "./terminalTheme";

type C = Messages["chat"];

/**
 * The real CLI in xterm.js. The pseudo terminal lives in `terminalStore`, keyed by
 * conversation: mounting attaches (replaying the scrollback) or starts it.
 */
export function TerminalView({ termKey, opts, c }: { termKey: string; opts: PtyOpenOptions; c: C }) {
  const entry = useTerminal(termKey);
  const host = React.useRef<HTMLDivElement>(null);
  const term = React.useRef<Terminal | null>(null);
  const optsRef = React.useRef(opts);
  optsRef.current = opts;

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
    if (!getTerminal(termKey)) void openTerminal(termKey, { ...optsRef.current, cols: t.cols, rows: t.rows });
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
    void openTerminal(termKey, { ...optsRef.current, cols: t?.cols ?? opts.cols, rows: t?.rows ?? opts.rows });
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
            {fmt(c.terminalFailed, { error: entry?.error ?? "" })}
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
      <div ref={host} data-testid="terminal-host" className="min-h-0 flex-1 overflow-hidden py-1 pl-2" />
    </div>
  );
}
