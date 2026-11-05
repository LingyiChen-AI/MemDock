import * as React from "react";
import { Download, Trash2 } from "lucide-react";
import { deleteSkill, installSkill, listSkills } from "@/api/skills";
import type { ManifestSkill, SkillEntry, SkillScope, SyncProject } from "@/api/types";
import { Button } from "@/components/ui/button";
import { useI18n } from "@/i18n/I18nProvider";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { fmt } from "@/pages/memory/paths";
import { errorMessage } from "@/pages/skills/skillUtils";
import type { ToastKind } from "@/pages/skills/Toasts";

export interface MissingSkillsProps {
  missing: ManifestSkill[];
  removed: ManifestSkill[];
  projects: SyncProject[] | null;
  notify: (kind: ToastKind, message: string) => void;
}

const keyOf = (m: ManifestSkill) => `${m.tool}|${m.scope.type === "global" ? "" : m.scope.key}|${m.name}`;

/** Local scope of a manifest entry; `null` when the project is not on this computer. */
function localScope(m: ManifestSkill, projects: SyncProject[] | null): SkillScope | null {
  if (m.scope.type === "global") return { type: "global" };
  const key = m.scope.key;
  const p = projects?.find((x) => x.key === key || x.dir_key === key);
  return p ? { type: "project", project_id: p.project_id } : null;
}

function sameScope(a: SkillScope, b: SkillScope): boolean {
  return a.type === "global" ? b.type === "global" : b.type === "project" && a.project_id === b.project_id;
}

