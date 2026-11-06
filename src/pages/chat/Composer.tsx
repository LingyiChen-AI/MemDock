import * as React from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { ArrowUp, ChevronDown, Cpu, ImagePlus, Loader2, Paperclip, Shield, Square, X } from "lucide-react";
import { readImageDataUri, type LiveAgent, type PermissionMode } from "@/api/live";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Select, SelectContent, SelectItem, SelectTrigger } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import type { Messages } from "@/i18n";
import { fmt } from "@/pages/memory/paths";
import { errorMessage } from "@/pages/skills/skillUtils";
import { MODEL_SUGGESTIONS, loadChatPrefs, rememberAgentSettings } from "./chatPrefs";
import { IMAGE_EXTS, MAX_IMAGE_BYTES, fileToDataUri, formatInsertPath, inRect, insertAtCursor, isImagePath } from "./composerUtils";

type C = Messages["chat"];

const MODES: PermissionMode[] = ["ask", "auto_edit", "plan", "bypass"];

/** What a send hands to the caller. `permission` / `model` matter only when the send (re)starts a process. */
export interface ComposerMessage {
  text: string;
  images: string[];
  permission: PermissionMode;
  model: string;
}

export interface ComposerProps {
  c: C;
  agent: LiveAgent;
  /** Session folder: dropped files inside it are inserted as relative paths. */
  cwd: string;
  disabled?: boolean;
  placeholder?: string;
  /** Small line under the card (e.g. "continues the session" / "queued"). */
  hint?: React.ReactNode;
  /** A turn is running: shows the stop button next to send. */
  running?: boolean;
  onStop?: () => void;
  /**
   * Settings of the attached (running) process. When set, the toolbar shows them read-only;
   * otherwise the toolbar edits the remembered prefs used when the next send starts a process.
   */
  attached?: { permission: PermissionMode; model: string | null } | null;
  onSend: (msg: ComposerMessage) => Promise<unknown>;
  onError: (e: string | null) => void;
}

function ToolbarTip({ tip, children }: { tip: React.ReactNode; children: React.ReactElement }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent side="top" className="max-w-64">
        {tip}
      </TooltipContent>
    </Tooltip>
  );
}

const PILL =
  "h-7 gap-1 rounded-lg border border-border bg-transparent px-2 text-xs font-normal text-muted-foreground shadow-none hover:bg-muted hover:text-foreground dark:bg-transparent dark:hover:bg-muted [&_svg:not([class*='size-'])]:size-3.5";
const PILL_WARN = "border-amber-500/50 bg-amber-500/10 text-amber-700 hover:bg-amber-500/15 hover:text-amber-800 dark:bg-amber-500/10 dark:text-amber-300 dark:hover:bg-amber-500/20 dark:hover:text-amber-200";

