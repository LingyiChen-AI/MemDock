import * as React from "react";
import { CheckCheck, FolderGit2, ListChecks, Sparkles } from "lucide-react";
import {
  acceptSuggestions,
  applySuggestion,
  listExtractRuns,
  listSuggestions,
  onSuggestionsChanged,
  rejectSuggestion,
} from "@/api/extract";
import { listMemories } from "@/api/memory";
import type { ApplyOutcome, MemoryIndex, RunRecord, Suggestion, SuggestionKind } from "@/api/types";
import { Button } from "@/components/ui/button";
import { SimpleSelect } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import type { NavKey } from "@/layout/AppShell";
import { useI18n } from "@/i18n/I18nProvider";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { formatSnapshotTime } from "@/pages/memory/HistoryDrawer";
import { fmt } from "@/pages/memory/paths";
import { ToastRegion, useToasts } from "@/pages/skills/Toasts";
import { asCommandError, errorMessage } from "@/pages/skills/skillUtils";
import { GithubImportDialog } from "@/pages/ghimport/GithubImportDialog";
import { EvidenceDrawer } from "./EvidenceDrawer";
import { SuggestionCard } from "./SuggestionCard";

export interface ReviewPageProps {
  onNavigate?: (key: NavKey) => void;
}

const GLOBAL = "__global__";
type KindFilter = "all" | SuggestionKind;

const norm = (p: string) => p.replace(/\\/g, "/").toLowerCase();

/** Project (id + name) a suggestion's target file belongs to; global for anything unknown. */
function projectOf(s: Suggestion, index: MemoryIndex | null): { key: string; name: string | null } {
  if (!index) return { key: GLOBAL, name: null };
  if (index.global.some((f) => f.id === s.file_id)) return { key: GLOBAL, name: null };
  const hit = index.projects.find((p) => p.files.some((f) => f.id === s.file_id));
  if (hit) return { key: hit.project.id, name: hit.project.name };
  const path = norm(s.path);
  let best: { key: string; name: string; len: number } | null = null;
  for (const { project } of index.projects) {
    const root = norm(project.path).replace(/\/+$/, "");
    if (path.startsWith(`${root}/`) && (!best || root.length > best.len)) best = { key: project.id, name: project.name, len: root.length };
  }
  return best ? { key: best.key, name: best.name } : { key: GLOBAL, name: null };
}

function reasonText(reason: string, r: { reasonTargetChanged: string; reasonDiskChanged: string; reasonUnknown: string }): string {
  if (reason === "target_changed") return r.reasonTargetChanged;
  if (reason === "disk_changed") return r.reasonDiskChanged;
  return r.reasonUnknown;
}

