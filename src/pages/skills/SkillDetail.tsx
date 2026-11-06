import * as React from "react";
import { Check, Copy, CopyPlus, History, Loader2, Pencil, RefreshCw, RefreshCcw, Trash2, TriangleAlert } from "lucide-react";
import { registerSkillSource } from "@/api/skills";
import type { SkillEntry, UpdateStatus } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Dialog, DialogClose, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { ToolTag } from "@/pages/memory/ToolTag";
import { fmt } from "@/pages/memory/paths";
import { SkillFiles } from "./SkillFiles";
import { tagClass } from "./SkillList";
import { errorMessage, formatSize, shortCommit, sourceLabel } from "./skillUtils";

type Tab = "overview" | "files";

export interface SkillDetailProps {
  entry: SkillEntry;
  /**
   * Display-only: installs of the same skill (same name and same source), the opened one included.
   * Must never be used as an action target; actions always use `entry`.
   */
  installs: SkillEntry[];
  projectName: (projectId: string) => string;
  update?: UpdateStatus;
  /** git is installed; updating needs it. */
  gitOk?: boolean;
  /** The skill is gone from disk but has unsaved file edits; the editor stays with a warning. */
  removed?: boolean;
  /** An update of this skill is running. */
  updating?: boolean;
  onUpdate: (entry: SkillEntry) => void;
  onSync: (entry: SkillEntry) => void;
  onDelete: (entry: SkillEntry) => void;
  onBackups: (entry: SkillEntry) => void;
  /** The skill's metadata changed on disk (e.g. a source was registered): reload the list. */
  onChanged?: () => void;
  onDirtyChange?: (dirty: boolean) => void;
}

const actionBtn = "h-7 shrink-0 cursor-pointer gap-1.5 bg-card px-3 text-xs hover:bg-muted";

export function SkillDetail({
  entry,
  installs,
  projectName,
  update,
  gitOk = true,
  removed = false,
  updating = false,
  onUpdate,
  onSync,
  onDelete,
  onBackups,
  onChanged,
  onDirtyChange,
}: SkillDetailProps) {
  const { t } = useI18n();
  const s = t.skills;
  const [tab, setTab] = React.useState<Tab>("overview");
  const [filesVisited, setFilesVisited] = React.useState(false);
  const [registerOpen, setRegisterOpen] = React.useState(false);

  const showTab = (next: Tab) => {
    setTab(next);
    if (next === "files") setFilesVisited(true);
  };
  const scopeText = entry.scope.type === "global" ? s.global : projectName(entry.scope.project_id);
  // Updating pulls from the source repository: it needs a recorded source and git.
  const updateBlock = !entry.source
    ? s.updateNeedSource
    : !gitOk
      ? s.needGit
      : update?.not_updatable_subdir
        ? fmt(s.updateSubdir, { root: update.repo_root ?? "" })
        : null;

  return (
    <div className="flex h-full min-w-0 flex-1 flex-col">
      <div className="@container flex min-h-12 shrink-0 flex-wrap items-center gap-x-2.5 gap-y-2 border-b border-border px-4 py-2">
        <h2 className="min-w-0 truncate font-mono text-[15px] font-semibold" title={entry.name}>
          {entry.name}
        </h2>
        <ToolTag tool={entry.tool} />
        <span className={tagClass}>{scopeText}</span>
        <span className={tagClass}>{sourceLabel(entry, s)}</span>
        {entry.readonly ? (
          <span className="shrink-0 rounded-full border border-border px-2 text-xs leading-5 text-muted-foreground">{s.readOnly}</span>
        ) : null}
        <span className="flex-1" />
        {entry.readonly ? (
          <Button type="button" variant="outline" disabled={removed} onClick={() => onSync(entry)} className={actionBtn}>
            <CopyPlus className="size-4" aria-hidden="true" />
            {s.copyToWritable}
          </Button>
        ) : (
          <>
            <span className="inline-flex shrink-0" title={updateBlock ?? undefined}>
              <Button
                type="button"
                variant={update?.has_update ? "default" : "outline"}
                disabled={updateBlock !== null || removed || updating}
                onClick={() => onUpdate(entry)}
                className={cn(actionBtn, update?.has_update && "bg-primary font-semibold hover:bg-primary/90")}
              >
                {updating ? (
                  <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />
                ) : (
                  <RefreshCw className="size-4" aria-hidden="true" />
                )}
                {updating ? s.updating : s.update}
              </Button>
            </span>
            <Button type="button" variant="outline" disabled={removed} onClick={() => showTab("files")} className={actionBtn}>
              <Pencil className="size-4" aria-hidden="true" />
              {s.editFiles}
            </Button>
            <Button type="button" variant="outline" disabled={removed} onClick={() => onSync(entry)} className={actionBtn}>
              <RefreshCcw className="size-4" aria-hidden="true" />
              {s.syncTo}
            </Button>
            <Button
              type="button"
              variant="outline"
              disabled={removed}
              onClick={() => onBackups(entry)}
              aria-label={s.backups}
              title={s.backups}
              className="h-7 w-7 shrink-0 cursor-pointer bg-card px-0 hover:bg-muted"
            >
              <History className="size-4" aria-hidden="true" />
            </Button>
            <Button
              type="button"
              variant="outline"
              disabled={removed}
              onClick={() => onDelete(entry)}
              className={cn(actionBtn, "text-destructive hover:text-destructive")}
            >
              <Trash2 className="size-4" aria-hidden="true" />
              {s.delete}
            </Button>
          </>
        )}
      </div>

      {removed ? (
        <div className="mx-4 mt-3 flex shrink-0 items-center gap-2.5 rounded-md bg-warning/15 px-3 py-2 text-warning">
          <TriangleAlert className="size-4 shrink-0" aria-hidden="true" />
          <span className="text-foreground">{s.removedBanner}</span>
        </div>
      ) : null}
      <div className="flex h-10 shrink-0 items-center border-b border-border px-4">
        <div role="radiogroup" aria-label={s.tabsLabel} className="inline-flex overflow-hidden rounded-md border border-border">
          {(["overview", "files"] as const).map((k) => (
            <button
              key={k}
              type="button"
              role="radio"
              aria-checked={tab === k}
              onClick={() => showTab(k)}
              className={cn(
                "cursor-pointer px-2.5 text-xs leading-[26px] outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset motion-reduce:transition-none",
                tab === k ? "seg-active" : "text-muted-foreground hover:text-foreground",
              )}
            >
              {k === "overview" ? s.tabOverview : s.tabFiles}
            </button>
          ))}
        </div>
      </div>

      {tab === "overview" ? (
        <Overview
          entry={entry}
          installs={installs}
          projectName={projectName}
          update={update}
          onRegister={() => setRegisterOpen(true)}
        />
      ) : null}
      {filesVisited ? (
        <div className={cn("flex min-h-0 min-w-0 flex-1", tab !== "files" && "hidden")}>
          <SkillFiles key={entry.id} skillId={entry.id} readonly={entry.readonly} onDirtyChange={onDirtyChange} />
        </div>
      ) : null}

      <RegisterDialog
        entry={entry}
        open={registerOpen}
        onOpenChange={setRegisterOpen}
        onDone={() => {
          setRegisterOpen(false);
          onChanged?.();
        }}
      />
    </div>
  );
}

