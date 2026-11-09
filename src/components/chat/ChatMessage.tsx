import * as React from "react";
import { Check, Copy, User, X } from "lucide-react";
import type { Tool } from "@/api/types";
import { ToolLogo } from "@/components/brand/ToolLogo";
import { useI18n } from "@/i18n/I18nProvider";
import { cn } from "@/lib/utils";
import { ChatMarkdown } from "./ChatMarkdown";

/**
 * Shared chat building blocks (session viewer, evidence drawer, ...):
 * ChatRow = avatar + name/time line + bubble; ChatText = markdown body that
 * folds when long and can highlight a quoted passage.
 */

export type ChatRole = "user" | "assistant";

export function ChatAvatar({ role, tool }: { role: ChatRole; tool?: Tool }) {
  if (role === "user" || !tool) {
    return (
      <span aria-hidden="true" className="grid size-7 shrink-0 place-items-center rounded-full bg-primary/15 text-primary">
        <User className="size-3.5" />
      </span>
    );
  }
  return (
    <span aria-hidden="true" className="grid size-7 shrink-0 place-items-center rounded-full border border-border bg-card">
      <ToolLogo tool={tool} className="size-4" aria-hidden="true" />
    </span>
  );
}

export interface ChatRowProps {
  role: ChatRole;
  /** Assistant side: whose logo to show. */
  tool?: Tool;
  name: string;
  time?: string;
  /** Marks the row (e.g. the message a quote came from). */
  highlighted?: boolean;
  /** Extra badge after the time, e.g. "hit". */
  badge?: React.ReactNode;
  /** The complete message as Markdown, even if its display is folded. */
  copyText?: string;
  children: React.ReactNode;
  className?: string;
  /** Pass-through for tests and anchors. */
  "data-hit"?: string;
}

export function ChatRow({ role, tool, name, time, highlighted, badge, copyText, children, className, ...rest }: ChatRowProps) {
  const user = role === "user";
  return (
    <li className={cn("group/message flex gap-2.5", user && "flex-row-reverse", className)} {...rest}>
      <ChatAvatar role={role} tool={tool} />
      <div className={cn("flex max-w-[86%] min-w-0 flex-col gap-1", user ? "items-end" : "flex-1")}>
        <div className={cn("flex items-center gap-2 px-1 text-[11px] text-muted-foreground", user && "flex-row-reverse")}>
          <span className="font-medium text-foreground/80">{name}</span>
          {time ? <span className="tabular-nums">{time}</span> : null}
          {badge}
          {copyText?.trim() ? <CopyMessage text={copyText} /> : null}
        </div>
        <div
          className={cn(
            "flex max-w-full min-w-0 flex-col gap-2.5 overflow-hidden px-3.5 py-2.5 text-sm leading-relaxed",
            user ? "rounded-2xl rounded-tr-md border border-primary/25 bg-primary/10" : "self-start rounded-2xl rounded-tl-md border border-border bg-card",
            highlighted && "ring-2 ring-warning/60 ring-offset-1 ring-offset-background",
          )}
        >
          {children}
        </div>
      </div>
    </li>
  );
}

function CopyMessage({ text }: { text: string }) {
  const { t: { sessions: s } } = useI18n();
  const [status, setStatus] = React.useState<"copied" | "failed" | null>(null);
  const reset = React.useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  React.useEffect(() => () => clearTimeout(reset.current), []);
  const label = status === "copied" ? s.copied : status === "failed" ? s.copyFailed : s.copy;
  const Icon = status === "copied" ? Check : status === "failed" ? X : Copy;

  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      setStatus("copied");
    } catch {
      setStatus("failed");
    }
    clearTimeout(reset.current);
    reset.current = setTimeout(() => setStatus(null), 1600);
  }

  return (
    <>
      <button
        type="button"
        aria-label={s.copyMessage}
        title={status ? label : s.copyMessage}
        onClick={() => void copy()}
        className={cn(
          "grid size-5 shrink-0 cursor-pointer place-items-center rounded text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring",
          !status && "opacity-0 group-hover/message:opacity-100 [@media(hover:none)]:opacity-100",
          status === "copied" && "text-primary",
          status === "failed" && "text-destructive",
        )}
      >
        <Icon aria-hidden="true" className="size-3.5" />
      </button>
      <span className="sr-only" aria-live="polite">{status ? label : ""}</span>
    </>
  );
}

const LONG_CHARS = 700;
const LONG_LINES = 18;
const HIGHLIGHT_NAME = "chat-quote";

/** Highlights `quote` inside `root`'s text with the CSS Custom Highlight API (no DOM changes). */
function useQuoteHighlight(root: React.RefObject<HTMLElement | null>, quote: string | undefined) {
  React.useEffect(() => {
    const el = root.current;
    const reg = (globalThis as { CSS?: { highlights?: Map<string, unknown> } }).CSS?.highlights;
    const HighlightCtor = (globalThis as { Highlight?: new (...r: Range[]) => unknown }).Highlight;
    const q = quote?.trim();
    if (!el || !q || !reg || !HighlightCtor) return;
    const nodes: Text[] = [];
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) nodes.push(n as Text);
    const full = nodes.map((n) => n.data).join("");
    // Whitespace-tolerant: the quote may differ from the text in spaces and line breaks.
    const words = q.split(/\s+/).filter(Boolean).map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
    const m = new RegExp(words.join("\\s*")).exec(full);
    if (!m) return;
    const at = m.index;
    const len = m[0].length;
    const pos = (offset: number): [Text, number] => {
      let acc = 0;
      for (const n of nodes) {
        if (offset <= acc + n.data.length) return [n, offset - acc];
        acc += n.data.length;
      }
      const last = nodes[nodes.length - 1];
      return [last, last.data.length];
    };
    const range = document.createRange();
    const [sn, so] = pos(at);
    const [en, eo] = pos(at + len);
    range.setStart(sn, so);
    range.setEnd(en, eo);
    const existing = reg.get(HIGHLIGHT_NAME) as { add?: (r: Range) => void; delete?: (r: Range) => void } | undefined;
    if (existing?.add) existing.add(range);
    else reg.set(HIGHLIGHT_NAME, new HighlightCtor(range));
    return () => {
      const h = reg.get(HIGHLIGHT_NAME) as { delete?: (r: Range) => void } | undefined;
      h?.delete?.(range);
    };
  }, [root, quote]);
}

export function ChatText({
  text,
  expandLabel,
  collapseLabel,
  highlight,
}: {
  text: string;
  expandLabel: string;
  collapseLabel: string;
  /** A passage to highlight (e.g. the quoted evidence); long texts start expanded when set. */
  highlight?: string;
}) {
  const long = text.length > LONG_CHARS || text.split("\n").length > LONG_LINES;
  const [open, setOpen] = React.useState(!!highlight);
  const ref = React.useRef<HTMLDivElement>(null);
  useQuoteHighlight(ref, highlight);
  return (
    <div className="min-w-0">
      <div ref={ref} className={cn(long && !open && "max-h-72 overflow-hidden [mask-image:linear-gradient(to_bottom,black_80%,transparent)]")}>
        <ChatMarkdown text={text} />
      </div>
      {long ? (
        <button type="button" onClick={() => setOpen((v) => !v)} className="mt-1.5 cursor-pointer text-xs font-medium text-primary hover:underline">
          {open ? collapseLabel : expandLabel}
        </button>
      ) : null}
    </div>
  );
}
