import * as React from "react";
import CodeMirror from "@uiw/react-codemirror";
import { MessageSquareQuote, TriangleAlert } from "lucide-react";
import type { Suggestion, SuggestionKind } from "@/api/types";
import { DiffView } from "@/components/diff/DiffView";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { memdockCmTheme } from "@/pages/memory/cmTheme";
import { formatSnapshotTime } from "@/pages/memory/HistoryDrawer";
import { basename, fmt } from "@/pages/memory/paths";
import { ToolTag } from "@/pages/memory/ToolTag";

export interface SuggestionCardProps {
  suggestion: Suggestion;
  /** Project name, or the localized "global". */
  projectLabel: string;
  /** The target changed under the suggestion: greyed out, only "reject" is left. */
  stale: boolean;
  /** A request for this card (or a bulk accept) is in flight. */
  busy: boolean;
  /** Resolves with an inline error message to show in the card, or null. */
  onAccept: (editedText: string | null) => Promise<string | null>;
  onReject: () => void;
  onOpenEvidence: () => void;
}

const BAR: Record<SuggestionKind, string> = {
  add: "bg-primary",
  modify: "bg-warning",
  obsolete: "bg-destructive",
};
const TAG: Record<SuggestionKind, string> = {
  add: "bg-primary/15 text-accent-text",
  modify: "bg-warning/15 text-warning",
  obsolete: "bg-destructive/15 text-destructive",
};

const extensions = [...memdockCmTheme];

export function SuggestionCard({ suggestion: s, projectLabel, stale, busy, onAccept, onReject, onOpenEvidence }: SuggestionCardProps) {
  const { t } = useI18n();
  const r = t.review;
  const [editing, setEditing] = React.useState(false);
  const [draft, setDraft] = React.useState(s.new_text ?? "");
  const [error, setError] = React.useState<string | null>(null);

  const kindLabel = { add: r.kindAdd, modify: r.kindModify, obsolete: r.kindObsolete }[s.kind];
  const oldText = s.kind === "add" ? "" : (s.old_text ?? "");
  const newText = s.new_text ?? "";
  const newLabel = s.kind === "add" ? r.addLabel : s.kind === "obsolete" && newText === "" ? r.deleteLabel : r.newLabel;
  const first = s.evidence[0];
  const toolName = (tool: "claude" | "codex") => (tool === "claude" ? t.memory.toolClaude : t.memory.toolCodex);

  const accept = async (edited: string | null) => {
    setError(null);
    setError(await onAccept(edited));
  };

  return (
    <article
      data-kind={s.kind}
      data-stale={stale ? "true" : undefined}
      aria-label={`${kindLabel} ${projectLabel} / ${basename(s.path)}`}
      className={cn("relative overflow-hidden rounded-lg border border-border bg-card", stale && "opacity-70")}
    >
      <span aria-hidden="true" className={cn("absolute inset-y-0 left-0 w-[3px]", stale ? "bg-muted-foreground" : BAR[s.kind])} />
      <div className="flex flex-wrap items-center gap-2 py-2.5 pr-4 pl-[17px]">
        <span className={cn("inline-flex h-[18px] items-center rounded-sm px-1.5 text-xs leading-[18px] font-semibold", TAG[s.kind])}>{kindLabel}</span>
        <ToolTag tool={s.tool_of_file} />
        <span title={s.path} className="min-w-0 truncate font-mono text-[12.5px] font-medium">
          {projectLabel} / {basename(s.path)}
        </span>
        <span className="flex-1" />
        <span className="shrink-0 text-xs text-muted-foreground">{formatSnapshotTime(s.created_at)}</span>
      </div>

      <div className="pr-4 pl-[17px]">
        <DiffView oldText={oldText} newText={newText} oldLabel={r.oldLabel} newLabel={newLabel} />
      </div>

      {s.reason ? <p className="pt-2 pr-4 pl-[17px] text-sm">{s.reason}</p> : null}

      {first ? (
        <div className="mt-2 mr-4 ml-[17px] flex items-start gap-2 rounded-md bg-muted px-2.5 py-2 text-xs text-muted-foreground">
          <MessageSquareQuote className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <p className="min-w-0 flex-1 break-words">
            {fmt(r.evidenceFrom, { tool: toolName(first.tool), time: formatSnapshotTime(first.ts) })}
            <q className="font-content text-foreground">{first.quote}</q>
            {s.evidence.length > 1 ? <span> {fmt(r.evidenceMore, { n: s.evidence.length - 1 })}</span> : null}
          </p>
          <button
            type="button"
            onClick={onOpenEvidence}
            className="shrink-0 cursor-pointer rounded-sm text-accent-text outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
          >
            {r.viewConversation}
          </button>
        </div>
      ) : null}

      {stale ? (
        <p role="status" className="flex items-center gap-1.5 pt-2.5 pr-4 pl-[17px] text-sm text-warning">
          <TriangleAlert className="size-4 shrink-0" aria-hidden="true" />
          {r.stale}
        </p>
      ) : null}

      {editing && !stale ? (
        <div className="mt-2.5 mr-4 ml-[17px]">
          <div className="max-h-80 overflow-auto rounded-md border border-border focus-within:ring-2 focus-within:ring-ring">
            <CodeMirror
              value={draft}
              onChange={setDraft}
              extensions={extensions}
              theme="none"
              minHeight="96px"
              aria-label={r.editLabel}
              basicSetup={{ lineNumbers: false, foldGutter: false, highlightActiveLine: false, highlightActiveLineGutter: false }}
            />
          </div>
        </div>
      ) : null}

      {error ? (
        <p role="alert" className="pt-2 pr-4 pl-[17px] text-sm text-destructive">
          {fmt(r.invalidEdit, { message: error })}
        </p>
      ) : null}

      <div className="flex flex-wrap gap-2 py-3 pr-4 pl-[17px]">
        {stale ? null : editing ? (
          <>
            <Button type="button" disabled={busy} onClick={() => void accept(draft)} className="h-7 cursor-pointer px-3 text-xs font-semibold">
              {r.confirmEdit}
            </Button>
            <Button
              type="button"
              variant="outline"
              disabled={busy}
              onClick={() => {
                setEditing(false);
                setError(null);
                setDraft(s.new_text ?? "");
              }}
              className="h-7 cursor-pointer px-3 text-xs"
            >
              {r.cancelEdit}
            </Button>
          </>
        ) : (
          <>
            <Button type="button" disabled={busy} onClick={() => void accept(null)} className="h-7 cursor-pointer px-3 text-xs font-semibold">
              {r.accept}
            </Button>
            <Button type="button" variant="outline" disabled={busy} onClick={() => setEditing(true)} className="h-7 cursor-pointer px-3 text-xs">
              {r.editAccept}
            </Button>
          </>
        )}
        <Button type="button" variant="outline" disabled={busy} onClick={onReject} className="h-7 cursor-pointer px-3 text-xs text-destructive hover:text-destructive">
          {r.reject}
        </Button>
      </div>
    </article>
  );
}
