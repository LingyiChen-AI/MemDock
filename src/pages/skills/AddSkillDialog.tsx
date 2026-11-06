import * as React from "react";
import { open as pickPath } from "@tauri-apps/plugin-dialog";
import { FileArchive, FolderOpen, Loader2 } from "lucide-react";
import { inspectGitSource, installSkill } from "@/api/skills";
import type { InspectResult, InstallFrom, Project, SkillEntry } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { fmt } from "@/pages/memory/paths";
import { FlowDialog, InlineError, TargetPicker, describeError, selectionProblem, targetsOf, type TargetSelection } from "./flowParts";
import { asCommandError, existingDirs, shortCommit } from "./skillUtils";

type Tab = "github" | "local" | "template";

export interface AddSkillDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  projects: Project[];
  /** git is installed; the GitHub tab needs it. */
  gitOk: boolean;
  /** Every skill that was installed (also called after a partial failure is not needed: the list reloads from disk events). */
  onInstalled: (entries: SkillEntry[]) => void;
}

/** Mirrors the backend rule: [a-z0-9][a-z0-9._-]*, max 64, no trailing dot. */
export function validSkillName(name: string): boolean {
  return /^[a-z0-9][a-z0-9._-]*$/.test(name) && name.length <= 64 && !name.endsWith(".");
}

const INITIAL_TARGET: TargetSelection = { tools: ["claude"], scope: "global", projectId: "" };

