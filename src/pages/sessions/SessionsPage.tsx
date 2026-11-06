import * as React from "react";
import { ChatSkeleton } from "@/components/chat/ChatSkeleton";
import { ArrowLeftRight, Check, Copy, Cpu, Folder, Hash, Info, Loader2, MessagesSquare, RefreshCw, Search, Square, SquarePen, SquareTerminal, TerminalSquare, TriangleAlert, X } from "lucide-react";
import { homeDir } from "@tauri-apps/api/path";
import { liveInterrupt, liveRespond, liveSend, type LiveAgent, type LiveSessionInfo } from "@/api/live";
import { TerminalView } from "@/pages/chat/TerminalView";
import {
  addShell,
  closeTerminal,
  initialMode,
  releaseGuiProcess,
  removeShell,
  saveChatMode,
  shellOptions,
  terminalKey,
  terminalOptions,
  useShells,
  useTerminal,
  type ChatMode,
  type ShellTerm,
} from "@/pages/chat/terminalStore";
import { Conversation } from "@/pages/chat/Conversation";
import { HeaderButton, LiveRow, StatePill } from "@/pages/chat/ChatPage";
import { ChatComposer, type ComposerMessage, type ProjectChoice } from "@/pages/chat/Composer";
import { isActive, loadChatPrefs, rememberLast } from "@/pages/chat/chatPrefs";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { ensureTranscript, selectLive, startLive, useLiveStore, type LiveConv } from "@/pages/chat/liveStore";
import { convertSession, convertTargetDir, listSessions, renameSession, sessionPreview } from "@/api/sessions";
import { Pencil } from "lucide-react";
import type { ConvertResult, SessionPreview, SessionSummary, Tool } from "@/api/types";
import { ToolLogo } from "@/components/brand/ToolLogo";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type { Messages } from "@/i18n";
import { fmt } from "@/pages/memory/paths";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { asCommandError, errorMessage } from "@/pages/skills/skillUtils";
import { Timeline } from "./Timeline";
import { filterSessions, formatTime, groupByProject, otherTool, resumeCommand, type ToolFilter } from "./sessionsUtils";

type S = Messages["sessions"];
type C = Messages["chat"];

const LIVE_PREFIX = "live:";
const DRAFT_KEY = "draft";
const SHELL_PREFIX = "term:";

/** Same folder, ignoring slash style, a trailing slash and (Windows) case. */
function samePath(a: string, b: string): boolean {
  const n = (p: string) => p.replace(/\\/g, "/").replace(/\/+$/, "");
  const x = n(a);
  const y = n(b);
  return /^[a-zA-Z]:/.test(x) || /^[a-zA-Z]:/.test(y) ? x.toLowerCase() === y.toLowerCase() : x === y;
}

/** Small icon button with a tooltip (project group header actions). */
function IconAction({ label, onClick, children }: { label: string; onClick: () => void; children: React.ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label={label}
          onClick={onClick}
          className="grid size-6 shrink-0 cursor-pointer place-items-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring [&_svg]:size-3.5"
        >
          {children}
        </button>
      </TooltipTrigger>
      <TooltipContent side="bottom">{label}</TooltipContent>
    </Tooltip>
  );
}

/** A plain shell terminal in the list: badge, shell name, folder and state; a close button. */
function ShellRow({ shell, selected, onSelect, onClose, c }: { shell: ShellTerm; selected: boolean; onSelect: () => void; onClose: () => void; c: C }) {
  const entry = useTerminal(shell.key);
  const name = entry?.command || c.shellName;
  const status = entry?.status ?? "starting";
  const state = status === "running" ? c.terminalRunning : status === "starting" ? c.terminalStartingShort : c.terminalEnded;
  return (
    <div className="group/shell relative">
      <button
        type="button"
        aria-current={selected ? "true" : undefined}
        onClick={onSelect}
        className={cn(
          "flex w-full cursor-pointer flex-col gap-1 rounded-md px-2.5 py-2 pr-9 text-left outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none",
          selected ? "list-active" : "hover:bg-muted/60",
        )}
      >
        <div className="flex min-w-0 items-center gap-2">
          <span className="inline-flex h-5 shrink-0 items-center gap-1 rounded bg-foreground/8 px-1.5 text-[11px] font-medium text-foreground/80">
            <SquareTerminal className="size-3" aria-hidden="true" />
            {c.terminalBadge}
          </span>
          <span className="min-w-0 flex-1 truncate text-sm font-medium">{name}</span>
        </div>
        <div className="flex min-w-0 items-center gap-2 text-[11px] text-muted-foreground">
          <span className="min-w-0 flex-1 truncate font-mono" title={shell.cwd}>
            {shell.cwd}
          </span>
          <span className={cn("shrink-0", status === "running" && "text-emerald-700 dark:text-emerald-400")}>{state}</span>
        </div>
      </button>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            aria-label={c.terminalClose}
            onClick={onClose}
            className="absolute top-2 right-2 grid size-6 cursor-pointer place-items-center rounded-md text-muted-foreground opacity-60 outline-none transition-opacity hover:bg-muted hover:text-foreground hover:opacity-100 focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring group-hover/shell:opacity-100"
          >
            <X className="size-3.5" aria-hidden="true" />
          </button>
        </TooltipTrigger>
        <TooltipContent side="right">{c.terminalCloseHint}</TooltipContent>
      </Tooltip>
    </div>
  );
}