function Overview({
  entry,
  installs,
  projectName,
  update,
  onRegister,
}: {
  entry: SkillEntry;
  installs: SkillEntry[];
  projectName: (id: string) => string;
  update?: UpdateStatus;
  onRegister: () => void;
}) {
  const { t } = useI18n();
  const s = t.skills;
  const [copied, setCopied] = React.useState(false);
  const src = entry.source;

  const copyUrl = async () => {
    if (!src) return;
    try {
      await navigator.clipboard.writeText(src.url);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // clipboard unavailable
    }
  };

  const others = installs.some((i) => i.dir === entry.dir) ? installs : [entry, ...installs];

  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
      <div className="max-w-[72ch]">
        {entry.description ? <p className="font-content text-[14px] leading-[25px]">{entry.description}</p> : null}
        {entry.readonly ? (
          <p className="mt-4 rounded-md bg-muted px-3 py-2 text-sm text-muted-foreground">{s.readOnlyBanner}</p>
        ) : null}

        <dl className="mt-5 grid grid-cols-[96px_minmax(0,1fr)] gap-x-4 gap-y-3 text-sm">
          <dt className="text-muted-foreground">{s.source}</dt>
          <dd className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
            {src ? (
              <>
                <span className="min-w-0 truncate font-mono text-[12.5px] select-text" title={src.url}>
                  {src.url}
                </span>
                <button
                  type="button"
                  aria-label={s.copyUrl}
                  title={copied ? s.copied : s.copyUrl}
                  onClick={copyUrl}
                  className="grid size-6 shrink-0 cursor-pointer place-items-center rounded-sm text-muted-foreground transition-colors duration-150 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none"
                >
                  {copied ? <Check className="size-3.5" aria-hidden="true" /> : <Copy className="size-3.5" aria-hidden="true" />}
                </button>
                {src.skill_path ? (
                  <span className="font-mono text-[12.5px] text-muted-foreground" title={src.skill_path}>
                    {src.skill_path}
                  </span>
                ) : null}
              </>
            ) : entry.kind === "user" ? (
              <>
                <span className="text-muted-foreground">{s.sourceUnknown}</span>
                {!entry.readonly ? (
                  <Button type="button" variant="outline" onClick={onRegister} className="h-7 cursor-pointer bg-card px-3 text-xs hover:bg-muted">
                    {s.registerSource}
                  </Button>
                ) : null}
              </>
            ) : (
              <span>{entry.kind === "plugin" && entry.plugin ? fmt(s.pluginSource, { name: entry.plugin }) : sourceLabel(entry, s)}</span>
            )}
          </dd>

          {src?.commit ? (
            <>
              <dt className="text-muted-foreground">{s.commit}</dt>
              <dd className="flex flex-wrap items-center gap-x-3 gap-y-1">
                <span className="font-mono text-[12.5px]">{shortCommit(src.commit)}</span>
                {update?.latest && update.has_update ? (
                  <span className="font-mono text-[12.5px] text-accent-text">{fmt(s.latestCommit, { commit: shortCommit(update.latest) })}</span>
                ) : null}
                {update?.error ? (
                  <span className="inline-flex items-center gap-1 text-warning">
                    <TriangleAlert className="size-3.5" aria-hidden="true" />
                    <span className="text-foreground">{fmt(s.updateError, { message: update.error })}</span>
                  </span>
                ) : null}
              </dd>
            </>
          ) : null}

          <dt className="text-muted-foreground">{s.locations}</dt>
          <dd className="flex min-w-0 flex-col gap-2">
            {others.map((i) => (
              <div key={`${i.tool}:${i.dir}`} className="flex min-w-0 flex-col gap-0.5">
                <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1">
                  <ToolTag tool={i.tool} />
                  <span className={tagClass}>{i.scope.type === "global" ? s.global : projectName(i.scope.project_id)}</span>
                  <span className={tagClass}>{sourceLabel(i, s)}</span>
                  <span
                    className={cn("min-w-0 font-mono text-[12.5px] break-all select-text", i.dir === entry.dir ? "text-foreground" : "text-muted-foreground")}
                    title={i.dir}
                  >
                    {i.dir}
                  </span>
                </div>
                {i.is_symlink && i.symlink_target ? (
                  <span className="pl-1 font-mono text-[12.5px] break-all text-muted-foreground" title={i.symlink_target}>
                    {fmt(s.symlinkTo, { target: i.symlink_target })}
                  </span>
                ) : null}
              </div>
            ))}
          </dd>

          <dt className="text-muted-foreground">{s.filesAndSize}</dt>
          <dd>{fmt(s.fileCountSize, { n: entry.file_count, size: formatSize(entry.size_bytes) })}</dd>

          {entry.modified_at ? (
            <>
              <dt className="text-muted-foreground">{s.modifiedAt}</dt>
              <dd>{new Date(entry.modified_at).toLocaleString()}</dd>
            </>
          ) : null}
        </dl>
      </div>
    </div>
  );
}

