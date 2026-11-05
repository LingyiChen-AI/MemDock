import * as React from "react";
import { diffLines, diffWordsWithSpace } from "diff";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";

export type DiffLayout = "split" | "unified";

const LAYOUT_KEY = "memdock.diffLayout";
/** Design system 7: below this container width the diff is always shown as a single column. */
const NARROW_PX = 760;

export interface DiffViewProps {
  oldText: string;
  newText: string;
  /** Header of the left column ("current"). */
  oldLabel?: string;
  /** Header of the right column ("proposed"). */
  newLabel?: string;
  /** Unchanged lines kept around every change. */
  context?: number;
  className?: string;
}

interface Seg {
  text: string;
  changed: boolean;
}
interface Cell {
  no: number;
  segs: Seg[];
}
type Row =
  | { kind: "ctx"; oldNo: number; newNo: number; text: string }
  | { kind: "chg"; left?: Cell; right?: Cell }
  | { kind: "fold"; count: number };

function loadLayout(): DiffLayout {
  try {
    return localStorage.getItem(LAYOUT_KEY) === "unified" ? "unified" : "split";
  } catch {
    return "split";
  }
}

function linesOf(value: string): string[] {
  if (value === "") return [];
  return (value.endsWith("\n") ? value.slice(0, -1) : value).split("\n");
}

/** Word-level segments of one replaced line, seen from the old and the new side. */
function wordSegs(oldLine: string, newLine: string): { left: Seg[]; right: Seg[] } {
  const left: Seg[] = [];
  const right: Seg[] = [];
  for (const p of diffWordsWithSpace(oldLine, newLine)) {
    if (p.added) right.push({ text: p.value, changed: true });
    else if (p.removed) left.push({ text: p.value, changed: true });
    else {
      left.push({ text: p.value, changed: false });
      right.push({ text: p.value, changed: false });
    }
  }
  return { left, right };
}

/** Bounds that keep huge or unrelated inputs cheap (DiffView is reused for arbitrary text). */
const MAX_EDIT_LENGTH = 3000;
const DIFF_TIMEOUT_MS = 250;
const MAX_WORD_PAIR_CHARS = 2000;
const MAX_WORD_PAIRS = 200;
const MIN_SIMILARITY = 0.3;

/** Common prefix + suffix share, or shared whitespace-separated words, relative to the longer line. */
function similarity(a: string, b: string): number {
  const max = Math.max(a.length, b.length);
  if (max === 0) return 1;
  let pre = 0;
  while (pre < a.length && pre < b.length && a[pre] === b[pre]) pre++;
  let suf = 0;
  while (suf < a.length - pre && suf < b.length - pre && a[a.length - 1 - suf] === b[b.length - 1 - suf]) suf++;
  const affix = (pre + suf) / max;
  const wa = a.split(/\s+/).filter(Boolean);
  const wb = new Set(b.split(/\s+/).filter(Boolean));
  const common = wa.filter((w) => wb.has(w)).length;
  const words = common / Math.max(wa.length, wb.size, 1);
  return Math.max(affix, words);
}

/** Aligned rows (context, replaced/added/removed lines) before folding. */
function buildRows(oldText: string, newText: string): Row[] {
  // With maxEditLength jsdiff returns undefined when the texts differ too much: show one replaced block.
  const parts: { value: string; added?: boolean; removed?: boolean }[] = diffLines(oldText, newText, {
    ignoreNewlineAtEof: true,
    maxEditLength: MAX_EDIT_LENGTH,
    timeout: DIFF_TIMEOUT_MS,
  }) ?? [
    { value: oldText, removed: true },
    { value: newText, added: true },
  ];
  let wordPairs = 0;
  const rows: Row[] = [];
  let oldNo = 1;
  let newNo = 1;
  let i = 0;
  while (i < parts.length) {
    const p = parts[i];
    if (!p.added && !p.removed) {
      for (const text of linesOf(p.value)) rows.push({ kind: "ctx", oldNo: oldNo++, newNo: newNo++, text });
      i++;
      continue;
    }
    const removed: string[] = [];
    const added: string[] = [];
    while (i < parts.length && (parts[i].added || parts[i].removed)) {
      (parts[i].added ? added : removed).push(...linesOf(parts[i].value));
      i++;
    }
    const n = Math.max(removed.length, added.length);
    for (let k = 0; k < n; k++) {
      const l = removed[k];
      const r = added[k];
      if (l !== undefined && r !== undefined) {
        const worthIt =
          wordPairs < MAX_WORD_PAIRS && l.length + r.length <= MAX_WORD_PAIR_CHARS && similarity(l, r) >= MIN_SIMILARITY;
        if (worthIt) {
          wordPairs++;
          const w = wordSegs(l, r);
          rows.push({ kind: "chg", left: { no: oldNo++, segs: w.left }, right: { no: newNo++, segs: w.right } });
        } else {
          rows.push({
            kind: "chg",
            left: { no: oldNo++, segs: [{ text: l, changed: false }] },
            right: { no: newNo++, segs: [{ text: r, changed: false }] },
          });
        }
      } else if (l !== undefined) {
        rows.push({ kind: "chg", left: { no: oldNo++, segs: [{ text: l, changed: false }] } });
      } else {
        rows.push({ kind: "chg", right: { no: newNo++, segs: [{ text: r, changed: false }] } });
      }
    }
  }
  return rows;
}