/** A plain shell terminal in the right pane: full height, no chat / terminal switch, no composer. */
function ShellPane({ shell, onClose, c }: { shell: ShellTerm; onClose: () => void; c: C }) {
  const entry = useTerminal(shell.key);
  const [opts] = React.useState(() => shellOptions(shell.cwd));
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center justify-between gap-3 border-b border-border p-4">
        <div className="flex min-w-0 flex-col gap-2">
          <h2 className="flex min-w-0 items-center gap-2 text-base font-semibold">
            <span className="inline-flex h-5 shrink-0 items-center gap-1 rounded bg-foreground/8 px-1.5 text-[11px] font-medium text-foreground/80">
              <SquareTerminal className="size-3" aria-hidden="true" />
              {c.terminalBadge}
            </span>
            <span className="truncate">{entry?.command || c.shellName}</span>
          </h2>
          <div className="flex min-w-0 flex-wrap items-center gap-1.5">
            <MetaChip icon={<Folder aria-hidden="true" />} title={shell.cwd} mono>
              {shell.cwd}
            </MetaChip>
          </div>
        </div>
        <HeaderButton label={c.terminalClose} hint={c.terminalCloseHint} onClick={onClose}>
          <X aria-hidden="true" />
          {c.terminalClose}
        </HeaderButton>
      </div>
      <TerminalView key={shell.key} termKey={shell.key} opts={opts} c={c} keepAlive />
    </div>
  );
}

function sessionKey(s: { tool: Tool; path: string }): string {
  return `${s.tool}:${s.path}`;
}

function ToolBadge({ tool, s }: { tool: Tool; s: S }) {
  return (
    <span
      className={cn(
        "inline-flex h-5 shrink-0 items-center gap-1 rounded px-1.5 text-[11px] font-medium",
        tool === "claude" ? "bg-orange-500/12 text-orange-700 dark:text-orange-300" : "bg-sky-500/12 text-sky-700 dark:text-sky-300",
      )}
    >
      <ToolLogo tool={tool} className="size-3" aria-hidden="true" />
      {s.tools[tool]}
    </span>
  );
}

