import * as React from "react";
import { LoadingState } from "@/components/ui/loading-state";
import { FileText } from "lucide-react";
import { listMemories, onMemoryChanged } from "@/api/memory";
import type { MemoryFile, MemoryIndex, Project } from "@/api/types";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { ConfirmDialog } from "./ConfirmDialog";
import { MemoryEditor } from "./MemoryEditor";
import { MemoryGraphPanel, type CachedMemoryGraph } from "./MemoryGraphPanel";
import { MemoryTree } from "./MemoryTree";
import { SearchPanel } from "./SearchPanel";
import { allFiles, fileKey, findByKey } from "./paths";
import { useSyncSkipped } from "./useSyncSkipped";

const WIDTH_KEY = "memdock.treeWidth";
const MIN_W = 220;
const MAX_W = 420;
const DEFAULT_W = 280;

const clamp = (w: number) => Math.min(MAX_W, Math.max(MIN_W, Math.round(w)));

function loadWidth(): number {
  try {
    const n = Number(localStorage.getItem(WIDTH_KEY));
    if (Number.isFinite(n) && n > 0) return clamp(n);
  } catch {
    // storage unavailable
  }
  return DEFAULT_W;
}

export interface MemoryPageProps {
  onDirtyChange?: (dirty: boolean) => void;
  /** Pending review suggestions per project id, shown as dots in the tree. */
  pendingByProject?: Record<string, number>;
  /** Asks the page to open a file (memory index id); `nonce` makes repeats count. */
  openRequest?: { fileId: string; nonce: number } | null;
}

type Destination =
  | { type: "file"; key: string; line?: number }
  | { type: "graph"; projectId: string };