function RegisterDialog({
  entry,
  open,
  onOpenChange,
  onDone,
}: {
  entry: SkillEntry;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onDone: () => void;
}) {
  const { t } = useI18n();
  const s = t.skills;
  const [url, setUrl] = React.useState("");
  const [path, setPath] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (open) {
      setUrl("");
      setPath("");
      setError(null);
    }
  }, [open]);

  const submit = async () => {
    if (busy || !url.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await registerSkillSource(entry.id, url.trim(), path.trim());
      onDone();
    } catch (e) {
      setError(fmt(s.registerFailed, { message: errorMessage(e) }));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent showCloseButton={false} className="w-[460px]">
          <DialogTitle>{s.registerTitle}</DialogTitle>
          <DialogDescription className="mt-2">{s.registerDesc}</DialogDescription>
          <form
            className="mt-4 flex flex-col gap-3"
            onSubmit={(e) => {
              e.preventDefault();
              void submit();
            }}
          >
            <div className="flex flex-col gap-1.5">
              <label htmlFor="skill-src-url" className="text-xs font-medium text-muted-foreground">
                {s.repoUrl}
              </label>
              <Input
                id="skill-src-url"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder={s.repoUrlPlaceholder}
                autoComplete="off"
                spellCheck={false}
                className="h-8 font-mono text-[12.5px]"
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <label htmlFor="skill-src-path" className="text-xs font-medium text-muted-foreground">
                {s.repoPath}
              </label>
              <Input
                id="skill-src-path"
                value={path}
                onChange={(e) => setPath(e.target.value)}
                placeholder={s.repoPathPlaceholder}
                autoComplete="off"
                spellCheck={false}
                className="h-8 font-mono text-[12.5px]"
              />
            </div>
            {error ? (
              <div role="alert" className="flex items-center gap-2.5 rounded-md bg-destructive/15 px-3 py-2 text-sm text-destructive">
                <TriangleAlert className="size-4 shrink-0" aria-hidden="true" />
                <span className="text-foreground">{error}</span>
              </div>
            ) : null}
            <div className="mt-2 flex justify-end gap-2">
              <DialogClose asChild>
                <Button type="button" variant="outline" className="h-7 cursor-pointer px-3 text-xs">
                  {s.cancel}
                </Button>
              </DialogClose>
              <Button type="submit" disabled={busy || !url.trim()} className="h-7 cursor-pointer px-3 text-xs font-semibold">
                {busy ? s.registering : s.register}
              </Button>
            </div>
          </form>
        </DialogContent>
    </Dialog>
  );
}
