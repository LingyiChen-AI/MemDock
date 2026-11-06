import * as React from "react";
import CodeMirror from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { EditorView } from "@codemirror/view";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Check, Copy, History, ImageOff, Info, Link2, Save, TriangleAlert, Unlink2 } from "lucide-react";
import { onMemoryChanged, readMemoryFile, writeMemoryFile } from "@/api/memory";
import type { CommandError, FileContentDto, MemoryFile, MemoryIndex } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { memdockCmTheme } from "./cmTheme";
import { rehypeSourceLine } from "./rehypeSourceLine";
import { useScrollSync } from "./useScrollSync";
import { HistoryDrawer } from "./HistoryDrawer";
import { CodexDbView } from "./CodexDbView";
import { ConfirmDialog } from "./ConfirmDialog";
import {
  basename,
  compactNumber,
  dirname,
  findByResolvedPath,
  fmt,
  indexIsWindows,
  normalizePath,
} from "./paths";

export type EditorMode = "edit" | "split" | "preview";
const MODE_KEY = "memdock.editorMode";
const MODES: EditorMode[] = ["edit", "split", "preview"];

function loadMode(): EditorMode {
  try {
    const v = localStorage.getItem(MODE_KEY);
    if (v && (MODES as string[]).includes(v)) return v as EditorMode;
  } catch {
    // storage unavailable: fall through
  }
  return "split";
}

const SYNC_KEY = "memdock.syncScroll";

function loadSync(): boolean {
  try {
    return localStorage.getItem(SYNC_KEY) !== "0";
  } catch {
    return true;
  }
}

function asCommandError(e: unknown): CommandError | null {
  return typeof e === "object" && e !== null && "code" in e && "message" in e ? (e as CommandError) : null;
}

function errorMessage(e: unknown): string {
  return asCommandError(e)?.message ?? (e instanceof Error ? e.message : String(e));
}

export interface MemoryEditorProps {
  file: MemoryFile;
  index: MemoryIndex;
  /** The file is no longer in the index (deleted externally or its project was removed). */
  removed?: boolean;
  /** Opens an imported file that exists in the index. */
  onOpenImport?: (file: MemoryFile) => void;
  /** Reports whether there are unsaved edits (the page confirms before switching files). */
  onDirtyChange?: (dirty: boolean) => void;
  /** Scrolls to a 1-based line and puts the cursor there; a new `nonce` repeats the jump. */
  reveal?: { line: number; nonce: number };
  /** Called once a reveal has been applied, so the page can drop it. */
  onRevealed?: () => void;
}

export function MemoryEditor(props: MemoryEditorProps) {
  if (props.file.kind === "codex_db") return <CodexDbView />;
  return <TextEditor {...props} />;
}

