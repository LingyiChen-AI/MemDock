import * as React from "react";
import { CalendarDays } from "lucide-react";
import { usageSummary } from "@/api/usage";
import type { Tool, UsageReport, UsageView } from "@/api/types";
import { cn } from "@/lib/utils";
import { htmlLang } from "@/i18n";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";
import { errorMessage } from "@/pages/skills/skillUtils";
import { buildHeatGrid, fmtCostFull, fmtInt, heatmapRange, wallStart, WALL_STEP, type HeatCell, type HeatMetric, type HeatStat, type HeatTool } from "./heatmapUtils";
import { fmtTokens } from "./usageUtils";

const LABEL_W = 28;
const TOP_H = 18;
const GAP = 3;

/** Level 0 is a faint track on the card; 1..4 mix the theme accent into it, so both themes and every accent work. */
const EMPTY = "color-mix(in oklab, var(--foreground) 9%, var(--card))";
const LEVEL_FILL = [
  EMPTY,
  `color-mix(in oklab, var(--primary) 30%, ${EMPTY})`,
  `color-mix(in oklab, var(--primary) 52%, ${EMPTY})`,
  `color-mix(in oklab, var(--primary) 76%, ${EMPTY})`,
  "var(--primary)",
];

function useWidth(ref: React.RefObject<HTMLElement | null>): number {
  const [w, setW] = React.useState(0);
  React.useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setW(el.clientWidth);
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => setW(el.clientWidth));
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
  return w;
}

