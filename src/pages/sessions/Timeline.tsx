import * as React from "react";
import {
  Bot,
  Check,
  FilePen,
  FilePlus2,
  FileText,
  Globe,
  ListTodo,
  Search,
  SquareTerminal,
  Wrench,
  X,
  type LucideIcon,
} from "lucide-react";
import type { SessionPreview, TimelineItem, Tool, ToolKind } from "@/api/types";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type { Messages } from "@/i18n";
import { fmt } from "@/pages/memory/paths";
import { ChatRow, ChatText } from "@/components/chat/ChatMessage";
import { formatIso } from "./sessionsUtils";

type S = Messages["sessions"];
type ToolItem = Extract<TimelineItem, { type: "tool" }>;
type TextItem = Exclude<TimelineItem, ToolItem>;
export type TimelineFilter = "all" | "chat" | "tools" | "shell" | "files" | "errors";

export const KIND_ICON: Record<ToolKind, LucideIcon> = {
  shell: SquareTerminal,
  read: FileText,
  edit: FilePen,
  write: FilePlus2,
  search: Search,
  web: Globe,
  agent: Bot,
  todo: ListTodo,
  other: Wrench,
};

export const KIND_TONE: Record<ToolKind, string> = {
  shell: "text-emerald-600 dark:text-emerald-400",
  read: "text-sky-600 dark:text-sky-400",
  edit: "text-amber-600 dark:text-amber-400",
  write: "text-violet-600 dark:text-violet-400",
  search: "text-cyan-600 dark:text-cyan-400",
  web: "text-blue-600 dark:text-blue-400",
  agent: "text-fuchsia-600 dark:text-fuchsia-400",
  todo: "text-teal-600 dark:text-teal-400",
  other: "text-muted-foreground",
};

/** Chips shown in one run before the rest fold into a "+N" chip. */
export const CHIP_FOLD = 8;

function matches(it: TimelineItem, f: TimelineFilter): boolean {
  switch (f) {
    case "all":
      return true;
    case "chat":
      return it.type !== "tool";
    case "tools":
      return it.type === "tool";
    case "shell":
      return it.type === "tool" && it.kind === "shell";
    case "files":
      return it.type === "tool" && (it.kind === "read" || it.kind === "edit" || it.kind === "write");
    case "errors":
      return it.type === "tool" && it.is_error;
  }
}

/** Inside one assistant card, in the order things happened. */
type Segment = { kind: "text"; item: TextItem } | { kind: "tools"; items: ToolItem[] };
type TurnBlock = { kind: "turn"; segments: Segment[]; ts: string; key: number };
/** A user message, or one assistant turn (everything until the next user message) as one card. */
type Block = { kind: "user"; item: TextItem; key: number } | TurnBlock;

function toBlocks(items: TimelineItem[]): Block[] {
  const out: Block[] = [];
  let turn: TurnBlock | null = null;
  items.forEach((it, i) => {
    if (it.type === "user") {
      turn = null;
      out.push({ kind: "user", item: it, key: i });
      return;
    }
    if (!turn) {
      turn = { kind: "turn", segments: [], ts: it.ts, key: i };
      out.push(turn);
    }
    const segs: Segment[] = turn.segments;
    const last = segs[segs.length - 1];
    if (it.type === "tool") {
      if (last?.kind === "tools") last.items.push(it);
      else segs.push({ kind: "tools", items: [it] });
    } else {
      segs.push({ kind: "text", item: it });
    }
  });
  return out;
}