function TextEditor({ file, index, removed, onOpenImport, onDirtyChange, reveal, onRevealed }: MemoryEditorProps) {
  const { t } = useI18n();
  const m = t.memory;

  const [base, setBase] = React.useState<FileContentDto | null>(null);
  const [text, setText] = React.useState("");
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [external, setExternal] = React.useState(false);
  const [conflict, setConflict] = React.useState(false);
  const [saving, setSaving] = React.useState(false);
  const [copied, setCopied] = React.useState(false);
  const [mode, setModeState] = React.useState<EditorMode>(loadMode);
  const [historyOpen, setHistoryOpen] = React.useState(false);
  const [view, setView] = React.useState<EditorView | null>(null);
  const [syncOn, setSyncOn] = React.useState(loadSync);
  const previewRef = React.useRef<HTMLDivElement>(null);
  const revealed = React.useRef(0);

  const dirty = base !== null && text !== base.text;
  // A not-yet-created file needs some text before it can be created.
  const canSave = dirty || (!file.exists && text !== "");

  // Latest values for async callbacks and the watcher subscription.
  const live = React.useRef({ path: file.path, dirty, hash: base?.hash ?? "", windows: false, exists: file.exists });
  live.current = {
    path: file.path,
    dirty,
    hash: base?.hash ?? "",
    windows: indexIsWindows(index),
    exists: file.exists,
  };
  const loadSeq = React.useRef(0);

  const apply = React.useCallback((dto: FileContentDto) => {
    setBase(dto);
    setText(dto.text);
    setExternal(false);
    setConflict(false);
    setSaveError(null);
  }, []);

  const load = React.useCallback(async () => {
    const seq = ++loadSeq.current;
    setLoadError(null);
    if (!live.current.exists) {
      apply({ text: "", hash: "", has_bom: false, crlf: false, tokens: 0, imports: [] });
      return;
    }
    try {
      const dto = await readMemoryFile(live.current.path);
      if (seq === loadSeq.current) apply(dto);
    } catch (e) {
      if (seq === loadSeq.current) setLoadError(errorMessage(e));
    }
  }, [apply]);

  // (Re)load when another file is opened.
  React.useEffect(() => {
    setBase(null);
    setText("");
    void load();
    return () => {
      loadSeq.current++;
    };
  }, [file.path, load]);

  React.useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);
  React.useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);

  // External modifications of the open file.
  React.useEffect(() => {
    let unlisten: (() => void) | undefined;
    let alive = true;
    onMemoryChanged(({ paths }) => {
      const cur = live.current;
      const mine = normalizePath(cur.path, cur.windows);
      if (!paths.some((p) => normalizePath(p, cur.windows) === mine)) return;
      const seq = loadSeq.current;
      readMemoryFile(cur.path).then(
        (dto) => {
          if (seq !== loadSeq.current) return;
          // Echo of our own save (or no real change).
          if (dto.hash === live.current.hash) return;
          if (live.current.dirty) setExternal(true);
          else apply(dto);
        },
        () => {
          // Deleted or unreadable: leave the editor content alone.
        },
      );
    }).then((u) => {
      if (alive) unlisten = u;
      else u();
    });
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [apply]);

  const save = React.useCallback(async () => {
    if (!base || saving) return;
    const sent = text;
    setSaving(true);
    setSaveError(null);
    try {
      const dto = await writeMemoryFile(file.path, sent, base.hash);
      setBase(dto);
      setText((cur) => (cur === sent ? dto.text : cur));
      setExternal(false);
    } catch (e) {
      const err = asCommandError(e);
      if (err?.code === "disk_changed") {
        setConflict(true);
        setExternal(true);
      } else {
        setSaveError(fmt(m.saveFailed, { message: errorMessage(e) }));
      }
    } finally {
      setSaving(false);
    }
  }, [base, saving, text, file.path, m]);

  const { leadEditor } = useScrollSync({ enabled: mode === "split" && syncOn, view, previewRef });

  // Jump to the requested line (search results). Waits until the editor exists and has the text.
  React.useEffect(() => {
    if (!reveal || reveal.nonce === revealed.current || !base) return;
    if (mode === "preview") {
      setModeState("split");
      return;
    }
    if (!view || !view.dom.isConnected) return;
    revealed.current = reveal.nonce;
    leadEditor();
    const line = view.state.doc.line(Math.min(Math.max(1, reveal.line), view.state.doc.lines));
    view.dispatch({
      selection: { anchor: line.from },
      effects: EditorView.scrollIntoView(line.from, { y: "center" }),
    });
    view.focus();
    onRevealed?.();
  }, [reveal, base, view, mode, onRevealed, leadEditor]);

  const reload = () => {
    setConflict(false);
    void load();
  };

  const setMode = (next: EditorMode) => {
    setModeState(next);
    try {
      localStorage.setItem(MODE_KEY, next);
    } catch {
      // ignore
    }
  };

  const toggleSync = () => {
    const next = !syncOn;
    setSyncOn(next);
    try {
      localStorage.setItem(SYNC_KEY, next ? "1" : "0");
    } catch {
      // ignore
    }
  };

  const copyPath = async () => {
    try {
      await navigator.clipboard.writeText(file.path);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // clipboard unavailable
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
      e.preventDefault();
      if (canSave) void save();
    }
  };

  const extensions = React.useMemo(() => [markdown(), ...memdockCmTheme], []);
  const modeLabels: Record<EditorMode, string> = { edit: m.modeEdit, split: m.modeSplit, preview: m.modePreview };
  const dir = dirname(file.path);
  const name = basename(file.path);
  const imports = base?.imports ?? [];

  return (
    <div className="flex h-full min-w-0 flex-1 flex-col" onKeyDown={onKeyDown}>
      <div className="@container flex h-10 shrink-0 items-center gap-2.5 border-b border-border px-4">
        {dirty ? (
          <span
            role="img"
            aria-label={m.unsaved}
            title={m.unsaved}
            className="size-[7px] shrink-0 rounded-full bg-foreground"
          />
        ) : null}
        <span
          className="flex min-w-[6rem] shrink font-mono text-[12.5px] text-muted-foreground select-text"
          title={file.path}
        >
          <span className="min-w-0 truncate">{dir}</span>
          <b className="min-w-0 truncate font-medium text-foreground">{name}</b>
        </span>
        <button
          type="button"
          aria-label={m.copyPath}
          title={copied ? m.copied : m.copyPath}
          onClick={copyPath}
          className="grid size-6 shrink-0 cursor-pointer place-items-center rounded-sm text-muted-foreground transition-colors duration-150 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none"
        >
          {copied ? <Check className="size-3.5" aria-hidden="true" /> : <Copy className="size-3.5" aria-hidden="true" />}
        </button>
        {base ? (
          <span className="shrink-0 rounded-full border border-border px-2 text-xs leading-5 whitespace-nowrap text-muted-foreground @max-[42rem]:hidden">
            {fmt(m.tokens, { n: compactNumber(base.tokens) })}
          </span>
        ) : null}
        <span className="flex-1" />
        {mode === "split" ? (
          <button
            type="button"
            aria-label={m.syncScroll}
            aria-pressed={syncOn}
            title={m.syncScroll}
            onClick={toggleSync}
            className={cn(
              "grid size-7 shrink-0 cursor-pointer place-items-center rounded-md outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none",
              syncOn ? "seg-active" : "text-muted-foreground hover:text-foreground",
            )}
          >
            {syncOn ? <Link2 className="size-4" aria-hidden="true" /> : <Unlink2 className="size-4" aria-hidden="true" />}
          </button>
        ) : null}
        <div
          role="radiogroup"
          aria-label={m.modeLabel}
          className="inline-flex shrink-0 overflow-hidden rounded-md border border-border"
        >
          {MODES.map((k) => (
            <button
              key={k}
              type="button"
              role="radio"
              aria-checked={mode === k}
              onClick={() => setMode(k)}
              className={cn(
                "cursor-pointer px-2.5 text-xs leading-[26px] outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset motion-reduce:transition-none",
                mode === k ? "seg-active" : "text-muted-foreground hover:text-foreground",
              )}
            >
              {modeLabels[k]}
            </button>
          ))}
        </div>
        <Button
          type="button"
          variant="outline"
          disabled={!base || !file.exists}
          onClick={() => setHistoryOpen(true)}
          title={m.history}
          className="h-7 shrink-0 cursor-pointer gap-1.5 bg-card px-3 text-xs hover:bg-muted @max-[42rem]:w-7 @max-[42rem]:px-0"
        >
          <History className="size-4" aria-hidden="true" />
          <span className="@max-[42rem]:sr-only">{m.history}</span>
        </Button>
        <Button
          type="button"
          disabled={!base || saving || !canSave}
          onClick={() => void save()}
          title={m.save}
          className="h-7 shrink-0 cursor-pointer gap-1.5 px-3 text-xs font-semibold @max-[42rem]:w-7 @max-[42rem]:px-0"
        >
          <Save className="size-4" aria-hidden="true" />
          <span className="@max-[42rem]:sr-only">{m.save}</span>
        </Button>
      </div>

      {removed || external || saveError || loadError || (!file.exists && base) ? (
        <div className="flex shrink-0 flex-col gap-2 px-4 py-3">
        {removed ? (
          <div className="flex items-center gap-2.5 rounded-md bg-warning/15 px-3 py-2 text-warning">
            <TriangleAlert className="size-4 shrink-0" aria-hidden="true" />
            <span className="text-foreground">{m.removedBanner}</span>
          </div>
        ) : null}
        {external ? (
          <div className="flex items-center gap-2.5 rounded-md bg-warning/15 px-3 py-2 text-warning">
            <TriangleAlert className="size-4 shrink-0" aria-hidden="true" />
            <span className="text-foreground">{m.externalChanged}</span>
            <span className="flex-1" />
            <Button
              type="button"
              variant="outline"
              onClick={reload}
              className="h-7 cursor-pointer bg-card px-3 text-xs text-foreground hover:bg-muted"
            >
              {dirty ? m.reloadDiscard : m.reload}
            </Button>
          </div>
        ) : null}
        {saveError ? (
          <div role="alert" className="flex items-center gap-2.5 rounded-md bg-destructive/15 px-3 py-2 text-destructive">
            <TriangleAlert className="size-4 shrink-0" aria-hidden="true" />
            <span className="text-foreground">{saveError}</span>
          </div>
        ) : null}
        {loadError ? (
          <div role="alert" className="flex items-center gap-2.5 rounded-md bg-destructive/15 px-3 py-2 text-destructive">
            <TriangleAlert className="size-4 shrink-0" aria-hidden="true" />
            <span className="text-foreground">{fmt(m.loadFailed, { message: loadError })}</span>
          </div>
        ) : null}
        {!file.exists && base ? (
          <div className="flex items-center gap-2.5 rounded-md bg-muted px-3 py-2 text-muted-foreground">
            <Info className="size-4 shrink-0" aria-hidden="true" />
            <span>{m.notCreatedHint}</span>
          </div>
        ) : null}
        </div>
      ) : null}

      <div className="flex min-h-0 flex-1">
        {base && mode !== "preview" ? (
          <div className={cn("min-h-0 min-w-0 flex-1 basis-0 overflow-hidden bg-card", mode === "split" && "border-r border-border")}>
            <CodeMirror
              value={text}
              onChange={setText}
              extensions={extensions}
              theme="none"
              onCreateEditor={setView}
              height="100%"
              style={{ height: "100%" }}
              basicSetup={{ foldGutter: false, highlightActiveLineGutter: true, searchKeymap: true }}
            />
          </div>
        ) : null}
        {base && mode !== "edit" ? (
          <div
            ref={previewRef}
            data-testid="preview-pane"
            className="min-h-0 min-w-0 flex-1 basis-0 overflow-x-hidden overflow-y-auto px-6 py-5"
          >
            <Preview text={text} />
          </div>
        ) : null}
      </div>

      {imports.length > 0 ? (
        <div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-t border-border px-4 py-2 text-xs text-muted-foreground">
          <Link2 className="size-4 shrink-0" aria-hidden="true" />
          <span>{fmt(m.importsCount, { n: imports.length })}</span>
          {imports.map((ref) => {
            const target = ref.exists ? findByResolvedPath(index, ref.resolved) : undefined;
            const key = `${ref.line}:${ref.raw}`;
            if (target) {
              return (
                <button
                  key={key}
                  type="button"
                  title={ref.resolved}
                  onClick={() => onOpenImport?.(target)}
                  className="cursor-pointer rounded-sm font-mono text-[12.5px] text-accent-text outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
                >
                  {ref.raw}
                </button>
              );
            }
            return ref.exists ? (
              <span key={key} title={ref.resolved} className="font-mono text-[12.5px]">
                {ref.raw}
              </span>
            ) : (
              <span key={key} title={m.importMissing} className="font-mono text-[12.5px] text-destructive line-through">
                {ref.raw}
              </span>
            );
          })}
        </div>
      ) : null}

      {base && file.exists ? (
        <HistoryDrawer
          open={historyOpen}
          onOpenChange={setHistoryOpen}
          path={file.path}
          currentText={text}
          currentHash={base.hash}
          dirty={dirty}
          onRestored={(_restored, dto) => apply(dto)}
        />
      ) : null}

      <ConfirmDialog
        open={conflict}
        title={m.externalChanged}
        body={m.conflictBody}
        confirmLabel={m.reloadDiscard}
        cancelLabel={m.cancel}
        destructive
        onConfirm={reload}
        onCancel={() => setConflict(false)}
      />
    </div>
  );
}

