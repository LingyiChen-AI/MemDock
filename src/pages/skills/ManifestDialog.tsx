import * as React from "react";
import { open as pickOpen, save as pickSave } from "@tauri-apps/plugin-dialog";
import { ChevronRight, Download, FileDown, FileUp, Info, Loader2 } from "lucide-react";
import { skillManifestCompare, skillManifestExport, skillManifestImport } from "@/api/skills";
import type { ManifestCompare, ManifestSkill, MissingManifestSkill, SkillManifest } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { LoadingState } from "@/components/ui/loading-state";
import { useI18n } from "@/i18n/I18nProvider";
import { cn } from "@/lib/utils";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { basename, fmt } from "@/pages/memory/paths";
import { FlowDialog, InlineError, describeError } from "./flowParts";
import { installFromManifest, isLocalOnly } from "./manifestInstall";
import { errorMessage } from "./skillUtils";
import type { ToastKind } from "./Toasts";

export interface ManifestDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  notify: (kind: ToastKind, message: string) => void;
  /** Opens the sync page; the button is hidden without it. */
  onOpenSync?: () => void;
  /** Called after at least one skill was installed. */
  onInstalled?: () => void;
}

type Source = { kind: "cloud" } | { kind: "file"; name: string; manifest: SkillManifest };
type Result = { ok: true } | { ok: false; message: string };

/** Identity used by the backend: url + path + tool + scope, or name + tool + scope for local-only entries. */
export function manifestKey(m: ManifestSkill): string {
  const scope = m.scope.type === "global" ? "" : m.scope.key;
  if (m.plugin) return `${m.tool}|${scope}|plugin:${m.plugin.key}|${Boolean(m.plugin.local_scope)}`;
  return isLocalOnly(m) ? `${m.tool}|${scope}|local:${m.name}` : `${m.tool}|${scope}|${m.url}|${m.skill_path}`;
}

const canInstall = (m: MissingManifestSkill) => !isLocalOnly(m) && m.local_scope !== null;

