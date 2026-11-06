import * as React from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import "@xterm/xterm/css/xterm.css";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Columns2, ImageIcon, Loader2, RotateCw, Rows2, SquareTerminal, X } from "lucide-react";
import type { LiveAgent } from "@/api/live";
import { saveTempImage, type PtyOpenOptions } from "@/api/pty";
import { Button } from "@/components/ui/button";
import type { Messages } from "@/i18n";
import { fmt } from "@/pages/memory/paths";
import { errorMessage } from "@/pages/skills/skillUtils";
import { fileToDataUri, quotePath } from "./composerUtils";
import {
  attachTerminal,
  closePane,
  closeTerminal,
  getLayout,
  getShells,
  getTerminal,
  getTerminalBuffer,
  leaves,
  MAX_PANES,
  movePaneFocus,
  openTerminal,
  paneGen,
  paneRects,
  removeShell,
  resizeTerminal,
  setPaneFocus,
  setSplitRatio,
  shellOptions,
  splitPane,
  terminalOptions,
  useLayout,
  useTerminal,
  writeTerminal,
  type FocusDir,
  type SplitBox,
  type SplitDir,
} from "./terminalStore";
import { TERMINAL_CHROME, TERMINAL_FONT, TERMINAL_THEME } from "./terminalTheme";

type C = Messages["chat"];


