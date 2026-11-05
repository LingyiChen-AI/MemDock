import * as React from "react";
import { Dialog } from "radix-ui";
import { TriangleAlert, X } from "lucide-react";
import type { InstallTarget, Project, SkillScope, Tool } from "@/api/types";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type { Messages } from "@/i18n";
import { fmt } from "@/pages/memory/paths";
import { asCommandError, errorMessage } from "./skillUtils";

/** User-facing text for a failed skill command (the raw message stays for git stderr and the like). */
export function describeError(e: unknown, s: Messages["skills"]): string {
  const ce = asCommandError(e);
  if (!ce) return errorMessage(e);
  if (ce.code === "git_unavailable") return s.gitUnavailableErr;
  if (ce.code === "read_only") return s.readOnlyErr;
  if (ce.code === "invalid" && /project directory not found/i.test(ce.message)) return s.projectMissing;
  return ce.message;
}

export const PARTIAL_SAVE_PREFIX = "files updated, registry not saved";

/** Modal shell shared by the add and sync dialogs. */
export function FlowDialog({
  open,
  onOpenChange,
  title,
  description,
  width = 520,
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  width?: number;
  children: React.ReactNode;
}) {
  const { t } = useI18n();
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/50 data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 motion-reduce:animate-none" />
        <Dialog.Content
          style={{ width }}
          onOpenAutoFocus={(e) => {
            // Land on the dialog itself, not on the close button.
            e.preventDefault();
            (e.currentTarget as HTMLElement).focus();
          }}
          className="fixed top-1/2 left-1/2 z-50 flex max-h-[calc(100vh-48px)] max-w-[calc(100vw-32px)] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-lg border border-border bg-popover p-5 text-popover-foreground shadow-xl outline-none data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95 motion-reduce:animate-none"
        >
          <Dialog.Title className="text-base font-semibold">{title}</Dialog.Title>
          <Dialog.Description className="mt-1 text-sm text-muted-foreground">{description}</Dialog.Description>
          <Dialog.Close asChild>
            <button
              type="button"
              aria-label={t.skills.close}
              className="absolute top-3 right-3 grid size-7 cursor-pointer place-items-center rounded-sm text-muted-foreground transition-colors duration-150 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none"
            >
              <X className="size-4" aria-hidden="true" />
            </button>
          </Dialog.Close>
          {children}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

/** Inline failure banner; long text (git stderr) is clamped to three lines with a toggle. */
export function InlineError({ message }: { message: string }) {
  const { t } = useI18n();
  const s = t.skills;
  const [expanded, setExpanded] = React.useState(false);
  React.useEffect(() => setExpanded(false), [message]);
  const long = message.length > 150 || message.split("\n").length > 3;
  return (
    <div role="alert" className="flex items-start gap-2.5 rounded-md bg-destructive/15 px-3 py-2 text-sm text-destructive">
      <TriangleAlert className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
      <div className="min-w-0 flex-1">
        <p className={cn("break-words whitespace-pre-wrap text-foreground", long && !expanded && "line-clamp-3")}>{message}</p>
        {long ? (
          <button
            type="button"
            aria-expanded={expanded}
            onClick={() => setExpanded((v) => !v)}
            className="mt-1 cursor-pointer rounded-sm text-xs text-accent-text outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
          >
            {expanded ? s.collapse : s.expand}
          </button>
        ) : null}
      </div>
    </div>
  );
}

export interface TargetSelection {
  tools: Tool[];
  scope: "global" | "project";
  projectId: string;
}

export function targetsOf(sel: TargetSelection): InstallTarget[] {
  const scope: SkillScope = sel.scope === "global" ? { type: "global" } : { type: "project", project_id: sel.projectId };
  return sel.tools.map((tool) => ({ tool, scope }));
}

/** Why the selection cannot be used yet (null = fine). */
export function selectionProblem(sel: TargetSelection, s: Messages["skills"]): string | null {
  if (sel.tools.length === 0) return s.needOneTool;
  if (sel.scope === "project" && !sel.projectId) return s.needProject;
  return null;
}

const nativeInput = "size-3.5 shrink-0 cursor-pointer accent-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
const optionLabel = "inline-flex cursor-pointer items-center gap-2 text-sm";

/** "Install to": tools (checkboxes, or radios when `single`) and scope (global / one project). */
export function TargetPicker({
  value,
  onChange,
  projects,
  single = false,
  disabled = false,
}: {
  value: TargetSelection;
  onChange: (next: TargetSelection) => void;
  projects: Project[];
  single?: boolean;
  disabled?: boolean;
}) {
  const { t } = useI18n();
  const s = t.skills;
  const usable = projects.filter((p) => p.exists);
  const toolName = (tool: Tool) => (tool === "claude" ? t.memory.toolClaude : t.memory.toolCodex);
  const problem = selectionProblem(value, s);
  const groupName = React.useId();
  return (
    <fieldset disabled={disabled} className="flex min-w-0 flex-col gap-3 border-0 p-0">
      <div className="flex flex-wrap items-center gap-x-6 gap-y-2">
        <span className="w-14 text-xs font-medium text-muted-foreground">{s.installToTools}</span>
        {(["claude", "codex"] as const).map((tool) => (
          <label key={tool} className={optionLabel}>
            <input
              type={single ? "radio" : "checkbox"}
              name={single ? `${groupName}-tool` : undefined}
              checked={value.tools.includes(tool)}
              onChange={(e) => {
                if (single) onChange({ ...value, tools: [tool] });
                else onChange({ ...value, tools: e.target.checked ? [...value.tools, tool] : value.tools.filter((x) => x !== tool) });
              }}
              className={nativeInput}
            />
            {toolName(tool)}
          </label>
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-x-6 gap-y-2">
        <span className="w-14 text-xs font-medium text-muted-foreground">{s.installToScope}</span>
        <label className={optionLabel}>
          <input
            type="radio"
            name={`${groupName}-scope`}
            checked={value.scope === "global"}
            onChange={() => onChange({ ...value, scope: "global" })}
            className={nativeInput}
          />
          {s.scopeGlobalOpt}
        </label>
        <label className={cn(optionLabel, usable.length === 0 && "cursor-not-allowed opacity-50")}>
          <input
            type="radio"
            name={`${groupName}-scope`}
            checked={value.scope === "project"}
            disabled={usable.length === 0}
            onChange={() => onChange({ ...value, scope: "project" })}
            className={nativeInput}
          />
          {s.scopeProjectOpt}
        </label>
        {usable.length === 0 ? <span className="text-xs text-muted-foreground">{s.noProjects}</span> : null}
      </div>
      {value.scope === "project" ? (
        <div className="flex items-center gap-x-6">
          <label htmlFor={`${groupName}-project`} className="w-14 text-xs font-medium text-muted-foreground">
            {s.scopeProjectLabel}
          </label>
          <select
            id={`${groupName}-project`}
            value={value.projectId}
            onChange={(e) => onChange({ ...value, projectId: e.target.value })}
            className="h-8 min-w-0 flex-1 cursor-pointer rounded-md border border-input bg-card px-2 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring"
          >
            <option value="">{s.pickProject}</option>
            {usable.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </div>
      ) : null}
      {problem && !single ? <p className="text-xs text-warning">{problem}</p> : null}
    </fieldset>
  );
}

/** "Claude Code · jev" style label of one target, for toasts. */
export function targetLabel(target: InstallTarget, projects: Project[], t: Messages): string {
  const tool = target.tool === "claude" ? t.memory.toolClaude : t.memory.toolCodex;
  const scope =
    target.scope.type === "global"
      ? t.skills.global
      : (projects.find((p) => p.id === (target.scope as { project_id: string }).project_id)?.name ?? "");
  return fmt("{tool} · {scope}", { tool, scope });
}