function ToolFilterBar({ value, onChange, s }: { value: ToolFilter; onChange: (v: ToolFilter) => void; s: S }) {
  const options: { value: ToolFilter; label: string }[] = [
    { value: "all", label: s.all },
    { value: "claude", label: s.tools.claude },
    { value: "codex", label: s.tools.codex },
  ];
  return (
    <div role="radiogroup" aria-label={s.title} className="inline-flex h-8 shrink-0 rounded-md border border-border bg-card p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          onClick={() => onChange(o.value)}
          className={cn(
            "cursor-pointer rounded-sm px-2.5 text-xs outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none",
            o.value === value ? "seg-active" : "text-muted-foreground hover:text-foreground",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** Loading state shaped like the list: a project header and a few session rows. */
function SessionListSkeleton({ label }: { label: string }) {
  const titles = ["w-40", "w-28", "w-36", "w-44", "w-32", "w-24"];
  return (
    <div role="status" aria-busy="true" aria-label={label} className="flex flex-col gap-1 p-1">
      {[0, 1].map((g) => (
        <div key={g} className="flex flex-col gap-1">
          <Skeleton className="mx-2.5 mt-3 mb-1 h-3.5 w-24" />
          {[0, 1, 2].map((r) => (
            <div key={r} className="flex flex-col gap-1.5 rounded-md px-2.5 py-2">
              <div className="flex items-center gap-2">
                <Skeleton className="h-5 w-20 rounded" />
                <Skeleton className={cn("h-4", titles[(g * 3 + r) % titles.length])} />
                <Skeleton className="ml-auto h-3 w-10" />
              </div>
              <Skeleton className="h-3 w-32" />
            </div>
          ))}
        </div>
      ))}
      <span className="sr-only">{label}</span>
    </div>
  );
}

function SessionRow({ session, live, selected, onSelect, s, c }: { session: SessionSummary; live?: LiveSessionInfo; selected: boolean; onSelect: () => void; s: S; c: C }) {
  const heading = session.title?.trim() || session.first_prompt || s.untitled;
  return (
    <button
      type="button"
      aria-current={selected ? "true" : undefined}
      onClick={onSelect}
      className={cn(
        "flex w-full cursor-pointer flex-col gap-1 rounded-md px-2.5 py-2 text-left outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none",
        selected ? "list-active" : "hover:bg-muted/60",
      )}
    >
      <div className="flex min-w-0 items-center gap-2">
        <ToolBadge tool={session.tool} s={s} />
        <span className="min-w-0 flex-1 truncate text-sm font-medium">{heading}</span>
        {live && live.state !== "exited" && live.state !== "idle" ? <StatePill state={live.state} c={c} /> : null}
        <span className="shrink-0 text-[11px] text-muted-foreground tabular-nums">{formatTime(live ? Math.max(live.updated_at, session.updated_at ?? 0) : session.updated_at)}</span>
      </div>
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-0.5 text-[11px] text-muted-foreground">
        {session.continued ? (
          <span className="rounded bg-muted px-1.5 text-foreground/70" title={s.continuedHint}>
            {s.continued}
          </span>
        ) : null}
        <span>{fmt(s.messages, { n: session.message_count })}</span>
        {session.tool_calls > 0 ? <span>{fmt(s.toolCalls, { n: session.tool_calls })}</span> : null}
        {session.converted_from ? (
          <span className="text-primary">{fmt(s.convertedFrom, { tool: s.tools[session.converted_from.tool] })}</span>
        ) : null}
        {session.converted_to.length > 0 ? (
          <span className="text-primary">{fmt(s.convertedTo, { tool: s.tools[otherTool(session.tool)] })}</span>
        ) : null}
      </div>
    </button>
  );
}

function CopyButton({ text, s, iconOnly = false }: { text: string; s: S; iconOnly?: boolean }) {
  const [state, setState] = React.useState<"idle" | "copied" | "failed">("idle");
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setState("copied");
    } catch {
      setState("failed");
    }
    window.setTimeout(() => setState("idle"), 2000);
  };
  const label = state === "copied" ? s.copied : state === "failed" ? s.copyFailed : s.copy;
  const icon = state === "copied" ? <Check className="text-primary" aria-hidden="true" /> : <Copy aria-hidden="true" />;
  if (iconOnly) {
    return (
      <Button type="button" variant="ghost" size="icon-xs" className="shrink-0 cursor-pointer" aria-label={label} title={label} onClick={() => void copy()}>
        {icon}
      </Button>
    );
  }
  return (
    <Button type="button" variant="outline" size="sm" className="shrink-0 cursor-pointer" onClick={() => void copy()}>
      {icon}
      {label}
    </Button>
  );
}

/** Terminal-style card: label + copy in the header, the command on one scrollable line. */
function CommandCard({ label, command, footer, s }: { label: string; command: string; footer?: React.ReactNode; s: S }) {
  return (
    <div className="overflow-hidden rounded-lg border border-border bg-muted/40">
      <div className="flex items-center gap-2 border-b border-border/70 px-3 py-1.5">
        <TerminalSquare className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{label}</span>
        <CopyButton text={command} s={s} iconOnly />
      </div>
      <div className="overflow-x-auto px-3 py-2.5">
        <code className="font-mono text-[13px] whitespace-nowrap select-all">
          <span className="mr-2 text-primary select-none" aria-hidden="true">
            $
          </span>
          {command}
        </code>
      </div>
      {footer ? <div className="border-t border-border/70 px-3 py-2">{footer}</div> : null}
    </div>
  );
}

/** Always-visible resume commands: this session itself, plus every copy converted from it. */
function ResumeCommands({ session, s }: { session: SessionSummary; s: S }) {
  const lines: { tool: Tool; label: string; command: string }[] = [];
  if (session.converted_from) {
    lines.push({ tool: session.tool, label: fmt(s.resumeHere, { tool: s.tools[session.tool] }), command: resumeCommand(session.tool, session.cwd, session.id) });
  }
  for (const r of session.converted_to) {
    lines.push({ tool: r.tool, label: fmt(s.resumeConverted, { tool: s.tools[r.tool] }), command: resumeCommand(r.tool, session.cwd, r.id) });
  }
  if (lines.length === 0) return null;
  return (
    <div className="flex flex-col gap-2">
      {lines.map((l) => (
        <CommandCard key={l.command} label={l.label} command={l.command} footer={l.tool === "codex" ? <CodexWriterHint s={s} /> : null} s={s} />
      ))}
    </div>
  );
}

function CodexWriterHint({ s }: { s: S }) {
  return (
    <p className="flex items-start gap-1.5 text-[11px] leading-relaxed text-muted-foreground">
      <Info className="mt-0.5 size-3 shrink-0" aria-hidden="true" />
      {s.codexWriterHint}
    </p>
  );
}

function ResultCard({ result, s }: { result: ConvertResult; s: S }) {
  const reason = (w: string) => (w in s.reasons ? s.reasons[w as keyof S["reasons"]] : w);
  const indexWarning = result.warnings.find((w) => w.startsWith("state_db"));
  const otherWarnings = result.warnings.filter((w) => !w.startsWith("state_db"));
  return (
    <section aria-label={fmt(s.doneTitle, { tool: s.tools[result.tool] })} className="flex flex-col gap-3 rounded-lg border border-primary/40 bg-primary/5 p-4">
      <h3 className="flex items-center gap-2 text-sm font-semibold">
        <Check className="size-4 text-primary" aria-hidden="true" />
        {fmt(s.doneTitle, { tool: s.tools[result.tool] })}
      </h3>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
        <dt className="text-muted-foreground">{s.sessionId}</dt>
        <dd className="font-mono break-all select-all">{result.session_id}</dd>
        <dt className="text-muted-foreground">{s.file}</dt>
        <dd className="font-mono break-all">{result.path}</dd>
      </dl>
      <CommandCard
        label={s.resumeHint}
        command={result.resume_command}
        footer={result.tool === "codex" ? <CodexWriterHint s={s} /> : null}
        s={s}
      />
      {result.tool === "codex" ? (
        <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
          <Info className="mt-px size-3.5 shrink-0" aria-hidden="true" />
          {result.indexed ? s.codexIndexed : fmt(s.codexNotIndexed, { reason: reason(indexWarning ?? "state_db_missing") })}
        </p>
      ) : null}
      {otherWarnings.map((w) => (
        <p key={w} className="flex items-start gap-1.5 text-xs text-amber-700 dark:text-amber-300">
          <TriangleAlert className="mt-px size-3.5 shrink-0" aria-hidden="true" />
          {fmt(s.warning, { text: w })}
        </p>
      ))}
    </section>
  );
}

function MetaChip({ icon, title, mono, children }: { icon?: React.ReactNode; title?: string; mono?: boolean; children: React.ReactNode }) {
  return (
    <span
      title={title}
      className={cn(
        "inline-flex h-6 max-w-full min-w-0 items-center gap-1 rounded-md bg-muted/70 px-2 text-xs text-muted-foreground [&_svg]:size-3 [&_svg]:shrink-0",
        mono && "font-mono text-[11px]",
      )}
    >
      {icon}
      <span className="truncate">{children}</span>
    </span>
  );
}

/** What the right pane shows: a session file from disk, a live chat not on disk yet, or both. */
interface Target {
  tool: Tool;
  cwd: string;
  /** Claude session id / Codex thread id (resume id). */
  nativeId: string | null;
  title: string;
  session: SessionSummary | null;
  live: LiveSessionInfo | null;
  /** A new chat that has not started yet (no session, no process). */
  draft?: boolean;
}

/** The session title; with `onSave`, a pencil button edits it in place (Enter saves, Esc cancels). */
function TitleEditor({ title, onSave, s }: { title: string; onSave?: (title: string) => Promise<unknown>; s: S }) {
  const [editing, setEditing] = React.useState(false);
  const [value, setValue] = React.useState(title);
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const start = () => {
    setValue(title);
    setError(null);
    setEditing(true);
  };
  const save = async () => {
    const v = value.trim();
    if (!onSave || !v || v === title) return setEditing(false);
    setSaving(true);
    try {
      await onSave(v);
      setEditing(false);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };
  if (editing) {
    return (
      <span className="flex min-w-0 flex-1 items-center gap-2">
        <Input
          autoFocus
          aria-label={s.renameLabel}
          value={value}
          disabled={saving}
          maxLength={200}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.nativeEvent.isComposing || e.keyCode === 229) return;
            if (e.key === "Enter") void save();
            if (e.key === "Escape") setEditing(false);
          }}
          onBlur={() => void save()}
          className="-my-0.5 h-7 min-w-0 flex-1 rounded-md border-transparent bg-muted/50 px-1.5 text-base font-semibold shadow-none focus-visible:border-ring/40 focus-visible:bg-background md:text-base"
        />
        {saving ? <Loader2 className="size-4 shrink-0 animate-spin motion-reduce:animate-none" aria-hidden="true" /> : null}
        {error ? <span role="alert" className="shrink-0 text-xs font-normal text-destructive">{fmt(s.renameFailed, { error })}</span> : null}
      </span>
    );
  }
  return (
    <span className="group/title flex min-w-0 items-center gap-1">
      <span className="truncate">{title}</span>
      {onSave ? (
        <Button
          type="button"
          size="icon-sm"
          variant="ghost"
          aria-label={s.rename}
          title={s.renameHint}
          onClick={start}
          className="size-7 shrink-0 cursor-pointer text-muted-foreground opacity-60 group-hover/title:opacity-100 focus-visible:opacity-100"
        >
          <Pencil className="size-3.5" aria-hidden="true" />
        </Button>
      ) : null}
    </span>
  );
}

