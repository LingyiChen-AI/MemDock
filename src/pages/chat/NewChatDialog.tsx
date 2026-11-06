import * as React from "react";
import { open as pickDir } from "@tauri-apps/plugin-dialog";
import { ChevronRight, FolderOpen, Loader2, TriangleAlert } from "lucide-react";
import type { LiveAgent, PermissionMode } from "@/api/live";
import { listMemories } from "@/api/memory";
import { ToolLogo } from "@/components/brand/ToolLogo";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { SimpleSelect } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";
import { errorMessage } from "@/pages/skills/skillUtils";
import { MODEL_SUGGESTIONS, loadChatPrefs, parseExtraArgs, rememberChoice } from "./chatPrefs";
import { startLive } from "./liveStore";

const MODES: PermissionMode[] = ["ask", "auto_edit", "plan", "bypass"];

interface KnownProject {
  path: string;
  name: string;
}

function useKnownProjects(open: boolean): KnownProject[] {
  const [list, setList] = React.useState<KnownProject[]>([]);
  React.useEffect(() => {
    if (!open) return;
    let alive = true;
    listMemories()
      .then((idx) => {
        if (!alive) return;
        setList(idx.projects.filter((p) => p.project.exists).map((p) => ({ path: p.project.path, name: p.project.name })));
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [open]);
  return list;
}

function Field({ label, htmlFor, children, hint }: { label: string; htmlFor?: string; children: React.ReactNode; hint?: React.ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <label htmlFor={htmlFor} className="text-xs font-medium text-foreground/80">
        {label}
      </label>
      {children}
      {hint ? <div className="text-[11px] leading-relaxed text-muted-foreground">{hint}</div> : null}
    </div>
  );
}

export function NewChatDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const { t } = useI18n();
  const c = t.chat;
  const d = c.dialog;
  const projects = useKnownProjects(open);
  const [agent, setAgent] = React.useState<LiveAgent>("claude");
  const [cwd, setCwd] = React.useState("");
  const [models, setModels] = React.useState<Record<LiveAgent, string>>({ claude: "", codex: "" });
  const [perms, setPerms] = React.useState<Record<LiveAgent, PermissionMode>>({ claude: "ask", codex: "ask" });
  const [advanced, setAdvanced] = React.useState(false);
  const [extra, setExtra] = React.useState("");
  const [prompt, setPrompt] = React.useState("");
  const [starting, setStarting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  // Each time the dialog opens, start from the remembered choices.
  React.useEffect(() => {
    if (!open) return;
    const p = loadChatPrefs();
    setAgent(p.agent);
    setCwd(p.cwd);
    setModels(p.model);
    setPerms(p.permission);
    setAdvanced(false);
    setExtra("");
    setPrompt("");
    setError(null);
  }, [open]);

  const permission = perms[agent];
  const model = models[agent];
  const projectOptions = React.useMemo(() => {
    const opts = projects.map((p) => ({ value: p.path, label: <ProjectOption name={p.name} path={p.path} /> }));
    if (cwd && !projects.some((p) => p.path === cwd)) opts.unshift({ value: cwd, label: <ProjectOption name={baseName(cwd)} path={cwd} /> });
    return opts;
  }, [projects, cwd]);

  const browse = async () => {
    try {
      const picked = await pickDir({ directory: true, multiple: false, title: d.browseTitle, defaultPath: cwd || undefined });
      if (typeof picked === "string" && picked) setCwd(picked);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const start = async () => {
    if (!cwd || starting) return;
    setStarting(true);
    setError(null);
    try {
      await startLive({
        agent,
        cwd,
        model: model.trim() || null,
        permission,
        extra_args: parseExtraArgs(agent, extra),
        prompt: prompt.trim() || null,
      });
      rememberChoice(agent, permission, model.trim(), cwd);
      onOpenChange(false);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setStarting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(o) => (!starting ? onOpenChange(o) : undefined)}>
      <DialogContent className="flex max-h-[90vh] flex-col gap-4 sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{d.title}</DialogTitle>
          <DialogDescription>{d.description}</DialogDescription>
        </DialogHeader>

        <div className="-mx-1 flex min-h-0 flex-col gap-4 overflow-y-auto px-1 pb-1">
          <Field label={d.agent}>
            <div role="radiogroup" aria-label={d.agent} className="inline-flex h-9 w-fit rounded-md border border-border bg-card p-0.5">
              {(["claude", "codex"] as LiveAgent[]).map((a) => (
                <button
                  key={a}
                  type="button"
                  role="radio"
                  aria-checked={agent === a}
                  onClick={() => setAgent(a)}
                  className={cn(
                    "inline-flex cursor-pointer items-center gap-1.5 rounded-sm px-3 text-sm outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none",
                    agent === a ? "seg-active" : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  <ToolLogo tool={a} className={cn("size-3.5", agent === a && "text-current")} aria-hidden="true" />
                  {c.agents[a]}
                </button>
              ))}
            </div>
          </Field>

          <Field label={d.project} hint={cwd ? <span className="font-mono [overflow-wrap:anywhere]">{cwd}</span> : null}>
            <div className="flex min-w-0 gap-2">
              <SimpleSelect
                aria-label={d.project}
                value={cwd}
                onValueChange={setCwd}
                options={projectOptions}
                placeholder={d.projectPlaceholder}
                className="min-w-0 flex-1"
              />
              <Button type="button" variant="outline" className="shrink-0 cursor-pointer" onClick={() => void browse()}>
                <FolderOpen aria-hidden="true" />
                {d.browse}
              </Button>
            </div>
          </Field>

          <Field label={d.model} htmlFor="chat-model">
            <Input
              id="chat-model"
              value={model}
              placeholder={d.modelPlaceholder}
              onChange={(e) => setModels((m) => ({ ...m, [agent]: e.target.value }))}
              className="font-mono"
            />
            <div className="flex flex-wrap gap-1.5">
              {MODEL_SUGGESTIONS[agent].map((m) => (
                <button
                  key={m}
                  type="button"
                  onClick={() => setModels((x) => ({ ...x, [agent]: m }))}
                  className={cn(
                    "h-6 cursor-pointer rounded-full border px-2.5 font-mono text-[11px] outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
                    model === m ? "seg-active border-transparent" : "border-border text-muted-foreground hover:text-foreground",
                  )}
                >
                  {m}
                </button>
              ))}
            </div>
          </Field>

          <Field label={d.permission} hint={c.permissionHints[permission]}>
            <SimpleSelect
              aria-label={d.permission}
              value={permission}
              onValueChange={(v) => setPerms((p) => ({ ...p, [agent]: v as PermissionMode }))}
              options={MODES.map((m) => ({ value: m, label: c.permissions[m] }))}
            />
          </Field>
          {permission === "bypass" ? (
            <p role="alert" className="-mt-2 flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/8 px-3 py-2 text-xs leading-relaxed text-destructive">
              <TriangleAlert className="mt-px size-3.5 shrink-0" aria-hidden="true" />
              <span>
                {c.bypassWarning}
                {agent === "codex" ? ` ${c.bypassCodex}` : ""}
              </span>
            </p>
          ) : null}

          <div className="flex min-w-0 flex-col gap-2">
            <button
              type="button"
              aria-expanded={advanced}
              onClick={() => setAdvanced((v) => !v)}
              className="inline-flex w-fit cursor-pointer items-center gap-1 rounded text-xs font-medium text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            >
              <ChevronRight className={cn("size-3.5 transition-transform duration-150 motion-reduce:transition-none", advanced && "rotate-90")} aria-hidden="true" />
              {d.advanced}
            </button>
            {advanced ? (
              <Field label={d.extraArgs} htmlFor="chat-extra" hint={agent === "claude" ? d.extraClaudeHint : d.extraCodexHint}>
                <Textarea
                  id="chat-extra"
                  value={extra}
                  rows={agent === "claude" ? 2 : 3}
                  placeholder={agent === "claude" ? d.extraClaudePlaceholder : d.extraCodexPlaceholder}
                  onChange={(e) => setExtra(e.target.value)}
                  className="max-h-40 font-mono text-xs"
                />
              </Field>
            ) : null}
          </div>

          <Field label={d.prompt} htmlFor="chat-prompt">
            <Textarea
              id="chat-prompt"
              value={prompt}
              rows={3}
              placeholder={d.promptPlaceholder}
              onChange={(e) => setPrompt(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  void start();
                }
              }}
              className="max-h-56"
            />
          </Field>

          {error ? (
            <p role="alert" className="text-sm text-destructive">
              {fmt(d.startFailed, { error })}
            </p>
          ) : null}
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" className="cursor-pointer" disabled={starting} onClick={() => onOpenChange(false)}>
            {d.cancel}
          </Button>
          <Button type="button" className="cursor-pointer" disabled={!cwd || starting} onClick={() => void start()}>
            {starting ? <Loader2 className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : null}
            {starting ? d.starting : d.start}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function baseName(p: string): string {
  const parts = p.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

function ProjectOption({ name, path }: { name: string; path: string }) {
  return (
    <span className="flex min-w-0 items-baseline gap-2">
      <span className="truncate font-medium">{name}</span>
      <span className="truncate font-mono text-[11px] text-muted-foreground">{path}</span>
    </span>
  );
}
