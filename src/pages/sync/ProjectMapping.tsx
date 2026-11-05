import { Check } from "lucide-react";
import { setIncludedLocal } from "@/api/sync";
import type { SyncProject } from "@/api/types";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type { ToastKind } from "@/pages/skills/Toasts";
import { syncErrorText } from "@/components/account/syncErrors";

export interface ProjectMappingProps {
  projects: SyncProject[] | null;
  onChange: (p: SyncProject[]) => void;
  notify: (kind: ToastKind, message: string) => void;
}

/** Which projects take part: remote-backed ones always, folder-matched ones when switched on. */
export function ProjectMapping({ projects, onChange, notify }: ProjectMappingProps) {
  const { t } = useI18n();
  const s = t.sync;

  const toggle = async (p: SyncProject) => {
    if (!projects) return;
    const keys = projects
      .filter((x) => x.key === null && (x.project_id === p.project_id ? !p.included : x.included))
      .map((x) => x.dir_key);
    try {
      onChange(await setIncludedLocal(keys));
    } catch (e) {
      notify("error", syncErrorText(t, e));
    }
  };

  return (
    <section aria-label={s.projectsTitle} className="rounded-lg border border-border bg-card">
      <div className="px-4 pt-4 pb-2">
        <h2 className="text-sm font-semibold">{s.projectsTitle}</h2>
        <p className="mt-0.5 text-xs text-muted-foreground">{s.projectsBody}</p>
      </div>
      {projects === null ? (
        <p className="px-4 pb-4 text-sm text-muted-foreground">{s.loading}</p>
      ) : projects.length === 0 ? (
        <p className="px-4 pb-4 text-sm text-muted-foreground">{s.noProjects}</p>
      ) : (
        <ul className="divide-y divide-border border-t border-border">
          {projects.map((p) => (
            <li key={p.project_id} className="flex items-center gap-3 px-4 py-2">
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium">{p.name}</p>
                <p className="truncate font-mono text-xs text-muted-foreground" title={p.key ?? p.dir_key}>
                  {p.key ?? `${s.noRemote} · ${p.dir_key}`}
                </p>
              </div>
              {p.key !== null ? (
                <span className="shrink-0 text-xs text-accent-text">{s.included}</span>
              ) : (
                <button
                  type="button"
                  role="switch"
                  aria-checked={p.included}
                  aria-label={`${s.include}: ${p.name}`}
                  onClick={() => void toggle(p)}
                  className="flex shrink-0 cursor-pointer items-center gap-2 rounded-sm text-xs text-muted-foreground transition-colors duration-150 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none"
                >
                  <span
                    className={cn(
                      "grid size-3.5 place-items-center rounded-sm border border-border",
                      p.included && "border-primary bg-primary text-primary-foreground",
                    )}
                  >
                    {p.included ? <Check className="size-3" aria-hidden="true" /> : null}
                  </span>
                  {s.include}
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
