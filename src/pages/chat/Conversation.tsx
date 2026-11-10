import * as React from "react";
import { toolCodeLocation, WorkspaceCodeContext } from "./workspaceCodeLinks";
import { useAccountName } from "./accountNames";
import { Archive, Check, ChevronRight, CircleAlert, Loader2, ShieldAlert, ShieldCheck, ShieldX, TriangleAlert, Wrench, X } from "lucide-react";
import { readImageDataUri, storedImagePath, type Decision, type LiveAgent, type QuestionAnswers } from "@/api/live";
import { ChatAvatar, ChatRow, ChatText } from "@/components/chat/ChatMessage";
import { ChatMarkdown } from "@/components/chat/ChatMarkdown";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type { Messages } from "@/i18n";
import { fmt } from "@/pages/memory/paths";
import { CHIP_FOLD, IoBlock, KIND_ICON, KIND_TONE, Thumbs } from "@/pages/sessions/Timeline";
import { getConvUi, patchConvUi } from "./convUiState";
import { isCodexWriterBusy, isPendingApproval, pendingApprovals, toConvBlocks, type ConvBlock, type ConvState, type LiveApproval, type LiveToolCall, type TurnSegment } from "./liveModel";

type C = Messages["chat"];
type Respond = (requestId: string, decision: Decision, answers?: QuestionAnswers) => Promise<void>;

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
  onRespond: Respond;
}) {
  const { t } = useI18n();
  const c = t.chat;
  const userName = useAccountName(agent, c.you);
  const [detail, setDetail] = React.useState<{ callId?: string; requestId?: string } | null>(null);
  const blocks = React.useMemo(() => toConvBlocks(conv.items), [conv.items]);
  const scroller = React.useRef<HTMLDivElement>(null);
  const saved = React.useRef(getConvUi(scrollKey));
  const stick = React.useRef(saved.current.scrollTop === null || saved.current.stuck);
  const waiting = pendingApprovals(conv).length > 0;

  // Where we last scrolled to ourselves. Content that grows after that (history, images) fires a
  // scroll event that looks "not at the bottom"; only a move *up* from here is the reader scrolling.
  const pinnedTop = React.useRef<number | null>(null);
  const toBottom = (el: HTMLDivElement) => {
    el.scrollTop = el.scrollHeight;
    pinnedTop.current = el.scrollTop;
  };

  const onScroll = () => {
    const el = scroller.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < STICK_SLACK;
    if (!atBottom && stick.current && pinnedTop.current !== null && el.scrollTop >= pinnedTop.current - 1) {
      toBottom(el); // the content grew under us: keep following
      return;
    }
    stick.current = atBottom;
    if (!atBottom) pinnedTop.current = null;
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
    if (el && stick.current) toBottom(el);
  }, [conv.items, running]);

  // The history arrives after the pane opens (and images load later): keep following the bottom
  // while the content grows, unless the reader scrolled up.
  const content = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    const el = scroller.current;
    const inner = content.current;
    if (!el || !inner || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => {
      if (stick.current) toBottom(el);
    });
    ro.observe(inner);
    ro.observe(el); // the visible area shrinks too (a notice above the composer)
    return () => ro.disconnect();
  }, []);

  const dialogSegments = detail ? blocks.flatMap((block) => block.kind === "turn" ? block.segments : []) : [];
  const requestedApproval = detail?.requestId;
  const liveDetail = dialogSegments.flatMap((seg) => seg.kind === "tools"
    ? seg.calls.map((call) => ({ call, approvals: seg.approvals?.get(call.id) ?? [] }))
    : []
  ).find(({ call, approvals }) => call.id === detail?.callId || approvals.some((approval) => approval.request_id === requestedApproval));
  const standaloneApproval = dialogSegments.find((seg) => seg.kind === "approval" && seg.approval.request_id === requestedApproval);
  const detailApprovals = liveDetail?.approvals ?? (standaloneApproval?.kind === "approval" ? [standaloneApproval.approval] : []);

  return (
    <div ref={scroller} onScroll={onScroll} className="min-h-0 flex-1 overflow-y-auto px-4 py-4" data-testid="chat-scroll">
      <div ref={content}>
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
                <ChatRow key={b.key} role="user" name={userName} copyText={b.text}>
                  {b.text ? <ChatText text={b.text} expandLabel={c.expand} collapseLabel={c.collapse} /> : null}
                  {b.images?.length ? <LiveThumbs images={b.images} label={c.imageLoading} /> : null}
                </ChatRow>
              );
            case "turn":
              return (
                <ChatRow key={b.key} role="assistant" tool={agent} name={c.agents[agent]} copyText={
                  b.segments.flatMap((seg) => seg.kind === "text" ? [seg.text] : []).join("\n\n")
                  || b.segments.flatMap((seg) => seg.kind === "tools" ? seg.calls.map((call) => call.summary) : []).join("\n")
                }>
                  {b.segments.map((seg, i) => (
                    <Segment key={seg.kind === "approval" ? seg.approval.request_id : i} seg={seg} showCaret={lastTurn && i === b.segments.length - 1} onRespond={onRespond} onOpen={(call) => setDetail({ callId: call.id })} onApproval={(approval) => setDetail({ requestId: approval.request_id })} c={c} />
                  ))}
                  {lastTurn && !streamingTail(b.segments) ? <InlineTyping c={c} /> : null}
                </ChatRow>
              );
            case "fatal":
              if (agent === "codex" && isCodexWriterBusy(b.message)) return null;
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
      </div>
      <ToolDialog call={liveDetail?.call ?? null} approvals={detailApprovals} onRespond={onRespond} onClose={() => setDetail(null)} c={c} />
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

