import * as React from "react";
import { Check, ChevronDown, ChevronRight, Globe, Plus, Search } from "lucide-react";
import { ContextMenu } from "radix-ui";
import { open } from "@tauri-apps/plugin-dialog";
import { addManualProject, removeManualProject } from "@/api/memory";
import type { MemoryFile, MemoryIndex, Project, ProjectMemories } from "@/api/types";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { ToolTag } from "./ToolTag";
import { basename, compactNumber, fileKey, fmt } from "./paths";

export interface MemoryTreeProps {
  index: MemoryIndex;
  /** `fileKey()` of the opened file. */
  selectedKey?: string;
  onSelect: (file: MemoryFile) => void;
  /** Called with the fresh index after adding / removing a manual project. */
  onIndexChange?: (index: MemoryIndex) => void;
  /**
   * Lets the page veto/confirm a project removal (e.g. unsaved edits in one of its files).
   * Call `remove()` to proceed; without this prop the project is removed immediately.
   */
  onRemoveRequest?: (project: Project, remove: () => Promise<void>) => void;
  /**
   * Full-text search hook. When given, typing in the search box (and Ctrl/Cmd+K) hands over to
   * the page's search panel instead of filtering the tree by name.
   */
  onSearchOpen?: (initialQuery: string) => void;
  /** Hides the search / add-project bar (used under the search panel's own box). */
  hideHeader?: boolean;
  /** Pending review suggestions per project id (`__global__` for the global group). */
  pendingByProject?: Record<string, number>;
}

type Row =
  | { kind: "file"; key: string; file: MemoryFile; depth: number; parent?: string }
  | { kind: "project"; key: string; entry: ProjectMemories; expanded: boolean }
  | { kind: "auto"; key: string; files: MemoryFile[]; expanded: boolean; parent: string };

type Item = Row | { kind: "heading"; key: string; label: string; icon?: boolean };

const GLOBAL_PENDING = "__global__";

/** Small `--warning` dot: this project (or the global group) has suggestions waiting for review. */
function PendingDot({ n, push }: { n: number; push?: boolean }) {
  const { t } = useI18n();
  if (n <= 0) return null;
  const label = fmt(t.review.pendingTip, { n });
  return (
    <span
      role="img"
      aria-label={label}
      title={label}
      className={cn("size-2 shrink-0 rounded-full bg-warning", push && "ml-auto")}
    />
  );
}

const projKey = (id: string) => `proj:${id}`;
const autoKey = (id: string) => `auto:${id}`;

function isManualOnly(entry: ProjectMemories): boolean {
  const s = entry.project.sources;
  return s.length > 0 && s.every((x) => x === "manual");
}

const EMPTY_KEY = "memdock.showEmptyProjects";

function loadShowEmpty(): boolean {
  try {
    return localStorage.getItem(EMPTY_KEY) === "1";
  } catch {
    return false;
  }
}

