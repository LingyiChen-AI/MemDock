import * as React from "react";
import { Loader2, Play, Sparkles, X } from "lucide-react";
import {
  cancelExtraction,
  extractRunners,
  getExtractConfig,
  listExtractRuns,
  listSuggestions,
  onExtractFinished,
  onExtractProgress,
  resetExtractData,
  setExtractConfig,
  onSuggestionsChanged,
  startExtraction,
} from "@/api/extract";
import { listMemories } from "@/api/memory";
import type { ExtractConfig, ExtractProgress, Project, RunRecord, RunnerInfo, Suggestion } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import type { NavKey } from "@/layout/AppShell";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";
import { ToastRegion, useToasts } from "@/pages/skills/Toasts";
import { asCommandError, errorMessage } from "@/pages/skills/skillUtils";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { PromptSettings } from "./PromptSettings";
import { ExtractConfigForm, type DraftState } from "./ExtractConfigForm";
import { RunsTable, runErrorText } from "./RunsTable";
import { runProgress } from "./runStats";

export interface ExtractPageProps {
  onNavigate?: (key: NavKey) => void;
  /** Reports unsaved config edits so the app can confirm before leaving the page. */
  onDirtyChange?: (dirty: boolean) => void;
  /** Opens the review page showing only this run's suggestions. */
  onOpenRunReview?: (run: RunRecord) => void;
}

