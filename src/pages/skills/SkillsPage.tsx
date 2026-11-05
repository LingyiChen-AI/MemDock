import * as React from "react";
import { Package, TriangleAlert } from "lucide-react";
import { listMemories } from "@/api/memory";
import { checkSkillUpdates, gitAvailable, listSkills, onSkillsChanged } from "@/api/skills";
import type { MemoryIndex, SkillEntry, UpdateStatus } from "@/api/types";
import { Button } from "@/components/ui/button";
import { useI18n } from "@/i18n/I18nProvider";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { basename, fmt } from "@/pages/memory/paths";
import { SkillDetail } from "./SkillDetail";
import { SkillList } from "./SkillList";
import { entryKey, errorMessage, sortEntries } from "./skillUtils";

const WIDTH_KEY = "memdock.skillListWidth";
const MIN_W = 280;
const MAX_W = 480;
const DEFAULT_W = 340;

const clamp = (w: number) => Math.min(MAX_W, Math.max(MIN_W, Math.round(w)));

function loadWidth(): number {
  try {
    const n = Number(localStorage.getItem(WIDTH_KEY));
    if (Number.isFinite(n) && n > 0) return clamp(n);
  } catch {
    // storage unavailable
  }
  return DEFAULT_W;
}

export interface SkillsPageProps {
  /** Plugged in by the add / update / sync / delete / rollback flows. */
  onAdd?: () => void;
  onUpdate?: (entry: SkillEntry) => void;
  onSync?: (entry: SkillEntry) => void;
  onDelete?: (entry: SkillEntry) => void;
  onBackups?: (entry: SkillEntry) => void;
}

const noop = () => {};

