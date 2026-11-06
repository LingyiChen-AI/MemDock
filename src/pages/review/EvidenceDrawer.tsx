import * as React from "react";
import { Check, Copy } from "lucide-react";
import { suggestionContext } from "@/api/extract";
import type { ContextTurn, Suggestion, SuggestionContext } from "@/api/types";
import { ChatRow, ChatText } from "@/components/chat/ChatMessage";
import { ChatSkeleton } from "@/components/chat/ChatSkeleton";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { useI18n } from "@/i18n/I18nProvider";
import { formatSnapshotTime } from "@/pages/memory/HistoryDrawer";
import { fmt } from "@/pages/memory/paths";
import { ToolTag } from "@/pages/memory/ToolTag";
import { errorMessage } from "@/pages/skills/skillUtils";

export interface EvidenceDrawerProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  suggestion: Suggestion | null;
}

/** First occurrence of the quote in the turn text; whitespace runs match any whitespace run. */
function locate(text: string, quote: string): [number, number] | null {
  const words = quote.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return null;
  const re = new RegExp(words.map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("\\s+"));
  const m = re.exec(text);
  return m ? [m.index, m.index + m[0].length] : null;
}

/** The turn the quote came from: a user turn first, then an assistant turn; -1 if none contains it. */
function findHit(turns: ContextTurn[], quote: string): number {
  for (const role of ["user", "assistant"] as const) {
    const i = turns.findIndex((x) => x.role === role && locate(x.text, quote) !== null);
    if (i >= 0) return i;
  }
  return -1;
}

const shortId = (id: string) => (id.length > 10 ? `${id.slice(0, 8)}…` : id);

function CopyId({ id }: { id: string }) {
  const { t } = useI18n();
  const r = t.review;
  const [copied, setCopied] = React.useState(false);
  const timer = React.useRef<number | undefined>(undefined);
  React.useEffect(() => () => window.clearTimeout(timer.current), []);
  const copy = () => {
    void navigator.clipboard
      ?.writeText(id)
      .then(() => {
        setCopied(true);
        window.clearTimeout(timer.current);
        timer.current = window.setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});
  };
  const Icon = copied ? Check : Copy;
  return (
    <button
      type="button"
      onClick={copy}
      title={id}
      aria-label={copied ? r.copied : r.copySession}
      className="inline-flex cursor-pointer items-center gap-1 rounded-sm font-mono text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
    >
      <span>{shortId(id)}</span>
      <Icon className="size-3.5" aria-hidden="true" />
    </button>
  );
}

function Turn({ turn, quote, hit }: { turn: ContextTurn; quote: string; hit: boolean }) {
  const { t } = useI18n();
  const r = t.review;
  const user = turn.role === "user";
  return (
    <ChatRow
      role={turn.role}
      tool={turn.tool}
      name={user ? r.roleUser : turn.tool === "claude" ? "Claude Code" : "Codex"}
      time={formatSnapshotTime(turn.ts)}
      highlighted={hit}
      data-hit={hit ? "true" : undefined}
      badge={hit ? <span className="rounded bg-warning/15 px-1.5 text-[10px] font-medium text-warning">{r.hitBadge}</span> : null}
    >
      <ChatText text={turn.text} expandLabel={r.expand} collapseLabel={r.collapse} highlight={hit ? quote : undefined} />
    </ChatRow>
  );
}

function Group({ ctx, index, total }: { ctx: SuggestionContext; index: number; total: number }) {
  const { t } = useI18n();
  const r = t.review;
  const { evidence, turns } = ctx;
  // The turn that contains the quote; if no turn does, nothing is highlighted.
  const hitIndex = findHit(turns, evidence.quote);
  return (
    <section className="space-y-3">
      <header className="flex flex-col gap-1.5 rounded-lg border border-border bg-muted/40 px-3 py-2.5">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
          {total > 1 ? <span className="font-semibold text-foreground">{fmt(r.evidenceN, { n: index + 1, total })}</span> : null}
          <ToolTag tool={evidence.tool} />
          <span className="inline-flex items-center gap-1">
            {r.sessionLabel}
            <CopyId id={evidence.session_id} />
          </span>
          <span className="tabular-nums">{formatSnapshotTime(evidence.ts)}</span>
        </div>
        {evidence.quote ? <p className="line-clamp-2 text-xs text-foreground/80">{fmt(r.quoteLine, { quote: evidence.quote })}</p> : null}
      </header>
      {turns.length > 0 && hitIndex < 0 ? <p className="text-xs text-warning">{r.quoteNotFound}</p> : null}
      {turns.length === 0 ? (
        <p className="rounded-md bg-muted px-3 py-2 text-sm text-muted-foreground">{r.drawerEmpty}</p>
      ) : (
        <ol className="flex flex-col gap-4" aria-label={fmt(r.sessionId, { id: shortId(evidence.session_id) })}>
          {turns.map((turn, i) => (
            <Turn key={`${turn.ts}-${i}`} turn={turn} quote={evidence.quote} hit={i === hitIndex} />
          ))}
        </ol>
      )}
    </section>
  );
}

export function EvidenceDrawer({ open, onOpenChange, suggestion }: EvidenceDrawerProps) {
  const { t } = useI18n();
  const r = t.review;
  const id = suggestion?.id ?? null;
  const [data, setData] = React.useState<SuggestionContext[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!open || !id) return;
    let alive = true;
    setData(null);
    setError(null);
    suggestionContext(id).then(
      (d) => alive && setData(d),
      (e) => alive && setError(errorMessage(e)),
    );
    return () => {
      alive = false;
    };
  }, [open, id]);

  // Once the turns render, bring the quoted message into view.
  const bodyRef = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    if (!data?.length) return;
    const id = window.setTimeout(() => {
      bodyRef.current?.querySelector('[data-hit="true"]')?.scrollIntoView({ block: "center" });
    }, 80);
    return () => window.clearTimeout(id);
  }, [data]);

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="w-[min(960px,94vw)] gap-0 data-[side=right]:w-[min(960px,94vw)] data-[side=right]:sm:max-w-none">
        <SheetHeader className="border-b border-border px-5 py-4 pr-12">
          <SheetTitle className="text-base font-semibold">{r.drawerTitle}</SheetTitle>
          <SheetDescription className="text-xs">{r.drawerDesc}</SheetDescription>
        </SheetHeader>
        <div ref={bodyRef} className="min-h-0 flex-1 space-y-8 overflow-y-auto p-5">
          {error ? (
            <p role="alert" className="text-sm text-destructive">
              {fmt(r.drawerFailed, { message: error })}
            </p>
          ) : data === null ? (
            <ChatSkeleton label={r.drawerLoading} rows={5} />
          ) : data.length === 0 ? (
            <p className="rounded-md bg-muted px-3 py-2 text-sm text-muted-foreground">{r.drawerEmpty}</p>
          ) : (
            data.map((ctx, i) => <Group key={`${ctx.evidence.session_id}-${ctx.evidence.ts}-${i}`} ctx={ctx} index={i} total={data.length} />)
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}