const MIN_W = 120;
const MIN_H = 80;
const ARROWS: Record<string, FocusDir> = { ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down" };

/** What a pane action starts in the new pane. */
type NewPane = "shell" | LiveAgent;

/**
 * The real CLI in xterm.js. The pseudo terminals live in `terminalStore`, keyed by
 * conversation: mounting attaches (replaying the scrollback) or starts it. The terminal
 * can be split into panes (each its own process in the same folder); the layout is kept
 * in the store too, so leaving and coming back restores it.
 * `keepAlive`: the process stays when the view is left even if nothing was typed.
 * `onClose`: closing the last pane (default: remove the shell from the list / close the terminal).
 */
export function TerminalView({
  termKey,
  opts,
  c,
  keepAlive = false,
  onClose,
}: {
  termKey: string;
  opts: PtyOpenOptions;
  c: C;
  keepAlive?: boolean;
  onClose?: () => void;
}) {
  const layout = useLayout(termKey);
  const focus = layout.focus;
  const entry = useTerminal(focus);
  const [closed, setClosed] = React.useState(false);
  const [toast, setToast] = React.useState<{ text: string; error: boolean } | null>(null);
  const [menu, setMenu] = React.useState<{ x: number; y: number; pane: string } | null>(null);
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
  const terms = React.useRef(new Map<string, Terminal>());
  const body = React.useRef<HTMLDivElement>(null);
  const optsRef = React.useRef(opts);
  optsRef.current = opts;

  const panes = leaves(layout.tree);
  const multi = panes.length > 1;
  const full = panes.length >= MAX_PANES;
  const rects = React.useMemo(() => paneRects(layout.tree), [layout.tree]);

  const focusTerm = (pane: string) => terms.current.get(pane)?.focus();

  // A pasted screenshot: saved as a temp file and its path typed at the cursor (both CLIs read image paths).
  const pasteImage = (e: ClipboardEvent, pane: string) => {
    const file = Array.from(e.clipboardData?.files ?? []).find((f) => f.type.startsWith("image/"));
    if (!file) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    void (async () => {
      try {
        const path = await saveTempImage(await fileToDataUri(file));
        writeTerminal(pane, quotePath(path));
        showToast(c.terminalImagePasted);
      } catch (err) {
        showToast(fmt(c.terminalImageFailed, { error: errorMessage(err) }), true);
      }
      focusTerm(pane);
    })();
  };
  const pasteRef = React.useRef(pasteImage);
  pasteRef.current = pasteImage;

  const closeAll = () => {
    setClosed(true);
    if (onClose) onClose();
    else if (getShells().some((s) => s.key === termKey)) void removeShell(termKey);
    else void closeTerminal(termKey);
  };

  const split = (pane: string, dir: SplitDir, what: NewPane = "shell") => {
    if (leaves(getLayout(termKey).tree).length >= MAX_PANES) {
      showToast(fmt(c.terminalPaneLimit, { max: MAX_PANES }), true);
      return;
    }
    const cwd = getTerminal(termKey)?.opts.cwd ?? optsRef.current.cwd;
    const t = terms.current.get(pane);
    const cols = Math.max(20, Math.floor((t?.cols ?? 80) / (dir === "row" ? 2 : 1)));
    const rows = Math.max(5, Math.floor((t?.rows ?? 24) / (dir === "column" ? 2 : 1)));
    const next = what === "shell" ? shellOptions(cwd, cols, rows) : terminalOptions({ tool: what, cwd, nativeId: null }, cols, rows);
    splitPane(termKey, pane, dir, next);
  };

  const close = (pane: string) => {
    if (!closePane(termKey, pane)) closeAll();
  };

  const restart = (pane: string) => {
    const t = terms.current.get(pane);
    t?.reset();
    const e = getTerminal(pane);
    const own = pane === termKey && (!e || e.opts.tool === optsRef.current.tool);
    const base = own ? optsRef.current : (e?.opts ?? optsRef.current);
    setClosed(false);
    void openTerminal(pane, { ...base, cols: t?.cols ?? base.cols, rows: t?.rows ?? base.rows }, own ? keepAlive : true);
    t?.focus();
  };

  const copy = (pane: string): boolean => {
    const t = terms.current.get(pane);
    const sel = t?.getSelection?.() ?? "";
    if (!sel) return false;
    void navigator.clipboard?.writeText(sel).catch(() => {});
    t?.clearSelection?.();
    return true;
  };

  const paste = (pane: string) => {
    void navigator.clipboard
      ?.readText()
      .then((text) => {
        if (!text) return;
        const t = terms.current.get(pane);
        if (t?.paste) t.paste(text);
        else writeTerminal(pane, text);
      })
      .catch(() => {});
    focusTerm(pane);
  };

  // Shortcuts on the focused pane; capture phase, before xterm turns the keys into input.
  const onKeyDownCapture = (e: React.KeyboardEvent) => {
    const pane = getLayout(termKey).focus;
    const k = e.key;
    const code = e.code;
    let done = true;
    if (e.altKey && e.shiftKey && !e.ctrlKey && (code === "Equal" || k === "+" || k === "=")) split(pane, "row");
    else if (e.altKey && e.shiftKey && !e.ctrlKey && (code === "Minus" || k === "_" || k === "-")) split(pane, "column");
    else if (e.ctrlKey && e.shiftKey && !e.altKey && (code === "KeyW" || k.toLowerCase() === "w")) close(pane);
    else if (e.altKey && !e.shiftKey && !e.ctrlKey && ARROWS[k]) {
      const to = movePaneFocus(termKey, ARROWS[k]);
      if (to) focusTerm(to);
    } else if (e.ctrlKey && e.shiftKey && !e.altKey && (code === "KeyC" || k.toLowerCase() === "c")) copy(pane);
    // Ctrl+C copies a selection; without one it stays the interrupt.
    else if (e.ctrlKey && !e.shiftKey && !e.altKey && (code === "KeyC" || k === "c")) done = copy(pane);
    else done = false;
    if (done) {
      e.preventDefault();
      e.stopPropagation();
    }
  };

  const menuAction = (f: (pane: string) => void) => {
    const pane = menu?.pane;
    setMenu(null);
    if (pane) {
      f(pane);
      focusTerm(getLayout(termKey).focus);
    }
  };

  const status = entry?.status ?? (closed ? "exited" : "starting");
  const focusCommand = entry?.command || (focus === termKey ? opts.tool : getTerminal(focus)?.opts.tool) || opts.tool;
  const tbBtn = "h-6 w-6 shrink-0 cursor-pointer p-0 hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-40";
  return (
    <div className="flex min-h-0 flex-1 flex-col" style={{ background: TERMINAL_CHROME.background }}>
      <div
        className="flex h-8 shrink-0 items-center gap-2 border-b px-3 text-xs"
        style={{ borderColor: TERMINAL_CHROME.border, color: TERMINAL_CHROME.text }}
      >
        <SquareTerminal className="size-3.5 shrink-0" aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate font-mono" title={entry?.command}>
          {fmt(c.terminalStatus, { command: focusCommand })}
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
            onClick={() => restart(focus)}
          >
            <RotateCw className="size-3" aria-hidden="true" />
            {c.terminalRestart}
          </Button>
        ) : null}
        <div className="flex shrink-0 items-center gap-0.5" role="group" aria-label={c.terminalPaneTools}>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className={tbBtn}
            style={{ color: TERMINAL_CHROME.strong }}
            aria-label={c.terminalSplitRight}
            title={full ? fmt(c.terminalPaneLimit, { max: MAX_PANES }) : `${c.terminalSplitRight} (Alt+Shift+=)`}
            disabled={full}
            onClick={() => {
              split(focus, "row");
            }}
          >
            <Columns2 className="size-3.5" aria-hidden="true" />
          </Button>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className={tbBtn}
            style={{ color: TERMINAL_CHROME.strong }}
            aria-label={c.terminalSplitDown}
            title={full ? fmt(c.terminalPaneLimit, { max: MAX_PANES }) : `${c.terminalSplitDown} (Alt+Shift+-)`}
            disabled={full}
            onClick={() => split(focus, "column")}
          >
            <Rows2 className="size-3.5" aria-hidden="true" />
          </Button>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className={tbBtn}
            style={{ color: TERMINAL_CHROME.strong }}
            aria-label={c.terminalClosePane}
            title={`${c.terminalClosePane} (Ctrl+Shift+W)`}
            onClick={() => close(focus)}
          >
            <X className="size-3.5" aria-hidden="true" />
          </Button>
        </div>
      </div>
      <div ref={body} className="relative min-h-0 flex-1 overflow-hidden" onKeyDownCapture={onKeyDownCapture}>
        {panes.map((pane, i) => {
          const r = rects.panes[pane];
          return (
            <Pane
              key={pane === termKey ? `${pane}@${paneGen(pane)}` : pane}
              paneKey={pane}
              index={i}
              rootKey={termKey}
              rootOpts={optsRef}
              keepAlive={keepAlive}
              multi={multi}
              full={full}
              focused={pane === focus}
              style={{ left: `${r.x * 100}%`, top: `${r.y * 100}%`, width: `${r.w * 100}%`, height: `${r.h * 100}%` }}
              c={c}
              register={(t) => {
                if (t) terms.current.set(pane, t);
                else terms.current.delete(pane);
              }}
              onPaste={(e) => pasteRef.current(e, pane)}
              onFocusPane={() => setPaneFocus(termKey, pane)}
              onMenu={(x, y) => {
                setPaneFocus(termKey, pane);
                setMenu({ x, y, pane });
              }}
              onSplit={(dir) => split(pane, dir)}
              onClose={() => close(pane)}
            />
          );
        })}
        {rects.splits.map((s) => (
          <Divider key={s.id} box={s} root={termKey} body={body} c={c} />
        ))}
        {toast ? (
          <div
            role={toast.error ? "alert" : "status"}
            className="pointer-events-none absolute right-3 bottom-3 z-20 flex max-w-[80%] items-center gap-2 rounded-md border px-3 py-1.5 text-xs shadow-lg"
            style={{ background: TERMINAL_CHROME.background, borderColor: TERMINAL_CHROME.border, color: toast.error ? TERMINAL_CHROME.error : TERMINAL_CHROME.strong }}
          >
            <ImageIcon className="size-3.5 shrink-0" aria-hidden="true" />
            <span className="min-w-0 truncate">{toast.text}</span>
          </div>
        ) : null}
      </div>
      {menu ? (
        <PaneMenu
          x={menu.x}
          y={menu.y}
          c={c}
          full={full}
          onDismiss={() => setMenu(null)}
          items={[
            { label: c.terminalSplitRight, keys: "Alt+Shift+=", needsRoom: true, run: () => menuAction((p) => split(p, "row")) },
            { label: c.terminalSplitDown, keys: "Alt+Shift+-", needsRoom: true, run: () => menuAction((p) => split(p, "column")) },
            { label: c.terminalRunClaude, needsRoom: true, run: () => menuAction((p) => split(p, "row", "claude")) },
            { label: c.terminalRunCodex, needsRoom: true, run: () => menuAction((p) => split(p, "row", "codex")), sepAfter: true },
            { label: c.terminalCopy, keys: "Ctrl+Shift+C", run: () => menuAction((p) => void copy(p)) },
            { label: c.terminalPaste, keys: "Ctrl+V", run: () => menuAction(paste), sepAfter: true },
            { label: c.terminalClosePaneShort, keys: "Ctrl+Shift+W", run: () => menuAction(close) },
          ]}
        />
      ) : null}
    </div>
  );
}

