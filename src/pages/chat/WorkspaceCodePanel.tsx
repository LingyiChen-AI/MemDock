import * as React from "react";
import { ContextMenu } from "radix-ui";
import CodeMirror, { type ReactCodeMirrorRef } from "@uiw/react-codemirror";
import { redo, selectAll, undo } from "@codemirror/commands";
import { openSearchPanel } from "@codemirror/search";
import { Decoration, EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";
import { javascript } from "@codemirror/lang-javascript";
import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import { yaml } from "@codemirror/lang-yaml";
import { diffLines } from "diff";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ChevronDown, ChevronRight, ExternalLink, File, FilePlus, Files, Folder, FolderOpen, RefreshCw, Save, Search, TriangleAlert, X } from "lucide-react";
import { codeEntries, codeStatus, createCodeFile, openCodeInVSCode, readCode, saveCode, searchCode, trashCodeFile, type CodeEntry, type CodeFile, type CodeMatch } from "@/api/workspaceCode";
import { worktreeStatus, type WorktreeInfo } from "@/api/worktree";
import { useI18n } from "@/i18n/I18nProvider";
import { cn } from "@/lib/utils";
import { WorkspaceFileIcon } from "./WorkspaceFileIcon";
import { type CodeLocation } from "./workspaceCodeLinks";

type TextSource = Extract<CodeFile, { kind: "text" }>;
type Draft = { source: TextSource; text: string };
type Tab = { path: string; file?: CodeFile; text?: string; error?: string };
type Labels = ReturnType<typeof useI18n>["t"]["workspaceCode"];
const workspaceDrafts = new Map<string, Draft>();
const widthKey = "memdock.workspaceCode.widthPercent";
const defaultWidth = 60;
const minWidth = 25;
const maxWidth = 80;
const parentOf = (path: string) => path.slice(0, Math.max(0, path.lastIndexOf("/")));
const nameOf = (path: string) => path.split("/").pop() ?? path;
const errorText = (e: unknown) => typeof e === "object" && e !== null && "message" in e ? String(e.message) : String(e);
const isAdded = (mark: string) => /^[AU]+$/.test(mark);
const isUntitled = (path: string) => path.startsWith("\0");
const tabName = (path: string, l: Labels) => isUntitled(path) ? l.untitled + "-" + path.slice(1) : nameOf(path);
const isDirty = (tab: Tab) => tab.file?.kind === "text" && tab.file.text !== tab.text;

type ExplorerActions = {
  newFile?: (directory: string) => void;
  focusNewFile: () => void;
  open: (path: string) => void;
  vscode?: (path: string) => void;
  copyPath: (path: string, relative: boolean) => void;
  remove?: (path: string) => void;
};

export interface CodePanelSource {
  entries: (path: string) => Promise<CodeEntry[]>;
  read: (path: string) => Promise<CodeFile>;
  save?: (path: string, revision: string, text: string) => Promise<string>;
  search: (query: string) => Promise<CodeMatch[]>;
  invalidate: () => void;
  watch: (refresh: () => void) => Promise<() => void>;
}

function WorkspaceMenu({ trigger, children, onCloseAutoFocus }: {
  trigger: React.ReactElement; children: React.ReactNode; onCloseAutoFocus?: (event: Event) => void;
}) {
  return (
    <ContextMenu.Root>
      <ContextMenu.Trigger asChild>{trigger}</ContextMenu.Trigger>
      <ContextMenu.Portal>
        <ContextMenu.Content
          className="z-50 min-w-44 rounded-md border border-zinc-700 bg-zinc-900 p-1 text-xs text-zinc-100 shadow-xl"
          onCloseAutoFocus={onCloseAutoFocus}
        >
          {children}
        </ContextMenu.Content>
      </ContextMenu.Portal>
    </ContextMenu.Root>
  );
}

function MenuItem({ children, onSelect, disabled, destructive }: {
  children: React.ReactNode; onSelect: () => void; disabled?: boolean; destructive?: boolean;
}) {
  return (
    <ContextMenu.Item disabled={disabled} onSelect={onSelect} className={cn(
      "flex cursor-default items-center rounded px-2.5 py-1.5 outline-none data-highlighted:bg-sky-900 data-highlighted:text-white data-disabled:opacity-40",
      destructive && "text-red-300",
    )}>{children}</ContextMenu.Item>
  );
}

function MenuSeparator() {
  return <ContextMenu.Separator className="my-1 h-px bg-zinc-700" />;
}

function PathMenuItems({ path, actions, l }: { path: string; actions: ExplorerActions; l: Labels }) {
  return (
    <>
      {actions.vscode && <><MenuItem onSelect={() => actions.vscode?.(path)}>{l.vscode}</MenuItem><MenuSeparator /></>}
      <MenuItem onSelect={() => actions.copyPath(path, false)}>{l.copyPath}</MenuItem>
      <MenuItem onSelect={() => actions.copyPath(path, true)}>{l.copyRelativePath}</MenuItem>
    </>
  );
}

function ExplorerMenu({ path, directory, actions, l, children }: {
  path: string; directory: boolean; actions: ExplorerActions; l: Labels; children: React.ReactElement;
}) {
  const creating = React.useRef(false);
  return (
    <WorkspaceMenu trigger={children} onCloseAutoFocus={(event) => {
      if (creating.current) {
        event.preventDefault();
        actions.focusNewFile();
      }
      creating.current = false;
    }}>
      {(!directory || actions.newFile) && <MenuItem onSelect={() => {
        if (directory) {
          creating.current = true;
          actions.newFile?.(path);
        } else {
          actions.open(path);
        }
      }}>{directory ? l.newFile : l.open}</MenuItem>}
      <PathMenuItems path={path} actions={actions} l={l} />
      {!directory && actions.remove && (
        <>
          <MenuSeparator />
          <MenuItem destructive onSelect={() => actions.remove?.(path)}>{l.deleteFile}</MenuItem>
        </>
      )}
    </WorkspaceMenu>
  );
}

type TreeProps = {
  entriesFor: (path: string) => Promise<CodeEntry[]>; path: string; refresh: number; status: Record<string, string>; active: string | null;
  directory: string; expanded: ReadonlySet<string>; folderMarks: ReadonlyMap<string, boolean>;
  toggleDirectory: (path: string) => void; actions: ExplorerActions; l: Labels;
  newFileDirectory: string | undefined; inlineFile: React.ReactNode;
};

