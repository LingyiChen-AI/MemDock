import * as React from "react";
import { Search, X } from "lucide-react";
import { searchMemories } from "@/api/memory";
import type { MemoryFile, MemoryIndex, SearchHit } from "@/api/types";
import { Input } from "@/components/ui/input";
import { useI18n } from "@/i18n/I18nProvider";
import { ToolTag } from "./ToolTag";
import { basename, fmt } from "./paths";

const DEBOUNCE_MS = 300;

export interface SearchPanelProps {
  /** Opens a file at a 1-based line. `fileId` is the normalized file id from the index. */
  onOpen: (fileId: string, line: number) => void;
  /** Used to label results with tool, project and file name. */
  index?: MemoryIndex;
  initialQuery?: string;
  /** Esc and the close button. */
  onClose?: () => void;
}

interface Group {
  fileId: string;
  file?: MemoryFile;
  projectName?: string;
  hits: SearchHit[];
}

/** Same id can be indexed under several scopes; the global occurrence wins. */
function locate(index: MemoryIndex | undefined, fileId: string): { file?: MemoryFile; projectName?: string } {
  if (!index) return {};
  const g = index.global.find((f) => f.id === fileId);
  if (g) return { file: g };
  for (const p of index.projects) {
    const f = p.files.find((x) => x.id === fileId);
    if (f) return { file: f, projectName: p.project.name };
  }
  return {};
}

/** Splits `text` around case-insensitive occurrences of `query` (no regex, so no escaping). */
function highlight(text: string, query: string): React.ReactNode {
  const q = query.trim().toLowerCase();
  if (!q) return text;
  const lower = text.toLowerCase();
  const out: React.ReactNode[] = [];
  let from = 0;
  for (;;) {
    const at = lower.indexOf(q, from);
    if (at < 0) break;
    if (at > from) out.push(text.slice(from, at));
    out.push(
      <mark key={at} className="rounded-xs bg-primary/25 px-px text-inherit">
        {text.slice(at, at + q.length)}
      </mark>,
    );
    from = at + q.length;
  }
  if (from === 0) return text;
  if (from < text.length) out.push(text.slice(from));
  return out;
}