export function Preview({ text }: { text: string }) {
  return (
    // The pane never scrolls sideways: long tokens wrap anywhere, `pre` and tables scroll inside themselves.
    <div className="max-w-[72ch] min-w-0 font-content text-[14px] leading-[25px] wrap-anywhere">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeSourceLine]}
        components={{
          h1: (p) => (
            <h1
              className="mt-0 mb-3 border-b border-border pb-2 text-[20px] leading-7 font-semibold"
              {...strip(p)}
            />
          ),
          h2: (p) => <h2 className="mt-5 mb-2 text-[16px] leading-6 font-semibold" {...strip(p)} />,
          h3: (p) => <h3 className="mt-4 mb-1.5 text-base leading-6 font-semibold" {...strip(p)} />,
          h4: (p) => <h4 className="mt-4 mb-1.5 text-base leading-6 font-semibold" {...strip(p)} />,
          p: (p) => <p className="my-2.5" {...strip(p)} />,
          ul: (p) => <ul className="my-2.5 ml-5 list-disc" {...strip(p)} />,
          ol: (p) => <ol className="my-2.5 ml-5 list-decimal" {...strip(p)} />,
          li: (p) => <li className="my-1" {...strip(p)} />,
          input: (p) =>
            p.type === "checkbox" ? (
              <Checkbox checked={!!p.checked} disabled aria-readonly className="mr-1.5 inline-grid align-middle disabled:cursor-default disabled:opacity-100" />
            ) : (
              <input {...strip(p)} className="mr-1.5 align-middle" />
            ),
          blockquote: (p) => (
            <blockquote className="my-3 border-l-[3px] border-border pl-3 text-muted-foreground" {...strip(p)} />
          ),
          hr: (p) => <hr className="my-4 border-border" {...strip(p)} />,
          // Links must never navigate the app webview.
          a: ({ children }) => (
            <span className="cursor-pointer text-accent-text hover:underline">{children}</span>
          ),
          // Zero network egress: never load images (remote, relative or data:);
          // show a muted placeholder instead.
          img: ({ alt, src }) => (
            <span className="inline-flex max-w-full items-center gap-1 align-middle text-[12px] text-muted-foreground">
              <ImageOff size={14} aria-hidden />
              {alt || (typeof src === "string" ? src : "")}
            </span>
          ),
          pre: (p) => (
            <pre
              className="my-3 max-w-full overflow-x-auto rounded-lg bg-muted p-3 font-mono text-[12.5px] leading-5 wrap-normal [&_code]:bg-transparent [&_code]:p-0 [&_code]:break-normal"
              {...strip(p)}
            />
          ),
          code: ({ className, children }) => (
            <code className={cn("rounded-sm bg-muted px-[5px] py-px font-mono text-[12.5px] break-all", className)}>
              {children}
            </code>
          ),
          table: (p) => (
            <div data-testid="preview-table-scroll" className="my-3 max-w-full overflow-x-auto">
              <table className="border-collapse text-sm" {...strip(p)} />
            </div>
          ),
          th: (p) => <th className="border border-border px-2.5 py-1.5 text-left font-semibold" {...strip(p)} />,
          td: (p) => <td className="border border-border px-2.5 py-1.5" {...strip(p)} />,
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}

/** react-markdown passes the hast `node`; it must not reach the DOM. */
function strip<T extends { node?: unknown }>(props: T): Omit<T, "node"> {
  const { node: _node, ...rest } = props;
  void _node;
  return rest;
}