function Tree(props: TreeProps) {
  const { entriesFor, path, refresh, status, active, directory, expanded, folderMarks, toggleDirectory, actions, newFileDirectory, inlineFile, l } = props;
  const [entries, setEntries] = React.useState<CodeEntry[]>([]);
  const [error, setError] = React.useState("");
  React.useEffect(() => {
    let current = true;
    setError("");
    void entriesFor(path).then((value) => {
      if (current) setEntries(value);
    }, (e) => {
      if (current) setError(errorText(e));
    });
    return () => { current = false; };
  }, [entriesFor, path, refresh]);

  if (error) return <p role="alert" className="break-words px-2 py-1 text-red-300">{error}</p>;
  return (
    <ul className={path ? "ml-2 border-l border-zinc-700/70 pl-1" : ""} aria-label={path || l.files}>
      {newFileDirectory === path && <li>{inlineFile}</li>}
      {entries.map((entry) => {
        const expandedDir = expanded.has(entry.path);
        const Arrow = expandedDir ? ChevronDown : ChevronRight;
        const mark = Object.prototype.hasOwnProperty.call(status, entry.path) ? status[entry.path] : undefined;
        const folderMark = entry.directory ? folderMarks.get(entry.path) : undefined;
        const modified = mark ? !isAdded(mark) : folderMark;
        const gitColor = modified === undefined ? undefined : modified ? "text-amber-300" : "text-emerald-300";
        return (
          <li key={entry.path}>
            <ExplorerMenu path={entry.path} directory={entry.directory} actions={actions} l={l}>
              <button
                type="button"
                aria-expanded={entry.directory ? expandedDir : undefined}
                aria-current={active === entry.path ? "true" : undefined}
                title={entry.path}
                className={cn(
                  "flex h-6 w-full items-center gap-1 px-1.5 text-left hover:bg-zinc-700/70",
                  active === entry.path && "bg-sky-950 text-sky-100",
                  entry.directory && directory === entry.path && "bg-zinc-700/70",
                )}
                onClick={() => entry.directory ? toggleDirectory(entry.path) : actions.open(entry.path)}
              >
                {entry.directory ? <><Arrow className="size-3 shrink-0" />
                  {expandedDir ? <FolderOpen className="size-3.5 shrink-0 text-amber-300/90" /> : <Folder className="size-3.5 shrink-0 text-amber-300/90" />}</>
                  : <WorkspaceFileIcon name={entry.name} className="ml-3" />}
                <span className={cn("min-w-0 flex-1 truncate", gitColor)}>{entry.name}</span>
                {folderMark !== undefined && (
                  <span
                    aria-hidden="true"
                    title={l.folderChanges}
                    className={cn("size-1.5 shrink-0 rounded-full", folderMark ? "bg-amber-300" : "bg-emerald-300")}
                  />
                )}
                {mark && <span className={gitColor}>{mark}</span>}
              </button>
            </ExplorerMenu>
            {expandedDir && <Tree {...props} path={entry.path} />}
          </li>
        );
      })}
    </ul>
  );
}

function syntax(path: string) {
  switch (path.split(".").pop()?.toLowerCase()) {
    case "js": case "mjs": case "cjs": case "jsx": return javascript({ jsx: true });
    case "ts": case "mts": case "cts": return javascript({ typescript: true });
    case "tsx": return javascript({ typescript: true, jsx: true });
    case "css": return css();
    case "html": case "htm": return html();
    case "json": return json();
    case "md": case "markdown": return markdown();
    case "py": return python();
    case "rs": return rust();
    case "yaml": case "yml": return yaml();
    default: return [];
  }
}

const changeLineTheme = EditorView.baseTheme({
  ".cm-line.workspace-line-added": {
    backgroundColor: "rgb(34 197 94 / 12%)",
    boxShadow: "inset 3px 0 rgb(74 222 128)",
  },
  ".cm-line.workspace-line-modified": {
    backgroundColor: "rgb(59 130 246 / 14%)",
    boxShadow: "inset 3px 0 rgb(96 165 250)",
  },
  ".cm-line.workspace-line-deleted-before": {
    borderTop: "2px solid rgb(248 113 113)",
  },
  ".cm-line.workspace-line-deleted-after": {
    borderBottom: "2px solid rgb(248 113 113)",
  },
});

function changedLines(baseline: string) {
  function decorate(view: EditorView) {
    const { doc } = view.state;
    const changes = diffLines(baseline, doc.toString(), { timeout: 50 });
    if (!changes) return Decoration.none;
    const ranges = [];
    let line = 1;
    for (let i = 0; i < changes.length; i++) {
      const part = changes[i];
      const count = part.count ?? 0;
      if (part.removed) {
        const replaced = changes[i + 1]?.added ? changes[i + 1].count ?? 0 : 0;
        if (count > replaced) {
          const after = line + replaced > doc.lines;
          const anchor = doc.line(Math.min(line + replaced, doc.lines));
          const edge = after ? "after" : "before";
          ranges.push(Decoration.line({ class: `workspace-line-deleted-${edge}` }).range(anchor.from));
        }
        continue;
      }
      if (part.added) {
        const removed = changes[i - 1]?.removed ? changes[i - 1].count ?? 0 : 0;
        for (let offset = 0; offset < count && line + offset <= doc.lines; offset++) {
          const kind = offset < removed ? "modified" : "added";
          ranges.push(Decoration.line({ class: `workspace-line-${kind}` }).range(doc.line(line + offset).from));
        }
      }
      line += count;
    }
    return Decoration.set(ranges, true);
  }

  return ViewPlugin.fromClass(class {
    decorations;
    constructor(view: EditorView) {
      this.decorations = decorate(view);
    }
    update(update: ViewUpdate) {
      if (update.docChanged) this.decorations = decorate(update.view);
    }
  }, { decorations: (plugin) => plugin.decorations });
}

