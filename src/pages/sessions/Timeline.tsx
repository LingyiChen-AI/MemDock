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
  User,
  Wrench,
  X,
  type LucideIcon,
} from "lucide-react";
import type { SessionPreview, TimelineItem, Tool, ToolKind } from "@/api/types";
import { ToolLogo } from "@/components/brand/ToolLogo";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type { Messages } from "@/i18n";
import { fmt } from "@/pages/memory/paths";
import { formatIso } from "./sessionsUtils";

type S = Messages["sessions"];
type ToolItem = Extract<TimelineItem, { type: "tool" }>;
type TextItem = Exclude<TimelineItem, ToolItem>;
export type TimelineFilter = "all" | "chat" | "tools" | "shell" | "files" | "errors";
/** How a turn's tool calls are drawn: chips, one summary line, or a terminal-like trace. */
export type ToolStyle = "chips" | "summary" | "trace";

const STYLE_KEY = "memdock.sessions.toolStyle";

function loadStyle(): ToolStyle {
  try {
    const v = localStorage.getItem(STYLE_KEY);
    return v === "chips" || v === "trace" || v === "summary" ? v : "summary";
  } catch {
    return "summary";
  }
}

const KIND_ICON: Record<ToolKind, LucideIcon> = {
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

const KIND_TONE: Record<ToolKind, string> = {
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

const LONG_TEXT = 700;

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

/**
 * A user message, or one assistant turn: the tool calls it ran (in order)
 * followed by the text it wrote. Calls come first because the assistant
 * runs them before it answers.
 */
type Block = { kind: "user"; item: TextItem; key: number } | { kind: "turn"; tools: ToolItem[]; text: TextItem | null; key: number };

function toBlocks(items: TimelineItem[]): Block[] {
  const out: Block[] = [];
  let pending: ToolItem[] = [];
  let pendingKey = 0;
  const flush = () => {
    if (pending.length) out.push({ kind: "turn", tools: pending, text: null, key: pendingKey });
    pending = [];
  };
  items.forEach((it, i) => {
    if (it.type === "tool") {
      if (!pending.length) pendingKey = i;
      pending.push(it);
    } else if (it.type === "assistant") {
      out.push({ kind: "turn", tools: pending, text: it, key: pending.length ? pendingKey : i });
      pending = [];
    } else {
      flush();
      out.push({ kind: "user", item: it, key: i });
    }
  });
  flush();
  return out;
}

export function Timeline({ preview, tool }: { preview: SessionPreview; tool: Tool }) {
  const { t } = useI18n();
  const s = t.sessions;
  const [filter, setFilter] = React.useState<TimelineFilter>("all");
  const [style, setStyleState] = React.useState<ToolStyle>(loadStyle);
  const [detail, setDetail] = React.useState<{ tools: ToolItem[]; focus: number } | null>(null);
  const setStyle = (v: ToolStyle) => {
    setStyleState(v);
    try {
      localStorage.setItem(STYLE_KEY, v);
    } catch {
      // not remembered
    }
  };
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
  const open = (tools: ToolItem[], focus = 0) => setDetail({ tools, focus });

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-2 border-b border-border bg-background px-4 py-2.5">
        <div role="radiogroup" aria-label={s.timeline.filterLabel} className="flex flex-wrap gap-1.5">
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
        <div role="radiogroup" aria-label={s.timeline.styleLabel} className="ml-auto inline-flex h-7 rounded-md border border-border bg-card p-0.5">
          {(["chips", "summary", "trace"] as const).map((v) => (
            <button
              key={v}
              type="button"
              role="radio"
              aria-checked={style === v}
              onClick={() => setStyle(v)}
              className={cn(
                "cursor-pointer rounded-sm px-2.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring",
                style === v ? "seg-active" : "text-muted-foreground hover:text-foreground",
              )}
            >
              {s.timeline.styles[v]}
            </button>
          ))}
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">
        {blocks.length === 0 ? <p className="py-6 text-center text-sm text-muted-foreground">{s.timeline.empty}</p> : null}
        <ol className="flex flex-col gap-5">
          {blocks.map((b) =>
            b.kind === "user" ? (
              <UserRow key={b.key} item={b.item} s={s} />
            ) : (
              <TurnRow key={b.key} tools={b.tools} text={b.text} tool={tool} style={style} onOpen={open} s={s} />
            ),
          )}
        </ol>
        {preview.timeline_total > preview.timeline.length ? (
          <p className="mt-4 text-center text-xs text-muted-foreground">{fmt(s.previewMore, { shown: preview.timeline.length, total: preview.timeline_total })}</p>
        ) : null}
      </div>

      <ToolDialog detail={detail} onClose={() => setDetail(null)} s={s} />
    </div>
  );
}

function LongText({ text, s }: { text: string; s: S }) {
  const [open, setOpen] = React.useState(false);
  const long = text.length > LONG_TEXT;
  return (
    <>
      <p className={cn("whitespace-pre-wrap [overflow-wrap:anywhere]", long && !open && "line-clamp-[12]")}>{text}</p>
      {long ? (
        <button type="button" onClick={() => setOpen((v) => !v)} className="mt-1.5 cursor-pointer text-xs font-medium text-primary hover:underline">
          {open ? s.collapse : s.expand}
        </button>
      ) : null}
    </>
  );
}

function UserRow({ item, s }: { item: TextItem; s: S }) {
  return (
    <li className="flex flex-row-reverse gap-2.5">
      <span aria-hidden="true" className="grid size-7 shrink-0 place-items-center rounded-full bg-primary/15 text-primary">
        <User className="size-3.5" />
      </span>
      <div className="flex max-w-[82%] min-w-0 flex-col items-end gap-1">
        <div className="flex flex-row-reverse items-center gap-2 px-1 text-[11px] text-muted-foreground">
          <span className="font-medium text-foreground/80">{s.roles.user}</span>
          <span className="tabular-nums">{formatIso(item.ts)}</span>
        </div>
        <div className="max-w-full min-w-0 overflow-hidden rounded-2xl rounded-tr-md border border-primary/25 bg-primary/10 px-3.5 py-2.5 text-sm leading-relaxed">
          <LongText text={item.text} s={s} />
        </div>
      </div>
    </li>
  );
}

function TurnRow({
  tools,
  text,
  tool,
  style,
  onOpen,
  s,
}: {
  tools: ToolItem[];
  text: TextItem | null;
  tool: Tool;
  style: ToolStyle;
  onOpen: (tools: ToolItem[], focus?: number) => void;
  s: S;
}) {
  const ts = text?.ts ?? tools[0]?.ts ?? "";
  return (
    <li className="flex gap-2.5">
      <span aria-hidden="true" className="grid size-7 shrink-0 place-items-center rounded-full border border-border bg-card">
        <ToolLogo tool={tool} className="size-4" aria-hidden="true" />
      </span>
      <div className="flex max-w-[86%] min-w-0 flex-1 flex-col gap-1.5">
        <div className="flex items-center gap-2 px-1 text-[11px] text-muted-foreground">
          <span className="font-medium text-foreground/80">{s.tools[tool]}</span>
          <span className="tabular-nums">{formatIso(ts)}</span>
        </div>
        {tools.length > 0 ? <ToolCalls tools={tools} style={style} onOpen={onOpen} s={s} /> : null}
        {text ? (
          <div className="max-w-full min-w-0 self-start overflow-hidden rounded-2xl rounded-tl-md border border-border bg-card px-3.5 py-2.5 text-sm leading-relaxed">
            <LongText text={text.text} s={s} />
          </div>
        ) : null}
      </div>
    </li>
  );
}

function StatusMark({ it, s, dot }: { it: ToolItem; s: S; dot?: boolean }) {
  if (it.output === null) return <span className="size-1.5 shrink-0 rounded-full bg-muted-foreground/40" title={s.timeline.noResult} />;
  if (dot) return <span className={cn("size-1.5 shrink-0 rounded-full", it.is_error ? "bg-destructive" : "bg-emerald-500")} aria-label={it.is_error ? s.timeline.failed : s.timeline.ok} />;
  return it.is_error ? (
    <X className="size-3 shrink-0 text-destructive" aria-label={s.timeline.failed} />
  ) : (
    <Check className="size-3 shrink-0 text-emerald-600 dark:text-emerald-400" aria-label={s.timeline.ok} />
  );
}

function ToolCalls({ tools, style, onOpen, s }: { tools: ToolItem[]; style: ToolStyle; onOpen: (tools: ToolItem[], focus?: number) => void; s: S }) {
  if (style === "summary") {
    const names = new Map<string, number>();
    for (const t of tools) names.set(t.name, (names.get(t.name) ?? 0) + 1);
    const label = [...names].map(([n, k]) => (k > 1 ? `${n} ×${k}` : n)).join(" · ");
    const errors = tools.filter((t) => t.is_error).length;
    return (
      <button
        type="button"
        onClick={() => onOpen(tools)}
        className="group inline-flex max-w-full min-w-0 cursor-pointer items-center gap-2 self-start rounded-md px-1 py-0.5 text-left text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
      >
        <span className="flex shrink-0 gap-0.5">
          {tools.slice(0, 12).map((t, i) => (
            <StatusMark key={i} it={t} s={s} dot />
          ))}
        </span>
        <span className="min-w-0 truncate">
          {fmt(s.timeline.usedTools, { n: tools.length })} · {label}
          {errors ? <span className="text-destructive"> · {fmt(s.timeline.groupErrors, { n: errors })}</span> : null}
        </span>
        <span className="shrink-0 font-medium text-primary group-hover:underline">{s.timeline.view}</span>
      </button>
    );
  }
  if (style === "chips") {
    return (
      <div className="flex flex-wrap gap-1.5">
        {tools.map((t, i) => {
          const Icon = KIND_ICON[t.kind];
          return (
            <button
              key={i}
              type="button"
              onClick={() => onOpen(tools, i)}
              title={t.summary}
              className="inline-flex h-6 max-w-full min-w-0 cursor-pointer items-center gap-1.5 rounded-full border border-border bg-muted/50 px-2.5 text-xs outline-none hover:border-primary/50 focus-visible:ring-2 focus-visible:ring-ring"
            >
              <StatusMark it={t} s={s} />
              <Icon className={cn("size-3 shrink-0", KIND_TONE[t.kind])} aria-hidden="true" />
              <span className="shrink-0 font-medium">{t.name}</span>
              <span className="max-w-64 min-w-0 truncate font-mono text-[11px] text-muted-foreground">{t.summary}</span>
            </button>
          );
        })}
      </div>
    );
  }
  return (
    <div className="ml-0.5 flex min-w-0 flex-col border-l-2 border-border pl-3">
      {tools.map((t, i) => (
        <button
          key={i}
          type="button"
          onClick={() => onOpen(tools, i)}
          title={t.summary}
          className="group flex min-w-0 cursor-pointer items-center gap-2 py-0.5 text-left text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <StatusMark it={t} s={s} dot />
          <span className="shrink-0 font-medium group-hover:text-primary">{t.name}</span>
          <span className="min-w-0 truncate font-mono text-[11px] text-muted-foreground">{t.summary}</span>
        </button>
      ))}
    </div>
  );
}

function ToolDialog({ detail, onClose, s }: { detail: { tools: ToolItem[]; focus: number } | null; onClose: () => void; s: S }) {
  const listRef = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    if (!detail) return;
    const id = window.setTimeout(() => {
      listRef.current?.querySelector(`[data-call="${detail.focus}"]`)?.scrollIntoView({ block: "start" });
    }, 50);
    return () => window.clearTimeout(id);
  }, [detail]);
  return (
    <Dialog open={!!detail} onOpenChange={(o) => (!o ? onClose() : undefined)}>
      <DialogContent className="flex max-h-[85vh] flex-col gap-3 sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>{fmt(s.timeline.dialogTitle, { n: detail?.tools.length ?? 0 })}</DialogTitle>
          <DialogDescription>{s.timeline.dialogDesc}</DialogDescription>
        </DialogHeader>
        <div ref={listRef} className="-mx-1 flex min-h-0 flex-col gap-4 overflow-y-auto px-1">
          {(detail?.tools ?? []).map((it, i) => {
            const Icon = KIND_ICON[it.kind];
            return (
              <section key={i} data-call={i} className={cn("flex flex-col gap-2 rounded-lg border p-3", i === detail?.focus ? "border-primary/50" : "border-border")}>
                <h3 className="flex min-w-0 flex-wrap items-center gap-2 text-sm">
                  <Icon className={cn("size-4 shrink-0", KIND_TONE[it.kind])} aria-hidden="true" />
                  <span className="font-mono font-semibold">{it.name}</span>
                  <StatusMark it={it} s={s} />
                  <span className="text-xs text-muted-foreground tabular-nums">{formatIso(it.ts)}</span>
                  <span
                    className={cn(
                      "ml-auto rounded px-1.5 py-0.5 text-[10px]",
                      it.mapped ? "bg-emerald-500/12 text-emerald-700 dark:text-emerald-300" : "bg-muted text-muted-foreground",
                    )}
                    title={it.mapped ? s.timeline.mappedHint : s.timeline.textHint}
                  >
                    {it.mapped ? s.timeline.mapped : s.timeline.asText}
                  </span>
                </h3>
                <IoBlock label={s.timeline.input} text={it.input} />
                {it.output !== null ? <IoBlock label={it.is_error ? s.timeline.error : s.timeline.output} text={it.output || s.timeline.emptyOutput} error={it.is_error} /> : null}
              </section>
            );
          })}
        </div>
      </DialogContent>
    </Dialog>
  );
}

function IoBlock({ label, text, error }: { label: string; text: string; error?: boolean }) {
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <span className={cn("text-[11px] font-medium", error ? "text-destructive" : "text-muted-foreground")}>{label}</span>
      <pre
        className={cn(
          "max-h-60 overflow-auto rounded-md border px-3 py-2 font-mono text-[12px] leading-relaxed whitespace-pre-wrap [overflow-wrap:anywhere]",
          error ? "border-destructive/30 bg-destructive/5" : "border-border bg-muted/40",
        )}
      >
        {text}
      </pre>
    </div>
  );
}