/** GUI (bubbles) / terminal (the real CLI) switch. */
function ModeSwitch({ value, onChange, disabled, c }: { value: ChatMode; onChange: (m: ChatMode) => void; disabled?: boolean; c: C }) {
  const options: { value: ChatMode; label: string; hint: string; icon: React.ReactNode }[] = [
    { value: "gui", label: c.modeGui, hint: c.modeGuiHint, icon: <MessagesSquare aria-hidden="true" /> },
    { value: "terminal", label: c.modeTerminal, hint: c.modeTerminalHint, icon: <SquareTerminal aria-hidden="true" /> },
  ];
  return (
    <div role="radiogroup" aria-label={c.modeLabel} className="inline-flex h-8 shrink-0 rounded-md border border-border bg-card p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          title={o.hint}
          disabled={disabled}
          onClick={() => o.value !== value && onChange(o.value)}
          className={cn(
            "inline-flex cursor-pointer items-center gap-1.5 rounded-sm px-2.5 text-xs outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60 motion-reduce:transition-none [&_svg]:size-3.5",
            o.value === value ? "seg-active" : "text-muted-foreground hover:text-foreground",
          )}
        >
          {o.icon}
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** The session history up to `startedAt` (ms): what was on disk before a live process took over. */
export function historyBefore(preview: SessionPreview, startedAt: number): SessionPreview {
  const timeline = preview.timeline.filter((it) => {
    const t = Date.parse(it.ts);
    return Number.isNaN(t) || t < startedAt;
  });
  if (timeline.length === preview.timeline.length) return preview;
  return { ...preview, timeline, timeline_total: preview.timeline_total - (preview.timeline.length - timeline.length) };
}

function ChatPane({
  target,
  preview,
  error,
  result,
  converting,
  onConvert,
  onRenamed,
  onRefresh,
  onDraftChange,
  projects,
  lc,
  s,
  c,
}: {
  target: Target;
  preview: SessionPreview | null;
  error: string | null;
  result: ConvertResult | null;
  converting: boolean;
  onConvert: () => void;
  onRenamed: () => void;
  /** Re-read the session file (preview + list), after the terminal wrote to it. */
  onRefresh: () => void;
  /** Draft only: the agent / folder picked in the composer. */
  onDraftChange?: (patch: { agent?: LiveAgent; cwd?: string }) => void;
  projects: ProjectChoice[];
  lc: LiveConv | undefined;
  s: S;
  c: C;
}) {
  const { session, live } = target;
  const [actionError, setActionError] = React.useState<string | null>(null);
  const termKey = terminalKey(target);
  const [mode, setMode] = React.useState<ChatMode>(() => (target.cwd ? initialMode(termKey, live) : "gui"));
  const [switching, setSwitching] = React.useState(false);
  const [confirmStop, setConfirmStop] = React.useState<((ok: boolean) => void) | null>(null);
  // Options are fixed when the terminal opens; later list refreshes must not restart it.
  const [termOpts, setTermOpts] = React.useState(() => terminalOptions(target));

  const switchMode = async (next: ChatMode) => {
    setActionError(null);
    setSwitching(true);
    try {
      if (next === "terminal") {
        const ok = await releaseGuiProcess(live, () => new Promise<boolean>((resolve) => setConfirmStop(() => resolve)));
        if (!ok) return;
        setTermOpts(terminalOptions(target));
        setMode("terminal");
      } else {
        await closeTerminal(termKey);
        setMode("gui");
        onRefresh();
      }
      saveChatMode(next);
    } catch (e) {
      setActionError(fmt(c.actionFailed, { error: errorMessage(e) }));
    } finally {
      setSwitching(false);
    }
  };
  const answerConfirm = (ok: boolean) => {
    confirmStop?.(ok);
    setConfirmStop(null);
  };
  const targetTool = otherTool(target.tool);
  const stats = preview?.stats;
  const running = live ? isActive(live.state) : false;
  const attached = live !== null && live.state !== "exited";

  // Any message continues the conversation: through the running process, or by resuming it.
  const send = async ({ text, images, permission, model, effort, args }: ComposerMessage) => {
    if (attached && live) return liveSend(live.live_id, text, images);
    await startLive({
      agent: target.tool,
      cwd: target.cwd,
      resume_id: target.nativeId,
      permission,
      model: model || null,
      effort: effort || null,
      extra_args: args,
      prompt: text,
      prompt_images: images,
    });
    if (target.draft) rememberLast({ agent: target.tool, cwd: target.cwd });
  };
  const respond = async (requestId: string, decision: Parameters<typeof liveRespond>[2]) => {
    if (!live) return;
    setActionError(null);
    try {
      await liveRespond(live.live_id, requestId, decision);
    } catch (e) {
      setActionError(fmt(c.actionFailed, { error: errorMessage(e) }));
    }
  };

  // With a live process, the file's messages from its start on are shown by the live view.
  const before = live && preview ? historyBefore(preview, live.created_at) : preview;
  const history = session ? (
    error ? (
      <p role="alert" className="mb-4 text-sm text-destructive">
        {fmt(s.previewFailed, { error })}
      </p>
    ) : !before ? (
      live ? null : <ChatSkeleton label={s.previewLoading} />
    ) : before.timeline.length === 0 ? null : (
      <Timeline bare preview={before} tool={session.tool} />
    )
  ) : null;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-col gap-3 border-b border-border p-4">
        <div className="flex min-w-0 items-start justify-between gap-3">
          <div className="flex min-w-0 flex-col gap-2">
            <h2 className="flex min-w-0 items-center gap-2 text-base font-semibold">
              <ToolBadge tool={target.tool} s={s} />
              <TitleEditor
                title={target.title}
                onSave={session ? (title) => renameSession(session.tool, session.path, title).then(onRenamed) : undefined}
                s={s}
              />
              {live && live.state !== "exited" && live.state !== "idle" ? <StatePill state={live.state} c={c} /> : null}
            </h2>
            <div className="flex min-w-0 flex-wrap items-center gap-1.5">
              <MetaChip icon={<Folder aria-hidden="true" />} title={target.cwd} mono>
                {target.cwd}
              </MetaChip>
              {target.nativeId ? (
                <MetaChip icon={<Hash aria-hidden="true" />} title={target.nativeId} mono>
                  {target.nativeId.slice(0, 8)}
                </MetaChip>
              ) : null}
              {live?.model ? <MetaChip icon={<Cpu aria-hidden="true" />}>{live.model}</MetaChip> : null}
              {stats ? (
                <>
                  <MetaChip icon={<MessagesSquare aria-hidden="true" />}>{fmt(s.messages, { n: stats.user_messages + stats.assistant_messages })}</MetaChip>
                  {stats.mapped_calls > 0 ? (
                    <MetaChip icon={<TerminalSquare aria-hidden="true" />}>{fmt(s.stats.mapped, { n: stats.mapped_calls })}</MetaChip>
                  ) : null}
                  {stats.degraded_calls > 0 ? <MetaChip>{fmt(s.stats.degraded, { n: stats.degraded_calls })}</MetaChip> : null}
                </>
              ) : null}
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <ModeSwitch value={mode} onChange={(m) => void switchMode(m)} disabled={switching || !target.cwd} c={c} />
            {mode === "gui" && running && live ? (
              <HeaderButton
                label={c.stop}
                hint={c.stopHint}
                onClick={() => {
                  setActionError(null);
                  liveInterrupt(live.live_id).catch((e) => setActionError(fmt(c.actionFailed, { error: errorMessage(e) })));
                }}
              >
                <Square className="fill-current" aria-hidden="true" />
                {c.stop}
              </HeaderButton>
            ) : null}
            {session && session.converted_from?.tool !== targetTool ? (
              <Button type="button" variant="outline" size="sm" className="cursor-pointer" disabled={converting || !preview || running} onClick={onConvert}>
                {converting ? <Loader2 className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <ArrowLeftRight aria-hidden="true" />}
                {converting ? s.converting : fmt(s.convertTo, { tool: s.tools[targetTool] })}
              </Button>
            ) : null}
          </div>
        </div>
        {result ? <ResultCard result={result} s={s} /> : session && mode === "gui" ? <ResumeCommands session={session} s={s} /> : null}
      </div>
      {actionError ? (
        <p role="alert" className="border-b border-border px-4 py-2 text-sm text-destructive">
          {actionError}
        </p>
      ) : null}
      <ConfirmDialog
        open={confirmStop !== null}
        title={c.switchConfirmTitle}
        body={c.switchConfirmBody}
        confirmLabel={c.switchConfirm}
        cancelLabel={s.cancel}
        destructive
        onConfirm={() => answerConfirm(true)}
        onCancel={() => answerConfirm(false)}
      />
      {mode === "terminal" ? (
        <TerminalView termKey={termKey} opts={termOpts} c={c} />
      ) : (
        <>
          {live && lc?.loaded ? (
            <Conversation
              conv={lc.conv}
              agent={live.agent}
              running={running}
              starting={live.state === "starting"}
              onRespond={respond}
              prefix={
                history ? (
                  <>
                    {history}
                    <div className="my-5 flex items-center gap-3 text-xs text-muted-foreground">
                      <span className="h-px flex-1 bg-border" />
                      {c.continuedHere}
                      <span className="h-px flex-1 bg-border" />
                    </div>
                  </>
                ) : undefined
              }
            />
          ) : session ? (
            error ? (
              <p role="alert" className="p-4 text-sm text-destructive">
                {fmt(s.previewFailed, { error })}
              </p>
            ) : !preview ? (
              <ChatSkeleton label={s.previewLoading} className="px-4 py-4" />
            ) : (
              <Timeline preview={preview} tool={session.tool} startAtBottom />
            )
          ) : target.draft ? (
            <div className="grid flex-1 place-items-center p-6 text-center text-sm text-muted-foreground">
              <div className="flex max-w-sm flex-col items-center gap-2">
                <MessagesSquare className="size-8 text-muted-foreground/60" aria-hidden="true" />
                <p className="font-medium text-foreground">{c.emptyConv}</p>
                <p className="text-xs">{c.draftHint}</p>
              </div>
            </div>
          ) : (
            <ChatSkeleton label={c.loading} className="px-4 py-4" />
          )}
          <ChatComposer
            key={target.nativeId ?? live?.live_id ?? "new"}
            c={c}
            agent={target.tool}
            cwd={target.cwd}
            placeholder={running ? c.inputQueued : undefined}
            draft={
              target.draft && onDraftChange
                ? { onAgentChange: (agent) => onDraftChange({ agent }), onCwdChange: (cwd) => onDraftChange({ cwd }) }
                : null
            }
            projects={projects}
            running={running}
            onStop={() => {
              if (!live) return;
              setActionError(null);
              liveInterrupt(live.live_id).catch((e) => setActionError(fmt(c.actionFailed, { error: errorMessage(e) })));
            }}
            attached={attached && live ? { permission: live.permission, model: live.model } : null}
            onError={setActionError}
            onSend={send}
          />
        </>
      )}
    </div>
  );
}

interface Confirm {
  session: SessionSummary;
  dir: string;
  force: boolean;
}

/** "New chat" / "new terminal" asked for from outside the page (the sidebar). */
export type NewRequest = "chat" | "terminal";

export interface SessionsPageProps {
  request?: NewRequest | null;
  onRequestHandled?: () => void;
}

/** The 对话 page: every Claude Code / Codex session on disk plus live chats; any of them can be continued. */
export function SessionsPage({ request = null, onRequestHandled = () => {} }: SessionsPageProps = {}) {
  const { t } = useI18n();
  const s = t.sessions;
  const c = t.chat;
  const store = useLiveStore();
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(id);
  }, []);
  const [sessions, setSessions] = React.useState<SessionSummary[] | null>(null);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [loading, setLoading] = React.useState(false);
  const [tool, setTool] = React.useState<ToolFilter>("all");
  const [query, setQuery] = React.useState("");
  const [selectedKey, setSelectedKey] = React.useState<string | null>(null);
  const [preview, setPreview] = React.useState<SessionPreview | null>(null);
  const [previewError, setPreviewError] = React.useState<string | null>(null);
  const [confirm, setConfirm] = React.useState<Confirm | null>(null);
  const [converting, setConverting] = React.useState(false);
  const [convertError, setConvertError] = React.useState<string | null>(null);
  const [result, setResult] = React.useState<ConvertResult | null>(null);
  const [draft, setDraft] = React.useState<{ agent: LiveAgent; cwd: string } | null>(null);
  const shells = useShells();
  const [pageError, setPageError] = React.useState<string | null>(null);

  const load = React.useCallback(async (quiet = false) => {
    if (!quiet) setLoading(true);
    try {
      setSessions(await listSessions());
      setLoadError(null);
    } catch (e) {
      setLoadError(errorMessage(e));
    } finally {
      setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    void load();
  }, [load]);

  const selected = React.useMemo(
    () => sessions?.find((x) => sessionKey(x) === selectedKey) ?? null,
    [sessions, selectedKey],
  );

  // Newest live process per resume id; live chats whose session file is not listed yet.
  const liveByNative = React.useMemo(() => {
    const m = new Map<string, LiveSessionInfo>();
    for (const x of [...store.sessions].sort((a, b) => a.created_at - b.created_at)) if (x.native_id) m.set(x.native_id, x);
    return m;
  }, [store.sessions]);
  const knownIds = React.useMemo(() => new Set((sessions ?? []).map((x) => x.id)), [sessions]);
  const liveOnly = React.useMemo(
    () =>
      [...store.sessions]
        .filter((x) => !x.native_id || !knownIds.has(x.native_id))
        .filter((x) => !x.native_id || liveByNative.get(x.native_id) === x)
        .sort((a, b) => b.created_at - a.created_at),
    [store.sessions, knownIds, liveByNative],
  );
  const selectedShell = selectedKey?.startsWith(SHELL_PREFIX) ? (shells.find((x) => x.key === selectedKey.slice(SHELL_PREFIX.length)) ?? null) : null;
  const selectedLive = selectedKey?.startsWith(LIVE_PREFIX)
    ? (store.sessions.find((x) => x.live_id === selectedKey.slice(LIVE_PREFIX.length)) ?? null)
    : selected
      ? (liveByNative.get(selected.id) ?? null)
      : null;

  // A chat started from a draft (or elsewhere) becomes the selection.
  const lastLiveSel = React.useRef(store.selectedId);
  React.useEffect(() => {
    if (store.selectedId && store.selectedId !== lastLiveSel.current) setSelectedKey(LIVE_PREFIX + store.selectedId);
    lastLiveSel.current = store.selectedId;
  }, [store.selectedId]);

  // Once a new chat's session file shows up in the list, select that entry instead.
  React.useEffect(() => {
    if (!selectedKey?.startsWith(LIVE_PREFIX) || !sessions) return;
    const info = store.sessions.find((x) => x.live_id === selectedKey.slice(LIVE_PREFIX.length));
    const match = info?.native_id ? sessions.find((x) => x.id === info.native_id) : undefined;
    if (match) setSelectedKey(sessionKey(match));
  }, [selectedKey, sessions, store.sessions]);

  // Live chats not on the list yet: reload the list (quietly) when one finishes a turn.
  const pendingFiles = liveOnly.filter((x) => x.native_id && x.state === "idle").map((x) => x.live_id).join(",");
  React.useEffect(() => {
    if (pendingFiles) void load(true);
  }, [pendingFiles, load]);

  React.useEffect(() => {
    if (selectedLive) void ensureTranscript(selectedLive.live_id);
  }, [selectedLive]);

  // Reload the preview whenever the selected file changes.
  const selectedPath = selected?.path;
  const selectedTool = selected?.tool;
  // Bumped to re-read the same file (it grew while the terminal had it); keeps the old preview meanwhile.
  const [previewNonce, setPreviewNonce] = React.useState(0);
  const shownPath = React.useRef<string | undefined>(undefined);
  React.useEffect(() => {
    if (!selectedPath || !selectedTool) return;
    let alive = true;
    if (shownPath.current !== selectedPath) setPreview(null);
    shownPath.current = selectedPath;
    setPreviewError(null);
    sessionPreview(selectedTool, selectedPath)
      .then((p) => alive && setPreview(p))
      .catch((e) => alive && setPreviewError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [selectedPath, selectedTool, previewNonce]);

  const visible = React.useMemo(() => filterSessions(sessions ?? [], tool, query), [sessions, tool, query]);
  const groups = React.useMemo(() => groupByProject(visible), [visible]);
  // Folders the composer's /cwd offers: every project, most recent first.
  const projects: ProjectChoice[] = React.useMemo(() => {
    const out: ProjectChoice[] = [];
    for (const g of groupByProject(sessions ?? [])) if (g.cwd && !out.some((p) => samePath(p.path, g.cwd))) out.push({ path: g.cwd, name: g.name });
    return out;
  }, [sessions]);
  // Shell terminals sit at the top of their folder's group; the rest in a "terminals" group.
  const looseShells = shells.filter((x) => !groups.some((g) => samePath(g.cwd, x.cwd)));

  const resetPane = () => {
    setResult(null);
    setConvertError(null);
    setPageError(null);
  };
  /** Opens a new, not yet started chat in the right pane. */
  const openDraft = (cwd?: string) => {
    const p = loadChatPrefs();
    setDraft({ agent: p.agent, cwd: cwd ?? p.cwd });
    setSelectedKey(DRAFT_KEY);
    resetPane();
  };
  const openShell = (cwd: string) => {
    const key = addShell(cwd);
    setSelectedKey(SHELL_PREFIX + key);
    resetPane();
  };
  const openHomeShell = async () => {
    try {
      openShell(await homeDir());
    } catch (e) {
      setPageError(fmt(c.actionFailed, { error: errorMessage(e) }));
    }
  };
  const closeShell = (key: string) => {
    if (selectedKey === SHELL_PREFIX + key) setSelectedKey(null);
    void removeShell(key);
  };
  const shellRow = (x: ShellTerm) => (
    <li key={x.key}>
      <ShellRow
        shell={x}
        selected={selectedKey === SHELL_PREFIX + x.key}
        onSelect={() => {
          setSelectedKey(SHELL_PREFIX + x.key);
          resetPane();
        }}
        onClose={() => closeShell(x.key)}
        c={c}
      />
    </li>
  );

  // Requests from the sidebar ("new chat" / "new terminal").
  const openDraftRef = React.useRef(openDraft);
  openDraftRef.current = openDraft;
  const openHomeShellRef = React.useRef(openHomeShell);
  openHomeShellRef.current = openHomeShell;
  React.useEffect(() => {
    if (!request) return;
    if (request === "chat") openDraftRef.current();
    else void openHomeShellRef.current();
    onRequestHandled();
  }, [request, onRequestHandled]);

  const select = (x: SessionSummary) => {
    setSelectedKey(sessionKey(x));
    setResult(null);
    setConvertError(null);
  };
  const selectLiveRow = (x: LiveSessionInfo) => {
    selectLive(x.live_id);
    setSelectedKey(LIVE_PREFIX + x.live_id);
    setResult(null);
    setConvertError(null);
  };

  const target: Target | null = selectedKey === DRAFT_KEY && draft
    ? { tool: draft.agent, cwd: draft.cwd, nativeId: null, title: c.untitled, session: null, live: null, draft: true }
    : selected
    ? {
        tool: selected.tool,
        cwd: selected.cwd,
        nativeId: selected.id,
        title: selected.title?.trim() || selected.first_prompt || s.untitled,
        session: selected,
        live: selectedLive,
      }
    : selectedLive
      ? {
          tool: selectedLive.agent,
          cwd: selectedLive.cwd,
          nativeId: selectedLive.native_id,
          title: selectedLive.title.trim() || c.untitled,
          session: null,
          live: selectedLive,
        }
      : null;

  const askConvert = async () => {
    if (!selected) return;
    setConvertError(null);
    try {
      const dir = await convertTargetDir(selected.tool, selected.path);
      setConfirm({ session: selected, dir, force: selected.converted_to.length > 0 });
    } catch (e) {
      setConvertError(errorMessage(e));
    }
  };

  const runConvert = async (c: Confirm) => {
    setConfirm(null);
    setConverting(true);
    try {
      const r = await convertSession(c.session.tool, c.session.path, c.force);
      setResult(r);
      await load();
    } catch (e) {
      const ce = asCommandError(e);
      if (ce?.code === "already_converted") {
        // The list was stale: ask again, now as a repeat conversion.
        setConfirm({ ...c, force: true });
      } else if (ce?.code === "empty_session") {
        setConvertError(s.empty_session);
      } else {
        setConvertError(errorMessage(e));
      }
    } finally {
      setConverting(false);
    }
  };

  const confirmTarget = confirm ? s.tools[otherTool(confirm.session.tool)] : "";
  const confirmBody = confirm
    ? fmt(s.confirmBody, { tool: confirmTarget, dir: confirm.dir }) +
      (confirm.force
        ? fmt(s.confirmAgain, { ids: confirm.session.converted_to.map((r) => r.id).join(", ") || "…" })
        : "")
    : "";

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
        <div className="flex items-center gap-2">
          <Button type="button" variant="outline" size="sm" disabled={loading} onClick={() => void load()} className="cursor-pointer">
            <RefreshCw className={cn("size-3.5", loading && "animate-spin motion-reduce:animate-none")} aria-hidden="true" />
            {s.refresh}
          </Button>
          <Button type="button" variant="outline" size="sm" className="cursor-pointer" onClick={() => void openHomeShell()}>
            <SquareTerminal aria-hidden="true" />
            {c.newTerminal}
          </Button>
          <Button type="button" size="sm" className="cursor-pointer" onClick={() => openDraft()}>
            <SquarePen aria-hidden="true" />
            {c.newChat}
          </Button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1">
        <aside className="flex w-[300px] min-w-[260px] shrink-0 flex-col border-r border-border">
          <div className="flex flex-col gap-2 border-b border-border p-3">
            <div className="relative">
              <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
              <Input
                type="search"
                aria-label={s.search}
                placeholder={s.search}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                className="pl-8"
              />
            </div>
            <div className="flex items-center justify-between gap-2">
              <ToolFilterBar value={tool} onChange={setTool} s={s} />
              {sessions ? <span className="text-xs text-muted-foreground">{fmt(s.count, { n: visible.length })}</span> : null}
            </div>
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto p-2">
            {looseShells.length > 0 ? (
              <section aria-label={c.terminals} className="mb-2">
                <h2 className="px-2.5 pt-2 pb-1 text-xs font-semibold">{c.terminals}</h2>
                <ul className="flex flex-col gap-0.5">{looseShells.map(shellRow)}</ul>
              </section>
            ) : null}
            {liveOnly.length > 0 ? (
              <section aria-label={c.newChats} className="mb-2">
                <h2 className="px-2.5 pt-2 pb-1 text-xs font-semibold">{c.newChats}</h2>
                <ul className="flex flex-col gap-0.5">
                  {liveOnly.map((x) => (
                    <li key={x.live_id}>
                      <LiveRow info={x} selected={selectedKey === LIVE_PREFIX + x.live_id} onSelect={() => selectLiveRow(x)} c={c} now={now} />
                    </li>
                  ))}
                </ul>
              </section>
            ) : null}
            {loadError ? (
              <p role="alert" className="p-2 text-sm text-destructive">
                {fmt(s.loadFailed, { error: loadError })}
              </p>
            ) : sessions === null ? (
              <SessionListSkeleton label={s.loading} />
            ) : sessions.length === 0 ? (
              <div className="flex flex-col gap-1 p-2">
                <p className="text-sm font-medium">{s.empty}</p>
                <p className="text-xs text-muted-foreground">{s.emptyHint}</p>
              </div>
            ) : groups.length === 0 ? (
              <p className="p-2 text-sm text-muted-foreground">{s.noMatch}</p>
            ) : (
              groups.map((g) => (
                <section key={g.projectId} aria-label={g.name} className="group/project mb-2">
                  <div className="flex min-h-7 items-center gap-1 pt-1 pr-1 pl-2.5">
                    <h2 className="flex min-w-0 flex-1 items-baseline gap-2 text-xs font-semibold" title={g.cwd}>
                      <span className="truncate">{g.name}</span>
                      <span className="font-normal text-muted-foreground">{g.sessions.length}</span>
                    </h2>
                    {g.cwd ? (
                      <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover/project:opacity-100 focus-within:opacity-100 motion-reduce:transition-none">
                        <IconAction label={c.newSessionHere} onClick={() => openDraft(g.cwd)}>
                          <SquarePen aria-hidden="true" />
                        </IconAction>
                        <IconAction label={c.newTerminalHere} onClick={() => openShell(g.cwd)}>
                          <SquareTerminal aria-hidden="true" />
                        </IconAction>
                      </span>
                    ) : null}
                  </div>
                  <ul className="flex flex-col gap-0.5">
                    {shells.filter((x) => samePath(x.cwd, g.cwd)).map(shellRow)}
                    {g.sessions.map((x) => (
                      <li key={sessionKey(x)}>
                        <SessionRow session={x} live={liveByNative.get(x.id)} selected={sessionKey(x) === selectedKey} onSelect={() => select(x)} s={s} c={c} />
                      </li>
                    ))}
                  </ul>
                </section>
              ))
            )}
          </div>
        </aside>

        <main className="flex min-w-0 flex-1 flex-col">
          {pageError ? (
            <p role="alert" className="border-b border-border px-4 py-2 text-sm text-destructive">
              {pageError}
            </p>
          ) : null}
          {convertError ? (
            <p role="alert" className="border-b border-border px-4 py-2 text-sm text-destructive">
              {fmt(s.convertFailed, { error: convertError })}
            </p>
          ) : null}
          {selectedShell ? (
            <ShellPane key={selectedShell.key} shell={selectedShell} onClose={() => closeShell(selectedShell.key)} c={c} />
          ) : target ? (
            <ChatPane
              key={target.draft ? DRAFT_KEY : selectedLive ? LIVE_PREFIX + selectedLive.live_id : selected ? sessionKey(selected) : selectedKey}
              target={target}
              preview={selected ? preview : null}
              error={selected ? previewError : null}
              result={result}
              converting={converting}
              onConvert={() => void askConvert()}
              lc={selectedLive ? store.convs[selectedLive.live_id] : undefined}
              onRenamed={() => void load(true)}
              onRefresh={() => {
                setPreviewNonce((n) => n + 1);
                void load(true);
              }}
              onDraftChange={(patch) => setDraft((d) => (d ? { ...d, ...patch } : d))}
              projects={projects}
              s={s}
              c={c}
            />
          ) : (
            <div className="grid flex-1 place-items-center p-6 text-sm text-muted-foreground">{s.selectHint}</div>
          )}
        </main>
      </div>

      <ConfirmDialog
        open={confirm !== null}
        title={fmt(s.confirmTitle, { tool: confirmTarget })}
        body={confirmBody}
        confirmLabel={s.confirm}
        cancelLabel={s.cancel}
        onConfirm={() => confirm && void runConvert(confirm)}
        onCancel={() => setConfirm(null)}
      />
    </div>
  );
}