function CodeEditor({ path, value, baseline, readOnly, reveal, onChange, onError, onSave, l }: {
  path: string; value: string; baseline: string | null; readOnly: boolean; reveal: CodeLocation | null; onChange: (text: string) => void;
  onError: (message: string) => void; onSave: () => void; l: Labels;
}) {
  const extensions = React.useMemo(
    () => baseline === null ? [syntax(path)] : [syntax(path), changeLineTheme, changedLines(baseline)],
    [path, baseline],
  );
  const editor = React.useRef<ReactCodeMirrorRef>(null);
  const jump = React.useCallback((view: EditorView) => {
    if (!reveal) return;
    const text = view.state.doc.toString();
    const anchor = reveal.anchor ? text.indexOf(reveal.anchor.replace(/\r\n/g, "\n")) : -1;
    if (anchor < 0 && !reveal.line) return;
    const pos = anchor >= 0 ? anchor : view.state.doc.line(Math.min(reveal.line ?? 1, view.state.doc.lines)).from;
    view.dispatch({ selection: { anchor: pos }, effects: EditorView.scrollIntoView(pos, { y: "center" }) });
  }, [reveal]);
  React.useEffect(() => {
    if (editor.current?.view) jump(editor.current.view);
  }, [jump]);

  const run = (command: (view: EditorView) => unknown) => {
    if (editor.current?.view) command(editor.current.view);
  };
  const clipboard = async (action: "cut" | "copy" | "paste") => {
    const view = editor.current?.view;
    if (!view || (action !== "copy" && view.state.readOnly)) return;
    const state = view.state;
    const canApply = () => editor.current?.view === view && !view.state.readOnly &&
      view.state.doc === state.doc && view.state.selection.eq(state.selection);
    try {
      if (action === "paste") {
        const text = await navigator.clipboard.readText();
        if (text && canApply()) view.dispatch(state.replaceSelection(text), { userEvent: "input.paste" });
        return;
      }
      const selected = state.selection.ranges.filter((range) => !range.empty);
      const line = state.doc.lineAt(state.selection.main.head);
      const ranges = selected.length ? selected : [{ from: line.from, to: Math.min(line.to + 1, state.doc.length) }];
      await navigator.clipboard.writeText(ranges.map(({ from, to }) => state.sliceDoc(from, to)).join("\n"));
      if (action === "cut" && canApply()) view.dispatch({ changes: ranges.map(({ from, to }) => ({ from, to })), userEvent: "delete.cut" });
    } catch (e) {
      onError(errorText(e));
    }
  };

  return (
    <WorkspaceMenu trigger={
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
        <CodeMirror
          ref={editor}
          autoFocus={isUntitled(path)}
          value={value}
          onChange={onChange}
          onCreateEditor={jump}
          extensions={extensions}
          readOnly={readOnly}
          theme="dark"
          height="100%"
          className="min-h-0 flex-1 overflow-hidden text-[13px]"
          basicSetup={{ lineNumbers: true, foldGutter: true, highlightActiveLine: true, searchKeymap: true }}
        />
      </div>
    } onCloseAutoFocus={(event) => {
      event.preventDefault();
      run((view) => {
        if (!view.dom.contains(document.activeElement)) view.focus();
      });
    }}>
      <MenuItem disabled={readOnly} onSelect={() => run(undo)}>{l.undo}</MenuItem>
      <MenuItem disabled={readOnly} onSelect={() => run(redo)}>{l.redo}</MenuItem>
      <MenuSeparator />
      <MenuItem disabled={readOnly} onSelect={() => void clipboard("cut")}>{l.cut}</MenuItem>
      <MenuItem onSelect={() => void clipboard("copy")}>{l.copy}</MenuItem>
      <MenuItem disabled={readOnly} onSelect={() => void clipboard("paste")}>{l.paste}</MenuItem>
      <MenuItem onSelect={() => run(selectAll)}>{l.selectAll}</MenuItem>
      <MenuSeparator />
      <MenuItem onSelect={() => run(openSearchPanel)}>{l.findInFile}</MenuItem>
      <MenuItem disabled={readOnly} onSelect={onSave}>{l.save}</MenuItem>
    </WorkspaceMenu>
  );
}

function CodeDiff({ base, text, empty }: { base: string; text: string; empty: string }) {
  const changes = React.useMemo(() => {
    if (base.split("\n").length + text.split("\n").length > 12_000) return undefined;
    return diffLines(base, text, { timeout: 250 });
  }, [base, text]);
  if (!changes) return <p className="p-4">{empty}</p>;
  let oldLine = 1;
  let newLine = 1;
  return (
    <div className="min-h-0 flex-1 overflow-auto font-mono text-xs leading-5">
      <div className="min-w-max">
        {changes.map((part, index) => (
          <React.Fragment key={index}>
            {part.value.replace(/\n$/, "").split("\n").map((line, row) => (
              <div key={row} className={cn("flex whitespace-pre", part.added && "bg-emerald-950 text-emerald-200", part.removed && "bg-red-950 text-red-200")}>
                <span className="inline-block w-10 shrink-0 select-none pr-2 text-right text-zinc-400">{part.added ? "" : oldLine++}</span>
                <span className="inline-block w-10 shrink-0 select-none pr-2 text-right text-zinc-400">{part.removed ? "" : newLine++}</span>
                <span className="w-4 shrink-0 select-none">{part.added ? "+" : part.removed ? "-" : " "}</span>{line}
              </div>
            ))}
          </React.Fragment>
        ))}
      </div>
    </div>
  );
}

function PanelPrompt({ title, children, onCancel, warning }: {
  title: string; warning?: boolean; children: React.ReactNode; onCancel: () => void;
}) {
  const id = React.useId();
  const content = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    const previous = document.activeElement;
    const target = content.current?.querySelector<HTMLElement>("[data-initial-focus]");
    target?.focus();
    return () => {
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
    };
  }, []);
  return (
    <div className="absolute inset-0 z-30 grid place-items-center bg-black/25 p-3">
      <div
        ref={content}
        role="dialog"
        aria-labelledby={id}
        className="w-full max-w-[350px] rounded-md border border-zinc-600/80 bg-zinc-800 p-4 text-sm text-zinc-100 shadow-xl"
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            onCancel();
          } else if (event.key === "Tab") {
            const focusable = event.currentTarget.querySelectorAll<HTMLElement>("button:not(:disabled), input:not(:disabled)");
            const first = focusable[0];
            const last = focusable[focusable.length - 1];
            if (first && ((event.shiftKey && document.activeElement === first) || (!event.shiftKey && document.activeElement === last))) {
              event.preventDefault();
              (event.shiftKey ? last : first).focus();
            }
          }
        }}
      >
        <div className="mb-2.5 flex items-center gap-2">
          {warning && <TriangleAlert aria-hidden="true" className="size-4 shrink-0 text-amber-300" />}
          <h3 id={id} className="min-w-0 font-medium">{title}</h3>
        </div>
        {children}
      </div>
    </div>
  );
}

function PromptButton({ tone = "primary", className, ...props }: React.ComponentProps<"button"> & {
  tone?: "primary" | "secondary" | "danger";
}) {
  return (
    <button {...props} className={cn(
      "inline-flex h-7 min-w-14 cursor-pointer items-center justify-center rounded border px-2.5 text-xs font-medium whitespace-nowrap outline-none focus-visible:ring-1 focus-visible:ring-sky-300 focus-visible:ring-offset-2 focus-visible:ring-offset-zinc-800 disabled:cursor-not-allowed disabled:opacity-50",
      tone === "primary" && "border-sky-700 bg-sky-700 text-white hover:bg-sky-600",
      tone === "secondary" && "border-zinc-600 bg-zinc-700 text-zinc-100 hover:bg-zinc-600",
      tone === "danger" && "border-red-800 bg-red-950 text-red-100 hover:bg-red-900",
      className,
    )} />
  );
}