export function SearchPanel({ onOpen, index, initialQuery = "", onClose }: SearchPanelProps) {
  const { t } = useI18n();
  const m = t.memory;
  const [input, setInput] = React.useState(initialQuery);
  const [result, setResult] = React.useState<{ query: string; hits: SearchHit[] } | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);
  const inputRef = React.useRef<HTMLInputElement>(null);
  const listRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  // Ctrl/Cmd+K while the panel is open jumps back to the input.
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        inputRef.current?.focus();
        inputRef.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  React.useEffect(() => {
    const q = input.trim();
    if (!q) {
      setResult(null);
      setError(null);
      setBusy(false);
      return;
    }
    let alive = true;
    setBusy(true);
    const timer = window.setTimeout(() => {
      searchMemories(q).then(
        (hits) => {
          if (!alive) return;
          setResult({ query: q, hits });
          setError(null);
          setBusy(false);
        },
        (e) => {
          if (!alive) return;
          const msg = typeof e === "object" && e !== null && "message" in e ? String((e as { message: unknown }).message) : String(e);
          setError(msg);
          setBusy(false);
        },
      );
    }, DEBOUNCE_MS);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [input]);

  const groups = React.useMemo<Group[]>(() => {
    const byId = new Map<string, Group>();
    for (const h of result?.hits ?? []) {
      let g = byId.get(h.file_id);
      if (!g) {
        g = { fileId: h.file_id, ...locate(index, h.file_id), hits: [] };
        byId.set(h.file_id, g);
      }
      g.hits.push(h);
    }
    return [...byId.values()];
  }, [result, index]);

  const moveFocus = (dir: 1 | -1) => {
    const items = [inputRef.current, ...(listRef.current?.querySelectorAll<HTMLElement>("button[data-hit]") ?? [])].filter(
      (x): x is HTMLElement => x !== null && x !== undefined,
    );
    const at = items.indexOf(document.activeElement as HTMLElement);
    const next = items[Math.min(items.length - 1, Math.max(0, at + dir))];
    next?.focus();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose?.();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      moveFocus(1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      moveFocus(-1);
    }
  };

  const total = result?.hits.length ?? 0;

  return (
    <div className="flex h-full min-h-0 w-full flex-col" onKeyDown={onKeyDown}>
      <div className="flex gap-2 border-b border-border p-3">
        <label className="flex h-[30px] min-w-0 flex-1 items-center gap-1.5 rounded-md border border-border bg-card px-2.5 text-muted-foreground focus-within:ring-2 focus-within:ring-ring">
          <Search className="size-4 shrink-0" aria-hidden="true" />
          <Input
            ref={inputRef}
            type="search"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder={m.search}
            aria-label={m.search}
            className="h-auto min-w-0 flex-1 rounded-none border-0 bg-transparent p-0 text-sm text-foreground shadow-none focus-visible:ring-0 md:text-sm dark:bg-transparent [&::-webkit-search-cancel-button]:hidden"
          />
        </label>
        {onClose ? (
          <button
            type="button"
            aria-label={m.searchClose}
            title={m.searchClose}
            onClick={onClose}
            className="grid size-[30px] shrink-0 cursor-pointer place-items-center rounded-md border border-border bg-card text-muted-foreground transition-colors duration-150 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none"
          >
            <X className="size-4" aria-hidden="true" />
          </button>
        ) : null}
      </div>

      <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto py-1" aria-label={m.searchResults} aria-busy={busy}>
        {error ? (
          <p role="alert" className="px-4 py-3 text-sm text-destructive">
            {fmt(m.searchFailed, { message: error })}
          </p>
        ) : !input.trim() ? (
          <p className="px-4 py-6 text-center text-sm text-muted-foreground">{m.searchHint}</p>
        ) : result === null ? (
          <p className="px-4 py-6 text-center text-sm text-muted-foreground">{m.searching}</p>
        ) : total === 0 ? (
          <p className="px-4 py-6 text-center text-sm text-muted-foreground">{m.noMatches}</p>
        ) : (
          <>
            <p aria-live="polite" className="px-4 py-1.5 text-xs text-muted-foreground">
              {fmt(m.searchCount, { n: total, f: groups.length })}
            </p>
            {groups.map((g) => {
              const name = g.file ? (g.file.kind === "codex_db" ? m.codexDb : basename(g.file.path)) : basename(g.fileId);
              return (
                <div key={g.fileId} role="group" aria-label={name} className="pb-1">
                  <div className="flex min-w-0 items-center gap-2 px-4 py-1.5">
                    {g.file ? <ToolTag tool={g.file.tool} /> : null}
                    <span className="min-w-0 truncate font-mono text-[12.5px] font-medium text-foreground">{name}</span>
                    <span className="shrink-0 truncate text-xs text-muted-foreground">{g.projectName ?? m.global}</span>
                  </div>
                  {g.hits.map((h) => (
                    <button
                      key={`${h.line}:${h.preview}`}
                      type="button"
                      data-hit=""
                      onClick={() => onOpen(g.fileId, h.line)}
                      className="flex w-full cursor-pointer items-baseline gap-2 py-1 pr-4 pl-4 text-left text-sm outline-none transition-colors duration-150 hover:bg-muted focus-visible:bg-muted focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset motion-reduce:transition-none"
                    >
                      <span className="w-8 shrink-0 text-right font-mono text-xs text-muted-foreground tabular-nums">{h.line}</span>
                      <span className="min-w-0 flex-1 font-content break-words text-foreground">{highlight(h.preview, result.query)}</span>
                    </button>
                  ))}
                </div>
              );
            })}
          </>
        )}
      </div>
    </div>
  );
}
