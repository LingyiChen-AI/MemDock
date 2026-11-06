import * as React from "react";
import { useAccountName } from "./accountNames";
import { Archive, Check, ChevronRight, CircleAlert, Loader2, ShieldAlert, ShieldCheck, ShieldX, TriangleAlert, Wrench, X } from "lucide-react";
import { readImageDataUri, storedImagePath, type Decision, type LiveAgent } from "@/api/live";
import { ChatAvatar, ChatRow, ChatText } from "@/components/chat/ChatMessage";
import { ChatMarkdown } from "@/components/chat/ChatMarkdown";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type { Messages } from "@/i18n";
import { fmt } from "@/pages/memory/paths";
import { CHIP_FOLD, IoBlock, KIND_ICON, KIND_TONE, Thumbs } from "@/pages/sessions/Timeline";
import { getConvUi, patchConvUi } from "./convUiState";
import { toConvBlocks, type ConvBlock, type ConvState, type LiveApproval, type LiveToolCall, type TurnSegment } from "./liveModel";

type C = Messages["chat"];

/** Pixels from the bottom that still count as "at the bottom" for auto-scroll. */
const STICK_SLACK = 48;

export function Conversation({
  conv,
  agent,
  running,
  starting,
  onRespond,
  prefix,
  scrollKey = null,
}: {
  /** Conversation key: the scroll position (and whether it follows the bottom) is kept under it. */
  scrollKey?: string | null;
  /** Rendered above the live messages inside the same scroll area (the session's history). */
  prefix?: React.ReactNode;
  conv: ConvState;
  agent: LiveAgent;
  /** The process is still starting (Codex takes ~10 s): show a clear notice. */
  starting: boolean;
  /** The agent is working: show the typing indicator. */
  running: boolean;
  onRespond: (requestId: string, decision: Decision) => Promise<void>;
}) {
  const { t } = useI18n();
  const c = t.chat;
  const userName = useAccountName(agent, c.you);
  const [detail, setDetail] = React.useState<LiveToolCall | null>(null);
  const blocks = React.useMemo(() => toConvBlocks(conv.items), [conv.items]);
  const scroller = React.useRef<HTMLDivElement>(null);
  const saved = React.useRef(getConvUi(scrollKey));
  const stick = React.useRef(saved.current.scrollTop === null || saved.current.stuck);
  const waiting = blocks.some((b) => b.kind === "approval" && b.approval.decision === null && !b.approval.expired);

  const onScroll = () => {
    const el = scroller.current;
    if (!el) return;
    stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < STICK_SLACK;
    patchConvUi(scrollKey, { scrollTop: el.scrollTop, stuck: stick.current });
  };

  // Back in a conversation that was scrolled up: return to where it was.
  React.useLayoutEffect(() => {
    const el = scroller.current;
    const top = saved.current.scrollTop;
    if (el && !stick.current && top !== null) el.scrollTop = top;
  }, []);

  React.useLayoutEffect(() => {
    const el = scroller.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [conv.items, running]);

  // Keep the open tool dialog in sync with streamed output.
  const liveDetail = detail
    ? (conv.items.find((it) => it.type === "tool" && it.call.id === detail.id) as { call: LiveToolCall } | undefined)?.call ?? detail
    : null;

  return (
    <div ref={scroller} onScroll={onScroll} className="min-h-0 flex-1 overflow-y-auto px-4 py-4" data-testid="chat-scroll">
      {prefix}
      {starting ? (
        <div role="status" className="mb-4 flex items-center justify-center gap-2 rounded-lg border border-border bg-muted/40 px-3 py-2.5 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin text-primary motion-reduce:animate-none" aria-hidden="true" />
          {fmt(c.startingAgent, { agent: c.agents[agent] })}
        </div>
      ) : null}
      {blocks.length === 0 && !running && !starting && !prefix ? <p className="py-10 text-center text-sm text-muted-foreground">{c.emptyConv}</p> : null}
      <ol className="flex flex-col gap-5">
        {blocks.map((b, bi) => {
          const lastTurn = running && !waiting && bi === blocks.length - 1;
          switch (b.kind) {
            case "user":
              return (
                <ChatRow key={b.key} role="user" name={userName}>
                  {b.text ? <ChatText text={b.text} expandLabel={c.expand} collapseLabel={c.collapse} /> : null}
                  {b.images?.length ? <LiveThumbs images={b.images} label={c.imageLoading} /> : null}
                </ChatRow>
              );
            case "turn":
              return (
                <ChatRow key={b.key} role="assistant" tool={agent} name={c.agents[agent]}>
                  {b.segments.map((seg, i) => (
                    <Segment key={i} seg={seg} onOpen={setDetail} c={c} />
                  ))}
                  {lastTurn && !streamingTail(b.segments) ? <InlineTyping c={c} /> : null}
                </ChatRow>
              );
            case "approval":
              return <ApprovalCard key={b.key} approval={b.approval} onRespond={onRespond} c={c} />;
            case "fatal":
              return (
                <li key={b.key} role="alert" className="flex items-start gap-2 rounded-lg border border-destructive/40 bg-destructive/8 px-3.5 py-2.5 text-sm text-destructive">
                  <CircleAlert className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                  <span className="min-w-0 [overflow-wrap:anywhere] whitespace-pre-wrap">{fmt(c.fatal, { message: b.message })}</span>
                </li>
              );
            case "notice": {
              const n = b.notice;
              const tokens = (v: number | null) => (v === null ? "?" : v >= 10_000 ? `${(v / 1000).toFixed(v >= 100_000 ? 0 : 1)}k` : String(v));
              if (n.kind === "command_output") {
                // Output of a local slash command (/cost...): a small card, monospace, left-aligned.
                return (
                  <li key={b.key} className="flex flex-col gap-1 pl-11">
                    <span className="text-[11px] text-muted-foreground">{c.notices.commandOutput}</span>
                    <pre className="max-w-[min(100%,48rem)] overflow-x-auto rounded-lg border border-border bg-muted/40 px-3 py-2 font-mono text-xs leading-5 whitespace-pre-wrap text-foreground/90 [overflow-wrap:anywhere]">
                      {n.text}
                    </pre>
                  </li>
                );
              }
              const done = n.kind === "compacted";
              const text = done
                ? n.before !== null || n.after !== null
                  ? fmt(c.notices.compacted, { before: tokens(n.before), after: tokens(n.after) })
                  : c.notices.compactedPlain
                : c.notices.compacting;
              return (
                <li key={b.key} className="flex items-center gap-3" role="status">
                  <span className="h-px flex-1 bg-border" />
                  <span
                    className={cn(
                      "inline-flex items-center gap-1.5 rounded-full border px-3 py-1 text-xs",
                      done ? "border-primary/25 bg-primary/6 text-primary" : "border-border bg-muted/50 text-muted-foreground",
                    )}
                  >
                    {done ? <Archive className="size-3.5 shrink-0" aria-hidden="true" /> : <Loader2 className="size-3.5 shrink-0 animate-spin motion-reduce:animate-none" aria-hidden="true" />}
                    {text}
                  </span>
                  <span className="h-px flex-1 bg-border" />
                </li>
              );
            }
            case "exited":
              return (
                <li key={b.key} className="flex items-center gap-3 text-xs text-muted-foreground">
                  <span className="h-px flex-1 bg-border" />
                  {b.code === null || b.code === 0 ? c.exited : fmt(c.exitedCode, { code: b.code })}
                  <span className="h-px flex-1 bg-border" />
                </li>
              );
          }
        })}
        {running && !waiting && !starting && showTyping(blocks) ? <TypingRow agent={agent} c={c} /> : null}
      </ol>
      <ToolDialog call={liveDetail} onClose={() => setDetail(null)} c={c} />
    </div>
  );
}

/** Loaded `memdock-image:` files (data URIs), shared by every view. */
const imageCache = new Map<string, Promise<string>>();

/** Tests. */
export function resetImageCache(): void {
  imageCache.clear();
}

function loadStoredImage(path: string): Promise<string> {
  let p = imageCache.get(path);
  if (!p) {
    p = readImageDataUri(path).catch((e) => {
      imageCache.delete(path);
      throw e;
    });
    imageCache.set(path, p);
  }
  return p;
}

/** Transparent 1x1 GIF shown while a stored image loads (or when it could not be read). */
const PLACEHOLDER = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

/**
 * Message thumbnails: data URIs as they are; stored transcripts reference images as
 * `memdock-image:<path>`, read lazily (when the message is shown) through `readImageDataUri`.
 */
export function LiveThumbs({ images, label }: { images: string[]; label: string }) {
  const { t } = useI18n();
  const [loaded, setLoaded] = React.useState<Record<string, string>>({});
  React.useEffect(() => {
    let alive = true;
    for (const src of images) {
      const path = storedImagePath(src);
      if (!path) continue;
      loadStoredImage(path)
        .then((uri) => alive && setLoaded((m) => (m[src] === uri ? m : { ...m, [src]: uri })))
        .catch(() => {});
    }
    return () => {
      alive = false;
    };
  }, [images]);
  const resolved = images.map((src) => (storedImagePath(src) === null ? src : (loaded[src] ?? PLACEHOLDER)));
  const waiting = images.some((src) => storedImagePath(src) !== null && !loaded[src]);
  return (
    <div aria-busy={waiting || undefined} aria-label={waiting ? label : undefined}>
      <Thumbs images={resolved} s={t.sessions} />
    </div>
  );
}

/** The reply's last part is text being streamed (its caret already shows activity). */
/** The waiting placeholder: after a plain message, not after a slash command or a notice. */
function showTyping(blocks: ConvBlock[]): boolean {
  const last = blocks[blocks.length - 1];
  if (!last || last.kind === "turn" || last.kind === "notice") return false;
  return !(last.kind === "user" && last.text.trim().startsWith("/"));
}

function streamingTail(segments: TurnSegment[]): boolean {
  const t = segments[segments.length - 1];
  return t?.kind === "text" && !!t.streaming;
}

/** Three bouncing dots inside the reply bubble while the agent works (e.g. waiting on a tool). */
function InlineTyping({ c }: { c: C }) {
  return (
    <span role="status" aria-label={c.typing} className="inline-flex h-4 items-center gap-1">
      {[0, 1, 2].map((i) => (
        <span key={i} className="size-1.5 animate-bounce rounded-full bg-muted-foreground/60 motion-reduce:animate-none" style={{ animationDelay: `${i * 150}ms` }} aria-hidden="true" />
      ))}
    </span>
  );
}

function TypingRow({ agent, c }: { agent: LiveAgent; c: C }) {
  return (
    <li className="flex items-center gap-2.5" role="status" aria-label={c.typing}>
      <ChatAvatar role="assistant" tool={agent} />
      <span className="inline-flex h-7 items-center gap-1 rounded-full border border-border bg-card px-3" aria-hidden="true">
        {[0, 1, 2].map((i) => (
          <span
            key={i}
            className="size-1.5 animate-bounce rounded-full bg-muted-foreground/60 motion-reduce:animate-none"
            style={{ animationDelay: `${i * 150}ms` }}
          />
        ))}
      </span>
      <span className="sr-only">{c.typing}</span>
    </li>
  );
}

function Segment({ seg, onOpen, c }: { seg: TurnSegment; onOpen: (call: LiveToolCall) => void; c: C }) {
  switch (seg.kind) {
    case "text":
      return seg.streaming ? (
        // Streaming text is re-rendered on every delta; skip the fold until it settles.
        // Same layout as the finished text; the caret is drawn after the last block (no layout change).
        <div className="min-w-0 [&>div>*:last-child]:after:ml-0.5 [&>div>*:last-child]:after:inline-block [&>div>*:last-child]:after:h-4 [&>div>*:last-child]:after:w-1.5 [&>div>*:last-child]:after:translate-y-0.5 [&>div>*:last-child]:after:animate-pulse [&>div>*:last-child]:after:bg-primary/60 [&>div>*:last-child]:after:content-[''] motion-reduce:[&>div>*:last-child]:after:animate-none">
          <ChatMarkdown text={seg.text} />
        </div>
      ) : (
        <ChatText text={seg.text} expandLabel={c.expand} collapseLabel={c.collapse} />
      );
    case "thinking":
      return <Thinking text={seg.text} c={c} />;
    case "tools":
      return <ChipRun calls={seg.calls} onOpen={onOpen} c={c} />;
    case "error":
      return (
        <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
          <TriangleAlert className="mt-px size-3.5 shrink-0 text-amber-600 dark:text-amber-400" aria-hidden="true" />
          <span className="min-w-0 [overflow-wrap:anywhere]">{fmt(c.error, { message: seg.message })}</span>
        </p>
      );
  }
}

function Thinking({ text, c }: { text: string; c: C }) {
  const [open, setOpen] = React.useState(false);
  return (
    <div className="min-w-0">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="inline-flex cursor-pointer items-center gap-1 rounded text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
      >
        <ChevronRight className={cn("size-3.5 transition-transform duration-150 motion-reduce:transition-none", open && "rotate-90")} aria-hidden="true" />
        {c.thinking}
      </button>
      {open ? (
        <div className="mt-1.5 border-l-2 border-border pl-3 text-xs leading-relaxed whitespace-pre-wrap text-muted-foreground [overflow-wrap:anywhere]">{text}</div>
      ) : null}
    </div>
  );
}

function CallMark({ call, c }: { call: LiveToolCall; c: C }) {
  if (!call.finished) return <Loader2 className="size-3 shrink-0 animate-spin text-muted-foreground motion-reduce:animate-none" aria-label={c.tool.running} />;
  return call.is_error ? (
    <X className="size-3 shrink-0 text-destructive" aria-label={c.tool.failed} />
  ) : (
    <Check className="size-3 shrink-0 text-emerald-600 dark:text-emerald-400" aria-label={c.tool.ok} />
  );
}

/** Same capsules as the history timeline: wrap inside the bubble, long runs fold behind "+N". */
function ChipRun({ calls, onOpen, c }: { calls: LiveToolCall[]; onOpen: (call: LiveToolCall) => void; c: C }) {
  const [all, setAll] = React.useState(false);
  const shown = all || calls.length <= CHIP_FOLD ? calls : calls.slice(0, CHIP_FOLD);
  const rest = calls.length - shown.length;
  return (
    <div className="flex max-w-full min-w-0 flex-wrap gap-1.5">
      {shown.map((call) => {
        const Icon = KIND_ICON[call.kind] ?? Wrench;
        return (
          <button
            key={call.id}
            type="button"
            onClick={() => onOpen(call)}
            title={`${call.name}  ${call.summary}`}
            className={cn(
              "inline-flex h-6 max-w-full min-w-0 cursor-pointer items-center gap-1.5 rounded-full border bg-background px-2.5 text-xs outline-none transition-colors hover:border-primary/50 focus-visible:ring-2 focus-visible:ring-ring",
              call.is_error ? "border-destructive/40" : "border-border",
            )}
          >
            <CallMark call={call} c={c} />
            <Icon className={cn("size-3 shrink-0", KIND_TONE[call.kind])} aria-hidden="true" />
            <span className="shrink-0 font-medium">{call.name}</span>
            {call.summary ? <span className="max-w-60 min-w-0 truncate font-mono text-[11px] text-muted-foreground">{call.summary}</span> : null}
          </button>
        );
      })}
      {rest > 0 ? (
        <button
          type="button"
          onClick={() => setAll(true)}
          className="inline-flex h-6 shrink-0 cursor-pointer items-center rounded-full border border-dashed border-border px-2.5 text-xs text-muted-foreground outline-none hover:border-primary/50 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
        >
          {fmt(c.more, { n: rest })}
        </button>
      ) : null}
    </div>
  );
}

function ToolDialog({ call, onClose, c }: { call: LiveToolCall | null; onClose: () => void; c: C }) {
  const Icon = call ? (KIND_ICON[call.kind] ?? Wrench) : Wrench;
  return (
    <Dialog open={!!call} onOpenChange={(o) => (!o ? onClose() : undefined)}>
      <DialogContent className="flex max-h-[85vh] flex-col gap-3 sm:max-w-3xl">
        {call ? (
          <>
            <DialogHeader>
              <DialogTitle className="flex min-w-0 flex-wrap items-center gap-2">
                <Icon className={cn("size-4 shrink-0", KIND_TONE[call.kind])} aria-hidden="true" />
                <span className="font-mono">{call.name}</span>
                <CallMark call={call} c={c} />
              </DialogTitle>
              <DialogDescription className="truncate font-mono text-xs">{call.summary || " "}</DialogDescription>
            </DialogHeader>
            <div className="-mx-1 flex min-h-0 flex-col gap-3 overflow-y-auto px-1">
              <IoBlock label={c.tool.input} text={call.input} />
              {call.finished || call.output ? (
                <IoBlock label={call.is_error ? c.tool.error : c.tool.output} text={call.output || c.tool.emptyOutput} error={call.is_error} />
              ) : null}
            </div>
          </>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

function ApprovalCard({ approval, onRespond, c }: { approval: LiveApproval; onRespond: (requestId: string, d: Decision) => Promise<void>; c: C }) {
  const [open, setOpen] = React.useState(false);
  const [busy, setBusy] = React.useState<Decision | null>(null);
  const pending = approval.decision === null && !approval.expired;
  const Icon = KIND_ICON[approval.kind] ?? Wrench;
  const respond = async (d: Decision) => {
    setBusy(d);
    try {
      await onRespond(approval.request_id, d);
    } finally {
      setBusy(null);
    }
  };
  return (
    <li
      aria-label={fmt(c.approval.title, { tool: approval.tool_name })}
      className={cn(
        "flex min-w-0 flex-col gap-2.5 rounded-xl border px-4 py-3",
        pending ? "border-amber-500/60 bg-amber-500/10 shadow-sm ring-2 ring-amber-500/20" : "border-border bg-muted/30",
      )}
    >
      <div className="flex min-w-0 items-start gap-2">
        <ShieldAlert className={cn("mt-0.5 size-4 shrink-0", pending ? "text-amber-600 dark:text-amber-400" : "text-muted-foreground")} aria-hidden="true" />
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="text-sm font-semibold">{fmt(c.approval.title, { tool: approval.tool_name })}</span>
          {approval.summary ? (
            <span className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
              <Icon className={cn("size-3 shrink-0", KIND_TONE[approval.kind])} aria-hidden="true" />
              <code className="min-w-0 font-mono [overflow-wrap:anywhere]">{approval.summary}</code>
            </span>
          ) : null}
        </div>
        {approval.decision ? <Resolved decision={approval.decision} c={c} /> : null}
        {approval.expired ? (
          <span className="inline-flex shrink-0 items-center rounded bg-muted px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground">{c.approval.expired}</span>
        ) : null}
      </div>
      {approval.detail ? (
        <div className="min-w-0">
          <button
            type="button"
            aria-expanded={open}
            onClick={() => setOpen((v) => !v)}
            className="inline-flex cursor-pointer items-center gap-1 rounded text-xs font-medium text-primary outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
          >
            <ChevronRight className={cn("size-3.5 transition-transform duration-150 motion-reduce:transition-none", open && "rotate-90")} aria-hidden="true" />
            {open ? c.approval.hideDetail : c.approval.showDetail}
          </button>
          {open ? (
            <pre className="mt-1.5 max-h-72 overflow-auto rounded-md border border-border bg-background px-3 py-2 font-mono text-[12px] leading-relaxed whitespace-pre-wrap [overflow-wrap:anywhere]">
              {approval.detail}
            </pre>
          ) : null}
        </div>
      ) : null}
      {pending ? (
        <div className="flex flex-wrap items-center gap-2">
          <Button type="button" size="sm" className="cursor-pointer" disabled={busy !== null} onClick={() => void respond("allow")}>
            {busy === "allow" ? <Loader2 className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <ShieldCheck aria-hidden="true" />}
            {c.approval.allow}
          </Button>
          {approval.can_allow_session ? (
            <Button type="button" size="sm" variant="outline" className="cursor-pointer" disabled={busy !== null} onClick={() => void respond("allow_session")}>
              {busy === "allow_session" ? <Loader2 className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : null}
              {c.approval.allowSession}
            </Button>
          ) : null}
          <Button type="button" size="sm" variant="destructive" className="cursor-pointer" disabled={busy !== null} onClick={() => void respond("deny")}>
            {busy === "deny" ? <Loader2 className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <ShieldX aria-hidden="true" />}
            {c.approval.deny}
          </Button>
        </div>
      ) : null}
    </li>
  );
}

function Resolved({ decision, c }: { decision: Decision; c: C }) {
  const label = decision === "deny" ? c.approval.denied : decision === "allow_session" ? c.approval.allowedSession : c.approval.allowed;
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1 rounded px-1.5 py-0.5 text-[11px] font-medium",
        decision === "deny" ? "bg-destructive/10 text-destructive" : "bg-emerald-500/12 text-emerald-700 dark:text-emerald-300",
      )}
    >
      {decision === "deny" ? <X className="size-3" aria-hidden="true" /> : <Check className="size-3" aria-hidden="true" />}
      {label}
    </span>
  );
}