export function MemoryTree({ index, selectedKey, onSelect, onIndexChange, onRemoveRequest, onSearchOpen, pendingByProject, hideHeader }: MemoryTreeProps) {
  const { t } = useI18n();
  const m = t.memory;
  const [query, setQuery] = React.useState("");
  const [showEmpty, setShowEmpty] = React.useState(loadShowEmpty);
  const [expanded, setExpanded] = React.useState<Set<string>>(new Set());
  const [focusKey, setFocusKey] = React.useState<string | undefined>(selectedKey);
  const treeRef = React.useRef<HTMLDivElement>(null);
  const searchRef = React.useRef<HTMLInputElement>(null);

  // Opening a file (also from an import link) reveals it.
  React.useEffect(() => {
    if (!selectedKey) return;
    const need: string[] = [];
    for (const p of index.projects) {
      for (const f of p.files) {
        if (fileKey(f) !== selectedKey) continue;
        need.push(projKey(p.project.id));
        if (f.kind === "claude_auto") need.push(autoKey(p.project.id));
      }
    }
    if (need.length) setExpanded((prev) => (need.every((k) => prev.has(k)) ? prev : new Set([...prev, ...need])));
    setFocusKey(selectedKey);
  }, [selectedKey, index]);

  // Ctrl/Cmd+K focuses the filter box.
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        if (onSearchOpen) onSearchOpen("");
        else searchRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onSearchOpen]);

  const q = query.trim().toLowerCase();
  const items = React.useMemo<Item[]>(() => {
    const out: Item[] = [];
    const nameOf = (f: MemoryFile) => (f.kind === "codex_db" ? m.codexDb : basename(f.path));
    const matches = (f: MemoryFile) => !q || nameOf(f).toLowerCase().includes(q);

    const globals = index.global.filter(matches);
    if (globals.length) {
      out.push({ kind: "heading", key: "h:global", label: m.global, icon: true });
      for (const file of globals) out.push({ kind: "file", key: fileKey(file), file, depth: 0 });
    }

    const visible = index.projects.filter((p) => showEmpty || p.files.length > 0);
    const projects = visible.filter((p) => !q || p.project.name.toLowerCase().includes(q) || p.files.some(matches));
    if (projects.length || !q) {
      out.push({ kind: "heading", key: "h:projects", label: fmt(m.projects, { n: visible.length }) });
    }
    for (const entry of projects) {
      const pk = projKey(entry.project.id);
      const nameHit = !q || entry.project.name.toLowerCase().includes(q);
      const files = nameHit ? entry.files : entry.files.filter(matches);
      const pOpen = q ? true : expanded.has(pk);
      out.push({ kind: "project", key: pk, entry, expanded: pOpen });
      if (!pOpen) continue;
      const autos = files.filter((f) => f.kind === "claude_auto");
      for (const file of files.filter((f) => f.kind !== "claude_auto")) {
        out.push({ kind: "file", key: fileKey(file), file, depth: 1, parent: pk });
      }
      if (autos.length) {
        const ak = autoKey(entry.project.id);
        const aOpen = q ? true : expanded.has(ak);
        out.push({ kind: "auto", key: ak, files: autos, expanded: aOpen, parent: pk });
        if (aOpen) {
          for (const file of autos) out.push({ kind: "file", key: fileKey(file), file, depth: 2, parent: ak });
        }
      }
    }
    return out;
  }, [index, q, expanded, showEmpty, m]);

  const emptyCount = React.useMemo(() => index.projects.filter((p) => p.files.length === 0).length, [index]);
  const toggleEmpty = () => {
    const next = !showEmpty;
    setShowEmpty(next);
    try {
      localStorage.setItem(EMPTY_KEY, next ? "1" : "0");
    } catch {
      // ignore
    }
  };

  const rows = React.useMemo(() => items.filter((i): i is Row => i.kind !== "heading"), [items]);
  const activeKey = rows.some((r) => r.key === focusKey) ? focusKey : rows[0]?.key;

  const toggle = React.useCallback((key: string, force?: boolean) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      const on = force ?? !next.has(key);
      if (on) next.add(key);
      else next.delete(key);
      return next;
    });
  }, []);

  const focusRow = (key: string) => {
    setFocusKey(key);
    const els = treeRef.current?.querySelectorAll<HTMLElement>('[role="treeitem"]');
    els?.forEach((el) => {
      if (el.dataset.key === key) el.focus();
    });
  };

  const activate = (row: Row) => {
    setFocusKey(row.key);
    if (row.kind === "file") onSelect(row.file);
    else toggle(row.key);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const target = (e.target as HTMLElement).closest<HTMLElement>('[role="treeitem"]');
    if (!target) return;
    const i = rows.findIndex((r) => r.key === target.dataset.key);
    if (i < 0) return;
    const row = rows[i];
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        if (i + 1 < rows.length) focusRow(rows[i + 1].key);
        break;
      case "ArrowUp":
        e.preventDefault();
        if (i > 0) focusRow(rows[i - 1].key);
        break;
      case "Home":
        e.preventDefault();
        focusRow(rows[0].key);
        break;
      case "End":
        e.preventDefault();
        focusRow(rows[rows.length - 1].key);
        break;
      case "ArrowRight":
        if (row.kind !== "file") {
          e.preventDefault();
          if (!row.expanded) toggle(row.key, true);
          else if (rows[i + 1]) focusRow(rows[i + 1].key);
        }
        break;
      case "ArrowLeft":
        e.preventDefault();
        if (row.kind !== "file" && row.expanded) toggle(row.key, false);
        else if (row.kind !== "project" && row.parent) focusRow(row.parent);
        break;
      case "Enter":
      case " ":
        e.preventDefault();
        activate(row);
        break;
    }
  };

  const addProject = async () => {
    try {
      const dir = await open({ directory: true });
      if (typeof dir !== "string" || !dir) return;
      onIndexChange?.(await addManualProject(dir));
    } catch (err) {
      console.error("add project failed", err);
    }
  };

  const removeProject = (project: Project) => {
    const remove = async () => {
      try {
        onIndexChange?.(await removeManualProject(project.path));
      } catch (err) {
        console.error("remove project failed", err);
      }
    };
    if (onRemoveRequest) onRemoveRequest(project, remove);
    else void remove();
  };

  return (
    <div className="flex h-full min-h-0 w-full flex-col">
      <div className={cn("flex gap-2 border-b border-border p-3", hideHeader && "hidden")}>
        <label className="flex h-[30px] min-w-0 flex-1 items-center gap-1.5 rounded-md border border-border bg-card px-2.5 text-muted-foreground focus-within:ring-2 focus-within:ring-ring">
          <Search className="size-4 shrink-0" aria-hidden="true" />
          <Input
            ref={searchRef}
            value={onSearchOpen ? "" : query}
            onClick={() => onSearchOpen?.("")}
            onChange={(e) => (onSearchOpen ? onSearchOpen(e.target.value) : setQuery(e.target.value))}
            placeholder={m.search}
            aria-label={m.search}
            className="h-auto min-w-0 flex-1 rounded-none border-0 bg-transparent p-0 text-sm text-foreground shadow-none focus-visible:ring-0 md:text-sm dark:bg-transparent"
          />
          <kbd className="shrink-0 rounded-sm border border-border px-1 font-sans text-xs leading-4">Ctrl K</kbd>
        </label>
        <button
          type="button"
          aria-label={m.addProject}
          title={m.addProject}
          onClick={addProject}
          className="grid size-[30px] shrink-0 cursor-pointer place-items-center rounded-md border border-border bg-card text-muted-foreground transition-colors duration-150 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none"
        >
          <Plus className="size-4" aria-hidden="true" />
        </button>
      </div>

      <div
        ref={treeRef}
        role="tree"
        aria-label={m.treeLabel}
        onKeyDown={onKeyDown}
        className="min-h-0 flex-1 overflow-auto p-2"
      >
        {items.length === 0 || (q && rows.length === 0) ? (
          <p className="px-2 py-4 text-xs text-muted-foreground">{q ? m.noMatches : m.treeEmpty}</p>
        ) : null}
        {items.map((item) => {
          if (item.kind === "heading") {
            return (
              <div
                key={item.key}
                role="presentation"
                className="flex items-center gap-1.5 px-2 pt-2.5 pb-1 text-xs font-semibold tracking-[0.04em] text-muted-foreground uppercase"
              >
                {item.icon ? <Globe className="size-3" aria-hidden="true" /> : null}
                {item.label}
                {item.key === "h:global" ? <PendingDot n={pendingByProject?.[GLOBAL_PENDING] ?? 0} /> : null}
              </div>
            );
          }
          return (
            <TreeRow
              key={item.key}
              row={item}
              selected={item.kind === "file" && item.key === selectedKey}
              tabbable={item.key === activeKey}
              onActivate={() => activate(item)}
              onFocus={() => setFocusKey(item.key)}
              onRemove={removeProject}
              pending={item.kind === "project" ? (pendingByProject?.[item.entry.project.id] ?? 0) : 0}
            />
          );
        })}
      </div>
      {emptyCount > 0 ? (
        <button
          type="button"
          role="switch"
          aria-checked={showEmpty}
          onClick={toggleEmpty}
          className="flex h-8 shrink-0 cursor-pointer items-center gap-2 border-t border-border px-3 text-xs text-muted-foreground transition-colors duration-150 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset motion-reduce:transition-none"
        >
          <span
            className={cn(
              "grid size-3.5 shrink-0 place-items-center rounded-sm border border-border",
              showEmpty && "border-primary bg-primary text-primary-foreground",
            )}
          >
            {showEmpty ? <Check className="size-3" aria-hidden="true" /> : null}
          </span>
          {fmt(m.showEmpty, { n: emptyCount })}
        </button>
      ) : null}
    </div>
  );
}

