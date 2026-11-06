import * as React from "react";
import { BarChart3, Download, Info, Loader2, RefreshCw, TriangleAlert } from "lucide-react";
import { save } from "@tauri-apps/plugin-dialog";
import { onUsageUpdated, setClaudeQuotaEnabled, setCodexQuotaEnabled, usageExportCsv, usageQuota, usageRescan, usageSummary } from "@/api/usage";
import type { QuotaDto, Tool, UsageQuery, UsageReport, UsageView } from "@/api/types";
import { Button } from "@/components/ui/button";
import { DatePicker } from "@/components/ui/date-picker";
import { SimpleSelect } from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";
import { ToastRegion, useToasts } from "@/pages/skills/Toasts";
import { errorMessage } from "@/pages/skills/skillUtils";
import { QuotaCards } from "./QuotaCards";
import { UsageChart } from "./UsageChart";
import { UsageHeatmap } from "./UsageHeatmap";
import { UsageKpis } from "./UsageKpis";
import { UsageTable } from "./UsageTable";
import { localDate, monthRange, presetRange, PRESETS, type RangePreset } from "./usageUtils";

const QUOTA_REFRESH_MS = 60_000;

const fieldCls = "border-border bg-card text-foreground hover:bg-muted dark:bg-card dark:hover:bg-muted";