/** One pane: its own xterm attached to its pty in the store, with a small header when split. */
function Pane({
  paneKey,
  index,
  rootKey,
  rootOpts,
  keepAlive,
  multi,
  full,
  focused,
  style,
  c,
  register,
  onPaste,
  onFocusPane,
  onMenu,
  onSplit,
  onClose,
}: {
  paneKey: string;
  index: number;
  rootKey: string;
  rootOpts: React.RefObject<PtyOpenOptions>;
  keepAlive: boolean;
  multi: boolean;
  full: boolean;
  focused: boolean;
  style: React.CSSProperties;
  c: C;
  register: (t: Terminal | null) => void;
  onPaste: (e: ClipboardEvent) => void;
  onFocusPane: () => void;
  onMenu: (x: number, y: number) => void;
  onSplit: (dir: SplitDir) => void;
  onClose: () => void;
}) {
  const entry = useTerminal(paneKey);
  const host = React.useRef<HTMLDivElement>(null);
  const term = React.useRef<Terminal | null>(null);
  const cb = React.useRef({ register, onPaste });
  cb.current = { register, onPaste };
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
    cb.current.register(t);
    try {
      fit.fit();
    } catch {
      // not laid out yet
    }
    const replay = getTerminalBuffer(paneKey);
    if (replay) t.write(replay);
    const detach = attachTerminal(paneKey, (d) => t.write(d));
    const input = t.onData((d) => writeTerminal(paneKey, d));
    const cur = getTerminal(paneKey);
    if (!cur && paneKey === rootKey) void openTerminal(paneKey, { ...rootOpts.current, cols: t.cols, rows: t.rows }, keepRef.current);
    else if (cur && (cur.opts.cols !== t.cols || cur.opts.rows !== t.rows)) resizeTerminal(paneKey, t.cols, t.rows);
    // Capture phase: runs before xterm's own paste handler on its hidden textarea.
    const paste = (e: ClipboardEvent) => cb.current.onPaste(e);
    el.addEventListener("paste", paste, true);
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
        resizeTerminal(paneKey, t.cols, t.rows);
      }
    });
    ro.observe(el);
    return () => {
      el.removeEventListener("paste", paste, true);
      ro.disconnect();
      input.dispose();
      detach();
      cb.current.register(null);
      term.current = null;
      t.dispose();
    };
  }, [paneKey]);

  React.useEffect(() => {
    if (focused) term.current?.focus();
  }, [focused]);

  const exited = entry?.status === "exited" || entry?.status === "error";
  const label = fmt(c.terminalPaneLabel, { n: index + 1 });
  const action = "grid size-5 cursor-pointer place-items-center rounded hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-40";
  return (
    <div
      role="group"
      aria-label={label}
      data-pane={paneKey}
      data-focused={focused ? "true" : undefined}
      className="group/pane absolute flex min-h-0 flex-col"
      style={{
        ...style,
        border: multi ? `1px solid ${focused ? TERMINAL_CHROME.focus : "transparent"}` : undefined,
      }}
      onFocus={onFocusPane}
      onMouseDown={onFocusPane}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu(e.clientX, e.clientY);
      }}
    >
      {multi ? (
        <div
          className="flex h-5 shrink-0 items-center gap-1 px-2 text-[11px]"
          style={{ color: focused ? TERMINAL_CHROME.strong : TERMINAL_CHROME.text, background: TERMINAL_CHROME.paneHeader }}
        >
          <span className="min-w-0 flex-1 truncate font-mono" title={entry?.command}>
            {index + 1} · {entry?.command || entry?.opts.tool || ""}
            {exited ? ` · ${entry?.status === "error" ? fmt(c.terminalFailed, { error: entry.error ?? "" }) : c.terminalEnded}` : ""}
          </span>
          <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover/pane:opacity-100 focus-within:opacity-100">
            <button type="button" className={action} aria-label={`${label} · ${c.terminalSplitRight}`} title={c.terminalSplitRight} disabled={full} onClick={() => onSplit("row")}>
              <Columns2 className="size-3" aria-hidden="true" />
            </button>
            <button type="button" className={action} aria-label={`${label} · ${c.terminalSplitDown}`} title={c.terminalSplitDown} disabled={full} onClick={() => onSplit("column")}>
              <Rows2 className="size-3" aria-hidden="true" />
            </button>
            <button type="button" className={action} aria-label={`${label} · ${c.terminalClosePaneShort}`} title={c.terminalClosePaneShort} onClick={onClose}>
              <X className="size-3" aria-hidden="true" />
            </button>
          </span>
        </div>
      ) : null}
      <div ref={host} data-testid="terminal-host" className="min-h-0 flex-1 overflow-hidden py-1 pl-2" />
    </div>
  );
}