function TreeRow({
  row,
  selected,
  tabbable,
  onActivate,
  onFocus,
  onRemove,
  pending,
}: {
  pending: number;
  row: Row;
  selected: boolean;
  tabbable: boolean;
  onActivate: () => void;
  onFocus: () => void;
  onRemove: (project: Project) => void;
}) {
  const { t } = useI18n();
  const m = t.memory;

  const base =
    "flex h-7 cursor-pointer items-center gap-2 rounded-md px-2 text-foreground transition-colors duration-150 outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none";
  const common = {
    role: "treeitem" as const,
    tabIndex: tabbable ? 0 : -1,
    "data-key": row.key,
    onClick: onActivate,
    onFocus,
  };

  if (row.kind === "project") {
    const { project } = row.entry;
    const Chevron = row.expanded ? ChevronDown : ChevronRight;
    const node = (
      <div
        {...common}
        aria-expanded={row.expanded}
        aria-level={1}
        title={project.path}
        className={cn(base, "font-medium", !project.exists && "text-muted-foreground")}
      >
        <Chevron className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        <span className="min-w-0 truncate">{project.name}</span>
        {!project.exists ? (
          <span className="ml-auto shrink-0 text-xs font-normal text-muted-foreground">{m.missingDir}</span>
        ) : null}
        <PendingDot n={pending} push={project.exists} />
      </div>
    );
    if (!isManualOnly(row.entry)) return node;
    return (
      <ContextMenu.Root>
        <ContextMenu.Trigger asChild>{node}</ContextMenu.Trigger>
        <ContextMenu.Portal>
          <ContextMenu.Content className="z-50 min-w-36 rounded-md border border-border bg-popover p-1 text-sm text-popover-foreground shadow-md">
            <ContextMenu.Item
              onSelect={() => onRemove(project)}
              className="flex h-7 cursor-pointer items-center rounded-sm px-2 text-destructive outline-none data-[highlighted]:bg-muted"
            >
              {m.removeProject}
            </ContextMenu.Item>
          </ContextMenu.Content>
        </ContextMenu.Portal>
      </ContextMenu.Root>
    );
  }

  if (row.kind === "auto") {
    const Chevron = row.expanded ? ChevronDown : ChevronRight;
    return (
      <div
        {...common}
        aria-expanded={row.expanded}
        aria-level={2}
        aria-label={fmt(m.autoLabel, { n: row.files.length })}
        className={cn(base, "pl-[30px]")}
      >
        <ToolTag tool="claude" />
        <span className="min-w-0 truncate">{m.autoMemory}</span>
        <span className="ml-auto flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
          {fmt(m.autoCount, { n: row.files.length })}
          <Chevron className="size-3.5" aria-hidden="true" />
        </span>
      </div>
    );
  }

  const { file } = row;
  const isDb = file.kind === "codex_db";
  const meta = isDb ? m.readOnly : !file.exists ? m.notCreated : compactNumber(file.size_bytes);
  return (
    <div
      {...common}
      aria-selected={selected}
      aria-level={row.depth + 1}
      className={cn(
        base,
        row.depth === 1 && "pl-[30px]",
        row.depth === 2 && "pl-[48px]",
        !file.exists && "text-muted-foreground",
        selected && "bg-muted shadow-[inset_2px_0_0_var(--primary)]",
      )}
    >
      {/* Auto-memory children sit under a group row that already carries the tag. */}
      {row.depth < 2 ? <ToolTag tool={file.tool} /> : null}
      <span className={cn("min-w-0 truncate", !isDb && "font-mono text-[12.5px]")}>
        {isDb ? m.codexDb : basename(file.path)}
      </span>
      <span className="ml-auto shrink-0 text-xs text-muted-foreground">{meta}</span>
    </div>
  );
}