function Seg<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: string; disabled?: boolean }[];
  onChange: (v: T) => void;
  label: string;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex h-7 rounded-md border border-border bg-card p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          disabled={o.disabled}
          onClick={() => onChange(o.value)}
          className={cn(
            "cursor-pointer rounded-sm px-2.5 text-xs outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-40 motion-reduce:transition-none",
            value === o.value ? "bg-muted font-medium text-foreground" : "text-muted-foreground enabled:hover:text-foreground",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

interface Props {
  view: UsageView;
  range: { from: string; to: string };
  tools: Tool[];
  models: string[];
  /** Changes whenever the page reloaded its data; the heatmap reloads too. */
  version: unknown;
}

/** GitHub-style daily activity grid: columns are weeks, rows Monday..Sunday. */
export function UsageHeatmap({ view, range, tools, models, version }: Props) {
  const { t, language } = useI18n();
  const h = t.usage.heatmap;
  const [metric, setMetric] = React.useState<HeatMetric>("tokens");
  const [pickedTool, setPickedTool] = React.useState<HeatTool>("all");
  const [report, setReport] = React.useState<UsageReport | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [hover, setHover] = React.useState<{ cell: HeatCell; x: number; y: number } | null>(null);
  const boxRef = React.useRef<HTMLDivElement>(null);
  const width = useWidth(boxRef);

  // The page's tool filter locks the heatmap to that tool.
  const lockedTool: Tool | null = tools.length === 1 ? tools[0] : null;
  const heatTool: HeatTool = lockedTool ?? pickedTool;

  const span = React.useMemo(() => heatmapRange(view, range, new Date()), [view, range]);
  const toolsKey = tools.join(",");
  const modelsKey = models.join("\u0000");

  React.useEffect(() => {
    let alive = true;
    usageSummary({ view: "daily", from: span.from, to: span.to, tools, models })
      .then((r) => {
        if (!alive) return;
        setReport(r);
        setError(null);
      })
      .catch((e: unknown) => {
        if (alive) setError(errorMessage(e));
      });
    return () => {
      alive = false;
    };
    // tools / models are compared by value through their keys.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [span.from, span.to, toolsKey, modelsKey, version]);

  // GitHub-style wall: fixed cell size, as many weeks as fit, ending at the range end.
  const wallFrom = width > 0 ? wallStart(span.from, span.to, width - LABEL_W) : span.from;
  const grid = React.useMemo(
    () => (report ? buildHeatGrid(report.rows, wallFrom, span.to, metric, heatTool, span.from) : null),
    [report, wallFrom, span, metric, heatTool],
  );
  const weeks = grid?.weeks.length ?? 5;
  // A range longer than the card scrolls; start at the newest week, like GitHub.
  React.useEffect(() => {
    const el = boxRef.current;
    if (el) el.scrollLeft = el.scrollWidth;
  }, [grid]);
  const step = WALL_STEP;
  const cell = step - GAP;
  const svgW = LABEL_W + weeks * step;
  const svgH = TOP_H + 7 * step;
  const statOf = (c: HeatCell): HeatStat => (heatTool === "claude" ? c.claude : heatTool === "codex" ? c.codex : c.total);
  const activeDays = grid ? grid.weeks.flat().filter((c) => c && !c.outside && statOf(c).tokens > 0).length : 0;
  const monthFmt = React.useMemo(() => new Intl.DateTimeFormat(htmlLang(language), { month: "short" }), [language]);
  const monthLabel = (m: string) => {
    const [y, mo] = m.split("-").map(Number);
    return monthFmt.format(new Date(y, mo - 1, 1));
  };

  const wrapRef = React.useRef<HTMLDivElement>(null);
  const show = (c: HeatCell, el: SVGRectElement) => {
    const base = wrapRef.current?.getBoundingClientRect();
    const r = el.getBoundingClientRect();
    setHover({ cell: c, x: r.left + r.width / 2 - (base?.left ?? 0), y: r.top - (base?.top ?? 0) });
  };

  const toolName = { claude: t.memory.toolClaude, codex: t.memory.toolCodex };
  const tipRows: { key: string; label: string; s: HeatStat; strong?: boolean }[] = hover
    ? [
        ...(lockedTool !== "codex" ? [{ key: "claude", label: toolName.claude, s: hover.cell.claude }] : []),
        ...(lockedTool !== "claude" ? [{ key: "codex", label: toolName.codex, s: hover.cell.codex }] : []),
        ...(lockedTool ? [] : [{ key: "total", label: h.total, s: hover.cell.total, strong: true }]),
      ]
    : [];

  return (
    <section className="flex flex-col gap-3 rounded-lg border border-border bg-card p-4">
      <header className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="flex flex-wrap items-center gap-2 text-sm font-semibold">
          <CalendarDays className="size-4 text-muted-foreground" aria-hidden="true" />
          {h.title}
          <span className="font-normal text-muted-foreground tabular-nums">
            {`${span.from} ~ ${span.to}`}
            {view === "today" ? ` · ${h.last30}` : ""}
            {grid ? ` · ${fmt(h.activeDays, { n: activeDays })}` : ""}
          </span>
        </h2>
        <div className="flex flex-wrap items-center gap-2">
          <Seg
            label={h.tool}
            value={heatTool}
            onChange={setPickedTool}
            options={(["all", "claude", "codex"] as const).map((v) => ({
              value: v,
              label: v === "all" ? h.allTools : toolName[v],
              disabled: lockedTool !== null && v !== lockedTool,
            }))}
          />
          <Seg label={h.metric} value={metric} onChange={setMetric} options={(["tokens", "cost"] as const).map((m) => ({ value: m, label: h[m] }))} />
        </div>
      </header>

      {error ? (
        <p role="alert" className="rounded-md bg-destructive/15 px-3 py-2 text-sm text-destructive">
          {fmt(h.loadFailed, { error })}
        </p>
      ) : null}

      <div ref={wrapRef} className="relative" onMouseLeave={() => setHover(null)}>
        <div ref={boxRef} className="w-full overflow-x-auto" onScroll={() => setHover(null)}>
        {grid ? (
          <svg width={svgW} height={svgH} role="img" aria-label={fmt(h.aria, { from: span.from, to: span.to })} className="mx-auto block">
            {grid.months.map((m) => (
              <text key={m.month} x={LABEL_W + m.week * step} y={12} fontSize={11} style={{ fill: "var(--muted-foreground)" }}>
                {monthLabel(m.month)}
              </text>
            ))}
            {(
              [
                [0, h.mon],
                [2, h.wed],
                [4, h.fri],
              ] as const
            ).map(([di, label]) => (
              <text key={di} x={0} y={TOP_H + di * step + cell / 2 + 4} fontSize={10} style={{ fill: "var(--muted-foreground)" }}>
                {label}
              </text>
            ))}
            {grid.weeks.map((col, wi) =>
              col.map((c, di) =>
                c ? (
                  <rect
                    key={c.date}
                    data-date={c.date}
                    data-level={c.level}
                    x={LABEL_W + wi * step}
                    y={TOP_H + di * step}
                    width={cell}
                    height={cell}
                    rx={cell > 16 ? 3 : 2}
                    style={{ fill: LEVEL_FILL[c.level], opacity: c.outside ? 0.45 : 1 }}
                    stroke={hover?.cell.date === c.date ? "var(--foreground)" : "none"}
                    strokeWidth={1}
                    onMouseEnter={(e) => (c.outside ? setHover(null) : show(c, e.currentTarget))}
                  />
                ) : (
                  // Outside the selected range: an empty, faded cell keeps the grid whole.
                  <rect
                    key={`pad-${wi}-${di}`}
                    data-pad="true"
                    x={LABEL_W + wi * step}
                    y={TOP_H + di * step}
                    width={cell}
                    height={cell}
                    rx={cell > 16 ? 3 : 2}
                    style={{ fill: LEVEL_FILL[0], opacity: 0.45 }}
                    onMouseEnter={() => setHover(null)}
                  />
                ),
              ),
            )}
          </svg>
        ) : (
          <div className="animate-pulse rounded-md bg-muted motion-reduce:animate-none" style={{ height: svgH }} />
        )}
        </div>
        {hover ? (
          <div
            role="tooltip"
            className="pointer-events-none absolute z-10 -translate-x-1/2 -translate-y-full rounded-md border border-border bg-popover px-3 py-2 text-xs whitespace-nowrap text-popover-foreground shadow-lg"
            style={{ left: hover.x, top: hover.y - 6 }}
          >
            <p className="mb-1 font-semibold tabular-nums">{hover.cell.date}</p>
            <table className="border-separate border-spacing-x-3 border-spacing-y-0.5 -mx-3">
              <thead>
                <tr className="text-muted-foreground">
                  <th className="text-left font-normal" />
                  <th className="text-right font-normal">{h.cost}</th>
                  <th className="text-right font-normal">{h.tokens}</th>
                  <th className="text-right font-normal">{h.requests}</th>
                </tr>
              </thead>
              <tbody>
                {tipRows.map((r) => (
                  <tr key={r.key} className={cn("tabular-nums", r.strong && "font-semibold")}>
                    <td className={cn("text-left", !r.strong && "text-muted-foreground")}>
                      {r.key !== "total" ? (
                        <span
                          className={cn("mr-1.5 inline-block size-2 rounded-full", r.key === "claude" ? "bg-tool-claude" : "bg-tool-codex")}
                          aria-hidden="true"
                        />
                      ) : null}
                      {r.label}
                    </td>
                    <td className="text-right">{fmtCostFull(r.s.cost)}</td>
                    <td className="text-right">{fmtTokens(r.s.tokens)}</td>
                    <td className="text-right">{fmtInt(r.s.requests)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : null}
      </div>

      <footer className="flex items-center justify-end gap-1.5 text-xs text-muted-foreground">
        <span>{h.less}</span>
        {LEVEL_FILL.map((f, i) => (
          <span key={i} className="inline-block size-3 rounded-[2px]" style={{ background: f }} aria-hidden="true" />
        ))}
        <span>{h.more}</span>
      </footer>
    </section>
  );
}