/** A draggable line between the two halves of a split; double-click splits evenly. */
function Divider({ box, root, body, c }: { box: SplitBox; root: string; body: React.RefObject<HTMLDivElement | null>; c: C }) {
  const row = box.dir === "row";
  const { rect } = box;
  const style: React.CSSProperties = row
    ? { left: `calc(${(rect.x + rect.w * box.ratio) * 100}% - 3px)`, top: `${rect.y * 100}%`, width: 6, height: `${rect.h * 100}%`, cursor: "col-resize" }
    : { top: `calc(${(rect.y + rect.h * box.ratio) * 100}% - 3px)`, left: `${rect.x * 100}%`, height: 6, width: `${rect.w * 100}%`, cursor: "row-resize" };
  const onPointerDown = (e: React.PointerEvent) => {
    const el = body.current;
    if (!el || e.button !== 0) return;
    e.preventDefault();
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture?.(e.pointerId);
    const b = el.getBoundingClientRect();
    const start = row ? b.left + rect.x * b.width : b.top + rect.y * b.height;
    const size = row ? rect.w * b.width : rect.h * b.height;
    const min = row ? MIN_W : MIN_H;
    const move = (ev: PointerEvent) => {
      if (size <= 0) return;
      const pos = (row ? ev.clientX : ev.clientY) - start;
      const lo = Math.min(0.5, min / size);
      setSplitRatio(root, box.id, Math.min(1 - lo, Math.max(lo, pos / size)));
    };
    const up = () => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
      target.removeEventListener("pointercancel", up);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
    target.addEventListener("pointercancel", up);
  };
  return (
    <div
      role="separator"
      aria-orientation={row ? "vertical" : "horizontal"}
      aria-label={c.terminalDivider}
      aria-valuenow={Math.round(box.ratio * 100)}
      title={c.terminalDivider}
      className="group/div absolute z-10 flex items-center justify-center"
      style={style}
      onPointerDown={onPointerDown}
      onDoubleClick={() => setSplitRatio(root, box.id, 0.5)}
    >
      <div className={row ? "h-full w-px" : "h-px w-full"} style={{ background: TERMINAL_CHROME.border }} />
    </div>
  );
}

