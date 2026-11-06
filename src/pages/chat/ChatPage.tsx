import * as React from "react";
import { Check, Copy, Cpu, Folder, Loader2, MessagesSquare, Power, Shield, Square, SquarePen, X } from "lucide-react";
import { liveInterrupt, liveRespond, liveSend, liveStop, type LiveAgent, type LiveSessionInfo, type LiveState, type PermissionMode } from "@/api/live";
import { ToolLogo } from "@/components/brand/ToolLogo";
import { ChatSkeleton } from "@/components/chat/ChatSkeleton";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type { Messages } from "@/i18n";
import { fmt } from "@/pages/memory/paths";
import { errorMessage } from "@/pages/skills/skillUtils";
import { formatTime, resumeCommand } from "@/pages/sessions/sessionsUtils";
import { isActive, relativeTime } from "./chatPrefs";
import { ChatComposer } from "./Composer";
import { Conversation } from "./Conversation";
import { closeLive, selectListSlice, selectLive, shallowObjectEqual, useLiveConv, useLiveStore, type LiveConv } from "./liveStore";

type C = Messages["chat"];

export function AgentBadge({ agent, c }: { agent: LiveAgent; c: C }) {
  return (
    <span
      className={cn(
        "inline-flex h-5 shrink-0 items-center gap-1 rounded px-1.5 text-[11px] font-medium",
        agent === "claude" ? "bg-orange-500/12 text-orange-700 dark:text-orange-300" : "bg-sky-500/12 text-sky-700 dark:text-sky-300",
      )}
    >
      <ToolLogo tool={agent} className="size-3" aria-hidden="true" />
      {c.agents[agent]}
    </span>
  );
}

const STATE_TONE: Record<LiveState, string> = {
  starting: "bg-muted text-muted-foreground",
  idle: "bg-muted text-muted-foreground",
  running: "bg-emerald-500/12 text-emerald-700 dark:text-emerald-300",
  waiting_approval: "bg-amber-500/20 text-amber-800 ring-1 ring-amber-500/50 dark:text-amber-200",
  exited: "bg-transparent text-muted-foreground/70 ring-1 ring-border",
};

export function StatePill({ state, c }: { state: LiveState; c: C }) {
  return (
    <span className={cn("inline-flex h-5 shrink-0 items-center gap-1 rounded-full px-2 text-[11px] font-medium", STATE_TONE[state])}>
      {state === "running" ? <span className="size-1.5 animate-pulse rounded-full bg-emerald-500 motion-reduce:animate-none" aria-hidden="true" /> : null}
      {state === "waiting_approval" ? <span className="size-1.5 rounded-full bg-amber-500" aria-hidden="true" /> : null}
      {state === "starting" ? <Loader2 className="size-3 animate-spin motion-reduce:animate-none" aria-hidden="true" /> : null}
      {c.states[state]}
    </span>
  );
}

function PermissionPill({ mode, c }: { mode: PermissionMode; c: C }) {
  return (
    <span
      title={c.permissionHints[mode]}
      className={cn(
        "inline-flex h-6 shrink-0 items-center gap-1 rounded-md px-2 text-xs [&_svg]:size-3",
        mode === "bypass" ? "bg-destructive/10 font-medium text-destructive" : "bg-muted/70 text-muted-foreground",
      )}
    >
      <Shield aria-hidden="true" />
      {c.permissions[mode]}
    </span>
  );
}

function MetaChip({ icon, title, children }: { icon: React.ReactNode; title?: string; children: React.ReactNode }) {
  return (
    <span
      title={title}
      className="inline-flex h-6 max-w-full min-w-0 items-center gap-1 rounded-md bg-muted/70 px-2 font-mono text-[11px] text-muted-foreground [&_svg]:size-3 [&_svg]:shrink-0"
    >
      {icon}
      <span className="truncate">{children}</span>
    </span>
  );
}

