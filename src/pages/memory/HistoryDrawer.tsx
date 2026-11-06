import * as React from "react";
import { History, RotateCcw } from "lucide-react";
import { listSnapshots, readSnapshot, writeMemoryFile } from "@/api/memory";
import type { CommandError, FileContentDto, SnapshotInfo } from "@/api/types";
import { DiffView } from "@/components/diff/DiffView";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { ConfirmDialog } from "./ConfirmDialog";
import { basename, fmt } from "./paths";

export interface HistoryDrawerProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  path: string;
  /** What the editor currently shows (including unsaved edits); the diff is taken against it. */
  currentText: string;
  /** Hash of the file as last read from / written to disk; sent as the write precondition. */
  currentHash: string;
  /** The editor has unsaved edits that restoring would overwrite. */
  dirty?: boolean;
  /** Called after the snapshot was written; the editor adopts `dto` as its new baseline. */
  onRestored: (text: string, dto: FileContentDto) => void;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** Unix ms -> local "11-05 14:02". */
export function formatSnapshotTime(ms: number): string {
  const d = new Date(ms);
  return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function formatKb(bytes: number): string {
  return `${(bytes / 1024).toFixed(1)} KB`;
}

function messageOf(e: unknown): string {
  return typeof e === "object" && e !== null && "message" in e ? String((e as CommandError).message) : String(e);
}

export function HistoryDrawer({ open, onOpenChange, path, currentText, currentHash, dirty, onRestored }: HistoryDrawerProps) {
  const { t } = useI18n();
  const m = t.memory;
  const [snaps, setSnaps] = React.useState<SnapshotInfo[] | null>(null);
  const [listError, setListError] = React.useState<string | null>(null);
  const [selectedId, setSelectedId] = React.useState<string | null>(null);
  const [texts, setTexts] = React.useState<Record<string, string>>({});
  const [textErrors, setTextErrors] = React.useState<Record<string, string>>({});
  const [restoring, setRestoring] = React.useState(false);
  const [restoreError, setRestoreError] = React.useState<string | null>(null);
  const [confirming, setConfirming] = React.useState(false);

  // Load the snapshot list whenever the drawer opens (a restore or save adds a snapshot).
  React.useEffect(() => {
    if (!open) return;
    let alive = true;
    setSnaps(null);
    setListError(null);
    setSelectedId(null);
    setTexts({});
    setTextErrors({});
    setRestoreError(null);
    listSnapshots(path).then(
      (list) => {
        if (!alive) return;
        // Newest first, whatever order the backend used.
        const sorted = [...list].sort((a, b) => b.created_at - a.created_at);
        setSnaps(sorted);
        setSelectedId(sorted[0]?.id ?? null);
      },
      (e) => alive && setListError(messageOf(e)),
    );
    return () => {
      alive = false;
    };
  }, [open, path]);

  // Fetch the text of the selected snapshot once.
  React.useEffect(() => {
    if (!open || !selectedId || texts[selectedId] !== undefined || textErrors[selectedId] !== undefined) return;
    let alive = true;
    readSnapshot(path, selectedId).then(
      (text) => alive && setTexts((prev) => ({ ...prev, [selectedId]: text })),
      (e) => alive && setTextErrors((prev) => ({ ...prev, [selectedId]: messageOf(e) })),
    );
    return () => {
      alive = false;
    };
  }, [open, path, selectedId, texts, textErrors]);

  const selected = snaps?.find((s) => s.id === selectedId);
  const selectedText = selectedId ? texts[selectedId] : undefined;

  const move = (dir: 1 | -1) => {
    if (!snaps?.length) return;
    const at = snaps.findIndex((s) => s.id === selectedId);
    const next = snaps[Math.min(snaps.length - 1, Math.max(0, at + dir))];
    setSelectedId(next.id);
    document.getElementById(`snap-${next.id}`)?.focus();
  };

  const doRestore = async () => {
    if (!selected || restoring) return;
    setRestoring(true);
    setRestoreError(null);
    try {
      const text = selectedText ?? (await readSnapshot(path, selected.id));
      const dto = await writeMemoryFile(path, text, currentHash);
      onRestored(text, dto);
      onOpenChange(false);
    } catch (e) {
      const code = typeof e === "object" && e !== null && "code" in e ? (e as CommandError).code : undefined;
      setRestoreError(code === "disk_changed" ? m.restoreConflict : fmt(m.restoreFailed, { message: messageOf(e) }));
    } finally {
      setRestoring(false);
    }
  };

  const onRestoreClick = () => {
    if (dirty) setConfirming(true);
    else void doRestore();
  };

  return (
    <>
      <Sheet open={open} onOpenChange={onOpenChange}>
        <SheetContent
          side="right"
          className="w-[min(880px,92vw)] gap-0 data-[side=right]:w-[min(880px,92vw)] data-[side=right]:sm:max-w-none"
        >
          <SheetHeader className="border-b border-border px-5 py-4 pr-12">
            <SheetTitle className="text-base font-semibold">{m.historyTitle}</SheetTitle>
            <SheetDescription className="min-w-0 truncate text-xs">
              <span className="font-mono text-[12.5px]">{basename(path)}</span> · {m.historyDesc}
            </SheetDescription>
          </SheetHeader>

          {listError ? (
            <p role="alert" className="px-5 py-4 text-sm text-destructive">
              {fmt(m.historyFailed, { message: listError })}
            </p>
          ) : snaps === null ? (
            <p className="px-5 py-6 text-sm text-muted-foreground" aria-busy="true">
              {m.loading}
            </p>
          ) : snaps.length === 0 ? (
            <div className="flex flex-1 flex-col items-center justify-center gap-3 px-5">
              <History className="size-8 text-muted-foreground" aria-hidden="true" />
              <p className="text-sm text-muted-foreground">{m.historyEmpty}</p>
            </div>
          ) : (
            <>
              <div
                role="listbox"
                aria-label={m.snapshotList}
                className="max-h-[172px] shrink-0 overflow-y-auto border-b border-border py-1"
              >
                {snaps.map((s) => {
                  const on = s.id === selectedId;
                  return (
                    <button
                      key={s.id}
                      id={`snap-${s.id}`}
                      type="button"
                      role="option"
                      aria-selected={on}
                      tabIndex={on ? 0 : -1}
                      onClick={() => setSelectedId(s.id)}
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
                      <span className="font-mono text-[12.5px] tabular-nums">{formatSnapshotTime(s.created_at)}</span>
                      <span className="text-xs tabular-nums">{formatKb(s.size_bytes)}</span>
                    </button>
                  );
                })}
              </div>

              <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto px-5 py-4">
                {selectedId && textErrors[selectedId] !== undefined ? (
                  <p role="alert" className="text-sm text-destructive">
                    {fmt(m.historyFailed, { message: textErrors[selectedId] })}
                  </p>
                ) : selectedText === undefined ? (
                  <p className="text-sm text-muted-foreground" aria-busy="true">
                    {m.loading}
                  </p>
                ) : (
                  <DiffView
                    oldText={currentText}
                    newText={selectedText}
                    oldLabel={m.diffCurrent}
                    newLabel={selected ? `${m.diffRestored} · ${formatSnapshotTime(selected.created_at)}` : m.diffRestored}
                  />
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
                <Button
                  type="button"
                  disabled={!selected || restoring}
                  onClick={onRestoreClick}
                  className="h-7 shrink-0 cursor-pointer gap-1.5 px-3 text-xs font-semibold"
                >
                  <RotateCcw className="size-4" aria-hidden="true" />
                  {restoring ? m.restoring : m.restore}
                </Button>
              </div>
            </>
          )}
        </SheetContent>
      </Sheet>
      <ConfirmDialog
        open={confirming}
        title={m.discardTitle}
        body={m.restoreDiscardBody}
        confirmLabel={m.restoreDiscardConfirm}
        cancelLabel={m.keepEditing}
        destructive
        onConfirm={() => {
          setConfirming(false);
          void doRestore();
        }}
        onCancel={() => setConfirming(false)}
      />
    </>
  );
}
