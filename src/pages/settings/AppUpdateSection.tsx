import { Download, LoaderCircle, RefreshCw } from "lucide-react";
import { ChatMarkdown } from "@/components/chat/ChatMarkdown";
import { Button } from "@/components/ui/button";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";
import type { AppUpdate } from "./useAppUpdate";

export function AppUpdateSection({ state }: { state: AppUpdate }) {
  const { t: { appUpdate: t } } = useI18n();
  const { enabled, version, update, status, progress, blocked, restartFailed } = state;
  const busy = status === "checking" || status === "downloading" || status === "installing";
  const installed = status === "installed";
  const transferring = status === "downloading" || status === "installing";
  const percent = status === "installing" ? 100 : progress;
  const message = !enabled ? t.development
    : status === "error" ? update ? t.installError : t.checkError
    : status === "available" && update ? fmt(t.available, { version: update.version })
    : status === "idle" ? t.schedule
    : t[status];

  return (
    <section className="flex flex-col gap-3 rounded-lg border border-border bg-card p-4" aria-labelledby="app-update">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div className="space-y-1">
          <h2 id="app-update" className="flex items-center gap-2 text-sm font-semibold">
            <Download className="size-4 text-muted-foreground" aria-hidden="true" />
            {t.title}
          </h2>
          <p className="text-xs text-muted-foreground">{fmt(t.version, { version: version || "—" })}</p>
        </div>
        {enabled && (
          <Button
            type="button"
            size="sm"
            variant={update ? "default" : "outline"}
            disabled={busy || (Boolean(update) && blocked)}
            onClick={() => void (installed ? state.restart() : update ? state.install() : state.checkNow())}
          >
            {update && !installed ? <Download aria-hidden="true" /> : <RefreshCw aria-hidden="true" />}
            {installed ? t.restart : update ? t.install : t.check}
          </Button>
        )}
      </header>
      <div role="status" className="space-y-2 text-xs text-muted-foreground">
        {transferring ? (
          <div className="space-y-2 rounded-md border border-border bg-muted/40 px-3 py-2.5">
            <div className="flex items-center justify-between gap-3">
              <p className="flex items-center gap-2 font-medium text-foreground">
                <LoaderCircle className="size-3.5 shrink-0 text-primary motion-safe:animate-spin" aria-hidden />
                {message}
              </p>
              {percent !== undefined && <span className="font-medium tabular-nums">{percent}%</span>}
            </div>
            <div
              role="progressbar"
              aria-label={message}
              aria-valuenow={percent}
              className="h-1.5 overflow-hidden rounded-full bg-primary/15"
            >
              {percent === undefined ? (
                <div className="h-full w-1/3 rounded-full bg-primary motion-safe:animate-[update-download_1.5s_ease-in-out_infinite]" />
              ) : (
                <div className="h-full rounded-full bg-primary motion-safe:transition-[width] motion-safe:duration-300" style={{ width: `${percent}%` }} />
              )}
            </div>
          </div>
        ) : <p>{message}</p>}
        {update && blocked && !busy && <p>{t.blocked}</p>}
        {restartFailed && <p>{t.restartError}</p>}
      </div>
      {update?.body && !installed && (
        <details className="rounded-md border border-border px-3 py-2 text-xs">
          <summary className="cursor-pointer font-medium">{t.notes}</summary>
          <div className="mt-2 max-h-72 overflow-auto pr-2 text-muted-foreground">
            <ChatMarkdown text={update.body} />
          </div>
        </details>
      )}
    </section>
  );
}