export function MissingSkills({ missing, removed, projects, notify }: MissingSkillsProps) {
  const { t } = useI18n();
  const s = t.sync;
  const [done, setDone] = React.useState<Set<string>>(() => new Set());
  const [busy, setBusy] = React.useState<Set<string>>(() => new Set());
  const [local, setLocal] = React.useState<SkillEntry[] | null>(null);
  const [confirm, setConfirm] = React.useState<{ m: ManifestSkill; skill: SkillEntry } | null>(null);

  React.useEffect(() => {
    if (removed.length === 0) return;
    let alive = true;
    listSkills()
      .then((l) => alive && setLocal(l))
      .catch(() => alive && setLocal([]));
    return () => {
      alive = false;
    };
  }, [removed]);

  const mark = (set: React.Dispatch<React.SetStateAction<Set<string>>>, k: string, on: boolean) =>
    set((prev) => {
      const next = new Set(prev);
      if (on) next.add(k);
      else next.delete(k);
      return next;
    });

  const install = async (m: ManifestSkill) => {
    const scope = localScope(m, projects);
    if (!scope) return;
    const k = keyOf(m);
    mark(setBusy, k, true);
    try {
      await installSkill(
        { type: "git", url: m.url, clone_url: m.url, git_ref: m.git_ref, skill_path: m.skill_path, commit: m.commit },
        [{ tool: m.tool, scope }],
        false,
      );
      mark(setDone, k, true);
      notify("success", fmt(s.installed, { name: m.name }));
    } catch (e) {
      notify("error", errorMessage(e));
    } finally {
      mark(setBusy, k, false);
    }
  };

  const localSkill = (m: ManifestSkill): SkillEntry | null => {
    const scope = localScope(m, projects);
    if (!scope || !local) return null;
    return local.find((x) => x.name === m.name && x.tool === m.tool && sameScope(x.scope, scope)) ?? null;
  };

  const remove = async (m: ManifestSkill, skill: SkillEntry) => {
    const k = keyOf(m);
    mark(setBusy, k, true);
    try {
      await deleteSkill(skill.id);
      mark(setDone, k, true);
      notify("success", fmt(s.deleted, { name: m.name }));
    } catch (e) {
      notify("error", errorMessage(e));
    } finally {
      mark(setBusy, k, false);
    }
  };

  const openMissing = missing.filter((m) => !done.has(keyOf(m)));
  const openRemoved = removed.filter((m) => !done.has(keyOf(m)));
  if (openMissing.length === 0 && openRemoved.length === 0) return null;

  const scopeText = (m: ManifestSkill) => (m.scope.type === "global" ? s.scopeGlobal : fmt(s.scopeProject, { key: m.scope.key }));
  const meta = (m: ManifestSkill) => (
    <p className="truncate text-xs text-muted-foreground" title={m.url}>
      <span className="font-mono">{m.url.replace(/^https?:\/\//, "")}</span>
      {" · "}
      {m.tool === "claude" ? "Claude" : "Codex"}
      {" · "}
      {scopeText(m)}
    </p>
  );

  return (
    <section aria-label={s.skillsTitle} className="rounded-lg border border-border bg-card">
      {openMissing.length > 0 ? (
        <>
          <div className="flex items-center gap-2 px-4 pt-4 pb-2">
            <h2 className="flex-1 text-sm font-semibold">{s.missingTitle}</h2>
            <Button
              type="button"
              variant="outline"
              disabled={openMissing.every((m) => !localScope(m, projects))}
              onClick={() => {
                for (const m of openMissing) if (localScope(m, projects)) void install(m);
              }}
              className="h-7 cursor-pointer px-3 text-xs"
            >
              <Download className="size-3.5" aria-hidden="true" />
              {s.installAll}
            </Button>
          </div>
          <ul className="divide-y divide-border border-t border-border">
            {openMissing.map((m) => {
              const ok = localScope(m, projects) !== null;
              return (
                <li key={keyOf(m)} className={ok ? "flex items-center gap-3 px-4 py-2" : "flex items-center gap-3 px-4 py-2 opacity-60"}>
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium">{m.name}</p>
                    {meta(m)}
                  </div>
                  {ok ? (
                    <Button
                      type="button"
                      variant="outline"
                      disabled={busy.has(keyOf(m))}
                      aria-label={`${s.install} ${m.name}`}
                      onClick={() => void install(m)}
                      className="h-7 shrink-0 cursor-pointer px-3 text-xs"
                    >
                      {s.install}
                    </Button>
                  ) : (
                    <span className="shrink-0 text-xs text-muted-foreground">{s.noLocalProject}</span>
                  )}
                </li>
              );
            })}
          </ul>
        </>
      ) : null}
      {openRemoved.length > 0 ? (
        <>
          <h2 className="px-4 pt-4 pb-2 text-sm font-semibold">{s.removedTitle}</h2>
          <ul className="divide-y divide-border border-t border-border">
            {openRemoved.map((m) => {
              const skill = localSkill(m);
              return (
                <li key={keyOf(m)} className="flex items-center gap-3 px-4 py-2">
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium">{m.name}</p>
                    {meta(m)}
                  </div>
                  {skill ? (
                    <Button
                      type="button"
                      variant="ghost"
                      disabled={busy.has(keyOf(m))}
                      onClick={() => setConfirm({ m, skill })}
                      className="h-7 shrink-0 cursor-pointer px-3 text-xs text-destructive"
                    >
                      <Trash2 className="size-3.5" aria-hidden="true" />
                      {s.deleteToo}
                    </Button>
                  ) : local ? (
                    <span className="shrink-0 text-xs text-muted-foreground">{s.notInstalledHere}</span>
                  ) : null}
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => mark(setDone, keyOf(m), true)}
                    className="h-7 shrink-0 cursor-pointer px-3 text-xs"
                  >
                    {s.keepFile}
                  </Button>
                </li>
              );
            })}
          </ul>
        </>
      ) : null}
      <div className="h-2" />
      <ConfirmDialog
        open={confirm !== null}
        title={fmt(s.deleteSkillTitle, { name: confirm?.m.name ?? "" })}
        body={s.deleteSkillBody}
        confirmLabel={s.deleteToo}
        cancelLabel={s.keep}
        destructive
        onConfirm={() => {
          if (confirm) void remove(confirm.m, confirm.skill);
          setConfirm(null);
        }}
        onCancel={() => setConfirm(null)}
      />
    </section>
  );
}
