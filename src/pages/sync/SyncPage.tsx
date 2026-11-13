import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  CircleCheck,
  CircleDot,
  CloudDownload,
  CloudUpload,
  ExternalLink,
  FileKey2,
  FileWarning,
  FolderX,
  Loader2,
  LogIn,
  LogOut,
  RefreshCw,
  ShieldAlert,
  SkipForward,
  SquareArrowOutUpRight,
  TriangleAlert,
  Upload,
} from "lucide-react";
import { logout, prefsSyncInfo, syncDiscard, syncFinish, syncProjects, syncSkippedFiles, syncStart, syncStatus } from "@/api/sync";
import { collectFrontendPrefs } from "@/prefs/prefsSync";
import type {
  FileSecretHit,
  ManifestSkill,
  SyncPlan,
  SyncAbsentFile,
  SyncDirection,
  SyncProject,
  SyncResolution,
  SyncSkipped,
  SyncSkippedRecord,
  SyncStatus,
  SyncSummary,
} from "@/api/types";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { syncErrorText } from "@/components/account/syncErrors";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { fmt } from "@/pages/memory/paths";
import { InlineError } from "@/pages/skills/flowParts";
import { asCommandError } from "@/pages/skills/skillUtils";
import { ToastRegion, useToasts, type ToastKind } from "@/pages/skills/Toasts";
import { ChoiceButton, ConflictResolver, resolutionOf, type ConflictState } from "./ConflictResolver";
import { MissingSkills } from "./MissingSkills";
import { ProjectMapping } from "./ProjectMapping";
import { SyncHistory } from "./SyncHistory";
import { middleEllipsis, splitPath } from "./pathText";

export interface SyncPageProps {
  status: SyncStatus | null;
  onStatus: (s: SyncStatus) => void;
  onOpenLogin: () => void;
  /** An open resolution counts as unsaved work. */
  onDirtyChange?: (dirty: boolean) => void;
  /** Opens a memory file (index id) on the memory page. */
  onOpenMemoryFile?: (fileId: string) => void;
}

interface Extras {
  skipped: SyncSkipped[];
  missing: ManifestSkill[];
  removed: ManifestSkill[];
  download: boolean;
  keptLocal: string[];
  absent: SyncAbsentFile[];
}

const extrasOf = (x: {
  skipped: SyncSkipped[];
  missing_skills: ManifestSkill[];
  removed_elsewhere: ManifestSkill[];
  download: boolean;
  kept_local: string[];
  absent: SyncAbsentFile[];
}): Extras => ({
  skipped: x.skipped,
  missing: x.missing_skills,
  removed: x.removed_elsewhere,
  download: !!x.download,
  keptLocal: x.kept_local ?? [],
  absent: x.absent ?? [],
});

/** Errors after which the plan can no longer be finished. */
const PLAN_DEAD = new Set(["plan_not_found", "local_changed", "remote_moved", "unauthorized"]);