export function WorkspaceCodePanel({ cwd, base, shown, request, source, onDirtyChange }: {
  cwd: string; base: WorktreeInfo | null; shown: boolean; request: CodeLocation | null;
  source?: CodePanelSource; onDirtyChange?: (dirty: boolean) => void;
}) {
  const { t: { workspaceCode: l, skills } } = useI18n();
  const [localDrafts] = React.useState(() => new Map<string, Draft>());
  const drafts = source ? localDrafts : workspaceDrafts;
  const readOnly = !!source && !source.save;
  const entriesFor = React.useCallback((path: string) => source ? source.entries(path) : codeEntries(cwd, path), [source, cwd]);
  const readFile = React.useCallback((path: string) => source ? source.read(path) : readCode(cwd, path, base), [source, cwd, base]);
  const [width, setWidth] = React.useState(() => {
    const saved = Number(localStorage.getItem(widthKey));
    return saved >= minWidth && saved <= maxWidth ? saved : defaultWidth;
  });
  const [directory, setDirectory] = React.useState("");
  const [expanded, setExpanded] = React.useState<ReadonlySet<string>>(() => new Set());
  const [newFile, setNewFile] = React.useState<{ directory: string; name: string } | null>(null);
  const [creating, setCreating] = React.useState(false);
  const newFileInput = React.useRef<HTMLInputElement>(null);
  const newFileDirectory = newFile?.directory;
  React.useEffect(() => {
    if (newFileDirectory === undefined) return;
    const frame = window.requestAnimationFrame(() => newFileInput.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [newFileDirectory]);
  const [tabs, setTabs] = React.useState<Tab[]>(() => {
    const prefix = cwd + "\0";
    return Array.from(drafts)
      .filter(([key]) => key.startsWith(prefix))
      .map(([key, draft]) => ({ path: key.slice(prefix.length), file: draft.source, text: draft.text }));
  });
  const [active, setActive] = React.useState<string | null>(() => tabs[0]?.path ?? null);
  const [reveal, setReveal] = React.useState<CodeLocation | null>(null);
  const [status, setStatus] = React.useState<Record<string, string>>({});
  const statusRequest = React.useRef(0);
  const folderMarks = React.useMemo(() => {
    const folders = new Map<string, boolean>();
    for (const [path, mark] of Object.entries(status)) {
      const modified = !isAdded(mark);
      for (let folder = parentOf(path); folder; folder = parentOf(folder)) {
        folders.set(folder, modified || folders.get(folder) === true);
      }
    }
    return folders;
  }, [status]);
  const [refresh, setRefresh] = React.useState(0);
  const [saving, setSaving] = React.useState<string | null>(null);
  const [pendingClose, setPendingClose] = React.useState<string[]>([]);
  const [saveAs, setSaveAs] = React.useState<{ tab: string; directory: string; name: string; error?: string; closeAfter?: boolean } | null>(null);
  const [reloadTarget, setReloadTarget] = React.useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = React.useState<string | null>(null);
  const [deleting, setDeleting] = React.useState<string | null>(null);
  const saveLock = React.useRef(false);
  const loads = React.useRef(new Map<string, object>(tabs.map(({ path }) => [path, {}])));
  const [mode, setMode] = React.useState<"code" | "diff" | "preview">("code");
  const [error, setError] = React.useState("");
  const [finder, setFinder] = React.useState<"files" | "content" | null>(null);
  const [query, setQuery] = React.useState("");
  const [matches, setMatches] = React.useState<CodeMatch[]>([]);
  const [searching, setSearching] = React.useState(false);
  const searchInput = React.useRef<HTMLInputElement>(null);
  const panel = React.useRef<HTMLElement>(null);
  const tab = tabs.find((item) => item.path === active);
  const file = tab?.file;
  const closing = tabs.find((item) => item.path === pendingClose[0]);
  const dirty = tab ? isDirty(tab) : false;
  const hasDirtyTabs = tabs.some(isDirty);
  React.useEffect(() => { onDirtyChange?.(hasDirtyTabs); }, [onDirtyChange, hasDirtyTabs]);
  React.useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);
  const update = (next: Tab) => setTabs((value) => value.map((item) => item.path === next.path ? next : item));
  const refreshStatus = React.useCallback(() => {
    if (source) return;
    const request = ++statusRequest.current;
    const marks = base
      ? worktreeStatus(base.path).then(({ files }) => Object.fromEntries(files.map(({ path, status }) => [path, status])))
      : codeStatus(cwd);
    void marks.then((next) => {
      if (request !== statusRequest.current) return;
      setStatus((current) => Object.keys(current).length === Object.keys(next).length &&
        Object.entries(next).every(([path, mark]) => current[path] === mark) ? current : next);
    }, () => { /* Keep the last successful Git status. */ });
  }, [base, cwd, source]);

  React.useEffect(() => {
    if (!shown || source) return;
    refreshStatus();
    const timer = window.setInterval(() => {
      if (!document.hidden && document.hasFocus()) refreshStatus();
    }, 3000);
    return () => window.clearInterval(timer);
  }, [shown, refreshStatus, source]);

  const refreshExplorer = React.useCallback(() => {
    refreshStatus();
    source?.invalidate();
    setRefresh((value) => value + 1);
    if (!tab?.file || isUntitled(tab.path) || saveLock.current) return;
    const dirty = isDirty(tab);
    if (dirty && !source) return;
    const token = loads.current.get(tab.path);
    void readFile(tab.path).then((file) => {
      if (loads.current.get(tab.path) !== token || saveLock.current) return;
      if (dirty) {
        if (file.kind === "text" && tab.file?.kind === "text" && file.revision !== tab.file.revision &&
          drafts.get(cwd + "\0" + tab.path)?.source.revision === tab.file.revision) setError(skills.externalChanged);
        return;
      }
      setTabs((current) => current.map((item) => {
        if (item !== tab) return item;
        if (file.kind === "text" && item.file?.kind === "text" &&
          file.revision === item.file.revision && file.baseline === item.file.baseline) return item;
        if (file.kind === "image" && item.file?.kind === "image" && file.data === item.file.data) return item;
        return { path: item.path, file, text: file.kind === "text" ? file.text : undefined };
      }));
    }, () => { /* The reload action can show any file access error. */ });
  }, [cwd, drafts, readFile, refreshStatus, skills.externalChanged, source, tab]);

  const latestRefresh = React.useRef(refreshExplorer);
  latestRefresh.current = refreshExplorer;
  React.useEffect(() => {
    if (!source) return;
    let active = true;
    let unlisten: (() => void) | undefined;
    void source.watch(() => { if (active) latestRefresh.current(); }).then((stop) => {
      if (active) unlisten = stop;
      else stop();
    }, () => {});
    return () => { active = false; unlisten?.(); };
  }, [source]);

  React.useEffect(() => {
    if (!shown) return;
    window.addEventListener("focus", refreshExplorer);
    return () => window.removeEventListener("focus", refreshExplorer);
  }, [shown, refreshExplorer]);

  const load = React.useCallback((path: string, reload = false) => {
    if (loads.current.has(path) && !reload) return;
    const token = {};
    loads.current.set(path, token);
    const key = cwd + "\0" + path;
    const draft = drafts.get(key);
    if (draft && !reload) {
      setTabs((value) => value.some((item) => item.path === path)
        ? value.map((item) => item.path === path ? { path, file: draft.source, text: draft.text } : item)
        : [...value, { path, file: draft.source, text: draft.text }]);
      return;
    }
    setTabs((value) => value.some((item) => item.path === path)
      ? value.map((item) => item.path === path ? { path } : item)
      : [...value, { path }]);
    void readFile(path).then((file) => {
      if (loads.current.get(path) !== token) return;
      if (reload && drafts.get(key) === draft) drafts.delete(key);
      update({ path, file, text: file.kind === "text" ? file.text : undefined });
    }, (e) => {
      if (loads.current.get(path) !== token) return;
      update({ path, file: draft?.source, text: draft?.text, error: errorText(e) });
    });
  }, [cwd, drafts, readFile]);

  const openState = React.useRef({ active, tabs, saving });
  openState.current = { active, tabs, saving };
  const open = React.useCallback((location: CodeLocation) => {
    setError("");
    setActive(location.path);
    setReveal(location);
    setMode("code");
    setFinder((current) => current === "files" ? null : current);
    if (!isUntitled(location.path)) {
      const state = openState.current;
      const cached = state.tabs.find((item) => item.path === location.path);
      const refreshCleanTab = !!source && state.active !== location.path && state.saving !== location.path &&
        !!cached && !!(cached.file || cached.error) && !isDirty(cached);
      load(location.path, refreshCleanTab);
    }
  }, [load, source]);

  const openUntitled = () => {
    let number = 1;
    while (drafts.has(cwd + "\0\0" + number)) number++;
    const path = "\0" + number;
    const file: TextSource = { kind: "text", text: "", baseline: null, revision: "" };
    drafts.set(cwd + "\0" + path, { source: file, text: "" });
    setTabs((current) => [...current, { path, file, text: "" }]);
    open({ path });
  };

  React.useEffect(() => {
    if (request) open(request);
  }, [request, open]);

  const saveTab = React.useCallback(async (target: Tab, closeAfter = false) => {
    if (readOnly || target.file?.kind !== "text" || target.text === undefined || deleting === target.path || saveLock.current) return;
    if (isUntitled(target.path)) {
      setSaveAs((current) => current ?? { tab: target.path, directory, name: "", closeAfter });
      return;
    }
    if (!isDirty(target)) return;
    const { path, text, file } = target;
    const draftKey = cwd + "\0" + path;
    const draft = drafts.get(draftKey);
    saveLock.current = true;
    setSaving(path);
    setError("");
    try {
      const revision = source?.save ? await source.save(path, file.revision, text) : await saveCode(cwd, path, file.revision, text);
      if (drafts.get(draftKey) === draft) drafts.delete(draftKey);
      if (closeAfter) {
        loads.current.delete(path);
        setTabs((current) => current.filter((item) => item.path !== path));
        setActive((current) => current === path ? null : current);
        setPendingClose((current) => current.slice(1));
      } else {
        update({ path, text, file: { ...file, text, revision } });
      }
      refreshStatus();
    } catch (e) {
      setError(errorText(e));
      if (!closeAfter && typeof e === "object" && e !== null && "code" in e && e.code === "disk_changed") setReloadTarget(path);
    } finally {
      saveLock.current = false;
      setSaving(null);
    }
  }, [cwd, deleting, directory, drafts, readOnly, refreshStatus, source]);

  const save = React.useCallback(() => {
    if (tab && !pendingClose.length) return saveTab(tab);
  }, [tab, pendingClose.length, saveTab]);

  React.useEffect(() => {
    if (!shown || saveAs || closing || deleteTarget !== null || reloadTarget !== null) return;
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.altKey) return;
      const key = event.key.toLowerCase();
      if (key === "s" && !event.shiftKey) {
        event.preventDefault();
        void save();
      } else if ((key === "p" && !event.shiftKey) || (!source && key === "f" && event.shiftKey)) {
        event.preventDefault();
        setFinder(key === "p" ? "files" : "content");
        searchInput.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [closing, deleteTarget, reloadTarget, save, saveAs, shown, source]);

  React.useEffect(() => {
    setMatches([]);
    if (!shown || !finder || !query.trim()) {
      setSearching(false);
      return;
    }
    let current = true;
    setSearching(true);
    const timer = window.setTimeout(() => {
      const pending = source ? source.search(query) : searchCode(cwd, query, finder === "content");
      void pending.then((value) => {
        if (current) setMatches(value);
      }, (e) => {
        if (current) setError(errorText(e));
      }).finally(() => {
        if (current) setSearching(false);
      });
    }, 250);
    return () => { current = false; window.clearTimeout(timer); };
  }, [cwd, finder, query, shown, source]);

  const removeTab = (path: string) => {
    loads.current.delete(path);
    setTabs((current) => current.filter((item) => item.path !== path));
    setActive((current) => current === path ? null : current);
  };

  const discardTabs = (items: Tab[]) => {
    const closable = items.filter((item) => item.path !== saving && item.path !== deleting);
    if (!closable.length) return;
    const paths = new Set(closable.map((item) => item.path));
    for (const item of closable) {
      const key = cwd + "\0" + item.path;
      const draft = drafts.get(key);
      if (item.file?.kind === "text" && draft?.text === item.text && draft?.source.revision === item.file.revision) drafts.delete(key);
      loads.current.delete(item.path);
    }
    setTabs((current) => current.filter((item) => !paths.has(item.path)));
    if (active && paths.has(active)) {
      setActive(tabs.filter((item) => !paths.has(item.path)).pop()?.path ?? null);
      setReveal(null);
      setMode("code");
      setError("");
    }
  };

  const reload = (path: string) => {
    setReloadTarget(null);
    setError("");
    load(path, true);
    refreshStatus();
  };

  const closeTabs = (items: Tab[]) => {
    const closable = items.filter((item) => item.path !== saving && item.path !== deleting);
    const dirtyTabs = closable.filter(isDirty);
    discardTabs(closable.filter((item) => !isDirty(item)));
    if (dirtyTabs.length) {
      setError("");
      setPendingClose(dirtyTabs.map((item) => item.path));
    }
  };

  const remove = async (path: string) => {
    setDeleteTarget(null);
    if (deleting || saving === path) return;
    setDeleting(path);
    setError("");
    try {
      await trashCodeFile(cwd, path);
      drafts.delete(cwd + "\0" + path);
      removeTab(path);
      setRefresh((value) => value + 1);
      refreshStatus();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setDeleting(null);
    }
  };

  const toggleDirectory = (path: string) => {
    setDirectory(path);
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const startNewFile = (path: string) => {
    if (creating) return;
    setError("");
    setFinder(null);
    setDirectory(path);
    setExpanded((current) => {
      const next = new Set(current);
      for (let folder = path; folder; folder = parentOf(folder)) next.add(folder);
      return next;
    });
    setNewFile((current) => current?.directory === path ? current : { directory: path, name: "" });
  };

  const explorerActions: ExplorerActions = {
    newFile: source ? undefined : startNewFile,
    focusNewFile: () => newFileInput.current?.focus(),
    remove: source ? undefined : (path) => {
      if (deleting || saving === path) return;
      if (drafts.has(cwd + "\0" + path) || tabs.some((item) => item.path === path && isDirty(item))) {
        setDeleteTarget(path);
      } else {
        void remove(path);
      }
    },
    open: (path) => open({ path }),
    vscode: source ? undefined : (path) => void openCodeInVSCode(cwd, path).catch((e) => setError(errorText(e))),
    copyPath: async (path, relative) => {
      const separator = cwd.includes("\\") ? "\\" : "/";
      const absolute = path ? cwd.replace(/[\\/]+$/, "") + separator + path.split("/").join(separator) : cwd;
      try {
        await navigator.clipboard.writeText(relative ? path || "." : absolute);
      } catch (e) {
        setError(errorText(e));
      }
    },
  };

  const create = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!newFile || !newFile.name.trim() || creating) return;
    setCreating(true);
    setError("");
    try {
      const path = await createCodeFile(cwd, newFile.directory, newFile.name.trim());
      setNewFile(null);
      setRefresh((value) => value + 1);
      refreshStatus();
      open({ path });
    } catch (e) {
      setError(errorText(e));
    } finally {
      setCreating(false);
    }
  };

  const cancelSaveAs = () => {
    if (saveLock.current) return;
    if (saveAs?.closeAfter) setPendingClose([]);
    setSaveAs(null);
  };

  const submitSaveAs = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!saveAs?.name.trim() || saveLock.current) return;
    const draft = tabs.find((item) => item.path === saveAs.tab);
    if (!draft || draft.text === undefined) return;
    saveLock.current = true;
    setSaving(draft.path);
    try {
      const path = await createCodeFile(cwd, saveAs.directory, saveAs.name.trim(), draft.text);
      drafts.delete(cwd + "\0" + draft.path);
      removeTab(draft.path);
      setSaveAs(null);
      setRefresh((value) => value + 1);
      refreshStatus();
      if (saveAs.closeAfter) {
        setPendingClose((current) => current.slice(1));
      } else {
        open({ path });
      }
    } catch (e) {
      setSaveAs({ ...saveAs, error: errorText(e) });
    } finally {
      saveLock.current = false;
      setSaving(null);
    }
  };

  const resize = (percent: number) => {
    const next = Math.round(Math.min(maxWidth, Math.max(minWidth, percent)));
    setWidth(next);
    localStorage.setItem(widthKey, String(next));
  };

  return (
    <aside
      ref={panel}
      onContextMenu={(event) => event.preventDefault()}
      hidden={!shown}
      aria-label={l.title}
      className={cn("relative min-h-0 min-w-0 shrink-0 flex-col border-l border-zinc-700 bg-zinc-900 text-zinc-100", source && "w-full", shown ? "flex" : "hidden")}
      style={source ? undefined : { width: `${width}%`, minWidth: "min(270px, 80%)" }}
    >
      {!source && <div
        role="separator"
        aria-label={l.resize}
        aria-orientation="vertical"
        aria-valuemin={minWidth}
        aria-valuemax={maxWidth}
        aria-valuenow={width}
        tabIndex={0}
        className="absolute inset-y-0 -left-1 z-10 w-2 cursor-col-resize touch-none hover:bg-sky-500/40 focus-visible:bg-sky-500/40"
        onPointerDown={(event) => event.currentTarget.setPointerCapture(event.pointerId)}
        onPointerMove={(event) => {
          if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
          const bounds = panel.current?.parentElement?.getBoundingClientRect();
          if (bounds?.width) resize((bounds.right - event.clientX) / bounds.width * 100);
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
            event.preventDefault();
            const parentWidth = panel.current?.parentElement?.clientWidth;
            const visibleWidth = parentWidth && panel.current ? panel.current.offsetWidth / parentWidth * 100 : width;
            resize(visibleWidth + (event.key === "ArrowLeft" ? 3 : -3));
          }
        }}
      />}
      {shown && saveAs ? (
        <PanelPrompt key="save-as" title={l.saveAs} onCancel={cancelSaveAs}>
          <p className="break-all text-xs text-zinc-300">{cwd}{saveAs.directory && " / " + saveAs.directory}</p>
          <form onSubmit={(event) => void submitSaveAs(event)}>
            <input
              data-initial-focus
              aria-label={l.saveFileName}
              placeholder={l.saveFileName}
              className="mt-3 w-full rounded border border-zinc-600 bg-zinc-950 px-2 py-1.5 text-sm"
              value={saveAs.name}
              disabled={saving !== null}
              onChange={(event) => setSaveAs({ ...saveAs, name: event.target.value, error: undefined })}
            />
            {saveAs.error && <p role="alert" className="mt-2 text-sm text-red-300">{saveAs.error}</p>}
            <div className="mt-4 flex flex-wrap justify-end gap-2">
              <PromptButton type="button" tone="secondary" disabled={saving !== null} onClick={cancelSaveAs}>{l.cancel}</PromptButton>
              <PromptButton type="submit" disabled={!saveAs.name.trim() || saving !== null}>{l.saveAs}</PromptButton>
            </div>
          </form>
        </PanelPrompt>
      ) : shown && closing ? (
        <PanelPrompt warning key={closing.path} title={l.saveBeforeClose} onCancel={() => { if (!saveLock.current) setPendingClose([]); }}>
          <p className="break-words font-mono text-xs text-zinc-200">{tabName(closing.path, l)}</p>
          <p className="mt-1.5 text-xs leading-relaxed text-zinc-400">{l.unsavedCloseBody}</p>
          {error && <p role="alert" className="mt-2 text-red-300">{error}</p>}
          <div className="mt-4 flex flex-wrap justify-end gap-2">
            <PromptButton type="button" data-initial-focus disabled={saving !== null} onClick={() => void saveTab(closing, true)}>{l.saveAction}</PromptButton>
            <PromptButton type="button" tone="secondary" disabled={saving !== null} onClick={() => {
              discardTabs([closing]);
              setPendingClose((current) => current.slice(1));
            }}>{l.dontSave}</PromptButton>
            <PromptButton type="button" tone="secondary" disabled={saving !== null} onClick={() => setPendingClose([])}>{l.cancel}</PromptButton>
          </div>
        </PanelPrompt>
      ) : shown && reloadTarget !== null ? (
        <PanelPrompt warning title={l.reload} onCancel={() => setReloadTarget(null)}>
          <p className="text-zinc-300">{source && error ? skills.conflictBody : l.discard}</p>
          <div className="mt-4 flex flex-wrap justify-end gap-2">
            <PromptButton type="button" data-initial-focus tone="secondary" onClick={() => setReloadTarget(null)}>{l.cancel}</PromptButton>
            <PromptButton type="button" tone="danger" onClick={() => reload(reloadTarget)}>{l.reload}</PromptButton>
          </div>
        </PanelPrompt>
      ) : shown && deleteTarget !== null ? (
        <PanelPrompt warning title={l.deleteFile} onCancel={() => setDeleteTarget(null)}>
          <p className="whitespace-pre-line break-words text-zinc-300">{`${deleteTarget}\n${l.deleteBody}\n${l.discard}`}</p>
          <div className="mt-4 flex flex-wrap justify-end gap-2">
            <PromptButton type="button" data-initial-focus tone="secondary" onClick={() => setDeleteTarget(null)}>{l.cancel}</PromptButton>
            <PromptButton type="button" tone="danger" onClick={() => void remove(deleteTarget)}>{l.deleteFile}</PromptButton>
          </div>
        </PanelPrompt>
      ) : null}
      {error && !closing && <p role="alert" className="border-b border-zinc-700 px-2 py-1.5 text-xs break-words text-red-300">{error}</p>}
      <div className="flex min-h-0 flex-1">
        <div className="flex w-8 shrink-0 flex-col items-center gap-1 border-r border-zinc-700 bg-zinc-950 py-1">
          <PanelButton label={l.files} icon={<Files size={17} />} pressed={!finder} onClick={() => setFinder(null)} />
          {!source && <PanelButton label={l.findText} icon={<Search size={17} />} pressed={finder === "content"} onClick={() => {
            setFinder("content");
            searchInput.current?.focus();
          }} />}
          {!source && <PanelButton label={l.vscode} icon={<ExternalLink size={16} />} onClick={() => void openCodeInVSCode(cwd, active && !isUntitled(active) ? active : "", reveal?.line).catch((e) => setError(errorText(e)))} />}
        </div>
        <div className="flex w-[29%] max-w-60 min-w-24 shrink-0 flex-col border-r border-zinc-700 bg-zinc-900 text-xs">
          <div className="flex min-w-0 flex-wrap items-center justify-between gap-0.5 px-2 py-1">
            <span className="min-w-0 truncate font-semibold">{finder === "content" ? l.findText : finder === "files" ? l.findFile : l.files}</span>
            <div className="ml-auto flex items-center">
              {!finder && !source && (
                <PanelButton label={l.newFile} icon={<FilePlus size={14} />} disabled={creating} onClick={() => startNewFile(directory)} />
              )}
              <PanelButton label={l.findFile} icon={<Search size={14} />} pressed={finder === "files"} onClick={() => {
                setFinder("files");
                searchInput.current?.focus();
              }} />
              {!finder && <PanelButton label={l.refresh} icon={<RefreshCw size={14} />} onClick={refreshExplorer} />}
            </div>
          </div>
          {finder ? (
            <>
              <input
                key={finder}
                ref={searchInput}
                autoFocus
                value={query}
                aria-label={finder === "files" ? l.findFile : l.findText}
                placeholder={finder === "files" ? l.findFile : l.findText}
                className="mx-2 mb-1 min-w-0 rounded border border-zinc-600 bg-zinc-950 px-2 py-1"
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Escape") setFinder(null);
                  if (event.key === "Enter" && matches[0]) open({ path: matches[0].path, line: matches[0].line ?? undefined });
                }}
              />
              <div className="min-h-0 flex-1 overflow-auto">
                {matches.map((match) => (
                  <button key={match.path + ":" + match.line} type="button" className="block w-full truncate px-2 py-1 text-left hover:bg-zinc-700" title={match.path}
                    onClick={() => open({ path: match.path, line: match.line ?? undefined })}>
                    {match.path}{match.line ? `:${match.line}  ${match.text}` : ""}
                  </button>
                ))}
                {query.trim() && !matches.length && <p className="p-2 text-zinc-400">{searching ? l.loading : l.noResults}</p>}
              </div>
            </>
          ) : (
            <>
              <ExplorerMenu path="" directory actions={explorerActions} l={l}>
                <button type="button" className={cn("w-full truncate bg-zinc-800 px-2 py-1 text-left font-semibold hover:bg-zinc-700", !directory && "text-sky-200")}
                  title={cwd} onClick={() => setDirectory("")}>{cwd.split(/[\\/]/).filter(Boolean).pop() ?? cwd}</button>
              </ExplorerMenu>
              <div className="min-h-0 flex-1 overflow-auto py-1">
                <Tree entriesFor={entriesFor} path="" refresh={refresh} status={status} active={active} directory={directory} expanded={expanded}
                  folderMarks={folderMarks} toggleDirectory={toggleDirectory} actions={explorerActions} l={l}
                  newFileDirectory={newFile?.directory}
                  inlineFile={newFile && (
                    <form onSubmit={(event) => void create(event)} className="flex h-6 min-w-0 items-center gap-1 px-1.5">
                      <File className="ml-3 size-3.5 shrink-0 text-sky-300" />
                      <input
                        key={newFile.directory}
                        ref={newFileInput}
                        aria-label={l.fileName}
                        placeholder={l.fileName}
                        value={newFile.name}
                        disabled={creating}
                        className="h-5 min-w-0 flex-1 border border-sky-500 bg-zinc-950 px-1 outline-none"
                        onChange={(event) => setNewFile({ ...newFile, name: event.target.value })}
                        onBlur={(event) => {
                          if (!event.currentTarget.value.trim() && !creating) setNewFile(null);
                        }}
                        onKeyDown={(event) => {
                          if (event.key === "Escape" && !creating) setNewFile(null);
                        }}
                      />
                    </form>
                  )}
                />
              </div>
            </>
          )}
        </div>
        <div className="flex min-w-0 flex-1 flex-col bg-zinc-800">
          <div className="flex shrink-0 overflow-x-auto border-b border-zinc-700 bg-zinc-900">
            {tabs.map((item, index) => (
              <WorkspaceMenu key={item.path} trigger={
                <div className={cn("flex shrink-0 items-center border-r border-zinc-700 text-xs", active === item.path && "bg-zinc-800 text-sky-200")}>
                  <button type="button" aria-pressed={active === item.path} className="flex max-w-36 min-w-0 items-center gap-1.5 px-2 py-1.5" title={isUntitled(item.path) ? tabName(item.path, l) : item.path} onClick={() => open({ path: item.path })}>
                    <WorkspaceFileIcon name={tabName(item.path, l)} />
                    <span className="truncate">{tabName(item.path, l)}</span>
                  </button>
                  <PanelButton label={l.close + " " + tabName(item.path, l)} icon={isDirty(item) ? (
                    <span className="grid size-3 place-items-center" aria-hidden="true">
                      <span className="size-2 rounded-full bg-current group-hover:hidden group-focus-visible:hidden" />
                      <X size={12} className="hidden group-hover:block group-focus-visible:block" />
                    </span>
                  ) : <X size={12} />} onClick={() => closeTabs([item])} />
                </div>
              }>
                <MenuItem disabled={item.path === saving || item.path === deleting} onSelect={() => closeTabs([item])}>{l.close}</MenuItem>
                <MenuItem disabled={tabs.length < 2} onSelect={() => closeTabs(tabs.filter((tab) => tab !== item))}>{l.closeOthers}</MenuItem>
                <MenuItem disabled={index === tabs.length - 1} onSelect={() => closeTabs(tabs.slice(index + 1))}>{l.closeRight}</MenuItem>
                <MenuItem onSelect={() => closeTabs(tabs)}>{l.closeAll}</MenuItem>
                {!isUntitled(item.path) && (
                  <>
                    <MenuSeparator />
                    <PathMenuItems path={item.path} actions={explorerActions} l={l} />
                  </>
                )}
              </WorkspaceMenu>
            ))}
            {!source && <WorkspaceMenu trigger={
              <button
                type="button"
                aria-label={l.newFileFromTabs}
                title={l.newFileFromTabs}
                disabled={creating}
                className="min-h-7 min-w-6 flex-1 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-sky-500"
                onDoubleClick={openUntitled}
                onClick={(event) => { if (event.detail === 0) openUntitled(); }}
              />
            } onCloseAutoFocus={(event) => {
              if (newFileInput.current) {
                event.preventDefault();
                newFileInput.current.focus();
              }
            }}>
              <MenuItem disabled={creating} onSelect={openUntitled}>{l.newFile}</MenuItem>
              <MenuItem disabled={!tabs.length} onSelect={() => closeTabs(tabs)}>{l.closeAll}</MenuItem>
            </WorkspaceMenu>}
          </div>
          {tab && (
            <div className="flex min-w-0 shrink-0 flex-wrap items-center gap-1 border-b border-zinc-700 px-2 py-1">
              {isUntitled(tab.path) ? (
                <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-zinc-300">{tabName(tab.path, l)}</span>
              ) : (
                <>
                  <WorkspaceMenu trigger={<span className="min-w-0 flex-1 truncate font-mono text-[11px] text-zinc-300" title={tab.path}>{tab.path.split("/").join(" › ")}</span>}>
                    <PathMenuItems path={tab.path} actions={explorerActions} l={l} />
                  </WorkspaceMenu>
                  <PanelButton label={l.reload} icon={<RefreshCw size={13} />} disabled={saving === tab.path} onClick={() => {
                    if (dirty) setReloadTarget(tab.path);
                    else reload(tab.path);
                  }} />
                </>
              )}
              {file?.kind === "text" && (
                <>
                  {file.baseline !== null && <PanelButton label={l.changes} pressed={mode === "diff"} onClick={() => setMode(mode === "diff" ? "code" : "diff")} />}
                  {/\.(md|markdown)$/i.test(tab.path) && <PanelButton label={l.preview} pressed={mode === "preview"} onClick={() => setMode(mode === "preview" ? "code" : "preview")} />}
                  {!readOnly && <PanelButton label={l.save} icon={<Save size={14} />} disabled={(!dirty && !isUntitled(tab.path)) || saving !== null || deleting === tab.path} onClick={() => void save()} />}
                </>
              )}
            </div>
          )}
          {tab?.error && <p role="alert" className="p-2 text-xs break-words text-red-300">{tab.error}</p>}
          {file?.kind === "image" ? (
            <div className="grid min-h-0 flex-1 place-items-center overflow-auto p-4"><img className="max-w-full object-contain" src={file.data} alt={nameOf(tab!.path)} /></div>
          ) : file?.kind === "text" && tab ? (
            mode === "diff" && file.baseline !== null ? <CodeDiff base={file.baseline} text={tab.text ?? ""} empty={l.diffTooLarge} />
              : mode === "preview" ? (
                <div className="min-h-0 flex-1 overflow-auto p-4 text-sm leading-relaxed [&_a]:text-sky-200 [&_h1]:text-xl [&_h2]:text-lg [&_li]:ml-4 [&_li]:list-disc [&_p]:my-2 [&_pre]:overflow-auto">
                  <ReactMarkdown remarkPlugins={[remarkGfm]} components={{ a: ({ children }) => <span>{children}</span>, img: ({ alt }) => <span>{alt}</span> }}>
                    {tab.text ?? ""}
                  </ReactMarkdown>
                </div>
              ) : (
                <CodeEditor key={tab.path} path={tab.path} value={tab.text ?? ""} baseline={file.baseline} readOnly={readOnly || saving === tab.path || deleting === tab.path} reveal={reveal} l={l} onError={setError} onSave={() => void save()} onChange={(text) => {
                  if (readOnly || text === tab.text || saving === tab.path || deleting === tab.path) return;
                  const key = cwd + "\0" + tab.path;
                  if (text === file.text && !isUntitled(tab.path)) drafts.delete(key);
                  else drafts.set(key, { source: file, text });
                  update({ ...tab, text });
                }} />
              )
          ) : !tab?.error && <p className="grid flex-1 place-items-center p-3 text-center text-xs text-zinc-400">{tab ? l.loading : l.select}</p>}
          {file?.kind === "text" && <div className="shrink-0 border-t border-zinc-700 px-2 py-1 text-[11px] text-zinc-300">{!source && `${base?.base_branch || "HEAD"} · `}UTF-8 · {readOnly ? skills.readonlyGroup : saving ? l.saving : dirty ? l.unsaved : l.editable}</div>}
        </div>
      </div>
    </aside>
  );
}

function PanelButton({ label, icon, disabled, pressed, onClick }: {
  label: string; icon?: React.ReactNode; disabled?: boolean; pressed?: boolean; onClick: () => void;
}) {
  return (
    <button type="button" aria-label={label} title={label} aria-pressed={pressed} disabled={disabled} onClick={onClick}
      className="group shrink-0 rounded px-1.5 py-1 text-xs hover:bg-zinc-700 aria-pressed:bg-sky-900 disabled:opacity-40">
      {icon ?? label}
    </button>
  );
}