export function Timeline({
  preview,
  tool,
  bare = false,
  startAtBottom = false,
}: {
  preview: SessionPreview;
  tool: Tool;
  /** Only the messages: no filter bar and no scroll container (embedded above a live chat). */
  bare?: boolean;
  /** Open scrolled to the newest message. */
  startAtBottom?: boolean;
}) {
  const { t } = useI18n();
  const scroller = React.useRef<HTMLDivElement>(null);
  const s = t.sessions;
  const [filter, setFilter] = React.useState<TimelineFilter>("all");
  const [detail, setDetail] = React.useState<ToolItem | null>(null);
  const c = preview.counts;
  const filters: { key: TimelineFilter; label: string; n: number }[] = [
    { key: "all", label: s.timeline.all, n: preview.timeline.length },
    { key: "chat", label: s.timeline.chat, n: c.user + c.assistant },
    { key: "tools", label: s.timeline.tools, n: c.tools },
    { key: "shell", label: s.timeline.shell, n: c.shell },
    { key: "files", label: s.timeline.files, n: c.files },
    { key: "errors", label: s.timeline.errors, n: c.errors },
  ];
  const blocks = toBlocks(preview.timeline.filter((it) => matches(it, filter)));

  React.useLayoutEffect(() => {
    const el = scroller.current;
    if (el && startAtBottom) el.scrollTop = el.scrollHeight;
  }, [preview, startAtBottom]);

  const list = (
    <>
      {preview.timeline_total > preview.timeline.length ? (
        <p className="mb-4 text-center text-xs text-muted-foreground">{fmt(s.previewMore, { shown: preview.timeline.length, total: preview.timeline_total })}</p>
      ) : null}
      {blocks.length === 0 ? <p className="py-6 text-center text-sm text-muted-foreground">{s.timeline.empty}</p> : null}
      <ol className="flex flex-col gap-5">
        {blocks.map((b) =>
          b.kind === "user" ? (
            <UserRow key={b.key} item={b.item} s={s} />
          ) : (
            <TurnCard key={b.key} segments={b.segments} ts={b.ts} tool={tool} onOpen={setDetail} s={s} />
          ),
        )}
      </ol>
      <ToolDialog it={detail} onClose={() => setDetail(null)} s={s} />
    </>
  );

  if (bare) return list;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div role="radiogroup" aria-label={s.timeline.filterLabel} className="flex shrink-0 flex-wrap gap-1.5 border-b border-border bg-background px-4 py-2.5">
        {filters.map((f) => (
          <button
            key={f.key}
            type="button"
            role="radio"
            aria-checked={filter === f.key}
            disabled={f.n === 0 && f.key !== "all"}
            onClick={() => setFilter(f.key)}
            className={cn(
              "inline-flex h-7 cursor-pointer items-center gap-1.5 rounded-full border px-3 text-xs transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-40",
              filter === f.key ? "seg-active border-transparent" : "border-border text-muted-foreground hover:text-foreground",
              f.key === "errors" && f.n > 0 && filter !== f.key && "text-destructive",
            )}
          >
            {f.label}
            <span className={cn("tabular-nums", filter === f.key ? "opacity-90" : "opacity-70")}>{f.n}</span>
          </button>
        ))}
      </div>

      <div ref={scroller} className="min-h-0 flex-1 overflow-y-auto px-4 py-4">
        {list}
      </div>
    </div>
  );
}

function LongText({ text, s }: { text: string; s: S }) {
  return <ChatText text={text} expandLabel={s.expand} collapseLabel={s.collapse} />;
}

function UserRow({ item, s }: { item: TextItem; s: S }) {
  return (
    <ChatRow role="user" name={s.roles.user} time={formatIso(item.ts)}>
      {item.text ? <LongText text={item.text} s={s} /> : null}
      {item.type === "user" && item.images?.length ? <Thumbs images={item.images} s={s} /> : null}
    </ChatRow>
  );
}

/** Pasted screenshots as thumbnails; click one to see it full size. */
function Thumbs({ images, s }: { images: string[]; s: S }) {
  const [open, setOpen] = React.useState<number | null>(null);
  return (
    <>
      <div className="flex flex-wrap justify-end gap-1.5">
        {images.map((src, i) => (
          <button
            key={i}
            type="button"
            onClick={() => setOpen(i)}
            className="cursor-zoom-in overflow-hidden rounded-lg border border-border bg-background outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={fmt(s.timeline.imageOpen, { n: i + 1 })}
          >
            <img src={src} alt="" loading="lazy" className="block max-h-40 max-w-[240px] object-contain" />
          </button>
        ))}
      </div>
      <Dialog open={open !== null} onOpenChange={(o) => (!o ? setOpen(null) : undefined)}>
        <DialogContent className="flex max-h-[92vh] flex-col gap-2 sm:max-w-[min(1200px,94vw)]">
          <DialogHeader>
            <DialogTitle>{fmt(s.timeline.imageTitle, { n: (open ?? 0) + 1, total: images.length })}</DialogTitle>
            <DialogDescription className="sr-only">{s.timeline.imageTitle}</DialogDescription>
          </DialogHeader>
          {open !== null ? (
            <div className="min-h-0 overflow-auto rounded-md bg-muted/40">
              <img src={images[open]} alt="" className="mx-auto block max-w-full" />
            </div>
          ) : null}
          {images.length > 1 ? (
            <div className="flex justify-center gap-1.5">
              {images.map((src, i) => (
                <button
                  key={i}
                  type="button"
                  onClick={() => setOpen(i)}
                  className={cn("overflow-hidden rounded border-2 outline-none", i === open ? "border-primary" : "border-transparent opacity-70 hover:opacity-100")}
                  aria-label={fmt(s.timeline.imageOpen, { n: i + 1 })}
                >
                  <img src={src} alt="" className="block h-12 w-16 object-cover" />
                </button>
              ))}
            </div>
          ) : null}
        </DialogContent>
      </Dialog>
    </>
  );
}

function TurnCard({ segments, ts, tool, onOpen, s }: { segments: Segment[]; ts: string; tool: Tool; onOpen: (it: ToolItem) => void; s: S }) {
  return (
    <ChatRow role="assistant" tool={tool} name={s.tools[tool]} time={formatIso(ts)}>
      {segments.map((seg, i) =>
        seg.kind === "text" ? <LongText key={i} text={seg.item.text} s={s} /> : <ChipRun key={i} items={seg.items} onOpen={onOpen} s={s} />,
      )}
    </ChatRow>
  );
}