function Segmented<T extends string>({ value, options, onChange, label }: { value: T; options: { value: T; label: string }[]; onChange: (v: T) => void; label: string }) {
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex h-8 rounded-md border border-border bg-card p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          onClick={() => onChange(o.value)}
          className={cn(
            "cursor-pointer rounded-sm px-3 text-sm outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none",
            o.value === value ? "seg-active" : "text-muted-foreground hover:text-foreground",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function UsagePage() {
  const { t } = useI18n();
  const u = t.usage;
  const [view, setView] = React.useState<UsageView>("daily");
  const [preset, setPreset] = React.useState<RangePreset>("last30");
  const [range, setRange] = React.useState(() => presetRange("last30", new Date()));
  const [tool, setTool] = React.useState<"all" | Tool>("all");
  const [model, setModel] = React.useState<string>("all");
  const [report, setReport] = React.useState<UsageReport | null>(null);
  const [quota, setQuota] = React.useState<QuotaDto | null>(null);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [focusKey, setFocusKey] = React.useState<string | null>(null);
  const [enabling, setEnabling] = React.useState(false);
  const [rescanning, setRescanning] = React.useState(false);
  const [now, setNow] = React.useState(() => Date.now());
  const { toasts, push, dismiss } = useToasts();

  const query: UsageQuery = React.useMemo(() => {
    const today = localDate(new Date());
    return {
      view,
      from: view === "today" ? today : range.from,
      to: view === "today" ? today : range.to,
      tools: tool === "all" ? [] : [tool],
      models: model === "all" ? [] : [model],
    };
  }, [view, range, tool, model]);

  const loadSummary = React.useCallback(async () => {
    try {
      setReport(await usageSummary(query));
      setLoadError(null);
    } catch (e) {
      setLoadError(errorMessage(e));
    }
  }, [query]);

  const loadQuota = React.useCallback(async (force = false) => {
    try {
      setQuota(await usageQuota(force));
    } catch {
      // Quota cards stay in their loading / previous state; statistics are unaffected.
    }
  }, []);

  React.useEffect(() => {
    void loadSummary();
  }, [loadSummary]);

  const reloadRef = React.useRef(loadSummary);
  reloadRef.current = loadSummary;
  React.useEffect(() => {
    void loadQuota();
    let unlisten: (() => void) | null = null;
    let alive = true;
    void onUsageUpdated(() => {
      void reloadRef.current();
      void loadQuota();
    }).then((f) => {
      if (alive) unlisten = f;
      else f();
    });
    const timer = window.setInterval(() => {
      setNow(Date.now());
      void loadQuota();
    }, QUOTA_REFRESH_MS);
    return () => {
      alive = false;
      unlisten?.();
      window.clearInterval(timer);
    };
  }, [loadQuota]);

  const changeView = (v: UsageView) => {
    setView(v);
    setFocusKey(null);
    if (v !== "today") {
      const p = PRESETS[v][v === "daily" ? 1 : 0];
      setPreset(p);
      setRange(presetRange(p, new Date()));
    }
  };

  const changePreset = (p: RangePreset) => {
    setPreset(p);
    if (p !== "custom") setRange(presetRange(p, new Date()));
  };

  const pickBar = (key: string) => {
    if (view === "monthly") {
      setView("daily");
      setPreset("custom");
      setRange(monthRange(key));
      setFocusKey(null);
    } else {
      setFocusKey(key);
    }
  };

  const enableQuota = async (set: (on: boolean) => Promise<unknown>) => {
    setEnabling(true);
    try {
      await set(true);
      await loadQuota();
    } catch (e) {
      push("error", errorMessage(e));
    } finally {
      setEnabling(false);
    }
  };

  const rescan = async () => {
    setRescanning(true);
    try {
      await usageRescan();
      await Promise.all([loadSummary(), loadQuota(true)]);
    } catch (e) {
      push("error", errorMessage(e));
    } finally {
      setRescanning(false);
    }
  };

  const exportCsv = async () => {
    try {
      const path = await save({
        defaultPath: `memdock-usage-${query.from}_${query.to}.csv`,
        filters: [{ name: "CSV", extensions: ["csv"] }],
      });
      if (!path) return;
      await usageExportCsv(query, path);
      push("success", fmt(u.exported, { path }));
    } catch (e) {
      push("error", fmt(u.exportFailed, { error: errorMessage(e) }));
    }
  };

  const modelOptions = report ? Array.from(new Set([...report.models, ...(model === "all" ? [] : [model])])) : [];

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex w-full max-w-[1440px] flex-col gap-4 px-6 py-6">
        <header className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex flex-col gap-0.5">
            <h1 className="flex items-center gap-2 text-lg font-semibold">
              <BarChart3 className="size-5 text-muted-foreground" aria-hidden="true" />
              {u.title}
            </h1>
            <p className="text-xs text-muted-foreground">{u.subtitle}</p>
          </div>
          <Button type="button" variant="outline" size="sm" disabled={rescanning} onClick={() => void rescan()} title={u.refreshHint} className="cursor-pointer">
            <RefreshCw className={cn("size-3.5", rescanning && "animate-spin motion-reduce:animate-none")} aria-hidden="true" />
            {u.refresh}
          </Button>
        </header>

        <QuotaCards
          claude={quota?.claude ?? null}
          codex={quota?.codex ?? null}
          now={now}
          onEnableClaude={() => void enableQuota(setClaudeQuotaEnabled)}
          onEnableCodex={() => void enableQuota(setCodexQuotaEnabled)}
          enabling={enabling}
        />

        <div className="flex flex-wrap items-center gap-2">
          <Segmented
            label={u.title}
            value={view}
            onChange={changeView}
            options={(["today", "daily", "monthly"] as const).map((v) => ({ value: v, label: u.views[v] }))}
          />
          {view !== "today" ? (
            <>
              <SimpleSelect
                aria-label={u.range.from}
                value={preset}
                onValueChange={(v) => changePreset(v as RangePreset)}
                className={fieldCls}
                options={[
                  ...PRESETS[view].map((p) => ({ value: p, label: u.range[p as Exclude<RangePreset, "custom">] })),
                  ...(preset === "custom" ? [{ value: "custom", label: `${range.from} ~ ${range.to}` }] : []),
                ]}
              />
              <DatePicker
                aria-label={u.range.from}
                value={range.from}
                max={range.to}
                onChange={(v) => {
                  setPreset("custom");
                  setRange((r) => ({ ...r, from: v }));
                }}
                className={fieldCls}
              />
              <span className="text-muted-foreground">~</span>
              <DatePicker
                aria-label={u.range.to}
                value={range.to}
                min={range.from}
                onChange={(v) => {
                  setPreset("custom");
                  setRange((r) => ({ ...r, to: v }));
                }}
                className={fieldCls}
              />
            </>
          ) : null}
          <SimpleSelect
            aria-label={u.filter.tool}
            value={tool}
            onValueChange={(v) => setTool(v as "all" | Tool)}
            className={fieldCls}
            options={[
              { value: "all", label: u.filter.allTools },
              { value: "claude", label: t.memory.toolClaude },
              { value: "codex", label: t.memory.toolCodex },
            ]}
          />
          <SimpleSelect
            aria-label={u.filter.model}
            value={model}
            onValueChange={setModel}
            className={cn(fieldCls, "max-w-56")}
            options={[{ value: "all", label: u.filter.allModels }, ...modelOptions.map((m) => ({ value: m, label: m }))]}
          />
          <span className="flex-1" />
          <Button type="button" variant="outline" size="sm" onClick={() => void exportCsv()} disabled={!report} className="cursor-pointer">
            <Download className="size-3.5" aria-hidden="true" />
            {u.export}
          </Button>
        </div>

        {report?.scanning ? (
          <p role="status" className="flex items-center gap-2 rounded-md bg-muted px-3 py-2 text-sm">
            <Loader2 className="size-4 animate-spin text-accent-text motion-reduce:animate-none" aria-hidden="true" />
            {u.scanning}
          </p>
        ) : null}
        {loadError ? (
          <p role="alert" className="rounded-md bg-destructive/15 px-3 py-2 text-sm text-destructive">
            {fmt(u.loadFailed, { error: loadError })}
          </p>
        ) : null}
        {report && report.unknown_models.length > 0 ? (
          <p className="flex items-start gap-2 rounded-md bg-warning/15 px-3 py-2 text-sm">
            <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
            <span>{fmt(u.unknownPrice, { models: report.unknown_models.join(", ") })}</span>
          </p>
        ) : null}

        <UsageKpis report={report} view={view} />

        {report ? <UsageChart report={report} onPick={pickBar} /> : <div className="h-64 animate-pulse rounded-lg bg-muted motion-reduce:animate-none" />}
        <UsageHeatmap view={view} range={range} tools={query.tools} models={query.models} version={report} />
        {report ? <UsageTable report={report} focusKey={focusKey} /> : null}

        {report ? (
          <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
            <Info className="mt-px size-3.5 shrink-0" aria-hidden="true" />
            {fmt(u.estimateNote, { date: report.pricing_updated })}
          </p>
        ) : null}
      </div>
      <ToastRegion toasts={toasts} onDismiss={dismiss} />
    </div>
  );
}
