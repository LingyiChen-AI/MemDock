import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { CircleDot, CircleCheck, ExternalLink, FileWarning, Loader2, LogIn, LogOut, RefreshCw, ShieldAlert, TriangleAlert } from "lucide-react";
import { logout, syncDiscard, syncFinish, syncProjects, syncStart, syncStatus } from "@/api/sync";
import type {
  FileSecretHit,
  ManifestSkill,
  SyncPlan,
  SyncProject,
  SyncResolution,
  SyncSkipped,
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

export interface SyncPageProps {
  status: SyncStatus | null;
  onStatus: (s: SyncStatus) => void;
  onOpenLogin: () => void;
  /** An open resolution counts as unsaved work. */
  onDirtyChange?: (dirty: boolean) => void;
}

interface Extras {
  skipped: SyncSkipped[];
  missing: ManifestSkill[];
  removed: ManifestSkill[];
}

/** Errors after which the plan can no longer be finished. */
const PLAN_DEAD = new Set(["plan_not_found", "local_changed", "remote_moved", "unauthorized"]);

export function SyncPage({ status, onStatus, onOpenLogin, onDirtyChange }: SyncPageProps) {
  const { t, language } = useI18n();
  const s = t.sync;
  const { toasts, push, dismiss } = useToasts();
  const [syncing, setSyncing] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [hits, setHits] = React.useState<FileSecretHit[] | null>(null);
  const [extras, setExtras] = React.useState<Extras | null>(null);
  const [plan, setPlan] = React.useState<SyncPlan | null>(null);
  const [projects, setProjects] = React.useState<SyncProject[] | null>(null);
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

  const run = async (secrets: "block" | "allow" | "skip" = "block") => {
    setSyncing(true);
    setError(null);
    setHits(null);
    setExtras(null);
    try {
      const r = await syncStart(secrets);
      if (r.kind === "done") {
        const sm = finished(r.summary, { skipped: r.skipped, missing: r.missing_skills, removed: r.removed_elsewhere });
        push("success", fmt(s.doneToast, { pulled: sm.pulled, pushed: sm.pushed, merged: sm.merged }));
      } else if (r.kind === "blocked") {
        setHits(r.secret_hits);
      } else {
        setPlan(r.plan);
      }
    } catch (e) {
      handleError(e);
    } finally {
      setSyncing(false);
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
            finished(summary, { skipped: p.skipped, missing: p.missing_skills, removed: p.removed_elsewhere });
            push("success", fmt(s.finishedToast, { pulled: summary.pulled, pushed: summary.pushed, merged: summary.merged }));
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

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex max-w-4xl flex-col gap-4 p-6">
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
            <Button type="button" onClick={onOpenLogin} className="mt-2 h-8 cursor-pointer px-4 text-sm font-semibold">
              <LogIn className="size-4" aria-hidden="true" />
              {t.account.login}
            </Button>
          </div>
        ) : (
          <>
            {status.needs ? <NeedsBanner status={status} onAction={onOpenLogin} /> : null}

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
                <div className="flex shrink-0 items-center gap-2">
                  <Button
                    type="button"
                    variant="ghost"
                    disabled={syncing}
                    onClick={() => setConfirmLogout(true)}
                    className="h-8 cursor-pointer px-3 text-sm text-muted-foreground"
                  >
                    <LogOut className="size-4" aria-hidden="true" />
                    {s.logout}
                  </Button>
                  <Button
                    type="button"
                    disabled={syncing || !!status.needs}
                    onClick={() => void run()}
                    className="h-8 cursor-pointer px-4 text-sm font-semibold"
                  >
                    {syncing ? (
                      <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />
                    ) : (
                      <RefreshCw className="size-4" aria-hidden="true" />
                    )}
                    {syncing ? s.syncing : s.syncNow}
                  </Button>
                </div>
              </div>
            </section>

            {error ? <InlineError message={error} /> : null}
            <SecretsDialog
              hits={hits}
              busy={syncing}
              onCancel={() => setHits(null)}
              onSkip={() => void run("skip")}
              onAllow={() => void run("allow")}
            />
            {extras && extras.skipped.length > 0 ? <SkippedList items={extras.skipped} /> : null}
            {extras ? <MissingSkills missing={extras.missing} removed={extras.removed} projects={projects} notify={push} /> : null}

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
  return (
    <Dialog open={!!hits} onOpenChange={(o) => (!o && !busy ? onCancel() : undefined)}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ShieldAlert className="size-5 text-destructive" aria-hidden="true" />
            {s.secretsTitle}
          </DialogTitle>
          <DialogDescription>{fmt(s.secretsBody, { n: files })}</DialogDescription>
        </DialogHeader>
        <ul className="max-h-64 space-y-1.5 overflow-y-auto rounded-md border border-border bg-muted/40 p-3">
          {(hits ?? []).map((h, i) => (
            <li key={i} className="flex flex-wrap items-baseline gap-x-2 text-sm">
              <span className="min-w-0 font-mono text-xs break-all">{h.repo_path}</span>
              <span className="text-xs text-muted-foreground">{fmt(s.hitLine, { line: h.line })}</span>
              <span className="rounded bg-destructive/10 px-1.5 text-xs font-semibold text-destructive">{h.rule}</span>
            </li>
          ))}
        </ul>
        <p className="text-xs text-muted-foreground">{s.secretsNote}</p>
        <DialogFooter className="gap-2">
          <Button type="button" variant="outline" disabled={busy} onClick={onCancel} className="cursor-pointer">
            {s.secretsCancel}
          </Button>
          <Button type="button" variant="outline" disabled={busy} onClick={onAllow} className="cursor-pointer border-destructive/50 text-destructive hover:bg-destructive/10">
            {s.secretsAllow}
          </Button>
          <Button type="button" disabled={busy} onClick={onSkip} autoFocus className="cursor-pointer font-semibold">
            {busy ? <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" /> : null}
            {s.secretsSkip}
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
        <h1 className="text-xl font-semibold">{s.resolveTitle}</h1>
        <p className="mt-1 text-sm text-muted-foreground">{s.resolveBody}</p>
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
          {busy ? s.finishing : s.finish}
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
