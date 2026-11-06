import * as React from "react";
import { CloudDownload, CloudUpload, Loader2, RefreshCw } from "lucide-react";
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { getExtractConfig, setExtractConfig } from "@/api/extract";
import { getSettings } from "@/api/memory";
import { prefsDownload, prefsMarkApplied, prefsSetWithMemory, prefsSyncInfo, prefsUpload, syncStatus } from "@/api/sync";
import { setClaudeQuotaEnabled, setCodexQuotaEnabled } from "@/api/usage";
import type { PrefsFile, PrefsInfo, SyncStatus } from "@/api/types";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";
import { asCommandError, errorMessage } from "@/pages/skills/skillUtils";
import { usePrefs } from "@/prefs/PrefsProvider";
import {
  FLAG_KEYS,
  applyFrontendPrefs,
  collectFrontendPrefs,
  diffPrefs,
  mergeExtract,
  type PrefChange,
} from "@/prefs/prefsSync";

type Notify = (kind: "success" | "error" | "info", message: string) => void;

function when(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

export interface PrefsSyncSectionProps {
  notify: Notify;
  /** Backend settings changed (the page re-reads its switches). */
  onApplied?: () => void;
  /** Renders the card frame around the content. */
  frame: (children: React.ReactNode) => React.ReactElement;
}

/** Settings card: upload / fetch memdock_settings.json; hidden until GitHub sync is set up. */
export function PrefsSyncSection({ notify, onApplied, frame }: PrefsSyncSectionProps) {
  const { t } = useI18n();
  const ps = t.settings.prefsSync;
  const prefs = usePrefs();
  const [status, setStatus] = React.useState<SyncStatus | null>(null);
  const [info, setInfo] = React.useState<PrefsInfo | null>(null);
  const [busy, setBusy] = React.useState<"up" | "down" | "apply" | null>(null);
  const [pending, setPending] = React.useState<{ file: PrefsFile; changes: PrefChange[] } | null>(null);
  const [needReload, setNeedReload] = React.useState(false);

  React.useEffect(() => {
    let alive = true;
    void syncStatus()
      .then((s) => alive && setStatus(s))
      .catch(() => undefined);
    void prefsSyncInfo()
      .then((i) => alive && setInfo(i))
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  if (!status?.logged_in || !status.repo || status.needs) return null;

  const failed = (e: unknown) => {
    const code = asCommandError(e)?.code;
    if (code === "prefs_missing") notify("info", ps.missing);
    else if (code === "prefs_too_new") notify("error", ps.tooNew);
    else if (code === "prefs_invalid") notify("error", fmt(ps.invalid, { error: errorMessage(e) }));
    else if (code === "secrets") notify("error", ps.secrets);
    else notify("error", errorMessage(e));
  };

  const upload = async () => {
    setBusy("up");
    try {
      const r = await prefsUpload(collectFrontendPrefs());
      setInfo(r.info);
      notify("success", r.committed ? ps.uploaded : ps.uploadSame);
    } catch (e) {
      failed(e);
    } finally {
      setBusy(null);
    }
  };

  const download = async () => {
    setBusy("down");
    try {
      const file = await prefsDownload();
      const [settings, extract] = await Promise.all([getSettings(), getExtractConfig().catch(() => null)]);
      const changes = diffPrefs(file, { settings, extract });
      if (changes.length === 0) {
        setInfo(await prefsMarkApplied(file.updated_at));
        notify("info", ps.upToDate);
      } else {
        setPending({ file, changes });
      }
    } catch (e) {
      failed(e);
    } finally {
      setBusy(null);
    }
  };

  const apply = async () => {
    if (!pending) return;
    const { file, changes } = pending;
    setPending(null);
    setBusy("apply");
    try {
      const be = file.backend;
      const touched = new Set(changes.filter((c) => c.group === "backend").map((c) => c.key));
      // Through the app's own setters, so theme / language / accent switch live.
      if (touched.has("language")) prefs.setLanguage(be.language ?? "system");
      if (touched.has("theme")) prefs.setTheme(be.theme ?? "dark");
      if (touched.has("accent")) prefs.setAccent(be.accent ?? "green");
      if (touched.has("claude_quota_enabled")) await setClaudeQuotaEnabled(Boolean(be.claude_quota_enabled));
      if (touched.has("codex_quota_enabled")) await setCodexQuotaEnabled(Boolean(be.codex_quota_enabled));
      if (changes.some((c) => c.group === "extract")) {
        const local = await getExtractConfig();
        await setExtractConfig(mergeExtract(local, be.extract));
      }
      const frontendChanged = applyFrontendPrefs(file.frontend ?? {});
      setInfo(await prefsMarkApplied(file.updated_at));
      setNeedReload(frontendChanged);
      onApplied?.();
      notify("success", ps.applied);
    } catch (e) {
      failed(e);
    } finally {
      setBusy(null);
    }
  };

  const toggleWithMemory = async () => {
    if (!info) return;
    try {
      setInfo(await prefsSetWithMemory(!info.with_memory));
    } catch (e) {
      failed(e);
    }
  };

  const label = (c: PrefChange) => {
    const labels = ps.labels as Record<string, string>;
    const base = labels[c.group === "extract" ? `extract.${c.key}` : c.key] ?? c.key;
    return c.path ? `${base} · ${c.path}` : base;
  };

  const show = (c: PrefChange, v: unknown): string => {
    if (v === null || v === undefined || v === "") return ps.default;
    if (typeof v === "boolean") return v ? ps.on : ps.off;
    if (FLAG_KEYS.has(c.key) && (v === "1" || v === "0")) return v === "1" ? ps.on : ps.off;
    const s = typeof v === "string" ? v : JSON.stringify(v);
    return s.length > 80 ? `${s.slice(0, 80)}…` : s;
  };

  const remote = info?.remote;
  return frame(
    <div className="flex flex-col gap-3">
      <p className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        {remote ? fmt(ps.remote, { time: when(remote.updated_at), device: remote.device || "?" }) : ps.remoteNone}
        {info?.newer ? <span className="rounded-full bg-primary/12 px-2 py-0.5 text-[11px] font-medium text-accent-text">{ps.newer}</span> : null}
      </p>
      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" variant="outline" size="sm" disabled={busy !== null} onClick={() => void upload()} className="cursor-pointer">
          {busy === "up" ? <Loader2 className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <CloudUpload aria-hidden="true" />}
          {busy === "up" ? ps.uploading : ps.upload}
        </Button>
        <Button type="button" variant="outline" size="sm" disabled={busy !== null} onClick={() => void download()} className="cursor-pointer">
          {busy === "down" || busy === "apply" ? <Loader2 className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <CloudDownload aria-hidden="true" />}
          {busy === "down" ? ps.downloading : ps.download}
        </Button>
      </div>
      <label className="flex cursor-pointer items-center gap-3 text-sm">
        <Switch checked={info?.with_memory ?? true} disabled={!info} onCheckedChange={() => void toggleWithMemory()} aria-label={ps.withMemory} />
        {ps.withMemory}
      </label>
      {needReload ? (
        <p className="flex flex-wrap items-center gap-2 rounded-md bg-muted px-3 py-2 text-sm">
          {ps.reloadHint}
          <Button type="button" variant="outline" size="xs" className="cursor-pointer" onClick={() => window.location.reload()}>
            <RefreshCw aria-hidden="true" />
            {ps.reload}
          </Button>
        </p>
      ) : null}
      <AlertDialog open={pending !== null} onOpenChange={(o) => !o && setPending(null)}>
        <AlertDialogContent className="sm:max-w-2xl">
          <AlertDialogTitle>{ps.confirmTitle}</AlertDialogTitle>
          <AlertDialogDescription className="mt-2">
            {pending ? fmt(ps.confirmBody, { device: pending.file.device || "?", time: when(pending.file.updated_at) }) : null}
          </AlertDialogDescription>
          {pending ? (
            <div className="mt-3 max-h-80 overflow-auto rounded-md border border-border">
              <table className="w-full text-left text-xs">
                <thead className="bg-muted text-muted-foreground">
                  <tr>
                    <th className="px-3 py-1.5 font-medium">{ps.colSetting}</th>
                    <th className="px-3 py-1.5 font-medium">{ps.colLocal}</th>
                    <th className="px-3 py-1.5 font-medium">{ps.colRemote}</th>
                  </tr>
                </thead>
                <tbody>
                  {pending.changes.map((c) => (
                    <tr key={`${c.group}:${c.key}:${c.path}`} className="border-t border-border">
                      <td className="px-3 py-1.5">{label(c)}</td>
                      <td className="px-3 py-1.5 font-mono text-muted-foreground break-all">{show(c, c.from)}</td>
                      <td className="px-3 py-1.5 font-mono break-all">{show(c, c.to)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : null}
          <AlertDialogFooter>
            <Button variant="outline" className="h-7 cursor-pointer px-3 text-xs" onClick={() => setPending(null)}>
              {ps.cancel}
            </Button>
            <Button className="h-7 cursor-pointer px-3 text-xs" onClick={() => void apply()}>
              {ps.apply}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>,
  );
}
