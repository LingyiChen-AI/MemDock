import * as React from "react";
import CodeMirror from "@uiw/react-codemirror";
import { Pencil } from "lucide-react";
import type { SyncConflict, SyncResolution } from "@/api/types";
import { Button } from "@/components/ui/button";
import { DiffView } from "@/components/diff/DiffView";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { memdockCmTheme } from "@/pages/memory/cmTheme";
import { fmt } from "@/pages/memory/paths";
import { InlineError } from "@/pages/skills/flowParts";
import { conflictCount, hasMarkers, parse, render, type Choice } from "./conflictBlocks";

/** What the user picked for one conflicted file. */
export type ConflictState =
  | { type: "blocks"; choices: Record<number, Choice>; manual: string | null }
  | { type: "deletion"; choice: "delete" | "keep" };

/** One side of the conflict no longer exists. */
export function isDeletion(c: SyncConflict): boolean {
  return c.local === null || c.remote === null;
}

/** The resolution to send, or `null` while the file is still unresolved. */
export function resolutionOf(c: SyncConflict, s: ConflictState | undefined): SyncResolution | null {
  if (isDeletion(c)) {
    if (s?.type !== "deletion") return null;
    return s.choice === "delete" ? { kind: "delete_local" } : { kind: "text", text: c.local ?? c.remote ?? "" };
  }
  if (s?.type === "blocks" && s.manual !== null) return hasMarkers(s.manual) ? null : { kind: "text", text: s.manual };
  const text = render(parse(c.merged), s?.type === "blocks" ? s.choices : {});
  return text === null ? null : { kind: "text", text };
}

const extensions = [...memdockCmTheme];

export function ChoiceButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <Button
      type="button"
      variant={active ? "default" : "outline"}
      aria-pressed={active}
      onClick={onClick}
      className={cn("h-7 cursor-pointer px-3 text-xs", active && "font-semibold")}
    >
      {children}
    </Button>
  );
}

export interface ConflictResolverProps {
  conflict: SyncConflict;
  value: ConflictState | undefined;
  onChange: (s: ConflictState) => void;
}

export function ConflictResolver({ conflict, value, onChange }: ConflictResolverProps) {
  const { t } = useI18n();
  const s = t.sync;
  const blocks = React.useMemo(() => parse(conflict.merged), [conflict.merged]);
  const total = conflictCount(blocks);
  const [editing, setEditing] = React.useState(false);
  const [draft, setDraft] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  React.useEffect(() => {
    setEditing(false);
    setError(null);
  }, [conflict.repo_path]);

  if (isDeletion(conflict)) {
    const choice = value?.type === "deletion" ? value.choice : null;
    return (
      <div className="flex flex-col gap-3">
        <p className="text-sm">{conflict.local === null ? s.localDeleted : s.remoteDeletedConflict}</p>
        <div className="flex gap-2">
          <ChoiceButton active={choice === "delete"} onClick={() => onChange({ type: "deletion", choice: "delete" })}>
            {s.delete}
          </ChoiceButton>
          <ChoiceButton active={choice === "keep"} onClick={() => onChange({ type: "deletion", choice: "keep" })}>
            {s.keepFile}
          </ChoiceButton>
        </div>
        <DiffView oldText={conflict.local ?? ""} newText={conflict.remote ?? ""} oldLabel={s.local} newLabel={s.remote} />
      </div>
    );
  }

  const choices = value?.type === "blocks" ? value.choices : {};
  const manual = value?.type === "blocks" ? value.manual : null;
  const pick = (i: number, c: Choice) => onChange({ type: "blocks", choices: { ...choices, [i]: c }, manual: null });

  const openEditor = () => {
    setDraft(manual ?? render(blocks, choices) ?? conflict.merged);
    setError(null);
    setEditing(true);
  };
  const applyManual = () => {
    if (hasMarkers(draft)) {
      setError(s.manualHasMarkers);
      return;
    }
    onChange({ type: "blocks", choices, manual: draft });
    setEditing(false);
  };

  let ordinal = -1;
  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-2">
        {manual !== null ? <span className="text-xs text-accent-text">{s.manualUsed}</span> : null}
        <span className="flex-1" />
        {editing ? null : (
          <Button type="button" variant="outline" onClick={openEditor} className="h-7 cursor-pointer px-3 text-xs">
            <Pencil className="size-3.5" aria-hidden="true" />
            {s.manualEdit}
          </Button>
        )}
      </div>

      {editing ? (
        <div className="flex flex-col gap-2">
          <div className="max-h-[60vh] overflow-auto rounded-md border border-border font-content focus-within:ring-2 focus-within:ring-ring">
            <CodeMirror
              value={draft}
              onChange={setDraft}
              extensions={extensions}
              theme="none"
              minHeight="160px"
              aria-label={s.manualLabel}
              basicSetup={{ foldGutter: false, highlightActiveLine: false }}
            />
          </div>
          {error ? <InlineError message={error} /> : null}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" onClick={() => setEditing(false)} className="h-7 cursor-pointer px-3 text-xs">
              {s.manualCancel}
            </Button>
            <Button type="button" onClick={applyManual} className="h-7 cursor-pointer px-3 text-xs font-semibold">
              {s.manualApply}
            </Button>
          </div>
        </div>
      ) : (
        blocks.map((b, i) => {
          if (b.kind === "text") return null;
          ordinal++;
          const n = ordinal;
          const c = manual === null ? choices[n] : undefined;
          return (
            <section key={i} aria-label={fmt(s.block, { i: n + 1, n: total })} className="rounded-md border border-border bg-card p-3">
              <div className="mb-2 flex flex-wrap items-center gap-2">
                <span className="text-xs font-semibold text-muted-foreground">{fmt(s.block, { i: n + 1, n: total })}</span>
                <span className="flex-1" />
                <ChoiceButton active={c === "local"} onClick={() => pick(n, "local")}>
                  {s.useLocal}
                </ChoiceButton>
                <ChoiceButton active={c === "remote"} onClick={() => pick(n, "remote")}>
                  {s.useRemote}
                </ChoiceButton>
                <ChoiceButton active={c === "both"} onClick={() => pick(n, "both")}>
                  {s.useBoth}
                </ChoiceButton>
              </div>
              <DiffView oldText={b.local} newText={b.remote} oldLabel={s.local} newLabel={s.remote} />
            </section>
          );
        })
      )}
    </div>
  );
}
