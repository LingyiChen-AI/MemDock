import * as React from "react";
import { Loader2 } from "lucide-react";
import { syncSkill } from "@/api/skills";
import type { InstallTarget, Project, SkillEntry } from "@/api/types";
import { Button } from "@/components/ui/button";
import { useI18n } from "@/i18n/I18nProvider";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { fmt } from "@/pages/memory/paths";
import { FlowDialog, InlineError, TargetPicker, describeError, selectionProblem, targetLabel, targetsOf, type TargetSelection } from "./flowParts";
import { asCommandError, existingDirs } from "./skillUtils";

export interface SyncDialogProps {
  entry: SkillEntry;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  projects: Project[];
  /** `label` is a readable form of the target ("Codex · 全局"). */
  onSynced: (result: SkillEntry, label: string) => void;
}

function sameLocation(entry: SkillEntry, target: InstallTarget): boolean {
  if (entry.tool !== target.tool || entry.scope.type !== target.scope.type) return false;
  return entry.scope.type === "global" || (entry.scope as { project_id: string }).project_id === (target.scope as { project_id: string }).project_id;
}

export function SyncDialog({ entry, open, onOpenChange, projects, onSynced }: SyncDialogProps) {
  const { t } = useI18n();
  const s = t.skills;
  const initial = React.useMemo<TargetSelection>(
    () => ({ tools: [entry.tool === "claude" ? "codex" : "claude"], scope: "global", projectId: "" }),
    [entry.tool],
  );
  const [target, setTarget] = React.useState<TargetSelection>(initial);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [confirm, setConfirm] = React.useState<{ dirs: string[] } | null>(null);
  const busyRef = React.useRef(false);

  React.useEffect(() => {
    if (!open) return;
    setTarget(initial);
    setBusy(false);
    setError(null);
    setConfirm(null);
    busyRef.current = false;
  }, [open, initial]);

  const one = targetsOf(target)[0];
  const same = one !== undefined && sameLocation(entry, one);
  const problem = selectionProblem(target, s);
  const canSync = !busy && !problem && !same && one !== undefined;

  const run = async (overwrite: boolean) => {
    if (busyRef.current || !one) return;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    try {
      const res = await syncSkill(entry.id, one, overwrite);
      onSynced(res, targetLabel(one, projects, t));
      onOpenChange(false);
    } catch (e) {
      const ce = asCommandError(e);
      if (ce?.code === "exists") setConfirm({ dirs: existingDirs(e) });
      else setError(fmt(s.syncFailed, { message: describeError(e, s) }));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };

  return (
    <>
      <FlowDialog
        open={open}
        onOpenChange={(o) => (busy && !o ? undefined : onOpenChange(o))}
        title={fmt(s.syncTitle, { name: entry.name })}
        description={s.syncDesc}
        width={480}
      >
        <div className="mt-4 flex flex-col gap-3">
          <TargetPicker single value={target} onChange={setTarget} projects={projects} disabled={busy} />
          {same ? <p className="text-xs text-warning">{s.syncSameLocation}</p> : null}
          {!same && problem ? <p className="text-xs text-warning">{problem}</p> : null}
          {error ? <InlineError message={error} /> : null}
        </div>
        <div className="mt-5 flex justify-end gap-2">
          <Button type="button" variant="outline" disabled={busy} onClick={() => onOpenChange(false)} className="h-7 cursor-pointer px-3 text-xs">
            {s.cancel}
          </Button>
          <Button type="button" disabled={!canSync} onClick={() => void run(false)} className="h-7 cursor-pointer gap-1.5 px-3 text-xs font-semibold">
            {busy ? <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" /> : null}
            {busy ? s.syncing : s.sync}
          </Button>
        </div>
      </FlowDialog>

      <ConfirmDialog
        open={confirm !== null}
        title={s.existsTitle}
        body={confirm && confirm.dirs.length > 0 ? fmt(s.existsBodyDirs, { dirs: confirm.dirs.join("\n") }) : s.existsBody}
        confirmLabel={s.overwrite}
        cancelLabel={s.cancel}
        destructive
        onConfirm={() => {
          setConfirm(null);
          void run(true);
        }}
        onCancel={() => setConfirm(null)}
      />
    </>
  );
}