function PermissionPicker({ value, onChange, locked, c }: { value: PermissionMode; onChange: (m: PermissionMode) => void; locked: boolean; c: C }) {
  const trigger = (
    <SelectTrigger size="sm" aria-label={c.composer.permission} className={cn(PILL, value === "bypass" && PILL_WARN, "disabled:opacity-70")}>
      <Shield aria-hidden="true" />
      <span className="truncate">{c.permissions[value]}</span>
    </SelectTrigger>
  );
  return (
    <Select value={value} onValueChange={(v) => onChange(v as PermissionMode)} disabled={locked}>
      <ToolbarTip tip={locked ? c.composer.lockedHint : c.permissionHints[value]}>
        {locked ? <span tabIndex={0} className="inline-flex rounded-lg outline-none focus-visible:ring-2 focus-visible:ring-ring">{trigger}</span> : trigger}
      </ToolbarTip>
      <SelectContent align="start" className="min-w-56">
        {MODES.map((m) => (
          <SelectItem key={m} value={m} className={cn(m === "bypass" && "text-amber-700 dark:text-amber-300")}>
            <span className="flex flex-col gap-0.5">
              <span>{c.permissions[m]}</span>
              <span className="text-[11px] text-muted-foreground">{c.permissionHints[m]}</span>
            </span>
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

function ModelPicker({ agent, value, onChange, locked, c }: { agent: LiveAgent; value: string; onChange: (m: string) => void; locked: boolean; c: C }) {
  const [open, setOpen] = React.useState(false);
  const [draft, setDraft] = React.useState(value);
  React.useEffect(() => {
    if (open) setDraft(value);
  }, [open, value]);
  const pick = (m: string) => {
    onChange(m.trim());
    setOpen(false);
  };
  const button = (
    <Button type="button" variant="outline" size="sm" disabled={locked} aria-label={c.composer.model} className={cn(PILL, "cursor-pointer font-mono disabled:opacity-70")}>
      <Cpu aria-hidden="true" />
      <span className="max-w-32 truncate">{value || c.modelDefault}</span>
      <ChevronDown className="size-3 opacity-60" aria-hidden="true" />
    </Button>
  );
  if (locked) {
    return (
      <ToolbarTip tip={c.composer.lockedHint}>
        <span tabIndex={0} className="inline-flex rounded-lg outline-none focus-visible:ring-2 focus-visible:ring-ring">
          {button}
        </span>
      </ToolbarTip>
    );
  }
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>{button}</PopoverTrigger>
      <PopoverContent align="start" className="flex w-60 flex-col gap-2 p-2">
        <Input
          autoFocus
          aria-label={c.composer.model}
          value={draft}
          placeholder={c.composer.modelPlaceholder}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.nativeEvent.isComposing) {
              e.preventDefault();
              pick(draft);
            }
          }}
          className="h-8 font-mono text-xs"
        />
        <div role="listbox" aria-label={c.composer.model} className="flex flex-col">
          {["", ...MODEL_SUGGESTIONS[agent]].map((m) => (
            <button
              key={m || "default"}
              type="button"
              role="option"
              aria-selected={value === m}
              onClick={() => pick(m)}
              className={cn(
                "cursor-pointer rounded-md px-2 py-1.5 text-left text-xs outline-none hover:bg-muted focus-visible:bg-muted",
                m ? "font-mono" : "text-muted-foreground",
                value === m && "seg-active",
              )}
            >
              {m || c.modelDefault}
            </button>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}

/** Card composer: attachments, auto-growing text, file/image buttons, permission + model, send / stop. */
export function ChatComposer({ c, agent, cwd, disabled = false, placeholder, hint, running = false, onStop, attached = null, onSend, onError }: ComposerProps) {
  const k = c.composer;
  const [text, setText] = React.useState("");
  const [images, setImages] = React.useState<string[]>([]);
  const [sending, setSending] = React.useState(false);
  const [dragging, setDragging] = React.useState(false);
  const [prefs, setPrefs] = React.useState(() => {
    const p = loadChatPrefs();
    return { permission: p.permission[agent], model: p.model[agent] };
  });
  const ref = React.useRef<HTMLTextAreaElement>(null);
  const rootRef = React.useRef<HTMLDivElement>(null);
  const textRef = React.useRef(text);
  textRef.current = text;

  const locked = attached !== null;
  const permission = attached ? attached.permission : prefs.permission;
  const model = attached ? (attached.model ?? "") : prefs.model;
  const changePrefs = (next: { permission: PermissionMode; model: string }) => {
    setPrefs(next);
    rememberAgentSettings(agent, next.permission, next.model);
  };

  const addImages = React.useCallback((uris: string[]) => {
    if (uris.length) setImages((xs) => [...xs, ...uris]);
  }, []);

  /** Inserts paths at the caret (relative to `cwd` when inside it). */
  const insertPaths = React.useCallback(
    (paths: string[]) => {
      if (!paths.length) return;
      const el = ref.current;
      const cur = textRef.current;
      const start = el?.selectionStart ?? cur.length;
      const end = el?.selectionEnd ?? cur.length;
      const r = insertAtCursor(cur, start, end, paths.map((p) => formatInsertPath(p, cwd)).join(" "));
      setText(r.text);
      requestAnimationFrame(() => {
        if (!ref.current) return;
        ref.current.focus();
        ref.current.setSelectionRange(r.caret, r.caret);
      });
    },
    [cwd],
  );

  const loadImageFiles = React.useCallback(
    async (paths: string[]) => {
      const out: string[] = [];
      for (const p of paths) {
        try {
          out.push(await readImageDataUri(p));
        } catch (e) {
          onError(fmt(k.imageFailed, { error: errorMessage(e) }));
        }
      }
      addImages(out);
    },
    [addImages, onError, k.imageFailed],
  );

  /** Dropped / picked paths: images become attachments, everything else is inserted as a path. */
  const handlePaths = React.useCallback(
    (paths: string[]) => {
      const imgs = paths.filter(isImagePath);
      insertPaths(paths.filter((p) => !isImagePath(p)));
      if (imgs.length) void loadImageFiles(imgs);
    },
    [insertPaths, loadImageFiles],
  );

  // Tauri hands dropped files to the webview as paths (HTML5 drop has none). Only react while the
  // pointer is over this composer's pane (hidden pages have an empty rect).
  const disabledRef = React.useRef(disabled);
  disabledRef.current = disabled;
  const handlePathsRef = React.useRef(handlePaths);
  handlePathsRef.current = handlePaths;
  React.useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    const over = (pos: { x: number; y: number }) => {
      const zone = rootRef.current?.parentElement ?? rootRef.current;
      if (!zone || disabledRef.current) return false;
      const r = zone.getBoundingClientRect();
      return r.width > 0 && r.height > 0 && inRect(pos, r, window.devicePixelRatio);
    };
    try {
      getCurrentWebview()
        .onDragDropEvent(({ payload }) => {
          if (payload.type === "leave") setDragging(false);
          else if (payload.type === "enter" || payload.type === "over") setDragging(over(payload.position));
          else if (payload.type === "drop") {
            setDragging(false);
            if (over(payload.position)) handlePathsRef.current(payload.paths);
          }
        })
        .then((f) => {
          if (alive) unlisten = f;
          else f();
        })
        .catch(() => {});
    } catch {
      // not running inside Tauri (tests, plain browser)
    }
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  const onPaste = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(e.clipboardData?.files ?? []).filter((f) => f.type.startsWith("image/"));
    if (!files.length) return;
    e.preventDefault();
    void (async () => {
      const out: string[] = [];
      for (const f of files) {
        if (f.size > MAX_IMAGE_BYTES) {
          onError(fmt(k.imageTooLarge, { name: f.name || "image" }));
          continue;
        }
        try {
          out.push(await fileToDataUri(f));
        } catch (err) {
          onError(fmt(k.imageFailed, { error: errorMessage(err) }));
        }
      }
      addImages(out);
    })();
  };

  const pick = async (wantImages: boolean) => {
    try {
      const r = await openDialog(
        wantImages ? { multiple: true, filters: [{ name: k.imageFilter, extensions: IMAGE_EXTS }] } : { multiple: true, defaultPath: cwd || undefined },
      );
      const paths = r === null ? [] : Array.isArray(r) ? r : [r];
      if (wantImages) void loadImageFiles(paths);
      else handlePaths(paths);
    } catch (e) {
      onError(fmt(k.pickFailed, { error: errorMessage(e) }));
    }
  };

  const canSend = !disabled && !sending && (text.trim().length > 0 || images.length > 0);
  const send = async () => {
    if (!canSend) return;
    setSending(true);
    onError(null);
    try {
      await onSend({ text: text.trim(), images, permission, model: model.trim() });
      setText("");
      setImages([]);
    } catch (e) {
      onError(fmt(c.sendFailed, { error: errorMessage(e) }));
    } finally {
      setSending(false);
      ref.current?.focus();
    }
  };

  return (
    <div ref={rootRef} className="flex flex-col gap-1.5 px-4 pt-2 pb-3">
      <div
        className={cn(
          "relative rounded-[14px] border border-border bg-card shadow-sm transition-[border-color,box-shadow] duration-150 motion-reduce:transition-none",
          "focus-within:border-primary focus-within:ring-3 focus-within:ring-primary/15",
          disabled && "opacity-70",
        )}
      >
        {images.length ? (
          <ul aria-label={k.attachImage} className="flex flex-wrap gap-2 px-3.5 pt-3">
            {images.map((src, i) => (
              <li key={i} className="relative">
                <img src={src} alt={fmt(k.imageAlt, { n: i + 1 })} className="block size-14 rounded-lg border border-border object-cover" />
                <button
                  type="button"
                  aria-label={fmt(k.removeImage, { n: i + 1 })}
                  onClick={() => setImages((xs) => xs.filter((_, j) => j !== i))}
                  className="absolute -top-1.5 -right-1.5 grid size-[18px] cursor-pointer place-items-center rounded-full bg-foreground text-background shadow outline-none hover:opacity-85 focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <X className="size-3" aria-hidden="true" />
                </button>
              </li>
            ))}
          </ul>
        ) : null}
        <Textarea
          ref={ref}
          aria-label={c.inputLabel}
          value={text}
          disabled={disabled}
          rows={1}
          placeholder={placeholder ?? fmt(k.placeholder, { agent: c.agents[agent] })}
          onChange={(e) => setText(e.target.value)}
          onPaste={onPaste}
          onKeyDown={(e) => {
            // IME composition (e.g. Chinese input) uses Enter to pick a candidate.
            if (e.key !== "Enter" || e.shiftKey || e.nativeEvent.isComposing || e.keyCode === 229) return;
            e.preventDefault();
            void send();
          }}
          className="max-h-[232px] min-h-11 resize-none overflow-y-auto rounded-none border-0 bg-transparent px-3.5 pt-3 pb-1 leading-5 shadow-none focus-visible:ring-0 disabled:bg-transparent dark:bg-transparent dark:disabled:bg-transparent"
        />
        <div className="flex min-w-0 flex-wrap items-center gap-1 px-2 pt-1 pb-2">
          <ToolbarTip tip={k.attachFile}>
            <Button type="button" variant="ghost" size="icon-sm" className="cursor-pointer text-muted-foreground" aria-label={k.attachFile} disabled={disabled} onClick={() => void pick(false)}>
              <Paperclip aria-hidden="true" />
            </Button>
          </ToolbarTip>
          <ToolbarTip tip={k.attachImage}>
            <Button type="button" variant="ghost" size="icon-sm" className="cursor-pointer text-muted-foreground" aria-label={k.attachImage} disabled={disabled} onClick={() => void pick(true)}>
              <ImagePlus aria-hidden="true" />
            </Button>
          </ToolbarTip>
          <PermissionPicker value={permission} locked={locked} c={c} onChange={(m) => changePrefs({ ...prefs, permission: m })} />
          <ModelPicker agent={agent} value={model} locked={locked} c={c} onChange={(m) => changePrefs({ ...prefs, model: m })} />
          <span className="flex-1" />
          <span className="hidden px-1 text-[11px] text-muted-foreground/80 sm:inline">{k.keysHint}</span>
          {running && onStop ? (
            <ToolbarTip tip={c.stopHint}>
              <Button type="button" variant="outline" size="icon-sm" className="cursor-pointer rounded-full" aria-label={c.stop} onClick={onStop}>
                <Square className="size-3 fill-current" aria-hidden="true" />
              </Button>
            </ToolbarTip>
          ) : null}
          <Button
            type="button"
            size="icon-sm"
            className="cursor-pointer rounded-full disabled:opacity-40"
            aria-label={c.send}
            disabled={!canSend}
            onClick={() => void send()}
          >
            {sending ? <Loader2 className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <ArrowUp aria-hidden="true" />}
          </Button>
        </div>
        {dragging ? (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-0 grid place-items-center rounded-[14px] border-2 border-dashed border-primary bg-primary/10 text-center backdrop-blur-[1px]"
          >
            <div className="flex flex-col gap-0.5 rounded-lg bg-card/85 px-3 py-1.5">
              <span className="text-sm font-semibold text-primary">{k.dropTitle}</span>
              <span className="text-[11px] text-muted-foreground">{k.dropHint}</span>
            </div>
          </div>
        ) : null}
      </div>
      {hint ? <p className="px-1 text-[11px] text-muted-foreground">{hint}</p> : null}
    </div>
  );
}