export function AddSkillDialog({ open, onOpenChange, projects, gitOk, onInstalled }: AddSkillDialogProps) {
  const { t } = useI18n();
  const s = t.skills;
  const [tab, setTab] = React.useState<Tab>(gitOk ? "github" : "local");
  // GitHub
  const [input, setInput] = React.useState("");
  const [inspecting, setInspecting] = React.useState(false);
  const [inspectError, setInspectError] = React.useState<string | null>(null);
  const [inspected, setInspected] = React.useState<InspectResult | null>(null);
  const [picked, setPicked] = React.useState<string[]>([]);
  // Local
  const [local, setLocal] = React.useState<{ kind: "folder" | "zip"; path: string } | null>(null);
  // Template
  const [name, setName] = React.useState("");
  const [description, setDescription] = React.useState("");
  // Shared
  const [target, setTarget] = React.useState<TargetSelection>(INITIAL_TARGET);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [confirm, setConfirm] = React.useState<{ dirs: string[] } | null>(null);
  // What is left to install after an `exists` prompt, and what already got installed.
  const queue = React.useRef<{ rest: InstallFrom[]; done: SkillEntry[] }>({ rest: [], done: [] });
  const busyRef = React.useRef(false);
  // Bumped on every inspect and on every (re)open: older responses are ignored.
  const inspectSeq = React.useRef(0);

  React.useEffect(() => {
    inspectSeq.current += 1;
    if (!open) return;
    setTab(gitOk ? "github" : "local");
    setInput("");
    setInspecting(false);
    setInspectError(null);
    setInspected(null);
    setPicked([]);
    setLocal(null);
    setName("");
    setDescription("");
    setTarget(INITIAL_TARGET);
    setBusy(false);
    setError(null);
    setConfirm(null);
    busyRef.current = false;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const inspect = async () => {
    const value = input.trim();
    if (!value || inspecting) return;
    const seq = ++inspectSeq.current;
    setInspecting(true);
    setInspectError(null);
    setInspected(null);
    setPicked([]);
    try {
      const r = await inspectGitSource(value);
      if (seq !== inspectSeq.current) return;
      setInspected(r);
      if (r.preselected && r.skills.some((x) => x.skill_path === r.preselected)) setPicked([r.preselected]);
      else if (r.skills.length === 1) setPicked([r.skills[0].skill_path]);
    } catch (e) {
      if (seq === inspectSeq.current) setInspectError(describeError(e, s));
    } finally {
      if (seq === inspectSeq.current) setInspecting(false);
    }
  };

  const choose = async (kind: "folder" | "zip") => {
    try {
      const p =
        kind === "folder"
          ? await pickPath({ directory: true, multiple: false, title: s.pickDialogFolder })
          : await pickPath({ directory: false, multiple: false, title: s.pickDialogZip, filters: [{ name: "zip", extensions: ["zip"] }] });
      if (typeof p === "string" && p) {
        setLocal({ kind, path: p });
        setError(null);
      }
    } catch (e) {
      setError(describeError(e, s));
    }
  };

  const sources = (): InstallFrom[] => {
    if (tab === "github" && inspected)
      return inspected.skills
        .filter((x) => picked.includes(x.skill_path))
        .map((x) => ({ type: "git", url: inspected.url, clone_url: inspected.clone_url, git_ref: inspected.git_ref, skill_path: x.skill_path, commit: inspected.commit }));
    if (tab === "local" && local) return [{ type: local.kind, path: local.path }];
    if (tab === "template") return [{ type: "template", name, description: description.trim() }];
    return [];
  };

  const nameOk = validSkillName(name);
  const sourceReady =
    tab === "github" ? picked.length > 0 : tab === "local" ? local !== null : nameOk;
  const problem = selectionProblem(target, s);
  const canInstall = sourceReady && !problem && !busy;

  const run = async (rest: InstallFrom[], done: SkillEntry[], overwriteFirst: boolean) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    const targets = targetsOf(target);
    let left = rest;
    let first = true;
    try {
      while (left.length > 0) {
        const res = await installSkill(left[0], targets, first && overwriteFirst);
        done = [...done, ...res];
        left = left.slice(1);
        first = false;
      }
      onInstalled(done);
      onOpenChange(false);
    } catch (e) {
      if (asCommandError(e)?.code === "exists") {
        queue.current = { rest: left, done };
        setConfirm({ dirs: existingDirs(e) });
      } else {
        setError(fmt(s.installFailed, { message: describeError(e, s) }));
        if (done.length > 0) onInstalled(done);
      }
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };

  const tabs: { key: Tab; label: string; disabled?: boolean }[] = [
    { key: "github", label: s.tabGithub, disabled: !gitOk },
    { key: "local", label: s.tabLocal },
    { key: "template", label: s.tabTemplate },
  ];

  const toggle = (path: string) => setPicked((p) => (p.includes(path) ? p.filter((x) => x !== path) : [...p, path]));

  return (
    <>
      <FlowDialog open={open} onOpenChange={(o) => (busy && !o ? undefined : onOpenChange(o))} title={s.addTitle} description={s.addDesc} width={560}>
        <div className="mt-4 flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pr-1">
          <div role="tablist" aria-label={s.addTabsLabel} className="inline-flex self-start overflow-hidden rounded-md border border-border">
            {tabs.map((x) => (
              <button
                key={x.key}
                type="button"
                role="tab"
                aria-selected={tab === x.key}
                disabled={x.disabled || busy}
                onClick={() => {
                  setTab(x.key);
                  setError(null);
                }}
                className={cn(
                  "cursor-pointer px-3 text-xs leading-7 outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset disabled:cursor-not-allowed disabled:opacity-50 motion-reduce:transition-none",
                  tab === x.key ? "bg-muted font-medium text-foreground" : "text-muted-foreground hover:text-foreground",
                )}
              >
                {x.label}
              </button>
            ))}
          </div>

          {!gitOk ? <p className="rounded-md bg-warning/15 px-3 py-2 text-sm text-foreground">{s.gitMissing}</p> : null}

          {tab === "github" ? (
            <div className="flex flex-col gap-3">
              <form
                className="flex items-end gap-2"
                onSubmit={(e) => {
                  e.preventDefault();
                  void inspect();
                }}
              >
                <div className="flex min-w-0 flex-1 flex-col gap-1.5">
                  <label htmlFor="add-git-input" className="text-xs font-medium text-muted-foreground">
                    {s.gitInputLabel}
                  </label>
                  <Input
                    id="add-git-input"
                    value={input}
                    onChange={(e) => setInput(e.target.value)}
                    placeholder={s.gitInputPlaceholder}
                    autoComplete="off"
                    spellCheck={false}
                    disabled={busy}
                    className="h-8 font-mono text-[12.5px]"
                  />
                </div>
                <Button type="submit" variant="outline" disabled={!input.trim() || inspecting || busy} className="h-8 cursor-pointer gap-1.5 bg-card px-3 text-xs hover:bg-muted">
                  {inspecting ? <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" /> : null}
                  {inspecting ? s.inspecting : s.inspect}
                </Button>
              </form>
              {inspectError ? <InlineError message={inspectError} /> : null}
              {inspected ? (
                inspected.skills.length === 0 ? (
                  <p className="text-sm text-muted-foreground">{s.noSkillsInRepo}</p>
                ) : (
                  <div className="flex flex-col gap-2">
                    <p className="text-xs text-muted-foreground">
                      {fmt(s.inspectResult, { n: inspected.skills.length, repo: inspected.url.replace(/^https?:\/\/github\.com\//, "") })} ·{" "}
                      <span className="font-mono">{fmt(s.inspectResultAt, { commit: shortCommit(inspected.commit) })}</span>
                    </p>
                    <ul aria-label={s.repoSkillsLabel} className="flex max-h-56 flex-col overflow-y-auto rounded-md border border-border">
                      {inspected.skills.map((x) => (
                        <li key={x.skill_path} className="border-b border-border last:border-b-0">
                          <label className="flex cursor-pointer items-start gap-2.5 px-3 py-2 hover:bg-muted/60">
                            <Checkbox
                              checked={picked.includes(x.skill_path)}
                              onCheckedChange={() => toggle(x.skill_path)}
                              disabled={busy}
                              className="mt-0.5"
                            />
                            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                              <span className="font-mono text-[13px] font-medium">{x.name}</span>
                              {x.description ? <span className="line-clamp-2 text-xs text-muted-foreground">{x.description}</span> : null}
                              <span className="truncate font-mono text-xs text-muted-foreground" title={x.skill_path}>
                                {x.skill_path || s.repoRoot}
                              </span>
                            </span>
                          </label>
                        </li>
                      ))}
                    </ul>
                  </div>
                )
              ) : null}
            </div>
          ) : null}

          {tab === "local" ? (
            <div className="flex flex-col gap-3">
              <div className="flex flex-wrap gap-2">
                <Button type="button" variant="outline" disabled={busy} onClick={() => void choose("folder")} className="h-8 cursor-pointer gap-1.5 bg-card px-3 text-xs hover:bg-muted">
                  <FolderOpen className="size-4" aria-hidden="true" />
                  {s.pickFolder}
                </Button>
                <Button type="button" variant="outline" disabled={busy} onClick={() => void choose("zip")} className="h-8 cursor-pointer gap-1.5 bg-card px-3 text-xs hover:bg-muted">
                  <FileArchive className="size-4" aria-hidden="true" />
                  {s.pickZip}
                </Button>
              </div>
              <p className="min-w-0 text-sm">
                {local ? (
                  <>
                    <span className="text-muted-foreground">{s.pickedPath}：</span>
                    <span className="font-mono text-[12.5px] break-all select-text">{local.path}</span>
                  </>
                ) : (
                  <span className="text-muted-foreground">{s.pickedNothing}</span>
                )}
              </p>
            </div>
          ) : null}

          {tab === "template" ? (
            <div className="flex flex-col gap-3">
              <div className="flex flex-col gap-1.5">
                <label htmlFor="add-tpl-name" className="text-xs font-medium text-muted-foreground">
                  {s.templateName}
                </label>
                <Input
                  id="add-tpl-name"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder={s.templateNamePlaceholder}
                  autoComplete="off"
                  spellCheck={false}
                  disabled={busy}
                  aria-invalid={name !== "" && !nameOk}
                  className="h-8 font-mono text-[12.5px]"
                />
                <p className={cn("text-xs", name !== "" && !nameOk ? "text-destructive" : "text-muted-foreground")}>
                  {name !== "" && !nameOk ? s.templateNameInvalid : s.templateNameHint}
                </p>
              </div>
              <div className="flex flex-col gap-1.5">
                <label htmlFor="add-tpl-desc" className="text-xs font-medium text-muted-foreground">
                  {s.templateDesc}
                </label>
                <Input
                  id="add-tpl-desc"
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  placeholder={s.templateDescPlaceholder}
                  autoComplete="off"
                  disabled={busy}
                  className="h-8"
                />
              </div>
            </div>
          ) : null}

          <div className="flex flex-col gap-2.5 border-t border-border pt-4">
            <h3 className="text-xs font-semibold">{s.installTo}</h3>
            <TargetPicker value={target} onChange={setTarget} projects={projects} disabled={busy} />
          </div>

          {error ? <InlineError message={error} /> : null}
        </div>

        <div className="mt-4 flex shrink-0 justify-end gap-2">
          <Button type="button" variant="outline" disabled={busy} onClick={() => onOpenChange(false)} className="h-7 cursor-pointer px-3 text-xs">
            {s.cancel}
          </Button>
          <Button type="button" disabled={!canInstall} onClick={() => void run(sources(), [], false)} className="h-7 cursor-pointer gap-1.5 px-3 text-xs font-semibold">
            {busy ? <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" /> : null}
            {busy ? s.installing : s.install}
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
          void run(queue.current.rest, queue.current.done, true);
        }}
        onCancel={() => {
          setConfirm(null);
          if (queue.current.done.length > 0) onInstalled(queue.current.done);
        }}
      />
    </>
  );
}
