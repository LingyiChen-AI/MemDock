import * as React from "react";
import { openPath } from "@tauri-apps/plugin-opener";
import { Bot, Eye, Folder, FolderOpen, GitMerge, Loader2, MoreHorizontal, SquareTerminal, Trash2, TriangleAlert } from "lucide-react";
import { worktreeDiff, worktreeMerge, worktreeRemove, type ChangedFile, type MergeMode, type MergeResult, type WorktreeInfo } from "@/api/worktree";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import type { Messages } from "@/i18n";
import { fmt } from "@/pages/memory/paths";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { errorMessage } from "@/pages/skills/skillUtils";
import { markWorktreeRemoved, refreshWorktreeStatus, useStatusRefreshOnTurnEnd, useWorktreeStatus } from "./worktreeStore";

type C = Messages["chat"];
type W = C["worktree"];

/** `🌿 <slug>` badge of a worktree session (muted once the worktree is deleted). */
export function WorktreeBadge({ slug, branch, deleted, w }: { slug: string; branch?: string; deleted?: boolean; w: W }) {
  if (deleted) {
    return (
      <span title={`${slug} · ${w.deletedHint}`} className="inline-flex h-5 shrink-0 items-center gap-1 rounded bg-muted px-1.5 text-[11px] font-medium text-muted-foreground">
        🌿 {w.deleted}
      </span>
    );
  }
  return (
    <span
      title={branch ? fmt(w.badgeHint, { branch }) : slug}
      className="inline-flex h-5 max-w-full min-w-0 items-center gap-1 rounded bg-violet-500/12 px-1.5 text-[11px] font-medium text-violet-700 dark:text-violet-300"
    >
      🌿 <span className="truncate">{slug}</span>
    </span>
  );
}

function totals(files: ChangedFile[]) {
  return files.reduce((a, f) => ({ added: a.added + f.added, deleted: a.deleted + f.deleted }), { added: 0, deleted: 0 });
}

const STATUS_TONE: Record<string, string> = {
  A: "text-emerald-700 dark:text-emerald-400",
  M: "text-amber-700 dark:text-amber-400",
  D: "text-destructive",
  R: "text-sky-700 dark:text-sky-400",
};

/** A unified diff, monospace, with added / removed lines coloured. */
export function UnifiedDiff({ text }: { text: string }) {
  const lines = text.replace(/\n$/, "").split("\n");
  return (
    <pre aria-label="diff" className="min-h-0 flex-1 overflow-auto rounded-md border border-border bg-muted/30 py-2 font-mono text-[12px] leading-5">
      {lines.map((l, i) => {
        const meta = l.startsWith("diff ") || l.startsWith("index ") || l.startsWith("+++") || l.startsWith("---") || l.startsWith("new file") || l.startsWith("deleted file");
        const tone = meta
          ? "text-muted-foreground"
          : l.startsWith("@@")
            ? "bg-sky-500/8 text-sky-700 dark:text-sky-300"
            : l.startsWith("+")
              ? "bg-emerald-500/12 text-emerald-800 dark:text-emerald-300"
              : l.startsWith("-")
                ? "bg-red-500/12 text-red-800 dark:text-red-300"
                : "";
        return (
          <div key={i} data-kind={meta ? "meta" : l[0] === "+" ? "add" : l[0] === "-" ? "del" : "ctx"} className={cn("px-3 whitespace-pre", tone)}>
            {l || " "}
          </div>
        );
      })}
    </pre>
  );
}