export function ExtractPage({ onNavigate, onDirtyChange, onOpenRunReview }: ExtractPageProps) {
  const { t } = useI18n();
  const x = t.extract;
  const [config, setConfig] = React.useState<ExtractConfig | null>(null);
  const [runners, setRunners] = React.useState<RunnerInfo[]>([]);
  const [projects, setProjects] = React.useState<Project[]>([]);
  const [runs, setRuns] = React.useState<RunRecord[] | null>(null);
  const [suggestions, setSuggestions] = React.useState<Suggestion[]>([]);
  React.useEffect(() => {
    let alive = true;
    const load = () =>
      listSuggestions()
        .then((v) => alive && setSuggestions(v))
        .catch(() => {});
    void load();
    let unlisten: (() => void) | undefined;
    onSuggestionsChanged(() => void load())
      .then((u) => (alive ? (unlisten = u) : u()))
      .catch(() => {});
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [runs]);
  const onOpenRun = onOpenRunReview ?? (onNavigate ? () => onNavigate("review") : undefined);
  const reviewProgress = React.useMemo(() => new Map((runs ?? []).map((r) => [r.id, runProgress(r, suggestions)])), [runs, suggestions]);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  /** The data files could not be loaded: offer a reset. */
  const [storeBroken, setStoreBroken] = React.useState(false);
  const [confirmReset, setConfirmReset] = React.useState(false);
  const [resetting, setResetting] = React.useState(false);
  const [saving, setSaving] = React.useState(false);
  const [starting, setStarting] = React.useState(false);
  const [cancelling, setCancelling] = React.useState(false);
  const [progress, setProgress] = React.useState<ExtractProgress | null>(null);
  const [draft, setDraft] = React.useState<DraftState | null>(null);
  const [confirmRun, setConfirmRun] = React.useState(false);
  const { toasts, push, dismiss } = useToasts();
  const xRef = React.useRef(x);
  xRef.current = x;

  const running = starting || (runs ?? []).some((r) => r.status === "running");
  const runningRef = React.useRef(running);
  runningRef.current = running;

  const dirty = draft?.dirty ?? false;
  const dirtyCb = React.useRef(onDirtyChange);
  dirtyCb.current = onDirtyChange;
  React.useEffect(() => {
    dirtyCb.current?.(dirty);
  }, [dirty]);
  React.useEffect(() => () => dirtyCb.current?.(false), []);

  // The tool that would run: the draft's choice when there are edits, else the saved one.
  const effTool = draft ? draft.config.tool : (config?.tool ?? null);
  const runnable = effTool ? (runners.find((r) => r.tool === effTool)?.available ?? false) : runners.some((r) => r.available);
  const draftValid = draft?.valid ?? true;
  const blockReason = !draftValid ? x.runInvalid : !runnable && config ? x.runNoRunner : null;

  const load = React.useCallback(async () => {
    setLoadError(null);
    setStoreBroken(false);
    try {
      const [c, rn, rs, idx] = await Promise.all([getExtractConfig(), extractRunners(), listExtractRuns(), listMemories()]);
      setConfig(c);
      setRunners(rn);
      setRuns(rs);
      setProjects(idx.projects.map((p) => p.project));
    } catch (e) {
      setLoadError(errorMessage(e));
      setStoreBroken(asCommandError(e)?.code === "store_unavailable");
    }
  }, []);

  React.useEffect(() => {
    void load();
  }, [load]);

  React.useEffect(() => {
    let alive = true;
    const unlisten: Array<() => void> = [];
    const keep = (p: Promise<() => void>) =>
      void p.then((u) => (alive ? unlisten.push(u) : u())).catch(() => {});
    keep(
      onExtractProgress((p) => {
        if (!alive) return;
        setProgress(p);
        // A run this page did not start (e.g. scheduled): pick up its record.
        if (!runningRef.current)
          listExtractRuns()
            .then((rs) => alive && setRuns(rs))
            .catch(() => {});
      }),
    );
    keep(
      onExtractFinished((r) => {
        if (!alive) return;
        setProgress(null);
        setCancelling(false);
        setStarting(false);
        // Replace the running record right away, then refresh from the backend.
        setRuns((l) => (l ? l.map((o) => (o.id === r.id ? r : o)) : l));
        const m = xRef.current;
        if (r.status === "succeeded") push("success", fmt(m.finishedOk, { n: r.suggestions_new }));
        else if (r.status === "failed") {
          const text = runErrorText(m, r);
          push("error", text ? `${m.finishedFailed}：${text}` : m.finishedFailed);
        }
        else if (r.status === "cancelled") push("warning", m.finishedCancelled);
        else if (r.status === "interrupted") push("warning", m.finishedInterrupted);
        listExtractRuns()
          .then((rs) => alive && setRuns(rs))
          .catch(() => {});
      }),
    );
    return () => {
      alive = false;
      unlisten.forEach((u) => u());
    };
  }, [push]);

  const save = async (next: ExtractConfig) => {
    setSaving(true);
    try {
      setConfig(await setExtractConfig(next));
      push("success", x.saved);
    } catch (e) {
      push("error", fmt(x.saveFailed, { message: errorMessage(e) }));
    } finally {
      setSaving(false);
    }
  };

  const onRunClick = () => {
    if (dirty) setConfirmRun(true);
    else void run();
  };

  const saveAndRun = async () => {
    setConfirmRun(false);
    if (!draft) return;
    setSaving(true);
    try {
      setConfig(await setExtractConfig(draft.config));
    } catch (e) {
      push("error", fmt(x.saveFailed, { message: errorMessage(e) }));
      return;
    } finally {
      setSaving(false);
    }
    await run();
  };

  const run = async () => {
    setStarting(true);
    setProgress(null);
    try {
      const rec = await startExtraction();
      setRuns((l) => [rec, ...(l ?? []).filter((o) => o.id !== rec.id)]);
    } catch (e) {
      setStarting(false);
      const code = asCommandError(e)?.code;
      if (code === "already_running") {
        push("warning", x.alreadyRunning);
        listExtractRuns()
          .then(setRuns)
          .catch(() => {});
      } else if (code === "not_installed") push("error", x.notInstalled);
      else push("error", fmt(x.startFailed, { message: errorMessage(e) }));
      return;
    }
    setStarting(false);
  };

  const cancel = async () => {
    setCancelling(true);
    try {
      await cancelExtraction();
    } catch (e) {
      setCancelling(false);
      push("error", errorMessage(e));
    }
  };

  const reset = async () => {
    setConfirmReset(false);
    setResetting(true);
    try {
      await resetExtractData();
      push("success", x.resetDone);
      await load();
    } catch (e) {
      push("error", fmt(x.resetFailed, { message: errorMessage(e) }));
    } finally {
      setResetting(false);
    }
  };

  const runningLabel = progress ? fmt(x.runningProgress, { i: progress.index, n: progress.total }) : x.running;

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex max-w-4xl flex-col gap-4 p-6">
        <header className="flex items-center justify-between gap-4">
          <h1 className="flex items-center gap-2 text-xl font-semibold">
            <Sparkles className="size-5 text-muted-foreground" aria-hidden="true" />
            {x.title}
          </h1>
          {running ? (
            <div className="flex items-center gap-2">
              <span role="status" className="inline-flex items-center gap-2 text-sm">
                <Loader2 className="size-4 animate-spin text-accent-text motion-reduce:animate-none" aria-hidden="true" />
                <span>{runningLabel}</span>
              </span>
              <Button type="button" variant="outline" disabled={cancelling} onClick={() => void cancel()} className="h-8 cursor-pointer px-3 text-sm">
                <X className="size-4" aria-hidden="true" />
                {cancelling ? x.cancelling : x.cancel}
              </Button>
            </div>
          ) : (
            <Button type="button" disabled={!config || !!blockReason || saving} title={blockReason ?? undefined} onClick={onRunClick} className="h-8 cursor-pointer px-3 text-sm font-semibold">
              <Play className="size-4" aria-hidden="true" />
              {x.runNow}
            </Button>
          )}
        </header>

        {!running && blockReason ? (
          <p role="note" className="-mt-2 text-right text-xs text-warning">
            {blockReason}
          </p>
        ) : null}

        {loadError ? (
          <div className="flex flex-col items-center gap-3 py-16">
            <p role="alert" className="max-w-md text-center text-sm text-destructive">
              {fmt(x.loadFailed, { message: loadError })}
            </p>
            <div className="flex items-center gap-2">
              <Button type="button" variant="outline" onClick={() => void load()} className="h-7 cursor-pointer bg-card px-3 text-xs hover:bg-muted">
                {x.retry}
              </Button>
              {storeBroken ? (
                <Button
                  type="button"
                  variant="outline"
                  disabled={resetting}
                  onClick={() => setConfirmReset(true)}
                  className="h-7 cursor-pointer bg-card px-3 text-xs text-destructive hover:bg-muted"
                >
                  {x.resetData}
                </Button>
              ) : null}
            </div>
          </div>
        ) : (
          <>
            <section aria-label={x.configTitle} className="rounded-lg border border-border bg-card p-4">
              <h2 className="mb-1 text-sm font-semibold">{x.configTitle}</h2>
              {config ? (
                <ExtractConfigForm config={config} runners={runners} projects={projects} saving={saving} onSave={(c) => void save(c)} onDraftChange={setDraft} />
              ) : (
                <div aria-label={x.loading} className="space-y-3 py-3">
                  <Skeleton className="h-8 w-60" />
                  <Skeleton className="h-8 w-80" />
                  <Skeleton className="h-8 w-40" />
                </div>
              )}
              <p className="mt-3 border-t border-border pt-3 text-xs text-muted-foreground">{x.notice}</p>
            </section>

            {config ? (
              <section aria-label={x.prompt.title} className="rounded-lg border border-border bg-card p-4">
                <h2 className="mb-1 text-sm font-semibold">{x.prompt.title}</h2>
                <p className="mb-3 text-xs text-muted-foreground">{x.prompt.desc}</p>
                <PromptSettings config={config} saving={saving} onSave={(c) => void save(c)} />
              </section>
            ) : null}

            <section aria-label={x.runsTitle} className="rounded-lg border border-border bg-card">
              <h2 className="px-4 pt-4 pb-2 text-sm font-semibold">{x.runsTitle}</h2>
              {runs === null ? (
                <div aria-label={x.loading} className="space-y-2 p-4">
                  <Skeleton className="h-8 w-full" />
                  <Skeleton className="h-8 w-full" />
                </div>
              ) : runs.length === 0 ? (
                <div className="flex flex-col items-center gap-1 px-4 py-10">
                  <p className="text-sm">{x.emptyRuns}</p>
                  <p className="text-xs text-muted-foreground">{x.emptyRunsHint}</p>
                </div>
              ) : (
                <RunsTable runs={runs} progress={reviewProgress} onOpenReview={onOpenRun} />
              )}
            </section>
          </>
        )}
      </div>
      <ConfirmDialog
        open={confirmRun}
        title={x.runUnsavedTitle}
        body={x.runUnsavedBody}
        confirmLabel={x.runUnsavedConfirm}
        cancelLabel={x.runUnsavedCancel}
        onConfirm={() => void saveAndRun()}
        onCancel={() => setConfirmRun(false)}
      />
      <ConfirmDialog
        open={confirmReset}
        title={x.resetTitle}
        body={x.resetBody}
        confirmLabel={x.resetConfirm}
        cancelLabel={x.resetCancel}
        destructive
        onConfirm={() => void reset()}
        onCancel={() => setConfirmReset(false)}
      />
      <ToastRegion toasts={toasts} onDismiss={dismiss} />
    </div>
  );
}