function StatusMark({ it, s }: { it: ToolItem; s: S }) {
  if (it.output === null) return <span className="size-1.5 shrink-0 rounded-full bg-muted-foreground/40" title={s.timeline.noResult} />;
  return it.is_error ? (
    <X className="size-3 shrink-0 text-destructive" aria-label={s.timeline.failed} />
  ) : (
    <Check className="size-3 shrink-0 text-emerald-600 dark:text-emerald-400" aria-label={s.timeline.ok} />
  );
}

/** Consecutive calls as chips that wrap inside the card; long runs fold behind "+N". */
function ChipRun({ items, onOpen, s }: { items: ToolItem[]; onOpen: (it: ToolItem) => void; s: S }) {
  const [all, setAll] = React.useState(false);
  const shown = all || items.length <= CHIP_FOLD ? items : items.slice(0, CHIP_FOLD);
  const rest = items.length - shown.length;
  return (
    <div className="flex max-w-full min-w-0 flex-wrap gap-1.5">
      {shown.map((it, i) => {
        const Icon = KIND_ICON[it.kind];
        return (
          <button
            key={i}
            type="button"
            onClick={() => onOpen(it)}
            title={`${it.name}  ${it.summary}`}
            className={cn(
              "inline-flex h-6 max-w-full min-w-0 cursor-pointer items-center gap-1.5 rounded-full border bg-background px-2.5 text-xs outline-none transition-colors hover:border-primary/50 focus-visible:ring-2 focus-visible:ring-ring",
              it.is_error ? "border-destructive/40" : "border-border",
            )}
          >
            <StatusMark it={it} s={s} />
            <Icon className={cn("size-3 shrink-0", KIND_TONE[it.kind])} aria-hidden="true" />
            <span className="shrink-0 font-medium">{it.name}</span>
            {it.summary ? <span className="max-w-60 min-w-0 truncate font-mono text-[11px] text-muted-foreground">{it.summary}</span> : null}
          </button>
        );
      })}
      {rest > 0 ? (
        <button
          type="button"
          onClick={() => setAll(true)}
          className="inline-flex h-6 shrink-0 cursor-pointer items-center rounded-full border border-dashed border-border px-2.5 text-xs text-muted-foreground outline-none hover:border-primary/50 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
        >
          {fmt(s.timeline.more, { n: rest })}
        </button>
      ) : null}
      {all && items.length > CHIP_FOLD ? (
        <button
          type="button"
          onClick={() => setAll(false)}
          className="inline-flex h-6 shrink-0 cursor-pointer items-center rounded-full px-2 text-xs text-primary outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
        >
          {s.collapse}
        </button>
      ) : null}
    </div>
  );
}

function ToolDialog({ it, onClose, s }: { it: ToolItem | null; onClose: () => void; s: S }) {
  const Icon = it ? KIND_ICON[it.kind] : Wrench;
  return (
    <Dialog open={!!it} onOpenChange={(o) => (!o ? onClose() : undefined)}>
      <DialogContent className="flex max-h-[85vh] flex-col gap-3 sm:max-w-3xl">
        {it ? (
          <>
            <DialogHeader>
              <DialogTitle className="flex min-w-0 flex-wrap items-center gap-2">
                <Icon className={cn("size-4 shrink-0", KIND_TONE[it.kind])} aria-hidden="true" />
                <span className="font-mono">{it.name}</span>
                <StatusMark it={it} s={s} />
              </DialogTitle>
              <DialogDescription className="flex flex-wrap items-center gap-2">
                <span className="tabular-nums">{formatIso(it.ts)}</span>
                <span
                  className={cn("rounded px-1.5 py-0.5 text-[11px]", it.mapped ? "bg-emerald-500/12 text-emerald-700 dark:text-emerald-300" : "bg-muted text-muted-foreground")}
                  title={it.mapped ? s.timeline.mappedHint : s.timeline.textHint}
                >
                  {it.mapped ? s.timeline.mapped : s.timeline.asText}
                </span>
              </DialogDescription>
            </DialogHeader>
            <div className="-mx-1 flex min-h-0 flex-col gap-3 overflow-y-auto px-1">
              <IoBlock label={s.timeline.input} text={it.input} />
              {it.output !== null ? <IoBlock label={it.is_error ? s.timeline.error : s.timeline.output} text={it.output || s.timeline.emptyOutput} error={it.is_error} /> : null}
            </div>
          </>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

export function IoBlock({ label, text, error }: { label: string; text: string; error?: boolean }) {
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <span className={cn("text-[11px] font-medium", error ? "text-destructive" : "text-muted-foreground")}>{label}</span>
      <pre
        className={cn(
          "max-h-72 overflow-auto rounded-md border px-3 py-2 font-mono text-[12px] leading-relaxed whitespace-pre-wrap [overflow-wrap:anywhere]",
          error ? "border-destructive/30 bg-destructive/5" : "border-border bg-muted/40",
        )}
      >
        {text}
      </pre>
    </div>
  );
}
