import * as React from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { ArrowUp, Check, ChevronDown, Cpu, ImagePlus, Loader2, Paperclip, RefreshCw, Search, Shield, Square, X } from "lucide-react";
import { liveModels, readImageDataUri, type LiveAgent, type ModelOption, type PermissionMode } from "@/api/live";
import { Button } from "@/components/ui/button";
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

/** Model list per agent, fetched once from the CLI (backend caches it for an hour). */
const modelLists: Partial<Record<LiveAgent, Promise<ModelOption[]>>> = {};

function loadModels(agent: LiveAgent, force = false): Promise<ModelOption[]> {
  if (force || !modelLists[agent]) {
    modelLists[agent] = liveModels(agent, force).catch((e) => {
      delete modelLists[agent];
      throw e;
    });
  }
  return modelLists[agent]!;
}

/** Rows of the picker: "" is the CLI's default model. */
interface ModelRow {
  value: string;
  name: string;
  description: string;
}

function toRows(agent: LiveAgent, models: ModelOption[] | null, c: C): ModelRow[] {
  if (!models) return [{ value: "", name: c.modelDefault, description: "" }, ...MODEL_SUGGESTIONS[agent].map((m) => ({ value: m, name: m, description: "" }))];
  const rows: ModelRow[] = [];
  const def = models.find((m) => m.is_default);
  // Claude lists its default as value "default"; Codex flags one model as the default.
  rows.push({
    value: "",
    name: c.modelDefault,
    description: def ? (agent === "claude" ? def.description : def.display_name) : "",
  });
  for (const m of models) if (m.value !== "default") rows.push({ value: m.value, name: m.display_name, description: m.description });
  return rows;
}

function ModelPicker({ agent, value, onChange, locked, c }: { agent: LiveAgent; value: string; onChange: (m: string) => void; locked: boolean; c: C }) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [models, setModels] = React.useState<ModelOption[] | null>(null);
  const [loading, setLoading] = React.useState(false);
  const [failed, setFailed] = React.useState(false);

  const fetchModels = React.useCallback(
    (force = false) => {
      setLoading(true);
      setFailed(false);
      loadModels(agent, force)
        .then((m) => setModels(m))
        .catch(() => setFailed(true))
        .finally(() => setLoading(false));
    },
    [agent],
  );
  React.useEffect(() => {
    setModels(null);
    fetchModels();
  }, [fetchModels]);
  React.useEffect(() => {
    if (open) setQuery("");
  }, [open]);

  const rows = toRows(agent, models, c);
  const label = rows.find((r) => r.value === value)?.name ?? (value || c.modelDefault);
  const q = query.trim().toLowerCase();
  const shown = q ? rows.filter((r) => `${r.value} ${r.name} ${r.description}`.toLowerCase().includes(q)) : rows;
  const custom = q && !rows.some((r) => r.value.toLowerCase() === q) ? query.trim() : null;
  const pick = (m: string) => {
    onChange(m.trim());
    setOpen(false);
  };

  const button = (
    <Button type="button" variant="outline" size="sm" disabled={locked} aria-label={c.composer.model} className={cn(PILL, "cursor-pointer disabled:opacity-70")}>
      <Cpu aria-hidden="true" />
      <span className="max-w-36 truncate">{label}</span>
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
      <PopoverContent align="start" side="top" className="flex w-80 flex-col gap-0 overflow-hidden p-0">
        <div className="flex items-center gap-1.5 border-b border-border px-2.5 py-2">
          <Search className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
          <input
            autoFocus
            aria-label={c.composer.modelSearch}
            value={query}
            placeholder={c.composer.modelSearch}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key !== "Enter" || e.nativeEvent.isComposing) return;
              e.preventDefault();
              if (custom) pick(custom);
              else if (shown[0]) pick(shown[0].value);
            }}
            className="h-7 min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
          />
          <ToolbarTip tip={c.composer.modelRefresh}>
            <button
              type="button"
              aria-label={c.composer.modelRefresh}
              onClick={() => fetchModels(true)}
              className="grid size-6 cursor-pointer place-items-center rounded-md text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            >
              <RefreshCw className={cn("size-3.5", loading && "animate-spin motion-reduce:animate-none")} aria-hidden="true" />
            </button>
          </ToolbarTip>
        </div>
        <div role="listbox" aria-label={c.composer.model} className="flex max-h-80 flex-col gap-0.5 overflow-y-auto p-1">
          {loading && !models ? (
            <div className="flex flex-col gap-1 p-1" aria-busy="true" aria-label={c.composer.modelLoading}>
              {[0, 1, 2, 3].map((i) => (
                <div key={i} className="flex flex-col gap-1 rounded-md px-2 py-1.5">
                  <div className="h-3.5 w-28 animate-pulse rounded bg-muted motion-reduce:animate-none" />
                  <div className="h-3 w-44 animate-pulse rounded bg-muted/70 motion-reduce:animate-none" />
                </div>
              ))}
            </div>
          ) : (
            shown.map((r) => {
              const selected = value === r.value;
              return (
                <button
                  key={r.value || "default"}
                  type="button"
                  role="option"
                  aria-selected={selected}
                  onClick={() => pick(r.value)}
                  className={cn(
                    "flex w-full cursor-pointer items-start gap-2 rounded-md px-2 py-1.5 text-left outline-none transition-colors hover:bg-muted focus-visible:bg-muted",
                    selected && "bg-primary/8 hover:bg-primary/12",
                  )}
                >
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className={cn("truncate text-sm text-foreground", selected && "font-semibold text-primary")}>{r.name}</span>
                    {r.description || (r.value && r.value !== r.name) ? (
                      <span className="truncate text-[11px] text-muted-foreground">
                        {r.description}
                        {r.value && r.value !== r.name ? <span className="font-mono">{r.description ? " · " : ""}{r.value}</span> : null}
                      </span>
                    ) : null}
                  </span>
                  {selected ? <Check className="mt-0.5 size-4 shrink-0 text-primary" aria-hidden="true" /> : null}
                </button>
              );
            })
          )}
          {custom ? (
            <button
              type="button"
              role="option"
              aria-selected={false}
              onClick={() => pick(custom)}
              className="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm outline-none hover:bg-muted focus-visible:bg-muted"
            >
              {fmt(c.composer.modelCustom, { model: custom })}
            </button>
          ) : null}
          {failed ? <p className="px-2 py-1.5 text-[11px] text-muted-foreground">{c.composer.modelListFailed}</p> : null}
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
          rows={3}
          placeholder={placeholder ?? fmt(k.placeholder, { agent: c.agents[agent] })}
          onChange={(e) => setText(e.target.value)}
          onPaste={onPaste}
          onKeyDown={(e) => {
            // IME composition (e.g. Chinese input) uses Enter to pick a candidate.
            if (e.key !== "Enter" || e.shiftKey || e.nativeEvent.isComposing || e.keyCode === 229) return;
            e.preventDefault();
            void send();
          }}
          className="max-h-[280px] min-h-[84px] resize-none overflow-y-auto rounded-none border-0 bg-transparent px-3.5 pt-3 pb-1 leading-5 shadow-none focus-visible:ring-0 disabled:bg-transparent dark:bg-transparent dark:disabled:bg-transparent"
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
