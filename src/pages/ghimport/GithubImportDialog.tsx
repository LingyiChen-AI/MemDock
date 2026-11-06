import * as React from "react";
import { BookText, CircleCheck, FolderGit2, Loader2, Puzzle } from "lucide-react";
import { ghImportMemories, ghImportPreview, ghImportScan, ghImportSkill } from "@/api/ghimport";
import type {
  GhImportItem,
  GhImportMemoriesResult,
  GhImportPreview,
  GhImportScan,
  GhImportTarget,
  InstallTarget,
  Project,
  Tool,
} from "@/api/types";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { SimpleSelect, type SimpleSelectOption } from "@/components/ui/select";
import { Input } from "@/components/ui/input";
import type { Messages } from "@/i18n";
import { useI18n } from "@/i18n/I18nProvider";
import { cn } from "@/lib/utils";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { fmt } from "@/pages/memory/paths";
import { FlowDialog, InlineError } from "@/pages/skills/flowParts";
import { asCommandError, existingDirs } from "@/pages/skills/skillUtils";

export interface GithubImportDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  projects: Project[];
  /** Shown after an import that created review suggestions. */
  onGoReview?: () => void;
}

/** Localized text for a failed import command. */
export function ghImportErrorText(t: Messages, e: unknown): string {
  const ce = asCommandError(e);
  const raw = ce?.message ?? (e instanceof Error ? e.message : String(e));
  const map = t.ghImport.errors as Record<string, string>;
  const template = ce && map[ce.code] ? map[ce.code] : map.other;
  return fmt(template, { msg: raw });
}

type PreviewState = { status: "loading" } | { status: "ok"; data: GhImportPreview } | { status: "error"; message: string };

interface Result {
  memories: GhImportMemoriesResult | null;
  installed: string[];
  skipped: string[];
  failures: string[];
}

const AUTO = "auto";

/** Target of one memory file for a select value (`auto`, `global_claude`, `pc:<id>` ...). */
export function targetFor(value: string, item: GhImportItem): GhImportTarget {
  if (value === "global_claude") return { type: "global_claude" };
  if (value === "global_agents") return { type: "global_agents" };
  if (value.startsWith("pc:")) return { type: "project_claude", project_id: value.slice(3) };
  if (value.startsWith("pa:")) return { type: "project_agents", project_id: value.slice(3) };
  return item.type === "memory" && item.memory_kind === "agents_md" ? { type: "global_agents" } : { type: "global_claude" };
}

function formatSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export function GithubImportDialog({ open, onOpenChange, projects, onGoReview }: GithubImportDialogProps) {
  const { t } = useI18n();
  const g = t.ghImport;
  const [input, setInput] = React.useState("");
  const [scanning, setScanning] = React.useState(false);
  const [scanError, setScanError] = React.useState<string | null>(null);
  const [scan, setScan] = React.useState<GhImportScan | null>(null);
  const [picked, setPicked] = React.useState<ReadonlySet<string>>(new Set());
  const [focus, setFocus] = React.useState<string | null>(null);
  const [previews, setPreviews] = React.useState<Record<string, PreviewState>>({});
  const previewsRef = React.useRef(previews);
  previewsRef.current = previews;
  const [memTarget, setMemTarget] = React.useState(AUTO);
  const [tools, setTools] = React.useState<Tool[]>(["claude"]);
  const [scope, setScope] = React.useState("global");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [result, setResult] = React.useState<Result | null>(null);
  const [ask, setAsk] = React.useState<{ name: string; dirs: string[] } | null>(null);
  const answer = React.useRef<((yes: boolean) => void) | null>(null);
  const seq = React.useRef(0);

  React.useEffect(() => {
    seq.current += 1;
    if (!open) return;
    setInput("");
    setScanning(false);
    setScanError(null);
    setScan(null);
    setPicked(new Set());
    setFocus(null);
    setPreviews({});
    setMemTarget(AUTO);
    setTools(["claude"]);
    setScope("global");
    setBusy(false);
    setError(null);
    setResult(null);
    setAsk(null);
  }, [open]);

  const usable = projects.filter((p) => p.exists);
  const items = scan?.items ?? [];
  const memories = items.filter((i) => i.type === "memory");
  const skills = items.filter((i) => i.type === "skill");
  const chosen = items.filter((i) => picked.has(i.id));
  const chosenMem = chosen.filter((i) => i.type === "memory");
  const chosenSkills = chosen.filter((i) => i.type === "skill");
  const skillName = (i: GhImportItem) => i.name || scan?.repo.split("/")[1] || "skill";
  const problem = chosenSkills.length > 0 && tools.length === 0 ? g.needTool : null;
  const canImport = !!scan && chosen.length > 0 && !problem && !busy;

  const doScan = async () => {
    const value = input.trim();
    if (!value || scanning) return;
    const mine = ++seq.current;
    setScanning(true);
    setScanError(null);
    setScan(null);
    setPicked(new Set());
    setFocus(null);
    setPreviews({});
    setError(null);
    try {
      const r = await ghImportScan(value);
      if (mine !== seq.current) return;
      setScan(r);
      // Pre-select everything importable when there is little to choose from.
      const ok = r.items.filter((i) => !i.too_large);
      if (ok.length > 0 && ok.length <= 5) setPicked(new Set(ok.map((i) => i.id)));
      const first = ok[0];
      if (first) void loadPreview(r.session_id, first.id, mine);
    } catch (e) {
      if (mine === seq.current) setScanError(ghImportErrorText(t, e));
    } finally {
      if (mine === seq.current) setScanning(false);
    }
  };

  const loadPreview = async (sessionId: string, id: string, mine = seq.current) => {
    setFocus(id);
    const known = previewsRef.current[id];
    if (known && known.status !== "error") return;
    previewsRef.current = { ...previewsRef.current, [id]: { status: "loading" } };
    setPreviews((p) => ({ ...p, [id]: { status: "loading" } }));
    try {
      const data = await ghImportPreview(sessionId, id);
      if (mine === seq.current) setPreviews((p) => ({ ...p, [id]: { status: "ok", data } }));
    } catch (e) {
      if (mine === seq.current) setPreviews((p) => ({ ...p, [id]: { status: "error", message: ghImportErrorText(t, e) } }));
    }
  };

  const toggle = (id: string, on: boolean) =>
    setPicked((prev) => {
      const next = new Set(prev);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });

  const confirmOverwrite = (name: string, dirs: string[]) =>
    new Promise<boolean>((resolve) => {
      answer.current = resolve;
      setAsk({ name, dirs });
    });
  const reply = (yes: boolean) => {
    setAsk(null);
    const r = answer.current;
    answer.current = null;
    r?.(yes);
  };

  const doImport = async () => {
    if (!scan || !canImport) return;
    setBusy(true);
    setError(null);
    const res: Result = { memories: null, installed: [], skipped: [], failures: [] };
    try {
      if (chosenMem.length > 0) {
        res.memories = await ghImportMemories(
          scan.session_id,
          chosenMem.map((i) => ({ item_id: i.id, target: targetFor(memTarget, i) })),
        );
      }
      const scopeValue: InstallTarget["scope"] = scope === "global" ? { type: "global" } : { type: "project", project_id: scope };
      const targets: InstallTarget[] = tools.map((tool) => ({ tool, scope: scopeValue }));
      for (const item of chosenSkills) {
        const name = skillName(item);
        try {
          const done = await ghImportSkill(scan.session_id, item.id, targets, false);
          res.installed.push(done[0]?.name ?? name);
        } catch (e) {
          if (asCommandError(e)?.code === "exists" && (await confirmOverwrite(name, existingDirs(e)))) {
            try {
              const done = await ghImportSkill(scan.session_id, item.id, targets, true);
              res.installed.push(done[0]?.name ?? name);
            } catch (e2) {
              res.failures.push(fmt(g.skillFailed, { name, msg: ghImportErrorText(t, e2) }));
            }
          } else if (asCommandError(e)?.code === "exists") {
            res.skipped.push(name);
          } else {
            res.failures.push(fmt(g.skillFailed, { name, msg: ghImportErrorText(t, e) }));
          }
        }
      }
      setResult(res);
    } catch (e) {
      setError(ghImportErrorText(t, e));
    } finally {
      setBusy(false);
    }
  };

  const memOptions: SimpleSelectOption[] = [
    { value: AUTO, label: g.targetAuto },
    { value: "global_claude", label: g.targetGlobalClaude },
    { value: "global_agents", label: g.targetGlobalAgents },
    ...usable.flatMap((p) => [
      { value: `pc:${p.id}`, label: fmt(g.targetProject, { project: p.name, file: "CLAUDE.md" }) },
      { value: `pa:${p.id}`, label: fmt(g.targetProject, { project: p.name, file: "AGENTS.md" }) },
    ]),
  ];
  const scopeOptions: SimpleSelectOption[] = [{ value: "global", label: g.scopeGlobal }, ...usable.map((p) => ({ value: p.id, label: p.name }))];

  const row = (i: GhImportItem) => {
    const sub = i.type === "memory" ? g.kinds[i.memory_kind] : fmt(g.files, { n: i.files });
    const label = i.type === "memory" ? i.path : i.path || g.rootSkill;
    return (
      <li key={i.id} className={cn("flex items-center gap-2.5 border-b border-border px-3 py-1.5 last:border-b-0", focus === i.id && "bg-muted/70")}>
        <Checkbox
          checked={picked.has(i.id)}
          disabled={i.too_large || busy}
          onCheckedChange={(v) => toggle(i.id, v === true)}
          aria-label={i.type === "skill" ? skillName(i) : i.path}
        />
        <button
          type="button"
          onClick={() => scan && void loadPreview(scan.session_id, i.id)}
          aria-pressed={focus === i.id}
          className="flex min-w-0 flex-1 cursor-pointer flex-col items-start rounded-sm py-0.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <span className="w-full truncate font-mono text-[12.5px] font-medium" title={label}>
            {i.type === "skill" ? skillName(i) : label}
          </span>
          <span className="w-full truncate text-xs text-muted-foreground">
            {i.type === "skill" ? `${label} · ` : ""}
            {sub} · {formatSize(i.size)}
            {i.too_large ? <span className="text-warning"> · {g.tooLarge}</span> : null}
          </span>
        </button>
      </li>
    );
  };

  const setAll = (list: GhImportItem[], on: boolean) =>
    setPicked((prev) => {
      const next = new Set(prev);
      for (const i of list) if (!i.too_large) (on ? next.add(i.id) : next.delete(i.id));
      return next;
    });

  const section = (title: string, Icon: typeof BookText, list: GhImportItem[]) =>
    list.length === 0 ? null : (
      <div className="flex flex-col gap-1.5">
        <div className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
          <Icon className="size-3.5" aria-hidden="true" />
          <span className="flex-1">
            {title} ({list.length})
          </span>
          <label className="inline-flex cursor-pointer items-center gap-1.5">
            <Checkbox
              checked={list.filter((i) => !i.too_large).every((i) => picked.has(i.id)) && list.some((i) => !i.too_large)}
              disabled={busy || list.every((i) => i.too_large)}
              onCheckedChange={(v) => setAll(list, v === true)}
              aria-label={`${g.selectAll} ${title}`}
            />
            {g.selectAll}
          </label>
        </div>
        <ul aria-label={title} className="flex flex-col rounded-md border border-border">
          {list.map(row)}
        </ul>
      </div>
    );

  const pv = focus ? previews[focus] : undefined;
  const focusItem = items.find((i) => i.id === focus);

  const done = result ? (
    <div className="mt-4 flex flex-col gap-3">
      <p className="flex items-center gap-2 text-sm font-medium">
        <CircleCheck className="size-4 text-primary" aria-hidden="true" />
        {g.doneTitle}
      </p>
      <ul className="flex list-disc flex-col gap-1 pl-6 text-sm">
        {result.memories && result.memories.added > 0 ? <li>{fmt(g.doneMemories, { n: result.memories.added })}</li> : null}
        {result.memories && result.memories.duplicates > 0 ? <li>{fmt(g.doneDuplicates, { n: result.memories.duplicates })}</li> : null}
        {result.memories && result.memories.skipped.length > 0 ? <li>{fmt(g.doneSkipped, { n: result.memories.skipped.length })}</li> : null}
        {result.installed.length > 0 ? <li>{fmt(g.doneSkills, { names: result.installed.join(", ") })}</li> : null}
        {result.skipped.map((n) => (
          <li key={`s-${n}`}>{fmt(g.skillSkipped, { name: n })}</li>
        ))}
        {result.failures.map((f) => (
          <li key={f} className="text-destructive">
            {f}
          </li>
        ))}
      </ul>
      <div className="flex justify-end gap-2">
        {result.memories && result.memories.added > 0 && onGoReview ? (
          <Button
            type="button"
            onClick={() => {
              onOpenChange(false);
              onGoReview();
            }}
            className="h-8 cursor-pointer px-3 text-sm font-semibold"
          >
            {g.goReview}
          </Button>
        ) : null}
        <Button type="button" variant="outline" onClick={() => onOpenChange(false)} className="h-8 cursor-pointer bg-card px-3 text-sm hover:bg-muted">
          {g.close}
        </Button>
      </div>
    </div>
  ) : null;

  return (
    <>
      <FlowDialog open={open} onOpenChange={(o) => (busy && !o ? undefined : onOpenChange(o))} title={g.title} description={g.desc} width={scan ? 900 : 600}>
        {done ?? (
          <div className="mt-4 flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto pr-1">
            <form
              className="flex items-end gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                void doScan();
              }}
            >
              <div className="flex min-w-0 flex-1 flex-col gap-1.5">
                <label htmlFor="gh-import-input" className="text-xs font-medium text-muted-foreground">
                  {g.linkLabel}
                </label>
                <Input
                  id="gh-import-input"
                  value={input}
                  onChange={(e) => setInput(e.target.value)}
                  placeholder={g.linkPlaceholder}
                  autoComplete="off"
                  spellCheck={false}
                  disabled={busy}
                  className="h-8 font-mono text-[12.5px]"
                />
              </div>
              <Button type="submit" variant="outline" disabled={!input.trim() || scanning || busy} className="h-8 cursor-pointer gap-1.5 bg-card px-3 text-xs hover:bg-muted">
                {scanning ? <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <FolderGit2 className="size-4" aria-hidden="true" />}
                {scanning ? g.scanning : g.scan}
              </Button>
            </form>
            {scanError ? <InlineError message={scanError} /> : null}

            {scan ? (
              <>
                <p className="text-xs text-muted-foreground">
                  <span className="font-mono">{fmt(g.source, { repo: scan.repo, ref: scan.git_ref, commit: scan.commit.slice(0, 7) })}</span>
                  {scan.scope ? <span className="font-mono"> · {scan.scope}</span> : null}
                </p>
                {!scan.authenticated ? <p className="rounded-md bg-muted px-3 py-2 text-xs text-muted-foreground">{g.loginHint}</p> : null}
                {scan.truncated ? <p className="text-xs text-warning">{fmt(g.truncated, { n: items.length })}</p> : null}
                {scan.tree_truncated ? <p className="text-xs text-warning">{g.treeTruncated}</p> : null}

                {items.length === 0 ? (
                  <p className="py-6 text-center text-sm text-muted-foreground">{g.nothing}</p>
                ) : (
                  <div className="grid min-h-0 grid-cols-[minmax(0,1fr)_minmax(0,1.15fr)] gap-4">
                    <div className="flex max-h-[44vh] min-h-0 flex-col gap-3 overflow-y-auto pr-1">
                      {section(g.memories, BookText, memories)}
                      {section(g.skills, Puzzle, skills)}
                    </div>
                    <section aria-label={g.preview} className="flex max-h-[44vh] min-h-48 flex-col overflow-hidden rounded-md border border-border bg-card">
                      <div className="truncate border-b border-border px-3 py-1.5 font-mono text-xs text-muted-foreground" title={focusItem?.path}>
                        {focusItem ? (focusItem.type === "skill" ? `${focusItem.path || g.rootSkill}/SKILL.md` : focusItem.path) : g.preview}
                      </div>
                      <div className="min-h-0 flex-1 overflow-auto p-3">
                        {!pv ? (
                          <p className="text-sm text-muted-foreground">{g.previewHint}</p>
                        ) : pv.status === "loading" ? (
                          <p className="flex items-center gap-2 text-sm text-muted-foreground">
                            <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />
                            {g.previewLoading}
                          </p>
                        ) : pv.status === "error" ? (
                          <p className="text-sm text-destructive">{fmt(g.previewFailed, { msg: pv.message })}</p>
                        ) : pv.data.binary ? (
                          <p className="text-sm text-muted-foreground">{g.previewBinary}</p>
                        ) : (
                          <>
                            <pre data-testid="gh-preview" className="font-content text-xs break-words whitespace-pre-wrap">
                              {pv.data.text}
                            </pre>
                            {pv.data.files.length > 0 ? (
                              <div className="mt-3 border-t border-border pt-2">
                                <p className="text-xs font-medium text-muted-foreground">{fmt(g.skillFiles, { n: pv.data.files.length })}</p>
                                <ul className="mt-1 font-mono text-xs text-muted-foreground">
                                  {pv.data.files.slice(0, 50).map((f) => (
                                    <li key={f.path} className="truncate">
                                      {f.path} · {formatSize(f.size)}
                                    </li>
                                  ))}
                                </ul>
                              </div>
                            ) : null}
                          </>
                        )}
                      </div>
                    </section>
                  </div>
                )}

                {chosenMem.length > 0 ? (
                  <div className="flex flex-col gap-1.5">
                    <div className="flex items-center gap-3">
                      <span className="w-24 shrink-0 text-xs font-medium text-muted-foreground">{g.memoryTarget}</span>
                      <SimpleSelect value={memTarget} onValueChange={setMemTarget} options={memOptions} disabled={busy} aria-label={g.memoryTarget} className="flex-1" />
                    </div>
                    <p className="text-xs text-muted-foreground">{g.memoryNote}</p>
                  </div>
                ) : null}
                {chosenSkills.length > 0 ? (
                  <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
                    <span className="w-24 shrink-0 text-xs font-medium text-muted-foreground">{g.skillTarget}</span>
                    {(["claude", "codex"] as const).map((tool) => (
                      <label key={tool} className="inline-flex cursor-pointer items-center gap-2 text-sm">
                        <Checkbox
                          checked={tools.includes(tool)}
                          disabled={busy}
                          onCheckedChange={(v) => setTools((l) => (v === true ? [...l.filter((x) => x !== tool), tool] : l.filter((x) => x !== tool)))}
                        />
                        {tool === "claude" ? t.memory.toolClaude : t.memory.toolCodex}
                      </label>
                    ))}
                    <SimpleSelect value={scope} onValueChange={setScope} options={scopeOptions} disabled={busy} aria-label={g.skillScope} className="min-w-40" />
                    {problem ? <span className="text-xs text-warning">{problem}</span> : null}
                  </div>
                ) : null}
                {error ? <InlineError message={error} /> : null}
                <div className="flex justify-end gap-2">
                  <Button type="button" variant="outline" disabled={busy} onClick={() => onOpenChange(false)} className="h-8 cursor-pointer bg-card px-3 text-sm hover:bg-muted">
                    {t.review.cancel}
                  </Button>
                  <Button type="button" disabled={!canImport} onClick={() => void doImport()} className="h-8 cursor-pointer gap-1.5 px-3 text-sm font-semibold">
                    {busy ? <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" /> : null}
                    {busy ? g.importing : fmt(g.import, { n: chosen.length })}
                  </Button>
                </div>
              </>
            ) : null}
          </div>
        )}
      </FlowDialog>
      <ConfirmDialog
        open={ask !== null}
        title={g.overwriteTitle}
        body={ask ? fmt(g.overwriteBody, { name: ask.name, dirs: ask.dirs.join(", ") }) : ""}
        confirmLabel={g.overwrite}
        cancelLabel={g.skip}
        destructive
        onConfirm={() => reply(true)}
        onCancel={() => reply(false)}
      />
    </>
  );
}
