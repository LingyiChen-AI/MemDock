import { Download, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";
import type { AppUpdate } from "./useAppUpdate";

export function AppUpdateSection({ state }: { state: AppUpdate }) {
  const { t: { appUpdate: t } } = useI18n();
  const { enabled, version, update, status, progress, blocked, restartFailed } = state;
  const busy = status === "checking" || status === "downloading" || status === "installing";
  const installed = status === "installed";
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
        <p>{message}</p>
        {status === "downloading" && <progress aria-label={t.downloading} value={progress} max={100} className="w-full accent-primary" />}
        {update && blocked && !busy && <p>{t.blocked}</p>}
        {restartFailed && <p>{t.restartError}</p>}
      </div>
      {update?.body && !installed && (
        <details className="rounded-md border border-border px-3 py-2 text-xs">
          <summary className="cursor-pointer font-medium">{t.notes}</summary>
          <p className="mt-2 max-h-44 overflow-auto whitespace-pre-wrap break-words text-muted-foreground">{update.body}</p>
        </details>
      )}
    </section>
  );
}
