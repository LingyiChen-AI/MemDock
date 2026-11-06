import * as React from "react";
import {
  Bot,
  Check,
  ChevronRight,
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
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type { Messages } from "@/i18n";
import { fmt } from "@/pages/memory/paths";
import { formatIso } from "./sessionsUtils";

type S = Messages["sessions"];
type ToolItem = Extract<TimelineItem, { type: "tool" }>;
export type TimelineFilter = "all" | "chat" | "tools" | "shell" | "files" | "errors";

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

/** Accent per tool kind (icon tint + left rule). */
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

/** Consecutive tool calls are shown together as one group. */
type Block = { kind: "msg"; item: Exclude<TimelineItem, ToolItem>; key: number } | { kind: "tools"; items: ToolItem[]; key: number };

function toBlocks(items: TimelineItem[]): Block[] {
  const out: Block[] = [];
  items.forEach((it, i) => {
    if (it.type === "tool") {
      const last = out[out.length - 1];
      if (last?.kind === "tools") last.items.push(it);
      else out.push({ kind: "tools", items: [it], key: i });
    } else {
      out.push({ kind: "msg", item: it, key: i });
    }
  });
  return out;
}

export function Timeline({ preview, tool }: { preview: SessionPreview; tool: Tool }) {
  const { t } = useI18n();
  const s = t.sessions;
  const [filter, setFilter] = React.useState<TimelineFilter>("all");
  const c = preview.counts;
  const filters: { key: TimelineFilter; label: string; n: number }[] = [
    { key: "all", label: s.timeline.all, n: preview.timeline.length },
    { key: "chat", label: s.timeline.chat, n: c.user + c.assistant },
    { key: "tools", label: s.timeline.tools, n: c.tools },
    { key: "shell", label: s.timeline.shell, n: c.shell },
    { key: "files", label: s.timeline.files, n: c.files },
    { key: "errors", label: s.timeline.errors, n: c.errors },
  ];
  const shown = preview.timeline.filter((it) => matches(it, filter));
  const blocks = toBlocks(shown);

  return (
    <div className="flex flex-col gap-3">
      <div role="radiogroup" aria-label={s.timeline.filterLabel} className="sticky top-0 z-10 -mx-4 -mt-4 flex flex-wrap gap-1.5 border-b border-border bg-background/95 px-4 py-2.5 backdrop-blur">
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

      {blocks.length === 0 ? <p className="py-6 text-center text-sm text-muted-foreground">{s.timeline.empty}</p> : null}
      <ol className="flex flex-col gap-4">
        {blocks.map((b) =>
          b.kind === "msg" ? <MessageRow key={b.key} item={b.item} tool={tool} s={s} /> : <ToolGroup key={b.key} items={b.items} s={s} />,
        )}
      </ol>
      {preview.timeline_total > preview.timeline.length ? (
        <p className="text-center text-xs text-muted-foreground">{fmt(s.previewMore, { shown: preview.timeline.length, total: preview.timeline_total })}</p>
      ) : null}
    </div>
  );
}

function Avatar({ who, tool }: { who: "user" | "assistant"; tool: Tool }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "grid size-7 shrink-0 place-items-center rounded-full text-[11px] font-semibold",
        who === "user"
          ? "bg-primary text-primary-foreground"
          : tool === "claude"
            ? "bg-tool-claude/15 text-tool-claude"
            : "bg-tool-codex/15 text-tool-codex",
      )}
    >
      {who === "user" ? <User className="size-3.5" /> : tool === "claude" ? "C" : "X"}
    </span>
  );
}

function MessageRow({ item, tool, s }: { item: Exclude<TimelineItem, ToolItem>; tool: Tool; s: S }) {
  const [open, setOpen] = React.useState(false);
  const isUser = item.type === "user";
  const long = item.text.length > LONG_TEXT;
  return (
    <li className={cn("flex gap-2.5", isUser && "flex-row-reverse")}>
      <Avatar who={isUser ? "user" : "assistant"} tool={tool} />
      <div className={cn("flex max-w-[82%] min-w-0 flex-col gap-1", isUser && "items-end")}>
        <div className={cn("flex items-center gap-2 px-1 text-[11px] text-muted-foreground", isUser && "flex-row-reverse")}>
          <span className="font-medium text-foreground/80">{isUser ? s.roles.user : s.tools[tool]}</span>
          <span className="tabular-nums">{formatIso(item.ts)}</span>
        </div>
        <div
          className={cn(
            "min-w-0 px-3.5 py-2.5 text-sm leading-relaxed",
            isUser ? "rounded-2xl rounded-tr-md bg-primary text-primary-foreground" : "rounded-2xl rounded-tl-md border border-border bg-card",
          )}
        >
          <p className={cn("break-words whitespace-pre-wrap", long && !open && "line-clamp-[12]")}>{item.text}</p>
          {long ? (
            <button
              type="button"
              onClick={() => setOpen((v) => !v)}
              className={cn("mt-1.5 cursor-pointer text-xs font-medium hover:underline", isUser ? "text-primary-foreground/80" : "text-primary")}
            >
              {open ? s.collapse : s.expand}
            </button>
          ) : null}
        </div>
      </div>
    </li>
  );
}

