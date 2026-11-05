import * as React from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { restoreFromHistory, syncFileAt, syncHistory } from "@/api/sync";
import type { SyncCommit } from "@/api/types";
import { Button } from "@/components/ui/button";
import { useI18n } from "@/i18n/I18nProvider";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { fmt } from "@/pages/memory/paths";
import { FlowDialog } from "@/pages/skills/flowParts";
import type { ToastKind } from "@/pages/skills/Toasts";
import { syncErrorText } from "@/components/account/syncErrors";

export interface SyncHistoryProps {
  /** Bump to reload. */
  reloadKey: number;
  notify: (kind: ToastKind, message: string) => void;
  formatTime: (ms: number) => string;
}

export function SyncHistory({ reloadKey, notify, formatTime }: SyncHistoryProps) {
  const { t } = useI18n();
  const s = t.sync;
  const [commits, setCommits] = React.useState<SyncCommit[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [open, setOpen] = React.useState<string | null>(null);
  const [viewing, setViewing] = React.useState<{ path: string; text: string | null } | null>(null);
  const [restore, setRestore] = React.useState<{ commit: string; path: string } | null>(null);
  const tRef = React.useRef(t);
  tRef.current = t;

  React.useEffect(() => {
    let alive = true;
    setError(null);
    syncHistory(30)
      .then((c) => alive && setCommits(c))
      .catch((e) => alive && setError(syncErrorText(tRef.current, e)));
    return () => {
      alive = false;
    };
  }, [reloadKey]);

  const view = async (commit: string, path: string) => {
    try {
      setViewing({ path, text: await syncFileAt(commit, path) });
    } catch (e) {
      notify("error", syncErrorText(t, e));
    }
  };

  const doRestore = async (commit: string, path: string) => {
    try {
      await restoreFromHistory(commit, path);
      notify("success", fmt(s.restored, { path }));
    } catch (e) {
      notify("error", syncErrorText(t, e));
    }
  };

  return (
    <section aria-label={s.historyTitle} className="rounded-lg border border-border bg-card">
      <h2 className="px-4 pt-4 pb-2 text-sm font-semibold">{s.historyTitle}</h2>
      {error ? (
        <p role="alert" className="px-4 pb-4 text-sm text-destructive">
          {error}
        </p>
      ) : commits === null ? (
        <p className="px-4 pb-4 text-sm text-muted-foreground">{s.loading}</p>
      ) : commits.length === 0 ? (
        <p className="px-4 pb-4 text-sm text-muted-foreground">{s.historyEmpty}</p>
      ) : (
        <ul className="divide-y divide-border border-t border-border">
          {commits.map((c) => {
            const expanded = open === c.id;
            const Chevron = expanded ? ChevronDown : ChevronRight;
            return (
              <li key={c.id}>
                <button
                  type="button"
                  aria-expanded={expanded}
                  onClick={() => setOpen(expanded ? null : c.id)}
                  className="flex w-full cursor-pointer items-center gap-2 px-4 py-2 text-left text-sm transition-colors duration-150 outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset motion-reduce:transition-none"
                >
                  <Chevron className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                  <span className="w-40 shrink-0 text-muted-foreground tabular-nums">{formatTime(c.time)}</span>
                  <span className="min-w-0 flex-1 truncate">{c.message}</span>
                  <span className="shrink-0 text-xs text-muted-foreground">{fmt(s.files, { n: c.files.length })}</span>
                </button>
                {expanded ? (
                  <ul className="bg-muted/40 pb-1">
                    {c.files.map((f) => (
                      <li key={f} className="flex items-center gap-2 py-1 pr-4 pl-10">
                        <span className="min-w-0 flex-1 truncate font-mono text-xs" title={f}>
                          {f}
                        </span>
                        <Button type="button" variant="ghost" onClick={() => void view(c.id, f)} className="h-6 shrink-0 cursor-pointer px-2 text-xs">
                          {s.view}
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          onClick={() => setRestore({ commit: c.id, path: f })}
                          className="h-6 shrink-0 cursor-pointer px-2 text-xs"
                        >
                          {s.restore}
                        </Button>
                      </li>
                    ))}
                  </ul>
                ) : null}
              </li>
            );
          })}
        </ul>
      )}
      <div className="h-2" />
      <FlowDialog open={viewing !== null} onOpenChange={(o) => (o ? undefined : setViewing(null))} title={viewing?.path ?? ""} description="" width={720}>
        {viewing?.text === null ? (
          <p className="mt-3 text-sm text-muted-foreground">{s.absent}</p>
        ) : (
          <pre className="mt-3 min-h-0 flex-1 overflow-auto rounded-md border border-border bg-card p-3 font-content text-sm whitespace-pre-wrap">
            {viewing?.text}
          </pre>
        )}
      </FlowDialog>
      <ConfirmDialog
        open={restore !== null}
        title={s.restoreTitle}
        body={fmt(s.restoreBody, { path: restore?.path ?? "" })}
        confirmLabel={s.restoreConfirm}
        cancelLabel={s.keep}
        onConfirm={() => {
          if (restore) void doRestore(restore.commit, restore.path);
          setRestore(null);
        }}
        onCancel={() => setRestore(null)}
      />
    </section>
  );
}