export function ManifestDialog({ open, onOpenChange, notify, onOpenSync, onInstalled }: ManifestDialogProps) {
  const { t } = useI18n();
  const s = t.skills.manifest;
  const [source, setSource] = React.useState<Source>({ kind: "cloud" });
  const [data, setData] = React.useState<ManifestCompare | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [selected, setSelected] = React.useState<Set<string>>(() => new Set());
  const [results, setResults] = React.useState<Map<string, Result>>(() => new Map());
  const [installing, setInstalling] = React.useState(false);
  const [confirm, setConfirm] = React.useState(false);
  const [bothOpen, setBothOpen] = React.useState(false);
  const [busyFile, setBusyFile] = React.useState(false);
  const seq = React.useRef(0);

  const load = React.useCallback(async (src: Source) => {
    const n = ++seq.current;
    setData(null);
    setError(null);
    try {
      const res = await skillManifestCompare(src.kind === "file" ? src.manifest : null);
      if (n === seq.current) setData(res);
    } catch (e) {
      if (n === seq.current) setError(errorMessage(e));
    }
  }, []);

  React.useEffect(() => {
    if (!open) return;
    setSelected(new Set());
    setResults(new Map());
    void load(source);
  }, [open, source, load]);

  const installable = (data?.missing_here ?? []).filter(canInstall);
  const chosen = installable.filter((m) => selected.has(manifestKey(m)));

  const toggle = (k: string, on: boolean) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (on) next.add(k);
      else next.delete(k);
      return next;
    });

  /** One at a time: installs share the skills directory and the registry. */
  const installChosen = async (list: MissingManifestSkill[]) => {
    setInstalling(true);
    let ok = 0;
    let fail = 0;
    for (const m of list) {
      const k = manifestKey(m);
      try {
        await installFromManifest(m, m.local_scope!);
        ok++;
        setResults((prev) => new Map(prev).set(k, { ok: true }));
      } catch (e) {
        fail++;
        setResults((prev) => new Map(prev).set(k, { ok: false, message: describeError(e, t.skills) }));
      }
      toggle(k, false);
    }
    setInstalling(false);
    notify(fail === 0 ? "success" : "error", fmt(s.installDone, { ok, fail }));
    if (ok > 0) onInstalled?.();
  };

  const doExport = async () => {
    try {
      const path = await pickSave({ defaultPath: "skills.json", filters: [{ name: s.fileFilter, extensions: ["json"] }] });
      if (!path) return;
      setBusyFile(true);
      const n = await skillManifestExport(path);
      notify("success", fmt(s.exported, { n, path }));
    } catch (e) {
      notify("error", fmt(s.exportFailed, { message: errorMessage(e) }));
    } finally {
      setBusyFile(false);
    }
  };

  const doImport = async () => {
    try {
      const path = await pickOpen({ multiple: false, directory: false, filters: [{ name: s.fileFilter, extensions: ["json"] }] });
      if (typeof path !== "string") return;
      setBusyFile(true);
      const manifest = await skillManifestImport(path);
      setSource({ kind: "file", name: basename(path), manifest });
    } catch (e) {
      notify("error", fmt(s.importFailed, { message: errorMessage(e) }));
    } finally {
      setBusyFile(false);
    }
  };

  const scopeText = (m: ManifestSkill) => (m.scope.type === "global" ? s.globalScope : fmt(s.projectScope, { key: m.scope.key }));
  const meta = (m: ManifestSkill) => (
    <p className="truncate text-xs text-muted-foreground" title={m.plugin?.key || m.url || undefined}>
      {m.plugin ? <span className="font-mono">{m.plugin.version ?? m.plugin.key}</span> : m.url ? <span className="font-mono">{m.url.replace(/^https?:\/\//, "")}{m.skill_path ? `/${m.skill_path}` : ""}</span> : s.localOnlyTag}
      {" · "}
      {m.tool === "claude" ? "Claude" : "Codex"}
      {" · "}
      {scopeText(m)}
    </p>
  );

  const sourceLine =
    source.kind === "file"
      ? fmt(s.sourceFile, { name: source.name })
      : data?.remote_updated_at
        ? fmt(s.sourceCloudAt, { time: new Date(data.remote_updated_at).toLocaleString() })
        : s.sourceCloud;

  let body: React.ReactNode;
  if (error) {
    body = (
      <div className="mt-4 space-y-3">
        <InlineError message={fmt(s.loadFailed, { message: error })} />
        <Button type="button" variant="outline" onClick={() => void load(source)} className="h-7 cursor-pointer px-3 text-xs">
          {s.retry}
        </Button>
      </div>
    );
  } else if (!data) {
    body = <LoadingState label={s.loading} className="min-h-48" />;
  } else {
    const missing = data.missing_here;
    const localOnlyHere = data.local_only.length;
    body = (
      <div className="mt-4 min-h-0 flex-1 space-y-4 overflow-y-auto pr-1">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <span className="min-w-0 flex-1 truncate">{sourceLine}</span>
          {source.kind === "file" ? (
            <Button type="button" variant="ghost" onClick={() => setSource({ kind: "cloud" })} className="h-6 cursor-pointer px-2 text-xs">
              {s.backToCloud}
            </Button>
          ) : null}
        </div>
        {data.remote === null ? (
          <div role="note" className="flex items-start gap-2 rounded-md bg-muted px-3 py-2 text-xs text-muted-foreground">
            <Info className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
            <span>{data.logged_in ? s.noRemoteLoggedIn : s.noRemoteLoggedOut}</span>
          </div>
        ) : null}

        {data.remote !== null ? (
          <section aria-label={fmt(s.missingTitle, { n: missing.length })} className="rounded-lg border border-border">
            <div className="flex items-center gap-2 px-3 pt-3 pb-2">
              <h3 className="flex-1 text-sm font-semibold">{fmt(s.missingTitle, { n: missing.length })}</h3>
              {installable.length > 0 ? (
                <Button
                  type="button"
                  variant="ghost"
                  disabled={installing}
                  onClick={() => setSelected(new Set(installable.map(manifestKey)))}
                  className="h-7 cursor-pointer px-2 text-xs"
                >
                  {s.selectAll}
                </Button>
              ) : null}
              <Button
                type="button"
                disabled={installing || chosen.length === 0}
                onClick={() => setConfirm(true)}
                className="h-7 cursor-pointer gap-1.5 px-3 text-xs font-semibold"
              >
                {installing ? (
                  <Loader2 className="size-3.5 animate-spin motion-reduce:animate-none" aria-hidden="true" />
                ) : (
                  <Download className="size-3.5" aria-hidden="true" />
                )}
                {installing ? s.installing : fmt(s.installSelected, { n: chosen.length })}
              </Button>
            </div>
            {missing.length === 0 ? (
              <p className="border-t border-border px-3 py-2.5 text-xs text-muted-foreground">{s.missingEmpty}</p>
            ) : (
              <ul className="divide-y divide-border border-t border-border">
                {missing.map((m) => {
                  const k = manifestKey(m);
                  const ok = canInstall(m);
                  const r = results.get(k);
                  const id = `manifest-${k}`;
                  return (
                    <li key={k} className={cn("flex items-center gap-3 px-3 py-2", !ok && "opacity-70")}>
                      <Checkbox
                        id={id}
                        aria-label={m.name}
                        checked={selected.has(k)}
                        disabled={!ok || installing || r?.ok === true}
                        onCheckedChange={(v) => toggle(k, v === true)}
                      />
                      <label htmlFor={id} className="min-w-0 flex-1">
                        <span className="block truncate text-sm font-medium">{m.name}</span>
                        {meta(m)}
                      </label>
                      {r ? (
                        <span className={cn("max-w-[45%] shrink-0 truncate text-xs", r.ok ? "text-emerald-600 dark:text-emerald-400" : "text-destructive")} title={r.ok ? undefined : r.message}>
                          {r.ok ? s.resultOk : fmt(s.resultFail, { message: r.message })}
                        </span>
                      ) : !ok ? (
                        <span className="shrink-0 text-xs text-muted-foreground">{isLocalOnly(m) ? s.manualCopy : s.noProject}</span>
                      ) : null}
                    </li>
                  );
                })}
              </ul>
            )}
          </section>
        ) : null}

        <section aria-label={fmt(s.onlyHereTitle, { n: data.only_here.length })} className="rounded-lg border border-border">
          <div className="flex items-center gap-2 px-3 pt-3 pb-2">
            <div className="min-w-0 flex-1">
              <h3 className="text-sm font-semibold">{fmt(s.onlyHereTitle, { n: data.only_here.length })}</h3>
              <p className="mt-0.5 text-xs text-muted-foreground">{s.onlyHereNote}</p>
            </div>
            {onOpenSync && data.only_here.length > 0 ? (
              <Button
                type="button"
                variant="outline"
                onClick={() => {
                  onOpenChange(false);
                  onOpenSync();
                }}
                className="h-7 shrink-0 cursor-pointer px-3 text-xs"
              >
                {s.goSync}
              </Button>
            ) : null}
          </div>
          {data.only_here.length === 0 ? (
            <p className="border-t border-border px-3 py-2.5 text-xs text-muted-foreground">{s.onlyHereEmpty}</p>
          ) : (
            <ul className="max-h-[200px] divide-y divide-border overflow-y-auto border-t border-border">
              {data.only_here.map((m) => (
                <li key={manifestKey(m)} className="flex items-center gap-3 px-3 py-2">
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium">{m.name}</p>
                    {meta(m)}
                  </div>
                </li>
              ))}
            </ul>
          )}
        </section>

        {data.remote !== null ? (
          <section className="rounded-lg border border-border">
            <button
              type="button"
              aria-expanded={bothOpen}
              onClick={() => setBothOpen((v) => !v)}
              className="flex w-full cursor-pointer items-center gap-2 rounded-lg px-3 py-2.5 text-left text-sm font-semibold outline-none hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring"
            >
              <ChevronRight className={cn("size-4 shrink-0 transition-transform motion-reduce:transition-none", bothOpen && "rotate-90")} aria-hidden="true" />
              {fmt(s.inBothTitle, { n: data.in_both.length })}
            </button>
            {bothOpen && data.in_both.length > 0 ? (
              <ul className="max-h-[200px] divide-y divide-border overflow-y-auto border-t border-border">
                {data.in_both.map((m) => (
                  <li key={manifestKey(m)} className="px-3 py-2">
                    <p className="truncate text-sm font-medium">{m.name}</p>
                    {meta(m)}
                  </li>
                ))}
              </ul>
            ) : null}
          </section>
        ) : null}

        {localOnlyHere > 0 ? <p className="text-xs text-muted-foreground">{fmt(s.localOnlyCount, { n: localOnlyHere })}</p> : null}
      </div>
    );
  }

  return (
    <FlowDialog open={open} onOpenChange={onOpenChange} title={s.title} description={s.intro} width={640}>
      {body}
      <div className="mt-4 flex shrink-0 items-center gap-2 border-t border-border pt-3">
        <Button type="button" variant="outline" disabled={busyFile} onClick={() => void doExport()} className="h-8 cursor-pointer gap-1.5 px-3 text-xs">
          <FileDown className="size-3.5" aria-hidden="true" />
          {s.export}
        </Button>
        <Button type="button" variant="outline" disabled={busyFile || installing} onClick={() => void doImport()} className="h-8 cursor-pointer gap-1.5 px-3 text-xs">
          <FileUp className="size-3.5" aria-hidden="true" />
          {s.import}
        </Button>
        <div className="flex-1" />
        <Button type="button" variant="ghost" onClick={() => onOpenChange(false)} className="h-8 cursor-pointer px-3 text-sm">
          {s.close}
        </Button>
      </div>
      <ConfirmDialog
        open={confirm}
        title={fmt(s.installTitle, { n: chosen.length })}
        body={`${s.installBody}\n${chosen.map((m) => `· ${m.name}`).join("\n")}`}
        confirmLabel={s.installConfirm}
        cancelLabel={s.cancel}
        onConfirm={() => {
          setConfirm(false);
          void installChosen(chosen);
        }}
        onCancel={() => setConfirm(false)}
      />
    </FlowDialog>
  );
}