/** Keeps `context` unchanged lines around changes and folds the rest into one marker row. */
function foldRows(rows: Row[], context: number): Row[] {
  const keep = rows.map((r) => r.kind === "chg");
  rows.forEach((r, i) => {
    if (r.kind !== "chg") return;
    for (let d = 1; d <= context; d++) {
      if (i - d >= 0) keep[i - d] = true;
      if (i + d < rows.length) keep[i + d] = true;
    }
  });
  const out: Row[] = [];
  let hidden = 0;
  rows.forEach((r, i) => {
    if (keep[i]) {
      if (hidden) out.push({ kind: "fold", count: hidden });
      hidden = 0;
      out.push(r);
    } else hidden++;
  });
  if (hidden) out.push({ kind: "fold", count: hidden });
  return out;
}

type UnifiedRow =
  | Exclude<Row, { kind: "chg" }>
  | { kind: "del" | "add"; cell: Cell };

/** Unified order: every removed line of a change block first, then its added lines. */
function unify(rows: Row[]): UnifiedRow[] {
  const out: UnifiedRow[] = [];
  let dels: UnifiedRow[] = [];
  let adds: UnifiedRow[] = [];
  const flush = () => {
    out.push(...dels, ...adds);
    dels = [];
    adds = [];
  };
  for (const r of rows) {
    if (r.kind === "chg") {
      if (r.left) dels.push({ kind: "del", cell: r.left });
      if (r.right) adds.push({ kind: "add", cell: r.right });
    } else {
      flush();
      out.push(r);
    }
  }
  flush();
  return out;
}

const HATCH =
  "repeating-linear-gradient(135deg, transparent 0 6px, color-mix(in srgb, var(--border) 40%, transparent) 6px 7px)";

function Segs({ segs, side }: { segs: Seg[]; side: "del" | "add" }) {
  return (
    <>
      {segs.map((s, i) =>
        s.changed ? (
          <mark
            key={i}
            data-change={side}
            className={cn("rounded-xs px-px text-inherit", side === "del" ? "bg-destructive/45" : "bg-primary/45")}
          >
            {s.text}
          </mark>
        ) : (
          <React.Fragment key={i}>{s.text}</React.Fragment>
        ),
      )}
    </>
  );
}

const NO = "w-9 shrink-0 pr-2 text-right text-muted-foreground select-none";
const TEXT = "min-w-0 flex-1 pr-2.5 break-all whitespace-pre-wrap";

function SplitCell({ cell, side, ctxText, ctxNo }: { cell?: Cell; side: "del" | "add"; ctxText?: string; ctxNo?: number }) {
  if (ctxText !== undefined) {
    return (
      <div className="flex min-w-0">
        <span className={NO}>{ctxNo}</span>
        <span className={cn(TEXT, "text-muted-foreground")}>{ctxText}</span>
      </div>
    );
  }
  if (!cell) {
    return <div data-empty="true" aria-hidden="true" className="flex min-w-0" style={{ backgroundImage: HATCH }} />;
  }
  return (
    <div className={cn("flex min-w-0", side === "del" ? "bg-destructive/12" : "bg-primary/12")}>
      <span className={NO}>{cell.no}</span>
      <span className={TEXT}>
        <Segs segs={cell.segs} side={side} />
      </span>
    </div>
  );
}

/**
 * Presentational text diff (design system 7): side by side by default, one column when the
 * container is narrower than 760px or the user picked "unified"; line tint plus word-level marks.
 */
