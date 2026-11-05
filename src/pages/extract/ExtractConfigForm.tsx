import * as React from "react";
import type { ExtractConfig, ExtractSchedule, Project, RunnerInfo, Tool } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";

const pad = (n: number) => String(n).padStart(2, "0");
const SELECT =
  "h-8 rounded-md border border-input bg-transparent px-2 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring dark:bg-input/30";

function Segmented<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: { value: T; label: React.ReactNode; disabled?: boolean }[];
  onChange: (v: T) => void;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex overflow-hidden rounded-md border border-border">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          disabled={o.disabled}
          onClick={() => onChange(o.value)}
          className={cn(
            "cursor-pointer px-3 text-sm leading-[28px] outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset disabled:cursor-not-allowed disabled:opacity-50 motion-reduce:transition-none",
            value === o.value ? "bg-muted font-medium text-foreground" : "text-muted-foreground hover:text-foreground",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function Row({ label, children, htmlFor }: { label: string; children: React.ReactNode; htmlFor?: string }) {
  return (
    <div className="flex items-start gap-4 py-3">
      <label htmlFor={htmlFor} className="w-28 shrink-0 pt-1 text-sm text-muted-foreground">
        {label}
      </label>
      <div className="min-w-0 flex-1">{children}</div>
    </div>
  );
}

export interface ExtractConfigFormProps {
  config: ExtractConfig;
  runners: RunnerInfo[];
  projects: Project[];
  saving: boolean;
  onSave: (config: ExtractConfig) => void;
}

export function ExtractConfigForm({ config, runners, projects, saving, onSave }: ExtractConfigFormProps) {
  const { t } = useI18n();
  const x = t.extract;
  const [draft, setDraft] = React.useState(config);
  const [lookbackText, setLookbackText] = React.useState(String(config.lookback_days));
  React.useEffect(() => {
    setDraft(config);
    setLookbackText(String(config.lookback_days));
  }, [config]);

  const lookbackOk = /^\d+$/.test(lookbackText) && Number(lookbackText) >= 1 && Number(lookbackText) <= 90;
  const effective: ExtractConfig = { ...draft, lookback_days: lookbackOk ? Number(lookbackText) : draft.lookback_days };
  const dirty = JSON.stringify(effective) !== JSON.stringify(config) || (!lookbackOk && lookbackText !== String(config.lookback_days));

  const set = (patch: Partial<ExtractConfig>) => setDraft((d) => ({ ...d, ...patch }));
  const runner = (tool: Tool) => runners.find((r) => r.tool === tool);
  const toolOptions: { value: "auto" | Tool; label: React.ReactNode; disabled?: boolean }[] = [
    { value: "auto", label: x.toolAuto },
    ...(["claude", "codex"] as const).map((tool) => {
      const ok = runner(tool)?.available ?? false;
      return {
        value: tool,
        disabled: !ok,
        label: (
          <>
            {tool === "claude" ? t.memory.toolClaude : t.memory.toolCodex}
            {!ok ? <span className="ml-1.5 text-xs">{x.notDetected}</span> : null}
          </>
        ),
      };
    }),
  ];

  const sched = draft.schedule;
  const setKind = (kind: ExtractSchedule["kind"]) => {
    if (kind === "off") set({ schedule: { kind: "off" } });
    else if (kind === "daily") set({ schedule: { kind: "daily", hh: sched.kind === "off" ? 9 : sched.hh, mm: sched.kind === "off" ? 0 : sched.mm } });
    else
      set({
        schedule: {
          kind: "weekly",
          weekday: sched.kind === "weekly" ? sched.weekday : 1,
          hh: sched.kind === "off" ? 9 : sched.hh,
          mm: sched.kind === "off" ? 0 : sched.mm,
        },
      });
  };
  const setTime = (v: string) => {
    const m = /^(\d{1,2}):(\d{2})$/.exec(v);
    if (!m || sched.kind === "off") return;
    set({ schedule: { ...sched, hh: Number(m[1]), mm: Number(m[2]) } });
  };

  const selectedIds = draft.projects.kind === "selected" ? draft.projects.ids : [];
  const toggleProject = (id: string) => {
    const ids = selectedIds.includes(id) ? selectedIds.filter((i) => i !== id) : [...selectedIds, id];
    set({ projects: { kind: "selected", ids } });
  };

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        if (dirty && lookbackOk) onSave(effective);
      }}
    >
      <div className="divide-y divide-border">
        <Row label={x.tool}>
          <Segmented
            label={x.tool}
            value={draft.tool ?? "auto"}
            options={toolOptions}
            onChange={(v) => set({ tool: v === "auto" ? null : v })}
          />
        </Row>

        <Row label={x.schedule} htmlFor="extract-schedule">
          <div className="flex flex-wrap items-center gap-2">
            <select id="extract-schedule" value={sched.kind} onChange={(e) => setKind(e.target.value as ExtractSchedule["kind"])} className={SELECT}>
              <option value="off">{x.scheduleOff}</option>
              <option value="daily">{x.scheduleDaily}</option>
              <option value="weekly">{x.scheduleWeekly}</option>
            </select>
            {sched.kind === "weekly" ? (
              <select
                aria-label={x.weekday}
                value={sched.weekday}
                onChange={(e) => set({ schedule: { ...sched, weekday: Number(e.target.value) } })}
                className={SELECT}
              >
                {x.weekdays.map((name, i) => (
                  <option key={i} value={i}>
                    {name}
                  </option>
                ))}
              </select>
            ) : null}
            {sched.kind !== "off" ? (
              <Input
                type="time"
                aria-label={x.time}
                value={`${pad(sched.hh)}:${pad(sched.mm)}`}
                onChange={(e) => setTime(e.target.value)}
                className="w-28"
              />
            ) : null}
          </div>
        </Row>

        <Row label={x.scope}>
          <Segmented
            label={x.scope}
            value={draft.projects.kind}
            options={[
              { value: "all", label: x.scopeAll },
              { value: "selected", label: x.scopeSelected },
            ]}
            onChange={(k) => set({ projects: k === "all" ? { kind: "all" } : { kind: "selected", ids: selectedIds } })}
          />
          {draft.projects.kind === "selected" ? (
            projects.length === 0 ? (
              <p className="mt-2 text-sm text-muted-foreground">{x.noProjects}</p>
            ) : (
              <ul aria-label={x.projectList} className="mt-2 max-h-44 overflow-y-auto rounded-md border border-border">
                {projects.map((p) => (
                  <li key={p.id} className="border-b border-border last:border-b-0">
                    <label className="flex cursor-pointer items-center gap-2.5 px-3 py-1.5 text-sm hover:bg-muted/50">
                      <input
                        type="checkbox"
                        checked={selectedIds.includes(p.id)}
                        onChange={() => toggleProject(p.id)}
                        className="size-4 shrink-0 cursor-pointer accent-primary"
                      />
                      <span className="shrink-0">{p.name}</span>
                      <span className="min-w-0 truncate font-mono text-xs text-muted-foreground">{p.path}</span>
                    </label>
                  </li>
                ))}
              </ul>
            )
          ) : null}
        </Row>

        <Row label={x.includeGlobal} htmlFor="extract-global">
          <input
            id="extract-global"
            type="checkbox"
            checked={draft.include_global}
            onChange={(e) => set({ include_global: e.target.checked })}
            className="mt-1 size-4 cursor-pointer accent-primary"
          />
        </Row>

        <Row label={x.lookback} htmlFor="extract-lookback">
          <div className="flex items-center gap-2">
            <Input
              id="extract-lookback"
              type="number"
              inputMode="numeric"
              min={1}
              max={90}
              value={lookbackText}
              aria-invalid={!lookbackOk}
              onChange={(e) => setLookbackText(e.target.value)}
              className="w-20"
            />
            <span className="text-sm text-muted-foreground">{x.lookbackUnit}</span>
            {!lookbackOk ? (
              <span role="alert" className="text-xs text-destructive">
                {x.lookbackInvalid}
              </span>
            ) : null}
          </div>
        </Row>
      </div>

      <div className="flex justify-end pt-3">
        <Button type="submit" disabled={!dirty || !lookbackOk || saving} className="h-8 cursor-pointer px-3 text-sm font-semibold">
          {saving ? x.saving : x.save}
        </Button>
      </div>
    </form>
  );
}