export function SyncPage({ status, onStatus, onOpenLogin, onDirtyChange, onOpenMemoryFile }: SyncPageProps) {
  const { t, language } = useI18n();
  const s = t.sync;
  const { toasts, push, dismiss } = useToasts();
  const [running, setRunning] = React.useState<SyncDirection | null>(null);
  const syncing = running !== null;
  const [error, setError] = React.useState<string | null>(null);
  const [hits, setHits] = React.useState<FileSecretHit[] | null>(null);
  const [extras, setExtras] = React.useState<Extras | null>(null);
  const [plan, setPlan] = React.useState<SyncPlan | null>(null);
  // Leaving the page mid-resolution drops the stored plan, so the next sync starts clean.
  const planRef = React.useRef<SyncPlan | null>(null);
  planRef.current = plan;
  React.useEffect(
    () => () => {
      const p = planRef.current;
      if (p) void syncDiscard(p.id).catch(() => {});
    },
    [],
  );
  const [projects, setProjects] = React.useState<SyncProject[] | null>(null);
  const [records, setRecords] = React.useState<SyncSkippedRecord[]>([]);
  const [historyKey, setHistoryKey] = React.useState(0);
  const [confirmLogout, setConfirmLogout] = React.useState(false);
  const loggedIn = !!status?.logged_in;

  const dirtyCb = React.useRef(onDirtyChange);
  dirtyCb.current = onDirtyChange;
  React.useEffect(() => {
    dirtyCb.current?.(plan !== null);
  }, [plan]);
  React.useEffect(() => () => dirtyCb.current?.(false), []);

  React.useEffect(() => {
    if (!loggedIn) return;
    let alive = true;
    syncProjects()
      .then((p) => alive && setProjects(p))
      .catch(() => alive && setProjects([]));
    return () => {
      alive = false;
    };
  }, [loggedIn]);

  // Persistent skip records: reloaded after every sync (historyKey changes).
  React.useEffect(() => {
    if (!loggedIn) return;
    let alive = true;
    Promise.resolve()
      .then(() => syncSkippedFiles())
      .then((r) => alive && setRecords(Array.isArray(r) ? r : []))
      .catch(() => alive && setRecords([]));
    return () => {
      alive = false;
    };
  }, [loggedIn, historyKey]);

  const formatTime = React.useCallback(
    (ms: number) =>
      new Date(ms).toLocaleString(language === "zh" ? "zh-CN" : "en-US", { dateStyle: "medium", timeStyle: "short" }),
    [language],
  );

  const refreshStatus = () => {
    syncStatus()
      .then(onStatus)
      .catch(() => {});
  };

  const handleError = (e: unknown) => {
    const ce = asCommandError(e);
    if (ce?.code === "secrets" && ce.secret_hits) {
      setHits(ce.secret_hits);
      return;
    }
    setError(syncErrorText(t, e));
    if (ce?.code === "unauthorized" || ce?.code === "public_not_acked" || ce?.code === "repo_not_ours") refreshStatus();
  };

  const finished = (summary: SyncSummary, x: Extras) => {
    setExtras(x);
    setHistoryKey((k) => k + 1);
    refreshStatus();
    return summary;
  };

  const toastFor = (download: boolean, sm: SyncSummary, after: boolean) =>
    download
      ? fmt(s.downloadDoneToast, { pulled: sm.pulled, merged: sm.merged })
      : fmt(after ? s.finishedToast : s.doneToast, { pulled: sm.pulled, pushed: sm.pushed, merged: sm.merged });

  // Preferences are never applied by a download; only point at newer ones.
  const hintNewerPrefs = () => {
    void prefsSyncInfo()
      .then((i) => {
        if (i.newer) push("info", t.settings.prefsSync.newerHint);
      })
      .catch(() => undefined);
  };

  const run = async (secrets: "block" | "allow" | "skip" = "block", direction: SyncDirection = "up") => {
    setRunning(direction);
    setError(null);
    setHits(null);
    setExtras(null);
    try {
      const r = await syncStart(secrets, direction, direction === "up" ? collectFrontendPrefs() : undefined);
      if (r.kind === "done") {
        const sm = finished(r.summary, extrasOf(r));
        push("success", toastFor(!!r.download, sm, false));
        if (r.download) hintNewerPrefs();
      } else if (r.kind === "blocked") {
        setHits(r.secret_hits);
      } else {
        setPlan(r.plan);
      }
    } catch (e) {
      handleError(e);
    } finally {
      setRunning(null);
    }
  };

  const doLogout = async () => {
    try {
      onStatus(await logout());
      setExtras(null);
      setHits(null);
    } catch (e) {
      handleError(e);
    }
  };

  if (plan) {
    return (
      <>
        <ResolveView
          plan={plan}
          notify={push}
          onFinished={(summary) => {
            const p = plan;
            setPlan(null);
            finished(summary, extrasOf(p));
            push("success", toastFor(!!p.download, summary, true));
            if (p.download) hintNewerPrefs();
          }}
          onBlocked={(h) => {
            setPlan(null);
            setHits(h);
          }}
          onAbort={(msg) => {
            setPlan(null);
            if (msg) setError(msg);
          }}
        />
        <ToastRegion toasts={toasts} onDismiss={dismiss} />
      </>
    );
  }

  const blocked = syncing || !!status?.needs;
  const fresh = !!status?.remote_has_memory && !status.last_sync_at;

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex w-full max-w-[1440px] flex-col gap-4 px-6 py-6">
        <header className="flex items-center gap-2">
          <h1 className="flex items-center gap-2 text-xl font-semibold">
            <RefreshCw className="size-5 text-muted-foreground" aria-hidden="true" />
            {s.title}
          </h1>
        </header>

        {status === null ? (
          <div className="space-y-3">
            <Skeleton className="h-28 w-full" />
            <Skeleton className="h-40 w-full" />
          </div>
        ) : !loggedIn ? (
          <div className="flex flex-col items-center gap-3 py-20 text-center">
            <RefreshCw className="size-8 text-muted-foreground" aria-hidden="true" />
            <h2 className="text-base font-semibold">{s.loggedOutTitle}</h2>
            <p className="max-w-md text-sm text-muted-foreground">{s.loggedOutBody}</p>
            {status.logout_reason ? (
              <p className="max-w-md rounded-md bg-warning/15 px-3 py-1.5 text-xs text-muted-foreground">
                {fmt(s.logoutReason, { reason: status.logout_reason })}
              </p>
            ) : null}
            <Button type="button" onClick={onOpenLogin} className="mt-2 h-8 cursor-pointer px-4 text-sm font-semibold">
              <LogIn className="size-4" aria-hidden="true" />
              {t.account.login}
            </Button>
          </div>
        ) : (
          <>
            {status.needs ? <NeedsBanner status={status} onAction={onOpenLogin} /> : null}

            {fresh && !status.needs ? (
              <section
                aria-label={s.freshTitle}
                className="flex flex-wrap items-center gap-3 rounded-lg border border-primary/40 bg-primary/10 p-4 dark:bg-primary/15"
              >
                <span className="grid size-9 shrink-0 place-items-center rounded-full bg-primary/15 text-primary dark:bg-primary/25">
                  <CloudDownload className="size-5" aria-hidden="true" />
                </span>
                <div className="min-w-0 flex-1">
                  <h2 className="text-sm font-semibold">{s.freshTitle}</h2>
                  <p className="text-sm text-muted-foreground">{s.freshBody}</p>
                </div>
                <Button
                  type="button"
                  disabled={blocked}
                  onClick={() => void run("block", "down")}
                  className="h-8 shrink-0 cursor-pointer px-4 text-sm font-semibold"
                >
                  {running === "down" ? (
                    <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />
                  ) : (
                    <CloudDownload className="size-4" aria-hidden="true" />
                  )}
                  {s.freshAction}
                </Button>
              </section>
            ) : null}

            <section aria-label={s.title} className="rounded-lg border border-border bg-card p-4">
              <div className="flex flex-wrap items-start gap-4">
                <div className="min-w-0 flex-1 space-y-1.5">
                  {status.repo ? (
                    <div className="flex flex-wrap items-center gap-2">
                      <button
                        type="button"
                        onClick={() => void openUrl(status.repo!.url).catch(() => {})}
                        className="inline-flex min-w-0 cursor-pointer items-center gap-1.5 rounded-sm font-mono text-sm font-semibold text-accent-text outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
                      >
                        <span className="truncate">{`${status.repo.owner}/${status.repo.name}`}</span>
                        <ExternalLink className="size-3.5 shrink-0" aria-hidden="true" />
                      </button>
                      <VisibilityTag priv={status.repo.private} />
                    </div>
                  ) : null}
                  <p className="text-sm text-muted-foreground">
                    {status.last_sync_at ? fmt(s.lastSync, { time: formatTime(status.last_sync_at) }) : s.never}
                  </p>
                  <p className={cn("text-sm", status.pending > 0 ? "font-medium text-foreground" : "text-muted-foreground")}>
                    {status.pending > 0 ? fmt(s.pending, { n: status.pending }) : s.upToDate}
                  </p>
                </div>
                <Button
                  type="button"
                  variant="ghost"
                  disabled={syncing}
                  onClick={() => setConfirmLogout(true)}
                  className="h-8 shrink-0 cursor-pointer px-3 text-sm text-muted-foreground"
                >
                  <LogOut className="size-4" aria-hidden="true" />
                  {s.logout}
                </Button>
              </div>
              <div className="mt-4 grid gap-3 border-t border-border pt-4 sm:grid-cols-2">
                <div className="flex flex-col gap-1.5">
                  <Button
                    type="button"
                    disabled={blocked}
                    onClick={() => void run("block", "up")}
                    aria-describedby="sync-up-hint"
                    className="h-9 cursor-pointer justify-center px-4 text-sm font-semibold"
                  >
                    {running === "up" ? (
                      <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />
                    ) : (
                      <CloudUpload className="size-4" aria-hidden="true" />
                    )}
                    {running === "up" ? s.uploading : s.uploadNow}
                  </Button>
                  <p id="sync-up-hint" className="text-xs text-muted-foreground">
                    {s.uploadHint}
                  </p>
                </div>
                <div className="flex flex-col gap-1.5">
                  <Button
                    type="button"
                    variant="outline"
                    disabled={blocked}
                    onClick={() => void run("block", "down")}
                    aria-describedby="sync-down-hint"
                    className="h-9 cursor-pointer justify-center px-4 text-sm font-semibold"
                  >
                    {running === "down" ? (
                      <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />
                    ) : (
                      <CloudDownload className="size-4" aria-hidden="true" />
                    )}
                    {running === "down" ? s.downloading : s.downloadNow}
                  </Button>
                  <p id="sync-down-hint" className="text-xs text-muted-foreground">
                    {s.downloadHint}
                  </p>
                </div>
              </div>
            </section>

            {error ? <InlineError message={error} /> : null}
            <SecretsDialog
              hits={hits}
              busy={syncing}
              onCancel={() => setHits(null)}
              onSkip={() => void run("skip", "up")}
              onAllow={() => void run("allow", "up")}
            />
            {extras && extras.download && extras.keptLocal.length > 0 ? (
              <p role="status" className="rounded-md bg-muted px-3 py-2 text-sm text-muted-foreground">
                {fmt(s.keptLocal, { n: extras.keptLocal.length })}
              </p>
            ) : null}
            {extras ? <MissingSkills missing={extras.missing} removed={extras.removed} projects={projects} notify={push} /> : null}
            {extras && extras.absent.length > 0 ? <AbsentList items={extras.absent} /> : null}
            {/* Files the persistent list below already shows are not repeated here. */}
            {extras && extras.skipped.some((x) => !records.some((r) => r.repo_path === x.repo_path)) ? (
              <SkippedList items={extras.skipped.filter((x) => !records.some((r) => r.repo_path === x.repo_path))} />
            ) : null}
            {records.length > 0 ? (
              <UnsyncedFiles records={records} formatTime={formatTime} onOpen={onOpenMemoryFile} />
            ) : null}

            <ProjectMapping projects={projects} onChange={setProjects} notify={push} />
            <SyncHistory reloadKey={historyKey} notify={push} formatTime={formatTime} />
          </>
        )}
      </div>
      <ConfirmDialog
        open={confirmLogout}
        title={s.logoutTitle}
        body={s.logoutBody}
        confirmLabel={s.logoutConfirm}
        cancelLabel={s.keep}
        destructive
        onConfirm={() => {
          setConfirmLogout(false);
          void doLogout();
        }}
        onCancel={() => setConfirmLogout(false)}
      />
      <ToastRegion toasts={toasts} onDismiss={dismiss} />
    </div>
  );
}