export function MemoryPage({ onDirtyChange, pendingByProject, openRequest }: MemoryPageProps = {}) {
  const { t } = useI18n();
  const m = t.memory;
  const skipped = useSyncSkipped();
  const [index, setIndex] = React.useState<MemoryIndex | null>(null);
  const [selectedKey, setSelectedKey] = React.useState<string | undefined>();
  const [dirty, setDirty] = React.useState(false);
  const [pending, setPending] = React.useState<Destination | null>(null);
  const [graphProjectId, setGraphProjectId] = React.useState<string>();
  const graphCache = React.useRef(new Map<string, CachedMemoryGraph>());
  React.useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);
  React.useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);
  const [search, setSearch] = React.useState<{ initial: string } | null>(null);
  const [reveal, setReveal] = React.useState<{ key: string; line: number; nonce: number } | null>(null);
  const revealNonce = React.useRef(0);
  const [width, setWidth] = React.useState(loadWidth);
  const widthRef = React.useRef(width);
  widthRef.current = width;

  // Initial index and live updates. The watcher already refreshed the cache, so a plain
  // listMemories is enough on every event.
  React.useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    const refresh = () =>
      listMemories().then(
        (idx) => alive && setIndex(idx),
        (e) => console.error("list_memories failed", e),
      );
    void refresh();
    onMemoryChanged(() => void refresh()).then((u) => {
      if (alive) unlisten = u;
      else u();
    });
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  // Open the first existing global file the first time the index arrives.
  const didAutoSelect = React.useRef(false);
  React.useEffect(() => {
    if (!index || didAutoSelect.current) return;
    didAutoSelect.current = true;
    const first = index.global.find((f) => f.exists && f.kind !== "codex_db");
    if (first) setSelectedKey(fileKey(first));
  }, [index]);

  // An open file can vanish from the index (deleted externally, project removed). With unsaved
  // edits the editor must stay so the text is not lost; otherwise fall back to the empty state.
  const lastSelected = React.useRef<{ key: string; file: MemoryFile } | null>(null);
  const found = index ? findByKey(index, selectedKey) : undefined;
  if (found && selectedKey) lastSelected.current = { key: selectedKey, file: found };
  const kept = !found && dirty && selectedKey && lastSelected.current?.key === selectedKey ? lastSelected.current.file : undefined;
  const selected = found ?? kept;
  const removed = !found && kept !== undefined;
  const graphProject = index?.projects.find(({ project }) => project.id === graphProjectId);
  React.useEffect(() => {
    if (!index) return;
    const ids = new Set(index.projects.map(({ project }) => project.id));
    for (const id of graphCache.current.keys()) {
      if (!ids.has(id)) graphCache.current.delete(id);
    }
    if (graphProjectId && !ids.has(graphProjectId)) setGraphProjectId(undefined);
  }, [index, graphProjectId]);
  const [pendingRemove, setPendingRemove] = React.useState<(() => Promise<void>) | null>(null);
  const requestRemove = (project: Project, remove: () => Promise<void>) => {
    const open = selected && selected.scope.type === "project" && selected.scope.project_id === project.id;
    if (dirty && open) setPendingRemove(() => remove);
    else void remove();
  };

  const navigate = (destination: Destination) => {
    if (destination.type === "graph") {
      setSelectedKey(undefined);
      setReveal(null);
      setGraphProjectId(destination.projectId);
      setSearch(null);
    } else {
      setGraphProjectId(undefined);
      setSelectedKey(destination.key);
      setReveal(destination.line === undefined ? null : {
        key: destination.key,
        line: destination.line,
        nonce: ++revealNonce.current,
      });
    }
  };
  const select = (file: MemoryFile, line?: number) => {
    const key = fileKey(file);
    const destination: Destination = { type: "file", key, line };
    if (key === selectedKey && line === undefined) return;
    if (dirty && key !== selectedKey) setPending(destination);
    else navigate(destination);
  };
  const openGraph = (projectId: string) => {
    if (projectId === graphProjectId) return;
    const destination: Destination = { type: "graph", projectId };
    if (dirty) setPending(destination);
    else navigate(destination);
  };

  // Search result -> open the file (global occurrence first) and jump to the line.
  const openHit = (fileId: string, line: number) => {
    if (!index) return;
    const matches = allFiles(index).filter((f) => f.id === fileId);
    const file = matches.find((f) => f.scope.type === "global") ?? matches[0];
    if (!file) return;
    select(file, line);
  };
  const clearReveal = React.useCallback(() => setReveal(null), []);

  // Requests from other pages (e.g. the sync page's skipped files) open the file once the index is in.
  const handledOpen = React.useRef<number | null>(null);
  React.useEffect(() => {
    if (!openRequest || !index || handledOpen.current === openRequest.nonce) return;
    handledOpen.current = openRequest.nonce;
    openHit(openRequest.fileId, 1);
    // openHit reads the latest index / selection; only a new request should re-run this.
  }, [openRequest, index]);
  const openSearch = React.useCallback((initial: string) => setSearch((prev) => prev ?? { initial }), []);

  const startResize = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = widthRef.current;
    const el = e.currentTarget;
    el.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => setWidth(clamp(startW + ev.clientX - startX));
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
      try {
        localStorage.setItem(WIDTH_KEY, String(widthRef.current));
      } catch {
        // ignore
      }
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
  };

  const onResizeKey = (e: React.KeyboardEvent) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    const next = clamp(widthRef.current + (e.key === "ArrowLeft" ? -16 : 16));
    setWidth(next);
    try {
      localStorage.setItem(WIDTH_KEY, String(next));
    } catch {
      // ignore
    }
  };

  const openGlobalClaude = () => {
    const f = index?.global.find((g) => g.kind === "claude_instructions") ?? index?.global[0];
    if (f) select(f);
  };

  if (!index) {
    return (
      <LoadingState label={m.loading} />
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-1">
      <div className="relative shrink-0" style={{ width }}>
        <div className={cn("h-full", search && "hidden")}>
          <MemoryTree
            index={index}
            selectedKey={selectedKey}
            onSelect={select}
            graphProjectId={graphProjectId}
            onOpenGraph={openGraph}
            onIndexChange={setIndex}
            onRemoveRequest={requestRemove}
            onSearchOpen={openSearch}
            pendingByProject={pendingByProject}
            skipped={skipped}
          />
        </div>
        {search ? (
          <SearchPanel
            index={index}
            initialQuery={search.initial}
            onOpen={openHit}
            onClose={() => setSearch(null)}
            idle={
              <MemoryTree
                hideHeader
                index={index}
                selectedKey={selectedKey}
                onSelect={select}
                graphProjectId={graphProjectId}
                onOpenGraph={openGraph}
                onIndexChange={setIndex}
                onRemoveRequest={requestRemove}
                pendingByProject={pendingByProject}
                skipped={skipped}
              />
            }
          />
        ) : null}
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label={m.resize}
          aria-valuemin={MIN_W}
          aria-valuemax={MAX_W}
          aria-valuenow={width}
          tabIndex={0}
          onPointerDown={startResize}
          onKeyDown={onResizeKey}
          className="absolute top-0 right-0 h-full w-1 translate-x-1/2 cursor-col-resize outline-none transition-colors duration-150 hover:bg-primary/40 focus-visible:bg-ring motion-reduce:transition-none"
        />
      </div>
      <div className="flex min-w-0 flex-1 border-l border-border">
        {graphProject ? (
          <MemoryGraphPanel
            key={graphProject.project.id}
            entry={graphProject}
            cache={graphCache.current}
            onOpenSource={(file, line) => {
              const current = graphProject.files.find((candidate) => fileKey(candidate) === fileKey(file));
              if (!current?.exists) return false;
              select(current, line);
              return true;
            }}
          />
        ) : selected ? (
          <MemoryEditor
            key={fileKey(selected)}
            file={selected}
            index={index}
            removed={removed}
            onOpenImport={select}
            onDirtyChange={setDirty}
            reveal={reveal && reveal.key === fileKey(selected) ? reveal : undefined}
            onRevealed={clearReveal}
          />
        ) : (
          <div className="flex flex-1 flex-col items-center justify-center gap-3">
            <FileText className="size-8 text-muted-foreground" aria-hidden="true" />
            <p className="text-sm text-muted-foreground">{m.emptyBody}</p>
            <Button type="button" onClick={openGlobalClaude} className="h-7 cursor-pointer px-3 text-xs font-semibold">
              {m.emptyAction}
            </Button>
          </div>
        )}
      </div>
      <ConfirmDialog
        open={pendingRemove !== null}
        title={m.discardTitle}
        body={m.removeDiscardBody}
        confirmLabel={m.removeDiscardConfirm}
        cancelLabel={m.keepEditing}
        destructive
        onConfirm={() => {
          const go = pendingRemove;
          setPendingRemove(null);
          if (go) {
            setDirty(false);
            void go();
          }
        }}
        onCancel={() => setPendingRemove(null)}
      />
      <ConfirmDialog
        open={pending !== null}
        title={m.discardTitle}
        body={m.discardBody}
        confirmLabel={m.discardConfirm}
        cancelLabel={m.keepEditing}
        destructive
        onConfirm={() => {
          if (pending) {
            setDirty(false);
            navigate(pending);
          }
          setPending(null);
        }}
        onCancel={() => setPending(null)}
      />
    </div>
  );
}