function ToolGroup({ items, s }: { items: ToolItem[]; s: S }) {
  const errors = items.filter((i) => i.is_error).length;
  return (
    <li className="ml-9.5">
      <div className="overflow-hidden rounded-lg border border-border bg-muted/30">
        {items.length > 1 ? (
          <div className="flex items-center gap-2 border-b border-border/70 px-3 py-1.5 text-[11px] text-muted-foreground">
            <Wrench className="size-3" aria-hidden="true" />
            {fmt(s.timeline.group, { n: items.length })}
            {errors > 0 ? <span className="text-destructive">{fmt(s.timeline.groupErrors, { n: errors })}</span> : null}
          </div>
        ) : null}
        <ul className="divide-y divide-border/60">
          {items.map((it, i) => (
            <ToolRow key={i} it={it} s={s} />
          ))}
        </ul>
      </div>
    </li>
  );
}

function ToolRow({ it, s }: { it: ToolItem; s: S }) {
  const [open, setOpen] = React.useState(false);
  const Icon = KIND_ICON[it.kind];
  const pending = it.output === null;
  return (
    <li>
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="flex w-full cursor-pointer items-center gap-2 px-3 py-2 text-left outline-none hover:bg-muted/60 focus-visible:bg-muted/60"
      >
        <ChevronRight className={cn("size-3.5 shrink-0 text-muted-foreground transition-transform", open && "rotate-90")} aria-hidden="true" />
        <Icon className={cn("size-4 shrink-0", KIND_TONE[it.kind])} aria-hidden="true" />
        <span className="shrink-0 font-mono text-xs font-semibold">{it.name}</span>
        <span className="min-w-0 flex-1 truncate font-mono text-xs text-muted-foreground" title={it.summary}>
          {it.summary}
        </span>
        <span
          className={cn(
            "hidden shrink-0 rounded px-1.5 py-0.5 text-[10px] sm:inline",
            it.mapped ? "bg-emerald-500/12 text-emerald-700 dark:text-emerald-300" : "bg-muted text-muted-foreground",
          )}
          title={it.mapped ? s.timeline.mappedHint : s.timeline.textHint}
        >
          {it.mapped ? s.timeline.mapped : s.timeline.asText}
        </span>
        {pending ? (
          <span className="size-2 shrink-0 rounded-full bg-muted-foreground/40" title={s.timeline.noResult} />
        ) : it.is_error ? (
          <X className="size-3.5 shrink-0 text-destructive" aria-label={s.timeline.failed} />
        ) : (
          <Check className="size-3.5 shrink-0 text-emerald-600 dark:text-emerald-400" aria-label={s.timeline.ok} />
        )}
        <span className="shrink-0 text-[11px] text-muted-foreground tabular-nums">{formatIso(it.ts).slice(-8)}</span>
      </button>
      {open ? (
        <div className="flex flex-col gap-2 px-3 pt-1 pb-3 pl-9">
          <IoBlock label={s.timeline.input} text={it.input} />
          {it.output !== null ? <IoBlock label={it.is_error ? s.timeline.error : s.timeline.output} text={it.output || s.timeline.emptyOutput} error={it.is_error} /> : null}
        </div>
      ) : null}
    </li>
  );
}

function IoBlock({ label, text, error }: { label: string; text: string; error?: boolean }) {
  return (
    <div className="flex flex-col gap-1">
      <span className={cn("text-[11px] font-medium", error ? "text-destructive" : "text-muted-foreground")}>{label}</span>
      <pre
        className={cn(
          "max-h-72 overflow-auto rounded-md border px-3 py-2 font-mono text-[12px] leading-relaxed whitespace-pre-wrap break-all",
          error ? "border-destructive/30 bg-destructive/5" : "border-border bg-background",
        )}
      >
        {text}
      </pre>
    </div>
  );
}
