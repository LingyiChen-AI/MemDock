import * as React from "react";
import { BarChart3, Download, Info, Loader2, RefreshCw, TriangleAlert } from "lucide-react";
import { save } from "@tauri-apps/plugin-dialog";
import { onUsageUpdated, setClaudeQuotaEnabled, usageExportCsv, usageQuota, usageRescan, usageSummary } from "@/api/usage";
import type { QuotaDto, Tool, UsageQuery, UsageReport, UsageView } from "@/api/types";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";
import { ToastRegion, useToasts } from "@/pages/skills/Toasts";
import { errorMessage } from "@/pages/skills/skillUtils";
import { QuotaCards } from "./QuotaCards";
import { UsageChart } from "./UsageChart";
import { UsageTable } from "./UsageTable";
import { fmtCost, fmtPct, fmtTokens, localDate, monthRange, presetRange, PRESETS, type RangePreset } from "./usageUtils";

const QUOTA_REFRESH_MS = 60_000;

const fieldCls =
  "h-8 cursor-pointer rounded-md border border-border bg-card px-2 text-sm text-foreground outline-none transition-colors duration-150 hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none";

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
            o.value === value ? "bg-muted font-medium text-foreground" : "text-muted-foreground hover:text-foreground",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function Stat({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="flex min-w-0 flex-col gap-1 rounded-lg border border-border bg-card px-4 py-3">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="truncate text-[22px] leading-7 font-semibold tabular-nums">{value}</span>
      {sub ? <span className="truncate text-xs text-muted-foreground tabular-nums">{sub}</span> : null}
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

  const loadQuota = React.useCallback(async () => {
    try {
      setQuota(await usageQuota());
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

  const enableClaude = async () => {
    setEnabling(true);
    try {
      await setClaudeQuotaEnabled(true);
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
      await Promise.all([loadSummary(), loadQuota()]);
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

  const s = report?.summary;
  const peakLabel = view === "today" ? u.summary.peakHour : view === "monthly" ? u.summary.peakMonth : u.summary.peakDay;
  const peakKey = s?.peak ? (view === "today" ? `${s.peak.key}:00` : s.peak.key) : null;
  const modelOptions = report ? Array.from(new Set([...report.models, ...(model === "all" ? [] : [model])])) : [];

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex max-w-6xl flex-col gap-4 p-6">
        <header className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex flex-col gap-0.5">
            <h1 className="flex items-center gap-2 text-lg font-semibold">
              <BarChart3 className="size-5 text-muted-foreground" aria-hidden="true" />
              {u.title}
            </h1>
            <p className="text-xs text-muted-foreground">{u.subtitle}</p>
          </div>
          <Button type="button" variant="outline" size="sm" disabled={rescanning} onClick={() => void rescan()} className="cursor-pointer">
            <RefreshCw className={cn("size-3.5", rescanning && "animate-spin motion-reduce:animate-none")} aria-hidden="true" />
            {u.refresh}
          </Button>
        </header>

        <QuotaCards claude={quota?.claude ?? null} codex={quota?.codex ?? null} now={now} onEnableClaude={() => void enableClaude()} enabling={enabling} />

        <div className="flex flex-wrap items-center gap-2">
          <Segmented
            label={u.title}
            value={view}
            onChange={changeView}
            options={(["today", "daily", "monthly"] as const).map((v) => ({ value: v, label: u.views[v] }))}
          />
          {view !== "today" ? (
            <>
              <select aria-label={u.range.from} value={preset} onChange={(e) => changePreset(e.target.value as RangePreset)} className={fieldCls}>
                {PRESETS[view].map((p) => (
                  <option key={p} value={p}>
                    {u.range[p as Exclude<RangePreset, "custom">]}
                  </option>
                ))}
                {preset === "custom" ? <option value="custom">{`${range.from} ~ ${range.to}`}</option> : null}
              </select>
              <input
                type="date"
                aria-label={u.range.from}
                value={range.from}
                max={range.to}
                onChange={(e) => {
                  if (!e.target.value) return;
                  setPreset("custom");
                  setRange((r) => ({ ...r, from: e.target.value }));
                }}
                className={cn(fieldCls, "tabular-nums")}
              />
              <span className="text-muted-foreground">~</span>
              <input
                type="date"
                aria-label={u.range.to}
                value={range.to}
                min={range.from}
                onChange={(e) => {
                  if (!e.target.value) return;
                  setPreset("custom");
                  setRange((r) => ({ ...r, to: e.target.value }));
                }}
                className={cn(fieldCls, "tabular-nums")}
              />
            </>
          ) : null}
          <select aria-label={u.filter.tool} value={tool} onChange={(e) => setTool(e.target.value as "all" | Tool)} className={fieldCls}>
            <option value="all">{u.filter.allTools}</option>
            <option value="claude">{t.memory.toolClaude}</option>
            <option value="codex">{t.memory.toolCodex}</option>
          </select>
          <select aria-label={u.filter.model} value={model} onChange={(e) => setModel(e.target.value)} className={cn(fieldCls, "max-w-56")}>
            <option value="all">{u.filter.allModels}</option>
            {modelOptions.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
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

        <div className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-5">
          <Stat label={u.summary.cost} value={s ? fmtCost(s.totals.cost) : "—"} />
          <Stat label={u.summary.tokens} value={s ? fmtTokens(s.totals.total_tokens) : "—"} />
          <Stat
            label={u.summary.dailyAvg}
            value={s ? fmtCost(s.daily_avg_cost) : "—"}
            sub={s ? `${fmtTokens(s.daily_avg_tokens)} tokens` : undefined}
          />
          <Stat
            label={peakLabel}
            value={s?.peak ? (s.peak.cost > 0 ? fmtCost(s.peak.cost) : fmtTokens(s.peak.tokens)) : u.summary.none}
            sub={peakKey ?? undefined}
          />
          <Stat label={u.summary.hitRate} value={s ? fmtPct(s.cache_hit_rate) : "—"} />
        </div>

        {report ? <UsageChart report={report} onPick={pickBar} /> : <div className="h-64 animate-pulse rounded-lg bg-muted motion-reduce:animate-none" />}
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
