import * as React from "react";
import { TriangleAlert } from "lucide-react";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { SimpleSelect } from "@/components/ui/select";
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
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        style={{ width }}
        closeLabel={t.skills.close}
        onOpenAutoFocus={(e) => {
          // Land on the dialog itself, not on the close button.
          e.preventDefault();
          (e.currentTarget as HTMLElement).focus();
        }}
        className="flex max-h-[calc(100vh-48px)] flex-col overflow-hidden"
      >
        <DialogTitle>{title}</DialogTitle>
        <DialogDescription className="mt-1">{description}</DialogDescription>
        {children}
      </DialogContent>
    </Dialog>
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
        {single ? (
          <RadioGroup
            aria-label={s.installToTools}
            value={value.tools[0] ?? ""}
            onValueChange={(tool) => onChange({ ...value, tools: [tool as Tool] })}
            className="flex flex-wrap items-center gap-x-6 gap-y-2"
          >
            {(["claude", "codex"] as const).map((tool) => (
              <label key={tool} className={optionLabel}>
                <RadioGroupItem value={tool} />
                {toolName(tool)}
              </label>
            ))}
          </RadioGroup>
        ) : (
          (["claude", "codex"] as const).map((tool) => (
            <label key={tool} className={optionLabel}>
              <Checkbox
                checked={value.tools.includes(tool)}
                onCheckedChange={(c) =>
                  onChange({ ...value, tools: c === true ? [...value.tools, tool] : value.tools.filter((x) => x !== tool) })
                }
              />
              {toolName(tool)}
            </label>
          ))
        )}
      </div>
      <div className="flex flex-wrap items-center gap-x-6 gap-y-2">
        <span className="w-14 text-xs font-medium text-muted-foreground">{s.installToScope}</span>
        <RadioGroup
          aria-label={s.installToScope}
          value={value.scope}
          onValueChange={(scope) => onChange({ ...value, scope: scope as TargetSelection["scope"] })}
          className="flex flex-wrap items-center gap-x-6 gap-y-2"
        >
          <label className={optionLabel}>
            <RadioGroupItem value="global" />
            {s.scopeGlobalOpt}
          </label>
          <label className={cn(optionLabel, usable.length === 0 && "cursor-not-allowed opacity-50")}>
            <RadioGroupItem value="project" disabled={usable.length === 0} />
            {s.scopeProjectOpt}
          </label>
        </RadioGroup>
        {usable.length === 0 ? <span className="text-xs text-muted-foreground">{s.noProjects}</span> : null}
      </div>
      {value.scope === "project" ? (
        <div className="flex items-center gap-x-6">
          <label htmlFor={`${groupName}-project`} className="w-14 text-xs font-medium text-muted-foreground">
            {s.scopeProjectLabel}
          </label>
          <SimpleSelect
            id={`${groupName}-project`}
            value={value.projectId}
            onValueChange={(projectId) => onChange({ ...value, projectId })}
            placeholder={s.pickProject}
            options={usable.map((p) => ({ value: p.id, label: p.name }))}
            className="w-auto flex-1"
          />
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