function Segment({ seg, showCaret, onOpen, onApproval, onRespond, c }: { seg: TurnSegment; showCaret: boolean; onOpen: (call: LiveToolCall) => void; onApproval: (approval: LiveApproval) => void; onRespond: Respond; c: C }) {
  switch (seg.kind) {
    case "text":
      return seg.streaming ? (
        // Streaming text is re-rendered on every delta; skip the fold until it settles.
        // Same layout as the finished text; the caret is drawn after the last block (no layout change).
        <div className={cn("min-w-0", showCaret && "[&>div>*:last-child]:after:ml-0.5 [&>div>*:last-child]:after:inline-block [&>div>*:last-child]:after:h-4 [&>div>*:last-child]:after:w-1.5 [&>div>*:last-child]:after:translate-y-0.5 [&>div>*:last-child]:after:animate-pulse [&>div>*:last-child]:after:bg-primary/60 [&>div>*:last-child]:after:content-[''] motion-reduce:[&>div>*:last-child]:after:animate-none")}>
          <ChatMarkdown text={seg.text} />
        </div>
      ) : (
        <ChatText text={seg.text} expandLabel={c.expand} collapseLabel={c.collapse} />
      );
    case "approval":
      if (isPendingApproval(seg.approval)) {
        return <ApprovalCard approval={seg.approval} onRespond={onRespond} collapsible c={c} />;
      }
      return (
        <button type="button" className={chipClass} onClick={() => onApproval(seg.approval)} title={seg.approval.summary}>
          <span className="shrink-0 font-medium">{seg.approval.tool_name}</span>
          <span className="max-w-60 truncate font-mono text-[11px] text-muted-foreground">{seg.approval.summary}</span>
          <ApprovalMark approval={seg.approval} c={c} />
        </button>
      );
    case "thinking":
      return <Thinking text={seg.text} c={c} />;
    case "tools":
      return <ChipRun calls={seg.calls} approvals={seg.approvals} onOpen={onOpen} c={c} />;
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

const chipClass = "inline-flex h-6 w-fit max-w-full min-w-0 cursor-pointer items-center gap-1.5 rounded-full border border-border bg-background px-2.5 text-xs outline-none transition-colors hover:border-primary/50 focus-visible:ring-2 focus-visible:ring-ring";

function ApprovalMark({ approval, c }: { approval: LiveApproval; c: C }) {
  if (approval.decision) return <Resolved decision={approval.decision} link c={c} />;
  return (
    <span className={cn("inline-flex shrink-0 items-center gap-1", isPendingApproval(approval) ? "text-amber-700 dark:text-amber-400" : "text-muted-foreground")}>
      <ShieldAlert aria-hidden="true" className="size-3" />
      {approval.expired ? c.approval.expired : c.approval.pending}
    </span>
  );
}

/** Same capsules as the history timeline: wrap inside the bubble, long runs fold behind "+N". */
function ChipRun({ calls, approvals, onOpen, c }: { calls: LiveToolCall[]; approvals?: Map<string, LiveApproval[]>; onOpen: (call: LiveToolCall) => void; c: C }) {
  const openCode = React.useContext(WorkspaceCodeContext);
  const { t: { workspaceCode } } = useI18n();
  const [all, setAll] = React.useState(false);
  const shown = all ? calls : calls.slice(0, CHIP_FOLD);
  const rest = calls.length - shown.length;
  return (
    <div className="flex max-w-full min-w-0 flex-wrap gap-1.5">
      {shown.map((call) => {
        const Icon = KIND_ICON[call.kind] ?? Wrench;
        const requests = approvals?.get(call.id) ?? [];
        const mark = requests[requests.length - 1];
        const location = !mark && openCode && toolCodeLocation(call);
        return (
          <button
            key={call.id}
            type="button"
            onClick={(event) => {
              if (event.altKey || !location || !openCode?.(location)) onOpen(call);
            }}
            title={`${call.name}  ${call.summary}${location ? "\n" + workspaceCode.toolDetails : ""}`}
            className={cn(
              chipClass,
              call.is_error ? "border-destructive/40" : "border-border",
            )}
          >
            <CallMark call={call} c={c} />
            <Icon className={cn("size-3 shrink-0", KIND_TONE[call.kind])} aria-hidden="true" />
            <span className="shrink-0 font-medium">{call.name}</span>
            {call.summary ? <span className="max-w-60 min-w-0 truncate font-mono text-[11px] text-muted-foreground">{call.summary}</span> : null}
            {mark ? <ApprovalMark approval={mark} c={c} /> : null}
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

function ToolDialog({ call, approvals, onRespond, onClose, c }: { call: LiveToolCall | null; approvals: LiveApproval[]; onRespond: Respond; onClose: () => void; c: C }) {
  const kind = call?.kind ?? approvals[0]?.kind;
  const Icon = kind ? KIND_ICON[kind] ?? Wrench : Wrench;
  return (
    <Dialog open={!!call || approvals.length > 0} onOpenChange={(o) => (!o ? onClose() : undefined)}>
      <DialogContent size="wide" className="flex max-h-[85vh] flex-col gap-3">
        {call || approvals.length ? (
          <>
            <DialogHeader>
              <DialogTitle className="flex min-w-0 flex-wrap items-center gap-2">
                <Icon className={cn("size-4 shrink-0", kind && KIND_TONE[kind])} aria-hidden="true" />
                <span className="font-mono">{call?.name ?? approvals[0]?.tool_name}</span>
                {call ? <CallMark call={call} c={c} /> : null}
              </DialogTitle>
              <DialogDescription className="truncate font-mono text-xs">{call?.summary || approvals[0]?.summary || " "}</DialogDescription>
            </DialogHeader>
            <div className="-mx-1 flex min-h-0 flex-col gap-3 overflow-y-auto px-1">
              {approvals.map((approval) => <ApprovalCard key={approval.request_id} approval={approval} onRespond={onRespond} c={c} />)}
              {call && !approvals.some((approval) => approval.detail === call?.input) ? <IoBlock label={c.tool.input} text={call.input} /> : null}
              {call && (call.finished || call.output) ? (
                <IoBlock label={call.is_error ? c.tool.error : c.tool.output} text={call.output || c.tool.emptyOutput} error={call.is_error} output />
              ) : null}
            </div>
          </>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

type QuestionOption = { label: string; description?: string };
type Question = {
  id: string;
  question: string;
  header?: string;
  multiSelect?: boolean;
  isOther?: boolean;
  isSecret?: boolean;
  options?: QuestionOption[] | null;
};

function questionList(approval: LiveApproval): Question[] {
  try {
    const { questions } = JSON.parse(approval.detail);
    if (!Array.isArray(questions)) return [];
    return questions.filter((q) => typeof q?.question === "string").map((q) => ({
      ...q,
      id: q.id ?? q.question,
      isOther: approval.tool_name === "AskUserQuestion" || q.isOther,
      options: Array.isArray(q.options) ? q.options.filter((option: QuestionOption) => typeof option?.label === "string") : [],
    }));
  } catch {
    return [];
  }
}

function ownValue<T>(values: Record<string, T>, key: string): T | undefined {
  return Object.prototype.hasOwnProperty.call(values, key) ? values[key] : undefined;
}

function QuestionCard({ approval, onRespond, c }: { approval: LiveApproval; onRespond: Respond; c: C }) {
  const formId = React.useId();
  const [selected, setSelected] = React.useState<QuestionAnswers>({});
  const [custom, setCustom] = React.useState<Record<string, string>>({});
  const [busy, setBusy] = React.useState(false);
  const questions = questionList(approval);
  const pending = isPendingApproval(approval);
  const answers = Object.fromEntries(questions.map(({ id }) => {
    const other = ownValue(custom, id)?.trim();
    const choices = ownValue(selected, id) ?? [];
    return [id, other ? [...choices, other] : choices];
  }));
  const reply = async (decision: Decision) => {
    setBusy(true);
    try {
      await onRespond(approval.request_id, decision, decision === "allow" ? answers : undefined);
    } finally {
      setBusy(false);
    }
  };
  return (
    <section aria-label={c.question.title} className="w-full max-w-xl space-y-3 rounded-lg border border-border bg-muted/25 p-3">
      <div className="flex items-center justify-between gap-2">
        <span className="text-sm font-semibold">{c.question.title}</span>
        {approval.decision ? <span className="text-xs text-muted-foreground">{approval.decision === "deny" ? c.question.skipped : c.question.answered}</span> : null}
        {approval.expired ? <span className="text-xs text-muted-foreground">{c.approval.expired}</span> : null}
      </div>
      {questions.map((q, i) => (
        <fieldset key={q.id} disabled={!pending || busy} className="space-y-1.5">
          <legend className="mb-1 text-sm font-medium">{q.question}</legend>
          {q.options?.map(({ label, description }) => {
            const checked = ownValue(selected, q.id)?.includes(label) ?? false;
            return (
              <label key={label} className={cn("flex items-start gap-2 rounded-md border px-2.5 py-1.5 text-sm", pending && "cursor-pointer hover:bg-muted", checked ? "border-primary/50 bg-primary/5" : "border-border")}>
                <input type={q.multiSelect ? "checkbox" : "radio"} name={`${formId}-${i}`} checked={checked} className="mt-0.5 accent-primary" onChange={() => {
                  if (!q.multiSelect) setCustom((prev) => ({ ...prev, [q.id]: "" }));
                  setSelected((prev) => {
                    const choices = ownValue(prev, q.id) ?? [];
                    const next = checked ? choices.filter((value) => value !== label) : [...choices, label];
                    return { ...prev, [q.id]: q.multiSelect ? next : [label] };
                  });
                }} />
                <span><span className="block font-medium">{label}</span>{description ? <span className="block text-xs text-muted-foreground">{description}</span> : null}</span>
              </label>
            );
          })}
          {pending && (q.isOther || !q.options?.length) ? (
            <Input type={q.isSecret ? "password" : "text"} aria-label={`${q.question} · ${c.question.other}`} placeholder={c.question.other} value={ownValue(custom, q.id) ?? ""} onChange={(e) => {
              const value = e.target.value;
              if (!q.multiSelect) setSelected((prev) => ({ ...prev, [q.id]: [] }));
              setCustom((prev) => ({ ...prev, [q.id]: value }));
            }} />
          ) : null}
        </fieldset>
      ))}
      {pending ? (
        <div className="flex gap-2">
          <Button type="button" size="sm" disabled={busy || !questions.length || Object.values(answers).some((a) => !a.length)} onClick={() => void reply("allow")}>{c.question.submit}</Button>
          <Button type="button" size="sm" variant="ghost" disabled={busy} onClick={() => void reply("deny")}>{c.question.skip}</Button>
        </div>
      ) : null}
    </section>
  );
}

function ApprovalCard({ approval, onRespond, collapsible = false, c }: { approval: LiveApproval; collapsible?: boolean; onRespond: Respond; c: C }) {
  if (approval.tool_name === "AskUserQuestion" || approval.tool_name === "request_user_input") {
    return <QuestionCard key={approval.request_id} approval={approval} onRespond={onRespond} c={c} />;
  }
  return <ToolApprovalCard approval={approval} onRespond={onRespond} collapsible={collapsible} c={c} />;
}

function ToolApprovalCard({ approval, onRespond, collapsible, c }: { approval: LiveApproval; collapsible: boolean; onRespond: Respond; c: C }) {
  const [busy, setBusy] = React.useState<Decision | null>(null);
  const pending = isPendingApproval(approval);
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
    <div
      role="group"
      aria-label={fmt(c.approval.title, { tool: approval.tool_name })}
      className={cn(
        "flex w-fit max-w-full min-w-0 flex-col gap-1.5 rounded-md border px-2.5 py-1.5",
        pending ? "border-amber-500/50 bg-amber-500/8" : "border-border bg-muted/30",
      )}
    >
      <div className="flex min-w-0 items-start gap-2">
        <ShieldAlert className={cn("mt-0.5 size-4 shrink-0", pending ? "text-amber-600 dark:text-amber-400" : "text-muted-foreground")} aria-hidden="true" />
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="text-sm font-semibold">{fmt(c.approval.title, { tool: approval.tool_name })}</span>
          {approval.summary ? (
            <span className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
              <Icon className={cn("size-3 shrink-0", KIND_TONE[approval.kind])} aria-hidden="true" />
              <code className={cn("min-w-0 font-mono", pending ? "[overflow-wrap:anywhere]" : "truncate")} title={approval.summary}>{approval.summary}</code>
            </span>
          ) : null}
        </div>
        {approval.decision ? <Resolved decision={approval.decision} c={c} /> : null}
        {approval.expired ? (
          <span className="inline-flex shrink-0 items-center rounded bg-muted px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground">{c.approval.expired}</span>
        ) : null}
      </div>
      {approval.detail && collapsible ? (
        <details className="min-w-0">
          <summary className="w-fit cursor-pointer rounded text-xs font-medium text-primary hover:underline focus-visible:ring-2 focus-visible:ring-ring">
            {c.approval.showDetail}
          </summary>
          <pre className="mt-1.5 max-h-72 overflow-auto rounded-md border border-border bg-background px-3 py-2 text-xs leading-relaxed whitespace-pre-wrap [overflow-wrap:anywhere]">
            {approval.detail}
          </pre>
        </details>
      ) : approval.detail ? <IoBlock label={c.approval.showDetail} text={approval.detail} /> : null}
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
    </div>
  );
}

function Resolved({ decision, link = false, c }: { decision: Decision; link?: boolean; c: C }) {
  const label = decision === "deny" ? c.approval.denied : decision === "allow_session" ? c.approval.allowedSession : c.approval.allowed;
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1 text-[11px] font-medium",
        link ? "underline underline-offset-2" : "rounded px-1.5 py-0.5",
        decision === "deny" ? "text-destructive" : "text-emerald-700 dark:text-emerald-300",
        !link && (decision === "deny" ? "bg-destructive/10" : "bg-emerald-500/12"),
      )}
    >
      {decision === "deny" ? <X className="size-3" aria-hidden="true" /> : <Check className="size-3" aria-hidden="true" />}
      {label}
    </span>
  );
}