export function SkillsPage({
  onAdd = noop,
  onUpdate = noop,
  onSync = noop,
  onDelete = noop,
  onBackups = noop,
}: SkillsPageProps) {
  const { t } = useI18n();
  const s = t.skills;
  const [entries, setEntries] = React.useState<SkillEntry[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [index, setIndex] = React.useState<MemoryIndex | null>(null);
  const [gitOk, setGitOk] = React.useState(true);
  const [updates, setUpdates] = React.useState<Map<string, UpdateStatus>>(new Map());
  const [checking, setChecking] = React.useState(false);
  const [checkedUpdates, setCheckedUpdates] = React.useState<number | null>(null);
  const [checkError, setCheckError] = React.useState<string | null>(null);
  const [selectedKey, setSelectedKey] = React.useState<string | undefined>();
  const [dirty, setDirty] = React.useState(false);
  const [pending, setPending] = React.useState<SkillEntry | null>(null);
  const [width, setWidth] = React.useState(loadWidth);
  const widthRef = React.useRef(width);
  widthRef.current = width;

  const load = React.useCallback(async () => {
    try {
      const list = await listSkills();
      setEntries(list);
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
      setEntries((prev) => prev ?? []);
    }
  }, []);

  React.useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    void load();
    listMemories().then(
      (i) => alive && setIndex(i),
      () => {
        // project names fall back to the folder name
      },
    );
    gitAvailable().then(
      (ok) => alive && setGitOk(ok),
      () => alive && setGitOk(false),
    );
    onSkillsChanged(() => void load()).then((u) => {
      if (alive) unlisten = u;
      else u();
    });
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [load]);

  const projectName = React.useCallback(
    (id: string) => index?.projects.find((p) => p.project.id === id)?.project.name ?? basename(id),
    [index],
  );

  // Open the first skill the first time the list arrives.
  const didAutoSelect = React.useRef(false);
  React.useEffect(() => {
    if (!entries || didAutoSelect.current || entries.length === 0) return;
    didAutoSelect.current = true;
    setSelectedKey(entryKey(sortEntries(entries)[0]));
  }, [entries]);

  const selected = entries?.find((e) => entryKey(e) === selectedKey);
  const installs = React.useMemo(
    () => (selected ? (entries ?? []).filter((e) => e.name === selected.name) : []),
    [entries, selected],
  );

  const select = (entry: SkillEntry) => {
    if (entryKey(entry) === selectedKey) return;
    if (dirty) setPending(entry);
    else setSelectedKey(entryKey(entry));
  };

  const check = async () => {
    setChecking(true);
    setCheckError(null);
    try {
      const result = await checkSkillUpdates();
      setUpdates(new Map(result.map((u) => [u.skill_id, u])));
      setCheckedUpdates(result.filter((u) => u.has_update).length);
    } catch (e) {
      setCheckError(fmt(s.checkFailed, { message: errorMessage(e) }));
    } finally {
      setChecking(false);
    }
  };

  const startResize = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = widthRef.current;
    const el = e.currentTarget;
    el.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => setWidth(clamp(startW + ev.clientX - startX));
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
      try {
        localStorage.setItem(WIDTH_KEY, String(widthRef.current));
      } catch {
        // ignore
      }
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
  };

  const onResizeKey = (e: React.KeyboardEvent) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    const next = clamp(widthRef.current + (e.key === "ArrowLeft" ? -16 : 16));
    setWidth(next);
    try {
      localStorage.setItem(WIDTH_KEY, String(next));
    } catch {
      // ignore
    }
  };

  if (entries === null) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground" aria-busy="true">
        {s.loading}
      </div>
    );
  }

  if (entries.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3">
        <Package className="size-8 text-muted-foreground" aria-hidden="true" />
        {error ? (
          <p role="alert" className="max-w-md text-center text-sm text-destructive">
            {fmt(s.loadFailed, { message: error })}
          </p>
        ) : (
          <p className="text-sm text-muted-foreground">{s.emptyTitle}</p>
        )}
        {error ? (
          <Button type="button" variant="outline" onClick={() => void load()} className="h-7 cursor-pointer bg-card px-3 text-xs hover:bg-muted">
            {s.retry}
          </Button>
        ) : (
          <Button type="button" onClick={onAdd} className="h-7 cursor-pointer px-3 text-xs font-semibold">
            {s.addSkill}
          </Button>
        )}
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-1">
      <div className="relative shrink-0" style={{ width }}>
        <SkillList
          entries={entries}
          projectName={projectName}
          updates={updates}
          selectedKey={selectedKey}
          onSelect={select}
          gitOk={gitOk}
          checking={checking}
          checkedUpdates={checkedUpdates}
          onCheck={() => void check()}
          onAdd={onAdd}
        />
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label={s.resize}
          aria-valuemin={MIN_W}
          aria-valuemax={MAX_W}
          aria-valuenow={width}
          tabIndex={0}
          onPointerDown={startResize}
          onKeyDown={onResizeKey}
          className="absolute top-0 right-0 h-full w-1 translate-x-1/2 cursor-col-resize outline-none transition-colors duration-150 hover:bg-primary/40 focus-visible:bg-ring motion-reduce:transition-none"
        />
      </div>
      <div className="flex min-w-0 flex-1 flex-col border-l border-border">
        {checkError || error ? (
          <div role="alert" className="mx-4 mt-3 flex shrink-0 items-center gap-2.5 rounded-md bg-destructive/15 px-3 py-2 text-sm text-destructive">
            <TriangleAlert className="size-4 shrink-0" aria-hidden="true" />
            <span className="text-foreground">{checkError ?? fmt(s.loadFailed, { message: error ?? "" })}</span>
          </div>
        ) : null}
        {selected ? (
          <SkillDetail
            key={entryKey(selected)}
            entry={selected}
            installs={installs}
            projectName={projectName}
            update={updates.get(selected.id)}
            onUpdate={onUpdate}
            onSync={onSync}
            onDelete={onDelete}
            onBackups={onBackups}
            onChanged={() => void load()}
            onDirtyChange={setDirty}
          />
        ) : (
          <div className="flex flex-1 flex-col items-center justify-center gap-3">
            <Package className="size-8 text-muted-foreground" aria-hidden="true" />
            <p className="text-sm text-muted-foreground">{s.pickOne}</p>
          </div>
        )}
      </div>
      <ConfirmDialog
        open={pending !== null}
        title={s.discardTitle}
        body={s.discardBody}
        confirmLabel={s.discardConfirm}
        cancelLabel={s.keepEditing}
        destructive
        onConfirm={() => {
          if (pending) {
            setDirty(false);
            setSelectedKey(entryKey(pending));
          }
          setPending(null);
        }}
        onCancel={() => setPending(null)}
      />
    </div>
  );
}
