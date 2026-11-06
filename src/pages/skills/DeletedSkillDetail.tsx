import * as React from "react";
import { Check, Copy, Loader2, RotateCcw, Trash2 } from "lucide-react";
import { restoreDeletedSkill, revealBackupFolder } from "@/api/skills";
import type { DeletedSkill } from "@/api/types";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { formatSnapshotTime } from "@/pages/memory/HistoryDrawer";
import { ToolTag } from "@/pages/memory/ToolTag";
import { fmt } from "@/pages/memory/paths";
import { describeError } from "./flowParts";
import { tagClass } from "./SkillList";
import { formatSize } from "./skillUtils";

export interface DeletedSkillDetailProps {
  item: DeletedSkill;
  projectName: (projectId: string) => string;
  /** The skill is back at `dir`. */
  onRestored: (item: DeletedSkill, dir: string) => void;
}

/** Detail of a deleted skill: its backups and a restore to the original location. */
export function DeletedSkillDetail({ item, projectName, onRestored }: DeletedSkillDetailProps) {
  const { t } = useI18n();
  const s = t.skills;
  const [selectedId, setSelectedId] = React.useState<string | null>(item.backups[0]?.id ?? null);
  const [folder, setFolder] = React.useState<string | null>(null);
  const [folderError, setFolderError] = React.useState<string | null>(null);
  const [copied, setCopied] = React.useState(false);
  const [confirming, setConfirming] = React.useState(false);
  const [restoring, setRestoring] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const busyRef = React.useRef(false);

  React.useEffect(() => {
    let alive = true;
    setFolder(null);
    setFolderError(null);
    revealBackupFolder(item.bucket).then(
      (p) => alive && setFolder(p),
      (e) => alive && setFolderError(fmt(s.deletedFolderFailed, { message: describeError(e, s) })),
    );
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [item.bucket]);

  const selected = item.backups.find((b) => b.id === selectedId);
  const scopeText = item.scope.type === "global" ? s.global : projectName(item.scope.project_id);

  const copy = async () => {
    if (!folder) return;
    try {
      await navigator.clipboard.writeText(folder);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // clipboard unavailable
    }
  };

  const move = (dir: 1 | -1) => {
    if (!item.backups.length) return;
    const at = item.backups.findIndex((b) => b.id === selectedId);
    const next = item.backups[Math.min(item.backups.length - 1, Math.max(0, at + dir))];
    setSelectedId(next.id);
    document.getElementById(`deleted-backup-${next.id}`)?.focus();
  };

  const restore = async () => {
    if (!selected || busyRef.current) return;
    busyRef.current = true;
    setRestoring(true);
    setError(null);
    try {
      const dir = await restoreDeletedSkill(item.bucket, selected.id);
      onRestored(item, dir);
    } catch (e) {
      setError(fmt(s.restoreFailed, { message: describeError(e, s) }));
    } finally {
      busyRef.current = false;
      setRestoring(false);
    }
  };

  return (
    <div className="flex h-full min-w-0 flex-1 flex-col">
      <div className="@container flex min-h-12 shrink-0 flex-wrap items-center gap-x-2.5 gap-y-2 border-b border-border px-4 py-2">
        <Trash2 className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
        <h2 className="min-w-0 truncate font-mono text-[15px] font-semibold" title={item.name}>
          {item.name}
        </h2>
        <ToolTag tool={item.tool} />
        <span className={tagClass}>{scopeText}</span>
        <span className="shrink-0 rounded-full border border-border px-2 text-xs leading-5 text-muted-foreground">{s.deletedBadge}</span>
        <span className="flex-1" />
        <Button
          type="button"
          disabled={!selected || restoring}
          onClick={() => setConfirming(true)}
          className="h-7 shrink-0 cursor-pointer gap-1.5 px-3 text-xs font-semibold"
        >
          {restoring ? (
            <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />
          ) : (
            <RotateCcw className="size-4" aria-hidden="true" />
          )}
          {restoring ? s.deletedRestoring : s.deletedRestore}
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
        <div className="max-w-[72ch]">
          <p className="text-sm text-muted-foreground">{s.deletedDesc}</p>
          {error ? (
            <p role="alert" className="mt-3 rounded-md bg-destructive/15 px-3 py-2 text-sm text-foreground">
              {error}
            </p>
          ) : null}

          <dl className="mt-5 grid grid-cols-[96px_minmax(0,1fr)] gap-x-4 gap-y-3 text-sm">
            <dt className="text-muted-foreground">{s.deletedOriginalDir}</dt>
            <dd className="min-w-0 font-mono text-[12.5px] break-all select-text" title={item.dir}>
              {item.dir}
            </dd>

            <dt className="text-muted-foreground">{s.deletedFolder}</dt>
            <dd className="flex min-w-0 items-start gap-2">
              {folderError ? (
                <span className="text-destructive">{folderError}</span>
              ) : (
                <>
                  <span className="min-w-0 font-mono text-[12.5px] break-all select-text" title={folder ?? undefined}>
                    {folder ?? "…"}
                  </span>
                  <button
                    type="button"
                    aria-label={s.deletedCopyPath}
                    title={copied ? s.deletedCopied : s.deletedCopyPath}
                    disabled={!folder}
                    onClick={copy}
                    className="grid size-6 shrink-0 cursor-pointer place-items-center rounded-sm text-muted-foreground transition-colors duration-150 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default disabled:opacity-50 motion-reduce:transition-none"
                  >
                    {copied ? <Check className="size-3.5" aria-hidden="true" /> : <Copy className="size-3.5" aria-hidden="true" />}
                  </button>
                </>
              )}
            </dd>

            <dt className="text-muted-foreground">{s.deletedBackups}</dt>
            <dd className="min-w-0">
              {item.backups.length === 0 ? (
                <span className="text-muted-foreground">{s.deletedNoBackups}</span>
              ) : (
                <div role="radiogroup" aria-label={s.backupsListLabel} className="max-h-[220px] overflow-y-auto rounded-md border border-border py-1">
                  {item.backups.map((b) => {
                    const on = b.id === selectedId;
                    return (
                      <button
                        key={b.id}
                        id={`deleted-backup-${b.id}`}
                        type="button"
                        role="radio"
                        aria-checked={on}
                        tabIndex={on ? 0 : -1}
                        onClick={() => setSelectedId(b.id)}
                        onKeyDown={(e) => {
                          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                            e.preventDefault();
                            move(e.key === "ArrowDown" ? 1 : -1);
                          }
                        }}
                        className={cn(
                          "relative flex h-8 w-full cursor-pointer items-center gap-4 px-3 text-left text-sm outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset motion-reduce:transition-none",
                          on ? "list-active" : "text-muted-foreground hover:bg-muted/60 hover:text-foreground",
                        )}
                      >
                        {on ? <span className="absolute inset-y-1 left-0 w-0.5 rounded-full bg-primary" aria-hidden="true" /> : null}
                        <span className="font-mono text-[12.5px] tabular-nums">{formatSnapshotTime(b.created_at)}</span>
                        <span className="text-xs tabular-nums">{formatSize(b.size_bytes)}</span>
                      </button>
                    );
                  })}
                </div>
              )}
            </dd>
          </dl>
        </div>
      </div>

      <ConfirmDialog
        open={confirming}
        title={s.deletedRestoreTitle}
        body={fmt(s.deletedRestoreBody, { time: selected ? formatSnapshotTime(selected.created_at) : "", dir: item.dir })}
        confirmLabel={s.deletedRestoreConfirm}
        cancelLabel={s.cancel}
        onConfirm={() => {
          setConfirming(false);
          void restore();
        }}
        onCancel={() => setConfirming(false)}
      />
    </div>
  );
}