function ChangesDialog({ info, files, open, onClose, w }: { info: WorktreeInfo; files: ChangedFile[]; open: boolean; onClose: () => void; w: W }) {
  const [file, setFile] = React.useState<string | null>(null);
  const [diff, setDiff] = React.useState<{ file: string; text: string | null; error: string | null } | null>(null);
  const current = file ?? files[0]?.path ?? null;
  React.useEffect(() => {
    if (!open) return;
    void refreshWorktreeStatus(info.path);
  }, [open, info.path]);
  React.useEffect(() => {
    if (!open || !current) return;
    let alive = true;
    setDiff({ file: current, text: null, error: null });
    worktreeDiff(info.path, current)
      .then((text) => alive && setDiff({ file: current, text, error: null }))
      .catch((e) => alive && setDiff({ file: current, text: null, error: errorMessage(e) }));
    return () => {
      alive = false;
    };
  }, [open, current, info.path]);
  return (
    <Dialog open={open} onOpenChange={(o) => (!o ? onClose() : undefined)}>
      <DialogContent className="flex h-[80vh] w-[960px] flex-col gap-3" closeLabel={w.dismiss}>
        <DialogHeader>
          <DialogTitle className="font-mono">{fmt(w.changesTitle, { branch: info.branch })}</DialogTitle>
          <DialogDescription className="font-mono text-xs">{info.path}</DialogDescription>
        </DialogHeader>
        {files.length === 0 ? (
          <p className="text-sm text-muted-foreground">{w.changesEmpty}</p>
        ) : (
          <div className="flex min-h-0 flex-1 gap-3">
            <ul aria-label={w.changesFiles} className="flex w-64 shrink-0 flex-col gap-0.5 overflow-y-auto">
              {files.map((f) => (
                <li key={f.path}>
                  <button
                    type="button"
                    aria-current={f.path === current ? "true" : undefined}
                    onClick={() => setFile(f.path)}
                    className={cn(
                      "flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1 text-left font-mono text-[12px] outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring",
                      f.path === current && "list-active",
                    )}
                  >
                    <span className={cn("w-3 shrink-0 font-semibold", STATUS_TONE[f.status])}>{f.status}</span>
                    <span className="min-w-0 flex-1 truncate" title={f.path}>
                      {f.path}
                    </span>
                    <span className="shrink-0 text-[11px] text-emerald-700 dark:text-emerald-400">+{f.added}</span>
                    <span className="shrink-0 text-[11px] text-red-700 dark:text-red-400">−{f.deleted}</span>
                  </button>
                </li>
              ))}
            </ul>
            <div className="flex min-h-0 min-w-0 flex-1 flex-col">
              {!diff || diff.file !== current || (diff.text === null && !diff.error) ? (
                <p className="flex items-center gap-2 text-sm text-muted-foreground">
                  <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />
                  {w.diffLoading}
                </p>
              ) : diff.error ? (
                <p role="alert" className="text-sm text-destructive">
                  {fmt(w.diffFailed, { error: diff.error })}
                </p>
              ) : !diff.text?.trim() ? (
                <p className="text-sm text-muted-foreground">{w.diffEmpty}</p>
              ) : (
                <UnifiedDiff text={diff.text} />
              )}
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

function MergeDialog({
  info,
  title,
  dirty,
  running,
  open,
  onClose,
  onMerge,
  w,
}: {
  info: WorktreeInfo;
  title: string;
  dirty: boolean;
  running: boolean;
  open: boolean;
  onClose: () => void;
  onMerge: (mode: MergeMode, message: string, deleteAfter: boolean) => Promise<void>;
  w: W;
}) {
  const [mode, setMode] = React.useState<MergeMode>("squash");
  const [message, setMessage] = React.useState(title);
  const [deleteAfter, setDeleteAfter] = React.useState(true);
  const [busy, setBusy] = React.useState(false);
  React.useEffect(() => {
    if (!open) return;
    setMode("squash");
    setMessage(title);
    setDeleteAfter(true);
  }, [open, title]);
  const options: { value: MergeMode; label: string; hint: string }[] = [
    { value: "squash", label: w.modeSquash, hint: fmt(w.modeSquashHint, { base: info.base_branch }) },
    { value: "merge", label: w.modeMerge, hint: w.modeMergeHint },
    { value: "keep", label: w.modeKeep, hint: w.modeKeepHint },
  ];
  const submit = async () => {
    setBusy(true);
    try {
      await onMerge(mode, message.trim() || title, deleteAfter);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog open={open} onOpenChange={(o) => (!o && !busy ? onClose() : undefined)}>
      <DialogContent className="flex flex-col gap-4" closeLabel={w.dismiss}>
        <DialogHeader>
          <DialogTitle>{fmt(w.mergeTitle, { branch: info.branch, base: info.base_branch })}</DialogTitle>
          <DialogDescription className="text-xs">{w.depsNote}</DialogDescription>
        </DialogHeader>
        <RadioGroup aria-label={w.modeLabel} value={mode} onValueChange={(v) => setMode(v as MergeMode)} className="gap-1">
          {options.map((o) => (
            <label
              key={o.value}
              className={cn(
                "flex cursor-pointer items-start gap-2.5 rounded-md border border-transparent px-2.5 py-2 hover:bg-muted/60",
                mode === o.value && "border-violet-500/40 bg-violet-500/6",
              )}
            >
              <RadioGroupItem value={o.value} aria-label={o.label} className="mt-0.5" />
              <span className="flex min-w-0 flex-col gap-0.5">
                <span className="flex items-center gap-1.5 text-sm font-medium">
                  {o.label}
                  {o.value === "squash" ? <span className="rounded bg-violet-500/12 px-1 text-[10px] text-violet-700 dark:text-violet-300">{w.recommended}</span> : null}
                </span>
                <span className="text-xs text-muted-foreground">{o.hint}</span>
              </span>
            </label>
          ))}
        </RadioGroup>
        {mode === "squash" ? (
          <label className="flex flex-col gap-1 text-xs">
            <span className="font-medium">{w.message}</span>
            <Input aria-label={w.message} value={message} maxLength={200} onChange={(e) => setMessage(e.target.value)} />
          </label>
        ) : null}
        {dirty ? (
          <p className="flex items-start gap-1.5 text-xs text-amber-700 dark:text-amber-300">
            <TriangleAlert className="mt-px size-3.5 shrink-0" aria-hidden="true" />
            {fmt(w.dirtyNote, { title })}
          </p>
        ) : null}
        <label className="flex cursor-pointer items-center gap-2 text-sm">
          <Checkbox checked={deleteAfter} onCheckedChange={(v) => setDeleteAfter(v === true)} aria-label={mode === "keep" ? w.deleteKeep : w.deleteAfter} />
          {mode === "keep" ? w.deleteKeep : w.deleteAfter}
        </label>
        <DialogFooter className="mt-0 items-center">
          {running ? <span className="mr-auto text-xs text-amber-700 dark:text-amber-300">{w.mergeRunning}</span> : null}
          <Button type="button" variant="outline" size="sm" className="cursor-pointer" disabled={busy} onClick={onClose}>
            {w.cancel}
          </Button>
          <Button type="button" size="sm" className="cursor-pointer bg-violet-600 text-white hover:bg-violet-600/90" disabled={busy || running} onClick={() => void submit()}>
            {busy ? <Loader2 className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <GitMerge aria-hidden="true" />}
            {busy ? w.merging : mode === "keep" ? w.keepConfirm : w.merge}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export interface WorktreePanelProps {
  info: WorktreeInfo;
  /** Session title: default squash message. */
  title: string;
  /** A turn is running: merging is disabled. */
  running: boolean;
  c: C;
  /** Success line shown by the pane (kept after the panel goes away). */
  onNotice: (text: string) => void;
  onError: (e: string | null) => void;
  /** Stops the session's process / terminal before the folder is removed. */
  beforeRemove: () => Promise<void>;
  /** Opens a shell terminal in the worktree. */
  onOpenShell?: (cwd: string) => void;
  /** Sends the session a message. */
  onAskAi: (text: string) => Promise<void>;
}

/** Violet bar under the session header: branch, changes, view / open / merge / discard. */
export function WorktreePanel({ info, title, running, c, onNotice, onError, beforeRemove, onOpenShell, onAskAi }: WorktreePanelProps) {
  const w = c.worktree;
  const status = useWorktreeStatus(info.path);
  useStatusRefreshOnTurnEnd(info.path, running);
  const [showChanges, setShowChanges] = React.useState(false);
  const [showMerge, setShowMerge] = React.useState(false);
  const [confirmDiscard, setConfirmDiscard] = React.useState(false);
  const [moreOpen, setMoreOpen] = React.useState(false);
  const [conflicts, setConflicts] = React.useState<string[] | null>(null);
  const files = status?.files ?? [];
  const sum = totals(files);
  const fail = (e: unknown) => onError(fmt(w.failed, { error: errorMessage(e) }));

  const merge = async (mode: MergeMode, message: string, deleteAfter: boolean) => {
    onError(null);
    setConflicts(null);
    let r: MergeResult;
    try {
      // Merge first; the session is stopped (and the worktree removed) only once the merge succeeded,
      // so after a conflict it keeps running and can be asked to resolve it.
      r = await worktreeMerge(info.path, mode, message, false);
      if (deleteAfter && r.conflicts.length === 0) {
        await beforeRemove();
        // "keep" keeps the branch; only the checkout goes away.
        await worktreeRemove(info.path, mode !== "keep");
      }
    } catch (e) {
      setShowMerge(false);
      fail(e);
      return;
    }
    setShowMerge(false);
    if (r.conflicts.length > 0) {
      setConflicts(r.conflicts);
      void refreshWorktreeStatus(info.path);
      return;
    }
    const parts = [mode === "keep" ? fmt(w.keptDone, { branch: info.branch }) : fmt(w.mergeDone, { base: info.base_branch }) + (r.commit ? ` (${r.commit.slice(0, 8)})` : "")];
    if (r.auto_committed) parts.push(w.autoCommitted);
    if (deleteAfter) parts.push(w.removedDone);
    onNotice(parts.join("；"));
    if (deleteAfter) markWorktreeRemoved(info.path);
    else void refreshWorktreeStatus(info.path);
  };

  const discard = async () => {
    setConfirmDiscard(false);
    onError(null);
    try {
      await beforeRemove();
      await worktreeRemove(info.path, true);
      markWorktreeRemoved(info.path);
      onNotice(w.removedDone);
    } catch (e) {
      fail(e);
    }
  };

  const askAi = async () => {
    if (!conflicts) return;
    onError(null);
    try {
      await onAskAi(fmt(w.aiPrompt, { base: info.base_branch, files: conflicts.join("、") }));
      setConflicts(null);
    } catch (e) {
      fail(e);
    }
  };

  const mergeLabel = fmt(w.mergeTo, { base: info.base_branch });
  return (
    <section aria-label={w.panelLabel} className="flex flex-col gap-2 border-b border-violet-500/25 bg-violet-500/6 px-4 py-2.5">
      <div className="flex min-w-0 flex-wrap items-center gap-2 text-xs">
        <span className="font-mono font-semibold text-violet-700 dark:text-violet-300">
          🌿 {info.branch} ← {info.base_branch}
        </span>
        <span title={info.path} className="inline-flex h-6 max-w-full min-w-0 items-center gap-1 rounded-md bg-violet-500/10 px-2 font-mono text-[11px] text-violet-800/80 dark:text-violet-200/80">
          <Folder className="size-3 shrink-0" aria-hidden="true" />
          <span className="truncate">{info.path}</span>
        </span>
      </div>
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <span data-testid="worktree-summary" className="min-w-0 flex-1 text-xs text-muted-foreground">
          {status === null ? "…" : files.length === 0 && status.ahead === 0 ? w.noChanges : fmt(w.summary, { files: files.length, added: sum.added, deleted: sum.deleted, commits: status.ahead })}
        </span>
        <Button type="button" variant="outline" size="sm" className="cursor-pointer" onClick={() => setShowChanges(true)}>
          <Eye aria-hidden="true" />
          {w.viewChanges}
        </Button>
        <Button type="button" variant="outline" size="sm" className="cursor-pointer" onClick={() => void openPath(info.path).catch(fail)}>
          <FolderOpen aria-hidden="true" />
          {w.openFolder}
        </Button>
        <Tooltip>
          <TooltipTrigger asChild>
            <span tabIndex={running ? 0 : -1}>
              <Button
                type="button"
                size="sm"
                className="cursor-pointer bg-violet-600 text-white hover:bg-violet-600/90"
                disabled={running}
                onClick={() => setShowMerge(true)}
              >
                <GitMerge aria-hidden="true" />
                {mergeLabel}
              </Button>
            </span>
          </TooltipTrigger>
          {running ? <TooltipContent side="bottom">{w.mergeRunning}</TooltipContent> : null}
        </Tooltip>
        <Popover open={moreOpen} onOpenChange={setMoreOpen}>
          <PopoverTrigger asChild>
            <Button type="button" size="icon-sm" variant="ghost" className="cursor-pointer" aria-label={w.more}>
              <MoreHorizontal aria-hidden="true" />
            </Button>
          </PopoverTrigger>
          <PopoverContent align="end" className="w-56 p-1">
            <div role="menu" aria-label={w.more}>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMoreOpen(false);
                  setConfirmDiscard(true);
                }}
                className="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-destructive outline-none hover:bg-destructive/10 focus-visible:bg-destructive/10"
              >
                <Trash2 className="size-4" aria-hidden="true" />
                {w.discard}
              </button>
            </div>
          </PopoverContent>
        </Popover>
      </div>
      {conflicts ? (
        <div role="alert" className="flex flex-col gap-2 rounded-md border border-amber-500/40 bg-amber-500/8 p-3 text-xs">
          <p className="flex items-center gap-1.5 font-semibold text-amber-800 dark:text-amber-200">
            <TriangleAlert className="size-3.5 shrink-0" aria-hidden="true" />
            {w.conflictsTitle}
          </p>
          <p className="text-muted-foreground">{w.conflictsBody}</p>
          <ul aria-label={w.conflictsBody} className="flex flex-col gap-0.5 font-mono">
            {conflicts.map((f) => (
              <li key={f}>{f}</li>
            ))}
          </ul>
          <div className="flex flex-wrap items-center gap-2">
            {onOpenShell ? (
              <Button type="button" size="sm" variant="outline" className="cursor-pointer" onClick={() => onOpenShell(info.path)}>
                <SquareTerminal aria-hidden="true" />
                {w.resolveInTerminal}
              </Button>
            ) : null}
            <Button type="button" size="sm" variant="outline" className="cursor-pointer" disabled={running} onClick={() => void askAi()}>
              <Bot aria-hidden="true" />
              {w.resolveWithAi}
            </Button>
            <Button type="button" size="sm" variant="ghost" className="ml-auto cursor-pointer" onClick={() => setConflicts(null)}>
              {w.dismiss}
            </Button>
          </div>
        </div>
      ) : null}
      <ChangesDialog info={info} files={files} open={showChanges} onClose={() => setShowChanges(false)} w={w} />
      <MergeDialog info={info} title={title} dirty={status?.dirty ?? false} running={running} open={showMerge} onClose={() => setShowMerge(false)} onMerge={merge} w={w} />
      <ConfirmDialog
        open={confirmDiscard}
        title={w.discardTitle}
        body={fmt(w.discardBody, { path: info.path, branch: info.branch })}
        confirmLabel={w.discardConfirm}
        cancelLabel={w.cancel}
        destructive
        onConfirm={() => void discard()}
        onCancel={() => setConfirmDiscard(false)}
      />
    </section>
  );
}