function sessionTitle(info: LiveSessionInfo, c: C): string {
  return info.title.trim() || c.untitled;
}

function baseName(p: string): string {
  const parts = p.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

export function LiveRow({ info, selected, onSelect, c, now }: { info: LiveSessionInfo; selected: boolean; onSelect: () => void; c: C; now: number }) {
  return (
    <button
      type="button"
      aria-current={selected ? "true" : undefined}
      onClick={onSelect}
      className={cn(
        "flex w-full cursor-pointer flex-col gap-1 rounded-md px-2.5 py-2 text-left outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none",
        selected ? "list-active" : "hover:bg-muted/60",
        info.state === "waiting_approval" && !selected && "bg-amber-500/8",
      )}
    >
      <div className="flex min-w-0 items-center gap-2">
        <AgentBadge agent={info.agent} c={c} />
        <span className="min-w-0 flex-1 truncate text-sm font-medium">{sessionTitle(info, c)}</span>
        <span className="shrink-0 text-[11px] text-muted-foreground tabular-nums">{relativeTime(info.updated_at, c.time, (ms) => formatTime(ms), now)}</span>
      </div>
      <div className="flex min-w-0 items-center gap-2 text-[11px] text-muted-foreground">
        <span className="min-w-0 flex-1 truncate font-mono" title={info.cwd}>
          {baseName(info.cwd)}
        </span>
        <StatePill state={info.state} c={c} />
      </div>
    </button>
  );
}

function ListSkeleton({ label }: { label: string }) {
  return (
    <div role="status" aria-busy="true" aria-label={label} className="flex flex-col gap-1 p-1">
      {[0, 1, 2, 3].map((r) => (
        <div key={r} className="flex flex-col gap-1.5 rounded-md px-2.5 py-2">
          <div className="flex items-center gap-2">
            <Skeleton className="h-5 w-20 rounded" />
            <Skeleton className={cn("h-4", ["w-32", "w-24", "w-36", "w-28"][r])} />
            <Skeleton className="ml-auto h-3 w-10" />
          </div>
          <Skeleton className="h-3 w-40" />
        </div>
      ))}
      <span className="sr-only">{label}</span>
    </div>
  );
}

/** Re-renders every 30 s so relative times stay fresh. */
function useNow(): number {
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    const h = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(h);
  }, []);
  return now;
}

export function HeaderButton({ label, hint, onClick, disabled, variant = "outline", children }: {
  label: string;
  hint: string;
  onClick: () => void;
  disabled?: boolean;
  variant?: "outline" | "ghost" | "destructive";
  children: React.ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button type="button" size="sm" variant={variant} className="cursor-pointer" disabled={disabled} onClick={onClick} aria-label={label}>
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="bottom">{hint}</TooltipContent>
    </Tooltip>
  );
}