interface MenuItem {
  label: string;
  keys?: string;
  needsRoom?: boolean;
  sepAfter?: boolean;
  run: () => void;
}

/** The right-click menu of a pane. */
function PaneMenu({ x, y, c, full, items, onDismiss }: { x: number; y: number; c: C; full: boolean; items: MenuItem[]; onDismiss: () => void }) {
  const ref = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    const down = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onDismiss();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") onDismiss();
    };
    document.addEventListener("mousedown", down, true);
    document.addEventListener("keydown", key, true);
    ref.current?.querySelector<HTMLElement>("[role=menuitem]:not([disabled])")?.focus();
    return () => {
      document.removeEventListener("mousedown", down, true);
      document.removeEventListener("keydown", key, true);
    };
  }, [onDismiss]);
  // Keep it on screen.
  const left = Math.max(4, Math.min(x, window.innerWidth - 248));
  const top = Math.max(4, Math.min(y, window.innerHeight - 280));
  return (
    <div
      ref={ref}
      role="menu"
      aria-label={c.terminalPaneMenu}
      className="fixed z-50 min-w-56 rounded-md border py-1 text-xs shadow-lg"
      style={{ left, top, background: TERMINAL_CHROME.menu, borderColor: TERMINAL_CHROME.border, color: TERMINAL_CHROME.strong }}
    >
      {items.map((it) => (
        <React.Fragment key={it.label}>
          <button
            type="button"
            role="menuitem"
            disabled={it.needsRoom && full}
            className="flex w-full cursor-pointer items-center justify-between gap-6 px-3 py-1.5 text-left outline-none hover:bg-white/10 focus-visible:bg-white/10 disabled:cursor-not-allowed disabled:opacity-40"
            onClick={it.run}
          >
            <span>{it.label}</span>
            {it.keys ? <span style={{ color: TERMINAL_CHROME.text }}>{it.keys}</span> : null}
          </button>
          {it.sepAfter ? <div role="separator" className="my-1 h-px" style={{ background: TERMINAL_CHROME.border }} /> : null}
        </React.Fragment>
      ))}
    </div>
  );
}
