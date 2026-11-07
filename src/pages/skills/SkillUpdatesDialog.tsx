import * as React from "react";
import { Check, CircleAlert, LoaderCircle } from "lucide-react";
import { Checkbox } from "@/components/ui/checkbox";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import type { SkillEntry } from "@/api/types";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";
import { ToolTag } from "@/pages/memory/ToolTag";
import { errorMessage } from "./skillUtils";

export interface SkillUpdateItem {
  entry: SkillEntry;
  latest?: string | null;
  status: "available" | "unverified";
}

interface Props {
  items: SkillUpdateItem[];
  onClose: () => void;
  onUpdate: (item: SkillUpdateItem) => Promise<void>;
  onComplete: () => Promise<unknown>;
}

export function SkillUpdatesDialog({ items, onClose, onUpdate, onComplete }: Props) {
  const { t } = useI18n();
  const s = t.skills;
  const [selected, setSelected] = React.useState(() => new Set(items.filter((item) => item.status === "available")));
  const [progress, setProgress] = React.useState<{ done: number; total: number; active?: SkillUpdateItem[] }>();
  const [results, setResults] = React.useState(new Map<SkillUpdateItem, string | null>());
  const running = progress?.active !== undefined;
  const activeItems = progress?.active ?? [];
  const failedCount = [...results.values()].filter((result) => result !== null).length;
  const started = React.useRef(false);

  const update = async () => {
    if (!selected.size || started.current) return;
    started.current = true;
    const queue = items.filter((item) => selected.has(item));
    const completed = new Map<SkillUpdateItem, string | null>();
    const active = new Set<SkillUpdateItem>();
    const pending = [...queue];
    const refreshProgress = () => setProgress({
      done: completed.size,
      total: queue.length,
      active: [...active],
    });
    const worker = async () => {
      while (pending.length > 0) {
        const index = pending.findIndex(({ entry }) =>
          !entry.plugin || ![...active].some((item) => item.entry.plugin && item.entry.tool === entry.tool),
        );
        if (index < 0) return;
        const [item] = pending.splice(index, 1);
        active.add(item);
        refreshProgress();
        try {
          await onUpdate(item);
          completed.set(item, null);
        } catch (error) {
          completed.set(item, errorMessage(error));
        } finally {
          active.delete(item);
          setResults(new Map(completed));
          refreshProgress();
        }
      }
    };
    await Promise.all(Array.from({ length: Math.min(3, queue.length) }, worker));
    try {
      await onComplete();
    } finally {
      setProgress({ done: completed.size, total: queue.length });
    }
  };

  const selectAllId = React.useId();
  const allSelected = selected.size === items.length;
  const close = () => { if (!running) onClose(); };
  return (
    <Dialog open onOpenChange={(open) => { if (!open) close(); }}>
      <DialogContent showCloseButton={!running} className="w-[540px] max-w-[calc(100vw-2rem)]">
        <DialogHeader>
          <DialogTitle>{s.batchTitle}</DialogTitle>
          <DialogDescription>
            {items.length ? s.batchDescription : s.batchNone} {s.batchIndividual}
          </DialogDescription>
        </DialogHeader>
        {items.length > 0 && (
          <div className="overflow-hidden rounded-md border border-border">
            <div className="flex min-h-9 items-center gap-3 border-b border-border bg-muted/40 px-3">
              <Checkbox
                id={selectAllId}
                checked={allSelected ? true : selected.size > 0 ? "indeterminate" : false}
                disabled={progress !== undefined}
                onCheckedChange={(checked) => setSelected(new Set(checked ? items : []))}
              />
              <label htmlFor={selectAllId} className="flex-1 cursor-pointer py-2 text-xs font-medium text-muted-foreground peer-disabled:cursor-default">
                {s.batchSelectAll}
              </label>
              <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
                {fmt(s.batchSelectedCount, { n: selected.size, total: items.length })}
              </span>
            </div>
            <ul className="max-h-[42vh] divide-y divide-border overflow-y-auto">
              {items.map((item, index) => {
                const { entry, latest, status } = item;
                const name = entry.plugin?.split("@")[0] ?? entry.name;
                const error = results.get(item);
                return (
                  <li key={index} className="flex items-start gap-3 px-3 py-2">
                    <Checkbox
                      id={`skill-batch-${index}`}
                      className="mt-1"
                      checked={selected.has(item)}
                      disabled={progress !== undefined}
                      onCheckedChange={(checked) => setSelected((previous) => {
                        const next = new Set(previous);
                        if (checked) next.add(item);
                        else next.delete(item);
                        return next;
                      })}
                    />
                    <label htmlFor={`skill-batch-${index}`} className="min-w-0 flex-1 cursor-pointer">
                      <span className="flex flex-wrap items-center gap-2 text-sm">
                        <span className="break-all font-medium">{name}</span>
                        <ToolTag tool={entry.tool} />
                        {entry.plugin && <span className="text-xs text-muted-foreground">{s.pluginGroup}</span>}
                      </span>
                      <span className="block break-words text-xs text-muted-foreground">
                        {entry.scope.type === "global" ? s.global : entry.scope.project_id}
                        {entry.plugin_version ? ` · ${entry.plugin_version}` : ""}
                        {latest ? ` → ${latest}` : ""}
                        {status === "unverified" ? ` · ${s.batchUnverified}` : ""}
                      </span>
                      {progress?.active?.includes(item) && <span className="block text-xs text-muted-foreground">{s.updating}</span>}
                      {error !== undefined && <span className={error ? "block break-words text-xs text-destructive" : "block text-xs text-muted-foreground"}>{error ?? s.batchDone}</span>}
                    </label>
                  </li>
                );
              })}
            </ul>
          </div>
        )}
        {progress && (
          <div className="space-y-2.5 rounded-lg border border-border bg-muted/40 px-3.5 py-3" role="status">
            <div className="flex items-center gap-2.5">
              <span className="grid size-7 shrink-0 place-items-center rounded-md bg-background/80">
                {running ? (
                  <LoaderCircle className="size-4 text-primary motion-safe:animate-spin" aria-hidden="true" />
                ) : failedCount > 0 ? (
                  <CircleAlert className="size-4 text-destructive" aria-hidden="true" />
                ) : (
                  <Check className="size-4 text-primary" aria-hidden="true" />
                )}
              </span>
              <span className="min-w-0 flex-1 text-sm font-medium">
                {activeItems.length ? fmt(s.batchActive, { n: activeItems.length }) : running ? s.batchRefreshing : s.batchFinished}
              </span>
              <span className="shrink-0 rounded-md bg-background/80 px-2 py-1 text-xs font-medium tabular-nums text-muted-foreground">
                {progress.done} / {progress.total}
              </span>
            </div>
            <div
              role="progressbar"
              aria-label={s.batchTitle}
              aria-valuemin={0}
              aria-valuemax={progress.total}
              aria-valuenow={progress.done}
              className="h-2 overflow-hidden rounded-full bg-primary/15"
            >
              {running && progress.done === 0 ? (
                <div className="hidden h-full w-1/3 rounded-full bg-primary/60 motion-safe:block motion-safe:animate-[update-download_1.5s_ease-in-out_infinite]" aria-hidden="true" />
              ) : (
                <div className="h-full rounded-full bg-primary motion-safe:transition-[width] motion-safe:duration-300" style={{ width: `${progress.done / progress.total * 100}%` }} />
              )}
            </div>
            {activeItems.length > 0 && (
              <div className="flex flex-wrap gap-1.5">
                {activeItems.map(({ entry }) => {
                  const name = entry.plugin?.split("@")[0] ?? entry.name;
                  return <span key={`${entry.tool}:${entry.id}`} title={name} className="max-w-full truncate rounded bg-background/80 px-2 py-0.5 text-xs text-muted-foreground">{name}</span>;
                })}
              </div>
            )}
            {!running && (
              <p className="text-xs text-muted-foreground">
                {fmt(s.batchSummary, { ok: progress.done - failedCount, failed: failedCount })}
              </p>
            )}
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" disabled={running} onClick={close}>{s.batchClose}</Button>
          {items.length > 0 && !progress && (
            <Button disabled={!selected.size} onClick={() => void update()}>{fmt(s.batchApply, { n: selected.size })}</Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