function ConvHeader({ info, c, onError }: { info: LiveSessionInfo; c: C; onError: (e: string | null) => void }) {
  const [copied, setCopied] = React.useState<"idle" | "copied" | "failed">("idle");
  const running = isActive(info.state);
  const exited = info.state === "exited";
  const run = (f: () => Promise<void>) => {
    onError(null);
    f().catch((e) => onError(fmt(c.actionFailed, { error: errorMessage(e) })));
  };
  const copy = async () => {
    if (!info.native_id) return;
    try {
      await navigator.clipboard.writeText(resumeCommand(info.agent, info.cwd, info.native_id));
      setCopied("copied");
    } catch {
      setCopied("failed");
    }
    window.setTimeout(() => setCopied("idle"), 2000);
  };
  return (
    <div className="flex flex-col gap-2 border-b border-border px-4 py-3">
      <div className="flex min-w-0 items-center justify-between gap-3">
        <h2 className="flex min-w-0 items-center gap-2 text-base font-semibold">
          <AgentBadge agent={info.agent} c={c} />
          <span className="truncate">{sessionTitle(info, c)}</span>
          <StatePill state={info.state} c={c} />
        </h2>
        <div className="flex shrink-0 items-center gap-1.5">
          {running ? (
            <HeaderButton label={c.stop} hint={c.stopHint} onClick={() => run(() => liveInterrupt(info.live_id))}>
              <Square className="fill-current" aria-hidden="true" />
              {c.stop}
            </HeaderButton>
          ) : null}
          <HeaderButton
            label={c.openInTerminal}
            hint={!info.native_id ? c.noNativeId : copied === "copied" ? c.copied : copied === "failed" ? c.copyFailed : c.openInTerminalHint}
            disabled={!info.native_id}
            onClick={() => void copy()}
          >
            {copied === "copied" ? <Check className="text-primary" aria-hidden="true" /> : <Copy aria-hidden="true" />}
            {c.openInTerminal}
          </HeaderButton>
          {!exited ? (
            <HeaderButton label={c.end} hint={c.endHint} variant="destructive" onClick={() => run(() => liveStop(info.live_id))}>
              <Power aria-hidden="true" />
              {c.end}
            </HeaderButton>
          ) : null}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button type="button" size="icon-sm" variant="ghost" className="cursor-pointer" aria-label={c.remove} onClick={() => run(() => closeLive(info.live_id))}>
                <X aria-hidden="true" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom">{c.removeHint}</TooltipContent>
          </Tooltip>
        </div>
      </div>
      <div className="flex min-w-0 flex-wrap items-center gap-1.5">
        <MetaChip icon={<Folder aria-hidden="true" />} title={info.cwd}>
          {info.cwd}
        </MetaChip>
        <MetaChip icon={<Cpu aria-hidden="true" />}>{info.model || c.modelDefault}</MetaChip>
        <PermissionPill mode={info.permission} c={c} />
      </div>
    </div>
  );
}

function LiveComposer({ info, c, onError }: { info: LiveSessionInfo; c: C; onError: (e: string | null) => void }) {
  const exited = info.state === "exited";
  const running = isActive(info.state);
  return (
    <ChatComposer
      c={c}
      agent={info.agent}
      cwd={info.cwd}
      disabled={exited}
      placeholder={exited ? c.inputExited : undefined}
      hint={exited ? c.inputExited : running ? c.inputQueued : null}
      running={running}
      onStop={() => {
        onError(null);
        liveInterrupt(info.live_id).catch((e) => onError(fmt(c.actionFailed, { error: errorMessage(e) })));
      }}
      attached={{ permission: info.permission, model: info.model }}
      onError={onError}
      onSend={(msg) => liveSend(info.live_id, msg.text, msg.images)}
    />
  );
}

/** Subscribes to its own conversation only. */
function SelectedConversation({ info, c }: { info: LiveSessionInfo; c: C }) {
  const lc = useLiveConv(info.live_id);
  return <ConversationPane info={info} lc={lc} c={c} />;
}

export function ConversationPane({ info, lc, c }: { info: LiveSessionInfo; lc: LiveConv | undefined; c: C }) {
  const [error, setError] = React.useState<string | null>(null);
  const respond = async (requestId: string, decision: Parameters<typeof liveRespond>[2]) => {
    setError(null);
    try {
      await liveRespond(info.live_id, requestId, decision);
    } catch (e) {
      setError(fmt(c.actionFailed, { error: errorMessage(e) }));
    }
  };
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ConvHeader info={info} c={c} onError={setError} />
      {error ? (
        <p role="alert" className="border-b border-border px-4 py-2 text-sm text-destructive">
          {error}
        </p>
      ) : null}
      {lc?.loadError ? (
        <p role="alert" className="border-b border-border px-4 py-2 text-sm text-destructive">
          {fmt(c.transcriptFailed, { error: lc.loadError })}
        </p>
      ) : null}
      {!lc || !lc.loaded ? (
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
          <ChatSkeleton label={c.loading} className="px-4 py-4" />
        </div>
      ) : (
        <Conversation scrollKey={`live:${info.live_id}`} conv={lc.conv} agent={info.agent} running={isActive(info.state)} starting={info.state === "starting"} onRespond={respond} />
      )}
      <LiveComposer key={info.live_id} info={info} c={c} onError={setError} />
    </div>
  );
}