/** Cloud files a download could not write here, grouped by project. */
export function AbsentList({ items }: { items: SyncAbsentFile[] }) {
  const { t } = useI18n();
  const s = t.sync;
  const groups = new Map<string, SyncAbsentFile[]>();
  for (const f of items) groups.set(f.project_key, [...(groups.get(f.project_key) ?? []), f]);
  return (
    <section aria-label={fmt(s.absentTitle, { n: items.length })} className="rounded-lg border border-border bg-card p-4">
      <h2 className="flex items-center gap-2 text-sm font-semibold">
        <FolderX className="size-4 text-warning" aria-hidden="true" />
        {fmt(s.absentTitle, { n: items.length })}
      </h2>
      <p className="mt-1 text-xs text-muted-foreground">{s.absentBody}</p>
      <ul className="mt-2 space-y-1.5">
        {[...groups.entries()].map(([key, files]) => (
          <li key={key} className="flex items-center gap-3 rounded-md border border-border bg-muted/40 px-3 py-2 dark:bg-muted/20">
            <div className="min-w-0 flex-1" title={files.map((f) => f.repo_path).join("\n")}>
              <p className="truncate font-mono text-sm font-semibold">{key}</p>
              <p className="text-xs text-muted-foreground">
                {files[0].reason === "no_place" ? s.absentNoPlace : s.absentNoProject}
              </p>
            </div>
            <span className="shrink-0 text-xs text-muted-foreground tabular-nums">{fmt(s.absentFiles, { n: files.length })}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** Files skipped by an earlier sync and not synced since (persisted on the backend). */
export function UnsyncedFiles({
  records,
  formatTime,
  onOpen,
}: {
  records: SyncSkippedRecord[];
  formatTime: (ms: number) => string;
  onOpen?: (fileId: string) => void;
}) {
  const { t } = useI18n();
  const s = t.sync;
  return (
    <section aria-label={fmt(s.unsyncedTitle, { n: records.length })} className="rounded-lg border border-warning/40 bg-card p-4">
      <h2 className="flex items-center gap-2 text-sm font-semibold">
        <FileWarning className="size-4 text-warning" aria-hidden="true" />
        {fmt(s.unsyncedTitle, { n: records.length })}
      </h2>
      <p className="mt-1 text-xs text-muted-foreground">{s.unsyncedBody}</p>
      <ul className="mt-2 space-y-1.5">
        {records.map((r) => {
          const shown = r.local_path ?? r.repo_path;
          const { name, dir } = splitPath(shown);
          const reason = s.reasons[r.reason] ?? r.reason;
          return (
            <li key={r.repo_path} className="flex items-center gap-3 rounded-md border border-border bg-muted/40 px-3 py-2 dark:bg-muted/20">
              <div className="min-w-0 flex-1" title={r.local_path ? `${r.local_path}\n${r.repo_path}` : r.repo_path}>
                <p className="truncate text-sm font-semibold">{name}</p>
                <p className="truncate font-mono text-xs text-muted-foreground">{middleEllipsis(dir, 60)}</p>
              </div>
              <div className="flex shrink-0 flex-col items-end gap-0.5">
                <span className="rounded-sm bg-warning/15 px-1.5 py-0.5 text-[11px] font-semibold text-warning">{reason}</span>
                <span className="text-[11px] text-muted-foreground tabular-nums">{fmt(s.skippedAt, { time: formatTime(r.at) })}</span>
              </div>
              {r.file_id && onOpen ? (
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => onOpen(r.file_id!)}
                  aria-label={`${s.openInMemory} ${name}`}
                  className="h-7 shrink-0 cursor-pointer px-2.5 text-xs"
                >
                  <SquareArrowOutUpRight className="size-3.5" aria-hidden="true" />
                  {s.openInMemory}
                </Button>
              ) : !r.local_path ? (
                <span className="shrink-0 text-xs text-muted-foreground">{s.noLocalFile}</span>
              ) : null}
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function VisibilityTag({ priv }: { priv: boolean | null }) {
  const { t } = useI18n();
  const s = t.sync;
  const text = priv === null ? s.unknownVisibility : priv ? s.private : s.public;
  return (
    <span
      className={cn(
        "inline-flex h-[18px] items-center rounded-sm px-1.5 text-xs font-semibold",
        priv === false ? "bg-destructive/15 text-destructive" : "bg-muted text-muted-foreground",
      )}
    >
      {text}
    </span>
  );
}

function NeedsBanner({ status, onAction }: { status: SyncStatus; onAction: () => void }) {
  const { t } = useI18n();
  const a = t.account;
  const name = status.repo ? `${status.repo.owner}/${status.repo.name}` : "MemDockHub";
  const text =
    status.needs === "public_ack"
      ? fmt(a.publicWarn, { name })
      : status.needs === "not_ours"
        ? fmt(a.notOurs, { name: status.repo?.name ?? "MemDockHub" })
        : a.relogin;
  return (
    <div role="alert" className="flex items-center gap-2.5 rounded-md bg-warning/15 px-3 py-2 text-sm">
      <TriangleAlert className="size-4 shrink-0 text-warning" aria-hidden="true" />
      <p className="min-w-0 flex-1">{text}</p>
      <Button type="button" variant="outline" onClick={onAction} className="h-7 shrink-0 cursor-pointer px-3 text-xs">
        {t.sync.needsAction}
      </Button>
    </div>
  );
}

export function SecretHits({ hits }: { hits: FileSecretHit[] }) {
  const { t } = useI18n();
  const s = t.sync;
  return (
    <section role="alert" aria-label={s.blockedTitle} className="rounded-lg border border-destructive/40 bg-destructive/10 p-4">
      <h2 className="flex items-center gap-2 text-sm font-semibold text-destructive">
        <ShieldAlert className="size-4" aria-hidden="true" />
        {s.blockedTitle}
      </h2>
      <p className="mt-1 text-sm">{s.blockedBody}</p>
      <ul className="mt-2 space-y-1">
        {hits.map((h, i) => (
          <li key={i} className="flex flex-wrap items-baseline gap-x-2 text-sm">
            <span className="font-mono text-xs">{h.repo_path}</span>
            <span className="text-xs text-muted-foreground">{fmt(s.hitLine, { line: h.line })}</span>
            <span className="text-xs font-semibold">{h.rule}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** Secrets found: let the user cancel, leave those files out, or sync them anyway. */
export function SecretsDialog({
  hits,
  busy,
  onCancel,
  onSkip,
  onAllow,
}: {
  hits: FileSecretHit[] | null;
  busy: boolean;
  onCancel: () => void;
  onSkip: () => void;
  onAllow: () => void;
}) {
  const { t } = useI18n();
  const s = t.sync;
  const files = hits ? new Set(hits.map((h) => h.repo_path)).size : 0;
  const skipHint = React.useId();
  const allowHint = React.useId();
  return (
    <Dialog open={!!hits} onOpenChange={(o) => (!o && !busy ? onCancel() : undefined)}>
      <DialogContent className="w-[600px] max-w-[calc(100vw-2rem)]">
        <DialogHeader className="pr-8">
          <DialogTitle className="flex items-center gap-2">
            <span className="grid size-8 shrink-0 place-items-center rounded-full bg-destructive/10 dark:bg-destructive/20">
              <ShieldAlert className="size-4 text-destructive" aria-hidden="true" />
            </span>
            {s.secretsTitle}
          </DialogTitle>
          <DialogDescription className="leading-relaxed">{fmt(s.secretsBody, { n: files })}</DialogDescription>
        </DialogHeader>
        <ul aria-label={s.secretsListLabel} className="mt-4 max-h-[280px] space-y-1.5 overflow-y-auto pr-1">
          {(hits ?? []).map((h, i) => {
            const { name, dir } = splitPath(h.repo_path);
            return (
              <li
                key={i}
                className="flex items-center gap-3 rounded-md border border-border bg-muted/40 px-3 py-2 dark:bg-muted/20"
              >
                <FileKey2 className="size-4 shrink-0 text-destructive/80" aria-hidden="true" />
                <div className="min-w-0 flex-1" title={h.repo_path}>
                  <p className="truncate text-sm font-semibold">{name}</p>
                  {dir ? <p className="truncate font-mono text-xs text-muted-foreground">{middleEllipsis(dir, 48)}</p> : null}
                </div>
                <span className="shrink-0 text-xs text-muted-foreground tabular-nums">{fmt(s.hitLine, { line: h.line })}</span>
                <span className="shrink-0 rounded-sm bg-destructive/10 px-1.5 py-0.5 font-mono text-[11px] font-semibold text-destructive dark:bg-destructive/20">
                  {h.rule}
                </span>
              </li>
            );
          })}
        </ul>
        <div className="mt-4 grid gap-2 sm:grid-cols-2">
          <button
            type="button"
            disabled={busy}
            onClick={onSkip}
            autoFocus
            aria-label={s.secretsSkip}
            aria-describedby={skipHint}
            className="flex cursor-pointer flex-col items-start gap-1 rounded-lg bg-primary px-3.5 py-3 text-left text-primary-foreground shadow-sm transition-colors duration-150 outline-none hover:bg-primary/90 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-60 motion-reduce:transition-none"
          >
            <span className="flex items-center gap-1.5 text-sm font-semibold">
              {busy ? (
                <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />
              ) : (
                <SkipForward className="size-4" aria-hidden="true" />
              )}
              {s.secretsSkip}
            </span>
            <span id={skipHint} className="text-xs leading-snug opacity-85">
              {s.secretsSkipHint}
            </span>
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={onAllow}
            aria-label={s.secretsAllow}
            aria-describedby={allowHint}
            className="flex cursor-pointer flex-col items-start gap-1 rounded-lg border border-destructive/60 bg-transparent px-3.5 py-3 text-left text-destructive transition-colors duration-150 outline-none hover:bg-destructive/10 focus-visible:ring-2 focus-visible:ring-destructive/50 disabled:cursor-not-allowed disabled:opacity-60 motion-reduce:transition-none dark:border-destructive/50 dark:hover:bg-destructive/15"
          >
            <span className="flex items-center gap-1.5 text-sm font-semibold">
              <Upload className="size-4" aria-hidden="true" />
              {s.secretsAllow}
            </span>
            <span id={allowHint} className="text-xs leading-snug text-destructive/80">
              {s.secretsAllowHint}
            </span>
          </button>
        </div>
        <DialogFooter className="mt-3">
          <Button type="button" variant="ghost" disabled={busy} onClick={onCancel} className="h-8 cursor-pointer px-3 text-sm text-muted-foreground">
            {s.secretsCancel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function SkippedList({ items }: { items: SyncSkipped[] }) {
  const { t } = useI18n();
  const s = t.sync;
  return (
    <section aria-label={fmt(s.skippedTitle, { n: items.length })} className="rounded-lg border border-border bg-card p-4">
      <h2 className="flex items-center gap-2 text-sm font-semibold">
        <FileWarning className="size-4 text-warning" aria-hidden="true" />
        {fmt(s.skippedTitle, { n: items.length })}
      </h2>
      <ul className="mt-2 space-y-1">
        {items.map((x) => (
          <li key={x.repo_path} className="flex flex-wrap items-baseline gap-x-2 text-sm">
            <span className="min-w-0 truncate font-mono text-xs">{x.repo_path}</span>
            <span className="text-xs text-muted-foreground">{s.reasons[x.reason] ?? x.reason}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

// ------------------------------------------------------------------ resolution view

type Sel = { kind: "conflict" | "remote_deleted"; key: string } | { kind: "held"; key: string };

interface ResolveViewProps {
  plan: SyncPlan;
  notify: (kind: ToastKind, message: string) => void;
  onFinished: (summary: SyncSummary) => void;
  onBlocked: (hits: FileSecretHit[]) => void;
  /** The plan is gone (discarded or no longer valid). */
  onAbort: (message: string | null) => void;
}

function ResolveView({ plan, notify, onFinished, onBlocked, onAbort }: ResolveViewProps) {
  const { t } = useI18n();
  const s = t.sync;
  const [conflicts, setConflicts] = React.useState<Record<string, ConflictState>>({});
  const [remoteDel, setRemoteDel] = React.useState<Record<string, "delete" | "keep">>({});
  const [held, setHeld] = React.useState<Record<string, "delete_remote" | "keep_remote">>({});
  const first: Sel | null = plan.conflicts[0]
    ? { kind: "conflict", key: plan.conflicts[0].repo_path }
    : plan.remote_deleted[0]
      ? { kind: "remote_deleted", key: plan.remote_deleted[0].repo_path }
      : plan.local_all_deleted[0]
        ? { kind: "held", key: plan.local_all_deleted[0].project_key }
        : null;
  const [sel, setSel] = React.useState<Sel | null>(first);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [confirmDiscard, setConfirmDiscard] = React.useState(false);

  const unresolved = plan.conflicts.filter((c) => resolutionOf(c, conflicts[c.repo_path]) === null);
  const ready = unresolved.length === 0;

  const finish = async () => {
    const res: Record<string, SyncResolution> = {};
    for (const c of plan.conflicts) {
      const r = resolutionOf(c, conflicts[c.repo_path]);
      if (r) res[c.repo_path] = r;
    }
    for (const f of plan.remote_deleted) {
      res[f.repo_path] = remoteDel[f.repo_path] === "delete" ? { kind: "delete_local" } : { kind: "keep_local" };
    }
    for (const g of plan.local_all_deleted) {
      const choice = held[g.project_key];
      if (choice) for (const f of g.files) res[f.repo_path] = { kind: choice };
    }
    setBusy(true);
    setError(null);
    try {
      onFinished(await syncFinish(plan.id, res));
    } catch (e) {
      const ce = asCommandError(e);
      if (ce?.code === "secrets" && ce.secret_hits) onBlocked(ce.secret_hits);
      else if (ce && PLAN_DEAD.has(ce.code)) onAbort(syncErrorText(t, e));
      else setError(syncErrorText(t, e));
    } finally {
      setBusy(false);
    }
  };

  const discard = async () => {
    try {
      await syncDiscard(plan.id);
    } catch (e) {
      notify("error", syncErrorText(t, e));
    }
    onAbort(null);
  };

  const item = (key: Sel, label: string, done: boolean, title?: string) => {
    const active = sel?.kind === key.kind && sel.key === key.key;
    const Icon = done ? CircleCheck : CircleDot;
    return (
      <li key={`${key.kind}:${key.key}`}>
        <button
          type="button"
          aria-current={active ? "true" : undefined}
          title={title ?? label}
          onClick={() => setSel(key)}
          className={cn(
            "flex h-8 w-full cursor-pointer items-center gap-2 rounded-md px-2 text-left text-sm transition-colors duration-150 outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none",
            active && "bg-muted font-medium shadow-[inset_2px_0_0_var(--primary)]",
          )}
        >
          <Icon className={cn("size-3.5 shrink-0", done ? "text-accent-text" : "text-warning")} aria-hidden="true" />
          <span className="min-w-0 flex-1 truncate font-mono text-xs">{label}</span>
          <span className="sr-only">{done ? s.resolved : s.unresolved}</span>
        </button>
      </li>
    );
  };

  const group = (title: string, children: React.ReactNode) => (
    <div className="mb-3">
      <h3 className="mb-1 px-2 text-xs font-semibold text-muted-foreground">{title}</h3>
      <ul className="space-y-0.5">{children}</ul>
    </div>
  );

  const selConflict = sel?.kind === "conflict" ? plan.conflicts.find((c) => c.repo_path === sel.key) : undefined;
  const selRemote = sel?.kind === "remote_deleted" ? plan.remote_deleted.find((f) => f.repo_path === sel.key) : undefined;
  const selHeld = sel?.kind === "held" ? plan.local_all_deleted.find((g) => g.project_key === sel.key) : undefined;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="border-b border-border px-6 py-4">
        <h1 className="text-xl font-semibold">{plan.download ? s.resolveTitleDown : s.resolveTitle}</h1>
        <p className="mt-1 text-sm text-muted-foreground">{plan.download ? s.resolveBodyDown : s.resolveBody}</p>
      </header>
      <div className="flex min-h-0 flex-1">
        <nav aria-label={s.resolveTitle} className="w-64 shrink-0 overflow-y-auto border-r border-border p-3">
          {plan.conflicts.length > 0
            ? group(
                s.groupConflicts,
                plan.conflicts.map((c) =>
                  item({ kind: "conflict", key: c.repo_path }, c.repo_path, resolutionOf(c, conflicts[c.repo_path]) !== null, c.local_path),
                ),
              )
            : null}
          {plan.remote_deleted.length > 0
            ? group(
                s.groupRemoteDeleted,
                plan.remote_deleted.map((f) => item({ kind: "remote_deleted", key: f.repo_path }, f.repo_path, true, f.local_path ?? f.repo_path)),
              )
            : null}
          {plan.local_all_deleted.length > 0
            ? group(
                s.groupLocalAllDeleted,
                plan.local_all_deleted.map((g) => item({ kind: "held", key: g.project_key }, g.project_key, !!held[g.project_key])),
              )
            : null}
          {plan.skipped.length > 0 ? <SkippedList items={plan.skipped} /> : null}
        </nav>
        <main className="min-w-0 flex-1 overflow-y-auto p-6">
          {selConflict ? (
            <>
              <p className="mb-3 truncate font-mono text-xs text-muted-foreground" title={selConflict.local_path}>
                {selConflict.local_path}
              </p>
              <ConflictResolver
                conflict={selConflict}
                value={conflicts[selConflict.repo_path]}
                onChange={(v) => setConflicts((prev) => ({ ...prev, [selConflict.repo_path]: v }))}
              />
            </>
          ) : selRemote ? (
            <div className="flex flex-col gap-3">
              <p className="truncate font-mono text-xs text-muted-foreground">{selRemote.local_path ?? selRemote.repo_path}</p>
              <p className="text-sm">{s.remoteDeletedHint}</p>
              <div className="flex gap-2">
                <ChoiceButton
                  active={remoteDel[selRemote.repo_path] === "delete"}
                  onClick={() => setRemoteDel((p) => ({ ...p, [selRemote.repo_path]: "delete" }))}
                >
                  {s.deleteLocal}
                </ChoiceButton>
                <ChoiceButton
                  active={remoteDel[selRemote.repo_path] !== "delete"}
                  onClick={() => setRemoteDel((p) => ({ ...p, [selRemote.repo_path]: "keep" }))}
                >
                  {s.keepFile}
                </ChoiceButton>
              </div>
            </div>
          ) : selHeld ? (
            <div className="flex flex-col gap-3">
              <p className="text-sm">{fmt(s.localAllDeletedHint, { project: selHeld.project_key })}</p>
              <div className="flex flex-wrap gap-2">
                <ChoiceButton
                  active={held[selHeld.project_key] === "delete_remote"}
                  onClick={() => setHeld((p) => ({ ...p, [selHeld.project_key]: "delete_remote" }))}
                >
                  {s.deleteRemoteAll}
                </ChoiceButton>
                <ChoiceButton
                  active={held[selHeld.project_key] === "keep_remote"}
                  onClick={() => setHeld((p) => ({ ...p, [selHeld.project_key]: "keep_remote" }))}
                >
                  {s.restoreAll}
                </ChoiceButton>
              </div>
              <ul className="mt-1 space-y-1 rounded-md border border-border bg-card p-3">
                {selHeld.files.map((f) => (
                  <li key={f.repo_path} className="truncate font-mono text-xs">
                    {f.repo_path}
                  </li>
                ))}
              </ul>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">{s.pickFile}</p>
          )}
        </main>
      </div>
      <footer className="flex items-center gap-3 border-t border-border px-6 py-3">
        {error ? (
          <div className="min-w-0 flex-1">
            <InlineError message={error} />
          </div>
        ) : (
          <p className="flex-1 text-sm text-muted-foreground">
            {ready ? s.resolved : `${s.unresolved}: ${unresolved.length}`}
          </p>
        )}
        <Button
          type="button"
          variant="ghost"
          disabled={busy}
          onClick={() => setConfirmDiscard(true)}
          className="h-8 cursor-pointer px-3 text-sm text-destructive"
        >
          {s.discard}
        </Button>
        <Button type="button" disabled={!ready || busy} onClick={() => void finish()} className="h-8 cursor-pointer px-4 text-sm font-semibold">
          {busy ? <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" /> : null}
          {busy ? s.finishing : plan.download ? s.finishDown : s.finish}
        </Button>
      </footer>
      <ConfirmDialog
        open={confirmDiscard}
        title={s.discardTitle}
        body={s.discardBody}
        confirmLabel={s.discardConfirm}
        cancelLabel={s.keep}
        destructive
        onConfirm={() => {
          setConfirmDiscard(false);
          void discard();
        }}
        onCancel={() => setConfirmDiscard(false)}
      />
    </div>
  );
}
