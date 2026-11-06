import * as React from "react";
import { ArrowLeftRight, Check, Copy, Info, Loader2, MessagesSquare, RefreshCw, Search, TerminalSquare, TriangleAlert } from "lucide-react";
import { convertSession, convertTargetDir, listSessions, sessionPreview } from "@/api/sessions";
import type { ConvertResult, SessionPreview, SessionSummary, Tool } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type { Messages } from "@/i18n";
import { fmt } from "@/pages/memory/paths";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { asCommandError, errorMessage } from "@/pages/skills/skillUtils";
import { filterSessions, formatIso, formatTime, groupByProject, otherTool, type ToolFilter } from "./sessionsUtils";

type S = Messages["sessions"];

function sessionKey(s: { tool: Tool; path: string }): string {
  return `${s.tool}:${s.path}`;
}

function ToolBadge({ tool, s }: { tool: Tool; s: S }) {
  return (
    <span
      className={cn(
        "inline-flex h-5 shrink-0 items-center rounded px-1.5 text-[11px] font-medium",
        tool === "claude" ? "bg-orange-500/12 text-orange-700 dark:text-orange-300" : "bg-sky-500/12 text-sky-700 dark:text-sky-300",
      )}
    >
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
            o.value === value ? "bg-muted font-medium text-foreground" : "text-muted-foreground hover:text-foreground",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function SessionRow({ session, selected, onSelect, s }: { session: SessionSummary; selected: boolean; onSelect: () => void; s: S }) {
  const heading = session.title?.trim() || session.first_prompt || s.untitled;
  return (
    <button
      type="button"
      aria-current={selected ? "true" : undefined}
      onClick={onSelect}
      className={cn(
        "flex w-full cursor-pointer flex-col gap-1 rounded-md px-2.5 py-2 text-left outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none",
        selected ? "bg-muted" : "hover:bg-muted/60",
      )}
    >
      <div className="flex min-w-0 items-center gap-2">
        <ToolBadge tool={session.tool} s={s} />
        <span className="min-w-0 flex-1 truncate text-sm font-medium">{heading}</span>
        <span className="shrink-0 text-[11px] text-muted-foreground tabular-nums">{formatTime(session.updated_at)}</span>
      </div>
      {session.title && session.first_prompt ? (
        <p className="line-clamp-2 text-xs text-muted-foreground">{session.first_prompt}</p>
      ) : null}
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-0.5 text-[11px] text-muted-foreground">
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

function CopyButton({ text, s }: { text: string; s: S }) {
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
  return (
    <Button type="button" variant="outline" size="sm" className="shrink-0 cursor-pointer" onClick={() => void copy()}>
      {state === "copied" ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
      {state === "copied" ? s.copied : state === "failed" ? s.copyFailed : s.copy}
    </Button>
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
      <div className="flex flex-col gap-1.5">
        <span className="text-xs text-muted-foreground">{s.resumeHint}</span>
        <div className="flex items-center gap-2">
          <code className="min-w-0 flex-1 rounded-md border border-border bg-card px-2.5 py-1.5 font-mono text-xs break-all select-all">
            {result.resume_command}
          </code>
          <CopyButton text={result.resume_command} s={s} />
        </div>
      </div>
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

function PreviewPane({
  session,
  preview,
  error,
  result,
  converting,
  onConvert,
  s,
}: {
  session: SessionSummary;
  preview: SessionPreview | null;
  error: string | null;
  result: ConvertResult | null;
  converting: boolean;
  onConvert: () => void;
  s: S;
}) {
  const target = otherTool(session.tool);
  const stats = preview?.stats;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-col gap-3 border-b border-border p-4">
        <div className="flex min-w-0 items-start justify-between gap-3">
          <div className="flex min-w-0 flex-col gap-1">
            <h2 className="flex min-w-0 items-center gap-2 text-base font-semibold">
              <ToolBadge tool={session.tool} s={s} />
              <span className="truncate">{session.title?.trim() || session.first_prompt || s.untitled}</span>
            </h2>
            <p className="truncate font-mono text-xs text-muted-foreground" title={session.cwd}>
              {session.cwd}
            </p>
            <p className="font-mono text-[11px] text-muted-foreground select-all">{session.id}</p>
          </div>
          <Button type="button" size="sm" className="shrink-0 cursor-pointer" disabled={converting || !preview} onClick={onConvert}>
            {converting ? <Loader2 className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <ArrowLeftRight aria-hidden="true" />}
            {converting ? s.converting : fmt(s.convertTo, { tool: s.tools[target] })}
          </Button>
        </div>
        {stats ? (
          <div className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
            <span>{fmt(s.messages, { n: stats.user_messages + stats.assistant_messages })}</span>
            {stats.tool_calls === 0 ? <span>{s.stats.none}</span> : null}
            {stats.mapped_calls > 0 ? <span>{fmt(s.stats.mapped, { n: stats.mapped_calls })}</span> : null}
            {stats.degraded_calls > 0 ? <span>{fmt(s.stats.degraded, { n: stats.degraded_calls })}</span> : null}
          </div>
        ) : null}
        {result ? <ResultCard result={result} s={s} /> : null}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        {error ? (
          <p role="alert" className="text-sm text-destructive">
            {fmt(s.previewFailed, { error })}
          </p>
        ) : !preview ? (
          <p className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />
            {s.previewLoading}
          </p>
        ) : (
          <div className="flex flex-col gap-2.5">
            <h3 className="text-xs font-medium text-muted-foreground">{s.previewTitle}</h3>
            <ol className="flex flex-col gap-2">
              {preview.messages.map((m, i) => (
                <li
                  key={i}
                  className={cn(
                    "flex flex-col gap-1 rounded-md border px-3 py-2",
                    m.role === "user" ? "border-primary/30 bg-primary/5" : "border-border bg-card",
                  )}
                >
                  <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
                    {m.role === "shell" ? <TerminalSquare className="size-3.5" aria-hidden="true" /> : null}
                    <span className="font-medium">{s.roles[m.role]}</span>
                    <span className="tabular-nums">{formatIso(m.ts)}</span>
                  </div>
                  <p className={cn("text-sm break-words whitespace-pre-wrap", m.role === "shell" && "font-mono text-xs")}>{m.text}</p>
                </li>
              ))}
            </ol>
            {preview.total_messages > preview.messages.length ? (
              <p className="text-xs text-muted-foreground">
                {fmt(s.previewMore, { shown: preview.messages.length, total: preview.total_messages })}
              </p>
            ) : null}
          </div>
        )}
      </div>
    </div>
  );
}

interface Confirm {
  session: SessionSummary;
  dir: string;
  force: boolean;
}

export function SessionsPage() {
  const { t } = useI18n();
  const s = t.sessions;
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

  const load = React.useCallback(async () => {
    setLoading(true);
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

  // Reload the preview whenever the selected file changes.
  const selectedPath = selected?.path;
  const selectedTool = selected?.tool;
  React.useEffect(() => {
    if (!selectedPath || !selectedTool) return;
    let alive = true;
    setPreview(null);
    setPreviewError(null);
    sessionPreview(selectedTool, selectedPath)
      .then((p) => alive && setPreview(p))
      .catch((e) => alive && setPreviewError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [selectedPath, selectedTool]);

  const visible = React.useMemo(() => filterSessions(sessions ?? [], tool, query), [sessions, tool, query]);
  const groups = React.useMemo(() => groupByProject(visible), [visible]);

  const select = (x: SessionSummary) => {
    setSelectedKey(sessionKey(x));
    setResult(null);
    setConvertError(null);
  };

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
            {s.title}
          </h1>
          <p className="text-xs text-muted-foreground">{s.subtitle}</p>
        </div>
        <Button type="button" variant="outline" size="sm" disabled={loading} onClick={() => void load()} className="cursor-pointer">
          <RefreshCw className={cn("size-3.5", loading && "animate-spin motion-reduce:animate-none")} aria-hidden="true" />
          {s.refresh}
        </Button>
      </header>

      <div className="flex min-h-0 flex-1">
        <aside className="flex w-[380px] min-w-[300px] shrink-0 flex-col border-r border-border">
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
            {loadError ? (
              <p role="alert" className="p-2 text-sm text-destructive">
                {fmt(s.loadFailed, { error: loadError })}
              </p>
            ) : sessions === null ? (
              <p className="flex items-center gap-2 p-2 text-sm text-muted-foreground">
                <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />
                {s.loading}
              </p>
            ) : sessions.length === 0 ? (
              <div className="flex flex-col gap-1 p-2">
                <p className="text-sm font-medium">{s.empty}</p>
                <p className="text-xs text-muted-foreground">{s.emptyHint}</p>
              </div>
            ) : groups.length === 0 ? (
              <p className="p-2 text-sm text-muted-foreground">{s.noMatch}</p>
            ) : (
              groups.map((g) => (
                <section key={g.projectId} aria-label={g.name} className="mb-2">
                  <h2 className="flex items-baseline gap-2 px-2.5 pt-2 pb-1 text-xs font-semibold" title={g.cwd}>
                    <span className="truncate">{g.name}</span>
                    <span className="font-normal text-muted-foreground">{g.sessions.length}</span>
                  </h2>
                  <ul className="flex flex-col gap-0.5">
                    {g.sessions.map((x) => (
                      <li key={sessionKey(x)}>
                        <SessionRow session={x} selected={sessionKey(x) === selectedKey} onSelect={() => select(x)} s={s} />
                      </li>
                    ))}
                  </ul>
                </section>
              ))
            )}
          </div>
        </aside>

        <main className="flex min-w-0 flex-1 flex-col">
          {convertError ? (
            <p role="alert" className="border-b border-border px-4 py-2 text-sm text-destructive">
              {fmt(s.convertFailed, { error: convertError })}
            </p>
          ) : null}
          {selected ? (
            <PreviewPane
              session={selected}
              preview={preview}
              error={previewError}
              result={result}
              converting={converting}
              onConvert={() => void askConvert()}
              s={s}
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