function EmptyState({ c, onNew }: { c: C; onNew: () => void }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
      <span className="grid size-12 place-items-center rounded-full bg-primary/10 text-primary">
        <MessagesSquare className="size-6" aria-hidden="true" />
      </span>
      <div className="flex max-w-sm flex-col gap-1">
        <p className="text-base font-semibold">{c.emptyTitle}</p>
        <p className="text-sm text-muted-foreground">{c.emptyHint}</p>
      </div>
      <Button type="button" className="cursor-pointer" onClick={onNew}>
        <SquarePen aria-hidden="true" />
        {c.newChat}
      </Button>
    </div>
  );
}

export interface ChatPageProps {
  /** Opens a new chat (the sessions page shows it as a draft). */
  onNewChat: () => void;
}

export function ChatPage({ onNewChat }: ChatPageProps) {
  const { t } = useI18n();
  const c = t.chat;
  const store = useLiveStore(selectListSlice, shallowObjectEqual);
  const now = useNow();
  const sessions = React.useMemo(() => [...store.sessions].sort((a, b) => b.created_at - a.created_at), [store.sessions]);
  const selected = sessions.find((x) => x.live_id === store.selectedId) ?? null;

  // Open the newest conversation when nothing is selected yet.
  React.useEffect(() => {
    if (!store.selectedId && sessions.length > 0) selectLive(sessions[0].live_id);
  }, [store.selectedId, sessions]);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-wrap items-center justify-between gap-3 border-b border-border px-6 py-4">
        <div className="flex flex-col gap-0.5">
          <h1 className="flex items-center gap-2 text-lg font-semibold">
            <MessagesSquare className="size-5 text-muted-foreground" aria-hidden="true" />
            {c.title}
          </h1>
          <p className="text-xs text-muted-foreground">{c.subtitle}</p>
        </div>
        <Button type="button" size="sm" className="cursor-pointer" onClick={onNewChat}>
          <SquarePen aria-hidden="true" />
          {c.newChat}
        </Button>
      </header>

      <div className="flex min-h-0 flex-1">
        <aside aria-label={c.listLabel} className="flex w-[320px] min-w-[260px] shrink-0 flex-col border-r border-border">
          <div className="min-h-0 flex-1 overflow-y-auto p-2">
            {store.listError ? (
              <p role="alert" className="p-2 text-sm text-destructive">
                {fmt(c.listFailed, { error: store.listError })}
              </p>
            ) : null}
            {!store.ready ? (
              <ListSkeleton label={c.loading} />
            ) : sessions.length === 0 ? (
              <p className="p-2 text-sm text-muted-foreground">{c.emptyList}</p>
            ) : (
              <ul className="flex flex-col gap-0.5">
                {sessions.map((x) => (
                  <li key={x.live_id}>
                    <LiveRow info={x} selected={x.live_id === store.selectedId} onSelect={() => selectLive(x.live_id)} c={c} now={now} />
                  </li>
                ))}
              </ul>
            )}
          </div>
        </aside>

        <main className="flex min-w-0 flex-1 flex-col">
          {selected ? (
            <SelectedConversation key={selected.live_id} info={selected} c={c} />
          ) : store.ready && sessions.length === 0 ? (
            <EmptyState c={c} onNew={onNewChat} />
          ) : (
            <div className="grid flex-1 place-items-center p-6 text-sm text-muted-foreground">{store.ready ? c.selectHint : null}</div>
          )}
        </main>
      </div>

    </div>
  );
}