export function ReviewPage({ onNavigate }: ReviewPageProps) {
  const { t } = useI18n();
  const r = t.review;
  const rRef = React.useRef(r);
  rRef.current = r;
  const { toasts, push, dismiss } = useToasts();

  const [fetched, setFetched] = React.useState<Suggestion[] | null>(null);
  const [staleMap, setStaleMap] = React.useState<Record<string, Suggestion>>({});
  const [index, setIndex] = React.useState<MemoryIndex | null>(null);
  const [lastRun, setLastRun] = React.useState<RunRecord | null>(null);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState<ReadonlySet<string>>(new Set());
  const [bulkBusy, setBulkBusy] = React.useState(false);
  const [confirm, setConfirm] = React.useState<{ s: Suggestion; reason: string; edited: string | null } | null>(null);
  const [bulkIds, setBulkIds] = React.useState<string[] | null>(null);
  const [evidenceFor, setEvidenceFor] = React.useState<Suggestion | null>(null);
  const [evidenceOpen, setEvidenceOpen] = React.useState(false);
  const [kindFilter, setKindFilter] = React.useState<KindFilter>("all");
  const [projectFilter, setProjectFilter] = React.useState("all");
  const [importOpen, setImportOpen] = React.useState(false);

  const seq = React.useRef(0);
  const load = React.useCallback(async () => {
    const mine = ++seq.current;
    setLoadError(null);
    try {
      const [list, idx, runs] = await Promise.all([
        listSuggestions("pending"),
        listMemories().catch(() => null),
        listExtractRuns().catch(() => [] as RunRecord[]),
      ]);
      if (mine !== seq.current) return;
      setFetched(list);
      setIndex(idx);
      setLastRun([...runs].filter((x) => x.status === "succeeded").sort((a, b) => b.started_at - a.started_at)[0] ?? null);
    } catch (e) {
      if (mine === seq.current) setLoadError(errorMessage(e));
    }
  }, []);

  React.useEffect(() => {
    void load();
    return () => {
      seq.current++;
    };
  }, [load]);

  // The backend reports every change (also from other windows / the scheduler): re-read the list.
  React.useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    onSuggestionsChanged(() => {
      if (alive) void load();
    })
      .then((u) => (alive ? (unlisten = u) : u()))
      .catch(() => {});
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [load]);

  // Fetched pending items plus the cards greyed out as stale (no longer pending in the backend).
  const all = React.useMemo(() => {
    if (!fetched) return null;
    const ids = new Set(fetched.map((s) => s.id));
    const extra = Object.values(staleMap).filter((s) => !ids.has(s.id));
    return [...fetched, ...extra].sort((a, b) => b.created_at - a.created_at);
  }, [fetched, staleMap]);

  const withProject = React.useMemo(() => (all ?? []).map((s) => ({ s, p: projectOf(s, index) })), [all, index]);

  const projectOptions = React.useMemo(() => {
    const m = new Map<string, string>();
    for (const { p } of withProject) m.set(p.key, p.name ?? r.projectGlobal);
    return [...m.entries()].sort((a, b) => (a[0] === GLOBAL ? -1 : b[0] === GLOBAL ? 1 : a[1].localeCompare(b[1])));
  }, [withProject, r.projectGlobal]);

  const effProject = projectFilter === "all" || projectOptions.some(([k]) => k === projectFilter) ? projectFilter : "all";
  const visible = withProject.filter(({ s, p }) => (kindFilter === "all" || s.kind === kindFilter) && (effProject === "all" || p.key === effProject));
  const acceptable = visible.filter(({ s }) => !staleMap[s.id]);

  const setBusyId = (id: string, on: boolean) =>
    setBusy((prev) => {
      const next = new Set(prev);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  const removeLocal = (id: string) => {
    setFetched((l) => (l ? l.filter((x) => x.id !== id) : l));
    setStaleMap((m) => {
      if (!(id in m)) return m;
      const { [id]: _gone, ...rest } = m;
      return rest;
    });
  };
  const markStale = (s: Suggestion) => setStaleMap((m) => ({ ...m, [s.id]: s }));

  /** Resolves with an inline error for the card (invalid edit), otherwise null. */
  const apply = async (s: Suggestion, edited: string | null, confirmChanged: boolean): Promise<string | null> => {
    setBusyId(s.id, true);
    try {
      const out: ApplyOutcome = await applySuggestion(s.id, edited, confirmChanged);
      if (out.kind === "applied") {
        removeLocal(s.id);
        if (!out.saved) push("warning", rRef.current.savedFailed);
      } else if (out.kind === "needs_confirm") {
        setConfirm({ s, reason: out.reason, edited });
      } else if (out.kind === "stale") {
        markStale(s);
      } else {
        removeLocal(s.id);
        push("info", rRef.current.alreadyApplied);
      }
      return null;
    } catch (e) {
      const err = asCommandError(e);
      if (err?.code === "invalid") return err.message;
      if (err?.code === "not_found") {
        removeLocal(s.id);
        push("warning", rRef.current.notFound);
      } else if (err?.code === "not_pending") {
        removeLocal(s.id);
        push("info", rRef.current.notPending);
      } else push("error", fmt(rRef.current.applyFailed, { message: errorMessage(e) }));
      return null;
    } finally {
      setBusyId(s.id, false);
    }
  };

  const reject = async (s: Suggestion) => {
    setBusyId(s.id, true);
    try {
      await rejectSuggestion(s.id);
      removeLocal(s.id);
    } catch (e) {
      const code = asCommandError(e)?.code;
      if (code === "not_pending" || code === "not_found") removeLocal(s.id);
      else push("error", fmt(rRef.current.rejectFailed, { message: errorMessage(e) }));
    } finally {
      setBusyId(s.id, false);
    }
  };

  const confirmApply = async () => {
    const c = confirm;
    setConfirm(null);
    if (!c) return;
    const inline = await apply(c.s, c.edited, true);
    if (inline) push("error", fmt(rRef.current.applyFailed, { message: inline }));
  };

  const acceptAll = async () => {
    const ids = bulkIds;
    setBulkIds(null);
    if (!ids || ids.length === 0) return;
    const byId = new Map((all ?? []).map((s) => [s.id, s]));
    setBulkBusy(true);
    try {
      const results = await acceptSuggestions(ids);
      let applied = 0;
      let needs = 0;
      let stale = 0;
      let failed = 0;
      let unsaved = 0;
      for (const res of results) {
        const o = res.outcome;
        if (!o || res.error) {
          failed++;
          const code = res.error?.code;
          if (code === "not_found" || code === "not_pending") removeLocal(res.id);
        } else if (o.kind === "applied") {
          applied++;
          if (!o.saved) unsaved++;
          removeLocal(res.id);
        } else if (o.kind === "already_applied") {
          applied++;
          removeLocal(res.id);
        } else if (o.kind === "needs_confirm") needs++;
        else {
          stale++;
          const s = byId.get(res.id);
          if (s) markStale(s);
        }
      }
      const m = rRef.current;
      const parts = [
        applied && fmt(m.bulkApplied, { n: applied }),
        needs && fmt(m.bulkNeedsConfirm, { n: needs }),
        stale && fmt(m.bulkStale, { n: stale }),
        failed && fmt(m.bulkFailed, { n: failed }),
      ].filter(Boolean);
      push(needs || stale || failed ? "warning" : "success", parts.length ? parts.join("；") : m.bulkNothing);
      if (unsaved) push("warning", m.savedFailed);
    } catch (e) {
      push("error", fmt(rRef.current.applyFailed, { message: errorMessage(e) }));
    } finally {
      setBulkBusy(false);
    }
  };

  const kindTabs: [KindFilter, string][] = [
    ["all", r.kindAll],
    ["add", r.kindAdd],
    ["modify", r.kindModify],
    ["obsolete", r.kindObsolete],
  ];
  const total = all?.length ?? 0;
  const summary = lastRun ? fmt(r.summaryFrom, { n: total, time: formatSnapshotTime(lastRun.started_at) }) : fmt(r.summary, { n: total });

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex max-w-5xl flex-col gap-4 p-6">
        <header className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
          <div className="min-w-0">
            <h1 className="flex items-center gap-2 text-xl font-semibold">
              <ListChecks className="size-5 text-muted-foreground" aria-hidden="true" />
              {r.title}
            </h1>
            {all ? <p className="mt-1 text-sm text-muted-foreground">{summary}</p> : null}
          </div>
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => setImportOpen(true)}
              className="h-8 cursor-pointer bg-card px-3 text-sm hover:bg-muted"
            >
              <FolderGit2 className="size-4" aria-hidden="true" />
              {t.ghImport.button}
            </Button>
            {all && total > 0 ? (
              <Button
                type="button"
                disabled={bulkBusy || busy.size > 0 || acceptable.length === 0}
                onClick={() => setBulkIds(acceptable.map(({ s }) => s.id))}
                className="h-8 cursor-pointer px-3 text-sm font-semibold"
              >
                <CheckCheck className="size-4" aria-hidden="true" />
                {r.acceptAll}
              </Button>
            ) : null}
          </div>
        </header>

        {all && total > 0 ? (
          <div className="flex flex-wrap items-center gap-3">
            <div role="radiogroup" aria-label={r.filterKind} className="inline-flex overflow-hidden rounded-md border border-border">
              {kindTabs.map(([k, label]) => (
                <button
                  key={k}
                  type="button"
                  role="radio"
                  aria-checked={kindFilter === k}
                  onClick={() => setKindFilter(k)}
                  className={cn(
                    "cursor-pointer px-3 text-xs leading-7 outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset motion-reduce:transition-none",
                    kindFilter === k ? "bg-muted font-medium text-foreground" : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  {label}
                </button>
              ))}
            </div>
            <SimpleSelect
              size="sm"
              aria-label={r.filterProject}
              value={effProject}
              onValueChange={setProjectFilter}
              className="max-w-60 bg-card text-xs dark:bg-card"
              options={[{ value: "all", label: r.projectAll }, ...projectOptions.map(([k, name]) => ({ value: k, label: name }))]}
            />
          </div>
        ) : null}

        {loadError ? (
          <div className="flex flex-col items-center gap-3 py-16">
            <p role="alert" className="max-w-md text-center text-sm text-destructive">
              {fmt(r.loadFailed, { message: loadError })}
            </p>
            <Button type="button" variant="outline" onClick={() => void load()} className="h-7 cursor-pointer bg-card px-3 text-xs hover:bg-muted">
              {r.retry}
            </Button>
          </div>
        ) : all === null ? (
          <div aria-label={r.loading} className="space-y-3">
            <Skeleton className="h-48 w-full" />
            <Skeleton className="h-48 w-full" />
          </div>
        ) : total === 0 ? (
          <div className="flex flex-col items-center gap-2 py-20">
            <ListChecks className="size-8 text-muted-foreground" aria-hidden="true" />
            <p className="text-base font-medium">{r.emptyTitle}</p>
            <p className="text-sm text-muted-foreground">{r.emptyHint}</p>
            {onNavigate ? (
              <Button type="button" variant="outline" onClick={() => onNavigate("extract")} className="mt-2 h-8 cursor-pointer bg-card px-3 text-sm hover:bg-muted">
                <Sparkles className="size-4" aria-hidden="true" />
                {r.goExtract}
              </Button>
            ) : null}
          </div>
        ) : visible.length === 0 ? (
          <p className="py-12 text-center text-sm text-muted-foreground">{r.noMatch}</p>
        ) : (
          <div className="flex flex-col gap-4">
            {visible.map(({ s, p }) => (
              <SuggestionCard
                key={s.id}
                suggestion={s}
                projectLabel={p.name ?? r.projectGlobal}
                stale={!!staleMap[s.id]}
                busy={busy.has(s.id) || bulkBusy}
                onAccept={(edited) => apply(s, edited, false)}
                onReject={() => void reject(s)}
                onOpenEvidence={() => {
                  setEvidenceFor(s);
                  setEvidenceOpen(true);
                }}
              />
            ))}
          </div>
        )}
      </div>

      <ConfirmDialog
        open={confirm !== null}
        title={r.confirmChangedTitle}
        body={confirm ? reasonText(confirm.reason, r) : ""}
        confirmLabel={r.confirmChanged}
        cancelLabel={r.cancel}
        onConfirm={() => void confirmApply()}
        onCancel={() => setConfirm(null)}
      />
      <ConfirmDialog
        open={bulkIds !== null}
        title={r.acceptAllTitle}
        body={fmt(r.acceptAllBody, { n: bulkIds?.length ?? 0 })}
        confirmLabel={fmt(r.acceptAllConfirm, { n: bulkIds?.length ?? 0 })}
        cancelLabel={r.cancel}
        onConfirm={() => void acceptAll()}
        onCancel={() => setBulkIds(null)}
      />
      <EvidenceDrawer open={evidenceOpen} onOpenChange={setEvidenceOpen} suggestion={evidenceFor} />
      <GithubImportDialog
        open={importOpen}
        onOpenChange={setImportOpen}
        projects={index?.projects.map((p) => p.project) ?? []}
        onGoReview={() => void load()}
      />
      <ToastRegion toasts={toasts} onDismiss={dismiss} />
    </div>
  );
}