export function DiffView({ oldText, newText, oldLabel, newLabel, context = 1, className }: DiffViewProps) {
  const { t } = useI18n();
  const d = t.diff;
  const rootRef = React.useRef<HTMLDivElement>(null);
  const [pref, setPref] = React.useState<DiffLayout>(loadLayout);
  const [width, setWidth] = React.useState<number | null>(null);

  React.useLayoutEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const w = el.clientWidth;
    if (w > 0) setWidth(w);
    const ro = new ResizeObserver((entries) => {
      const last = entries[entries.length - 1];
      if (last) setWidth(last.contentRect.width);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // 0 / unknown means "not laid out yet" (also jsdom): do not collapse then.
  const narrow = width !== null && width > 0 && width < NARROW_PX;
  const layout: DiffLayout = narrow ? "unified" : pref;

  const choose = (next: DiffLayout) => {
    setPref(next);
    try {
      localStorage.setItem(LAYOUT_KEY, next);
    } catch {
      // storage unavailable: the choice just is not remembered
    }
  };

  const all = React.useMemo(() => buildRows(oldText, newText), [oldText, newText]);
  const identical = all.every((r) => r.kind === "ctx");
  const rows = React.useMemo(() => foldRows(all, Math.max(0, context)), [all, context]);

  const foldLabel = (n: number) => d.folded.replace("{n}", String(n));
  const hasLabels = oldLabel !== undefined || newLabel !== undefined;

  return (
    <div ref={rootRef} data-diff-layout={layout} className={cn("min-w-0", className)}>
      <div className="mb-2 flex min-h-6 items-center gap-2 text-xs text-muted-foreground">
        {layout === "unified" && hasLabels ? (
          <span className="truncate">
            {oldLabel}
            {oldLabel && newLabel ? " → " : ""}
            {newLabel}
          </span>
        ) : null}
        <span className="flex-1" />
        {narrow ? null : (
          <div role="radiogroup" aria-label={d.layoutLabel} className="inline-flex shrink-0 overflow-hidden rounded-md border border-border">
            {(["split", "unified"] as const).map((k) => (
              <button
                key={k}
                type="button"
                role="radio"
                aria-checked={pref === k}
                onClick={() => choose(k)}
                className={cn(
                  "cursor-pointer px-2.5 text-xs leading-6 outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset motion-reduce:transition-none",
                  pref === k ? "bg-muted font-medium text-foreground" : "text-muted-foreground hover:text-foreground",
                )}
              >
                {k === "split" ? d.split : d.unified}
              </button>
            ))}
          </div>
        )}
      </div>

      {identical ? (
        <p className="rounded-md border border-border bg-muted px-3 py-2 text-sm text-muted-foreground">{d.identical}</p>
      ) : (
        <div className="overflow-hidden rounded-md border border-border bg-card font-mono text-[12.5px] leading-[22px]">
          {layout === "split" ? (
            <>
              {hasLabels ? (
                <div className="grid grid-cols-2 border-b border-border bg-muted text-xs leading-5 font-semibold text-muted-foreground">
                  <div className="px-2.5 py-0.5">{oldLabel}</div>
                  <div className="border-l border-border px-2.5 py-0.5">{newLabel}</div>
                </div>
              ) : null}
              {rows.map((r, i) =>
                r.kind === "fold" ? (
                  <FoldRow key={i} label={foldLabel(r.count)} />
                ) : (
                  <div key={i} className="grid min-h-[22px] grid-cols-2 [&>*+*]:border-l [&>*+*]:border-border">
                    {r.kind === "ctx" ? (
                      <>
                        <SplitCell side="del" ctxText={r.text} ctxNo={r.oldNo} />
                        <SplitCell side="add" ctxText={r.text} ctxNo={r.newNo} />
                      </>
                    ) : (
                      <>
                        <SplitCell side="del" cell={r.left} />
                        <SplitCell side="add" cell={r.right} />
                      </>
                    )}
                  </div>
                ),
              )}
            </>
          ) : (
            unify(rows).map((r, i) => {
              if (r.kind === "fold") return <FoldRow key={i} label={foldLabel(r.count)} />;
              if (r.kind === "ctx") {
                return (
                  <div key={i} className="flex min-h-[22px]">
                    <span className={NO}>{r.newNo}</span>
                    <span className="w-4 shrink-0" />
                    <span className={cn(TEXT, "text-muted-foreground")}>{r.text}</span>
                  </div>
                );
              }
              const del = r.kind === "del";
              return (
                <div key={i} className={cn("flex min-h-[22px]", del ? "bg-destructive/12" : "bg-primary/12")}>
                  <span className={NO}>{r.cell.no}</span>
                  <span className="w-4 shrink-0 text-muted-foreground">{del ? "-" : "+"}</span>
                  <span className={TEXT}>
                    <Segs segs={r.cell.segs} side={del ? "del" : "add"} />
                  </span>
                </div>
              );
            })
          )}
        </div>
      )}
    </div>
  );
}

function FoldRow({ label }: { label: string }) {
  return (
    <div className="border-y border-border bg-muted px-2.5 font-sans text-xs leading-5 text-muted-foreground first:border-t-0 last:border-b-0">
      {label}
    </div>
  );
}
