import * as React from "react";
import { LoadingState } from "@/components/ui/loading-state";
import { History, Loader2, RotateCcw } from "lucide-react";
import { listSkillBackups, readSkillBackupFile, readSkillFile, rollbackSkill } from "@/api/skills";
import type { BackupInfo, SkillEntry } from "@/api/types";
import { DiffView } from "@/components/diff/DiffView";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { formatSnapshotTime } from "@/pages/memory/HistoryDrawer";
import { fmt } from "@/pages/memory/paths";
import { describeError } from "./flowParts";
import { formatSize } from "./skillUtils";

export interface BackupsDrawerProps {
  entry: SkillEntry;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** `time` is the formatted creation time of the restored backup. */
  onRestored: (result: SkillEntry, time: string) => void;
}

export function BackupsDrawer({ entry, open, onOpenChange, onRestored }: BackupsDrawerProps) {
  const { t } = useI18n();
  const s = t.skills;
  const m = t.memory;
  const [backups, setBackups] = React.useState<BackupInfo[] | null>(null);
  const [listError, setListError] = React.useState<string | null>(null);
  const [selectedId, setSelectedId] = React.useState<string | null>(null);
  const [restoring, setRestoring] = React.useState(false);
  const [restoreError, setRestoreError] = React.useState<string | null>(null);
  const [confirming, setConfirming] = React.useState(false);
  const [current, setCurrent] = React.useState<string | null>(null);
  const [texts, setTexts] = React.useState<Record<string, string>>({});
  const [textErrors, setTextErrors] = React.useState<Record<string, string>>({});
  const busyRef = React.useRef(false);

  // The current SKILL.md is the right-hand side of every diff.
  React.useEffect(() => {
    if (!open) return;
    let alive = true;
    setCurrent(null);
    readSkillFile(entry.id, "SKILL.md").then(
      (f) => alive && setCurrent(f.binary || f.too_large ? "" : f.text),
      () => alive && setCurrent(""),
    );
    return () => {
      alive = false;
    };
  }, [open, entry.id]);

  React.useEffect(() => {
    if (!open) return;
    let alive = true;
    setBackups(null);
    setListError(null);
    setSelectedId(null);
    setRestoreError(null);
    setTexts({});
    setTextErrors({});
    setConfirming(false);
    listSkillBackups(entry.id).then(
      (list) => {
        if (!alive) return;
        const sorted = [...list].sort((a, b) => b.created_at - a.created_at);
        setBackups(sorted);
        setSelectedId(sorted[0]?.id ?? null);
      },
      (e) => alive && setListError(describeError(e, s)),
    );
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, entry.id]);

  // The selected backup's SKILL.md, fetched once per backup.
  React.useEffect(() => {
    if (!open || !selectedId || texts[selectedId] !== undefined || textErrors[selectedId] !== undefined) return;
    let alive = true;
    readSkillBackupFile(entry.id, selectedId).then(
      (f) => {
        if (!alive) return;
        if (f.binary || f.too_large) setTextErrors((p) => ({ ...p, [selectedId]: s.backupNoSkillMd }));
        else setTexts((p) => ({ ...p, [selectedId]: f.text }));
      },
      (e) => alive && setTextErrors((p) => ({ ...p, [selectedId]: fmt(s.backupDiffFailed, { message: describeError(e, s) }) })),
    );
    return () => {
      alive = false;
    };
  }, [open, entry.id, selectedId, texts, textErrors, s]);

  const selected = backups?.find((b) => b.id === selectedId);

  const move = (dir: 1 | -1) => {
    if (!backups?.length) return;
    const at = backups.findIndex((b) => b.id === selectedId);
    const next = backups[Math.min(backups.length - 1, Math.max(0, at + dir))];
    setSelectedId(next.id);
    document.getElementById(`backup-${next.id}`)?.focus();
  };

  const restore = async () => {
    if (!selected || busyRef.current) return;
    busyRef.current = true;
    setRestoring(true);
    setRestoreError(null);
    try {
      const res = await rollbackSkill(entry.id, selected.id);
      onRestored(res, formatSnapshotTime(selected.created_at));
      onOpenChange(false);
    } catch (e) {
      setRestoreError(fmt(s.restoreFailed, { message: describeError(e, s) }));
    } finally {
      busyRef.current = false;
      setRestoring(false);
    }
  };

  return (
    <>
      <Sheet open={open} onOpenChange={onOpenChange}>
        <SheetContent side="right" className="w-[min(880px,92vw)] gap-0 data-[side=right]:w-[min(880px,92vw)] data-[side=right]:sm:max-w-none">
          <SheetHeader className="border-b border-border px-5 py-4 pr-12">
            <SheetTitle className="text-base font-semibold">{s.backupsTitle}</SheetTitle>
            <SheetDescription className="text-xs">
              <span className="font-mono text-[12.5px]">{entry.name}</span> · {s.backupsDesc}
            </SheetDescription>
          </SheetHeader>

          {listError ? (
            <p role="alert" className="px-5 py-4 text-sm text-destructive">
              {fmt(s.backupsFailed, { message: listError })}
            </p>
          ) : backups === null ? (
            <LoadingState label={s.loading} />
          ) : backups.length === 0 ? (
            <div className="flex flex-1 flex-col items-center justify-center gap-3 px-5">
              <History className="size-8 text-muted-foreground" aria-hidden="true" />
              <p className="text-sm text-muted-foreground">{s.backupsEmpty}</p>
            </div>
          ) : (
            <>
              <div role="radiogroup" aria-label={s.backupsListLabel} className="max-h-[172px] shrink-0 overflow-y-auto border-b border-border py-1">
                {backups.map((b) => {
                  const on = b.id === selectedId;
                  return (
                    <button
                      key={b.id}
                      id={`backup-${b.id}`}
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
                        "relative flex h-8 w-full cursor-pointer items-center gap-4 px-5 text-left text-sm outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset motion-reduce:transition-none",
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

              <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto px-5 py-4">
                {selectedId && textErrors[selectedId] !== undefined ? (
                  <p role="alert" className="text-sm text-destructive">
                    {textErrors[selectedId]}
                  </p>
                ) : selectedId && texts[selectedId] !== undefined && current !== null ? (
                  <DiffView
                    oldText={current}
                    newText={texts[selectedId]}
                    oldLabel={m.diffCurrent}
                    newLabel={selected ? `${m.diffRestored} · ${formatSnapshotTime(selected.created_at)}` : m.diffRestored}
                  />
                ) : (
                  <LoadingState label={s.loading} />
                )}
              </div>

              <div className="flex shrink-0 items-center gap-3 border-t border-border px-5 py-3">
                {restoreError ? (
                  <p role="alert" className="min-w-0 flex-1 text-sm text-destructive">
                    {restoreError}
                  </p>
                ) : (
                  <span className="flex-1" />
                )}
                <Button type="button" disabled={!selected || restoring} onClick={() => setConfirming(true)} className="h-7 shrink-0 cursor-pointer gap-1.5 px-3 text-xs font-semibold">
                  {restoring ? <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <RotateCcw className="size-4" aria-hidden="true" />}
                  {restoring ? s.restoring : s.restore}
                </Button>
              </div>
            </>
          )}
        </SheetContent>
      </Sheet>
      <ConfirmDialog
        open={confirming}
        title={s.restoreConfirmTitle}
        body={s.restoreConfirmBody}
        confirmLabel={s.restoreConfirm}
        cancelLabel={s.cancel}
        onConfirm={() => {
          setConfirming(false);
          void restore();
        }}
        onCancel={() => setConfirming(false)}
      />
    </>
  );
}
