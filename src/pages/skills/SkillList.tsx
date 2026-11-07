import * as React from "react";
import { ArrowLeftRight, ChevronDown, ChevronRight, Plus, RefreshCw, Search, Trash2 } from "lucide-react";
import type { DeletedSkill, SkillEntry, UpdateStatus } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { SimpleSelect } from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { formatSnapshotTime } from "@/pages/memory/HistoryDrawer";
import { ToolTag } from "@/pages/memory/ToolTag";
import { fmt } from "@/pages/memory/paths";
import { entryKey, sortEntries, sourceLabel } from "./skillUtils";

type ToolFilter = "all" | "claude" | "codex";
type ScopeFilter = "all" | "global" | "project" | "readonly";

export interface SkillListProps {
  entries: SkillEntry[];
  /** Display name for a project id. */
  projectName: (projectId: string) => string;
  /** Update-check results by skill id. */
  updates: Map<string, UpdateStatus>;
  /** `entryKey()` of the opened skill. */
  selectedKey?: string;
  onSelect: (entry: SkillEntry) => void;
  gitOk: boolean;
  checking: boolean;
  /** Number of updates found by the last check; null = not checked yet. */
  checkedUpdates: number | null;
  onCheck: () => void;
  onAdd: () => void;
  /** Opens the skill manifest (sync) dialog; the button is hidden without it. */
  onManifest?: () => void;
  onUpdatePlugin?: (entry: SkillEntry) => void;
  updatingPlugin?: SkillEntry | null;
  /** Deleted skills whose backups remain; the group is hidden when empty. */
  deleted?: DeletedSkill[];
  /** Bucket of the opened deleted skill. */
  selectedDeleted?: string;
  onSelectDeleted?: (item: DeletedSkill) => void;
}

export const tagClass =
  "inline-flex h-[18px] max-w-full shrink-0 items-center rounded-sm bg-muted px-[5px] text-xs leading-[18px] text-muted-foreground";

const pluginGroupKey = (entry: SkillEntry) => JSON.stringify([entry.tool, entry.plugin, entry.scope, entry.plugin_install_path]);

const selectClass = "h-[30px] w-full min-w-0 flex-1 bg-card px-2.5 text-xs dark:bg-card";

export function SkillList({
  entries,
  projectName,
  updates,
  selectedKey,
  onSelect,
  gitOk,
  checking,
  checkedUpdates,
  onCheck,
  onAdd,
  onManifest,
  onUpdatePlugin,
  updatingPlugin,
  deleted = [],
  selectedDeleted,
  onSelectDeleted,
}: SkillListProps) {
  const { t } = useI18n();
  const s = t.skills;
  const [query, setQuery] = React.useState("");
  const [tool, setTool] = React.useState<ToolFilter>("all");
  const [scope, setScope] = React.useState<ScopeFilter>("all");
  const listRef = React.useRef<HTMLDivElement>(null);
  const [deletedOpen, setDeletedOpen] = React.useState(false);
  const [openPlugins, setOpenPlugins] = React.useState<Record<string, boolean>>({});
  const [searchOpenPlugins, setSearchOpenPlugins] = React.useState<Record<string, boolean>>({});
  const search = query.trim().toLowerCase();
  // An opened deleted skill keeps its group visible.
  const showDeleted = deletedOpen || selectedDeleted !== undefined;

  const visible = React.useMemo(() => {
    const list = entries.filter((e) => {
      if (tool !== "all" && e.tool !== tool) return false;
      if (scope === "global" && e.scope.type !== "global") return false;
      if (scope === "project" && e.scope.type !== "project") return false;
      if (scope === "readonly" && !e.readonly) return false;
      return !search || [e.name, e.description, e.plugin ?? ""].some((value) => value.toLowerCase().includes(search));
    });
    return sortEntries(list);
  }, [entries, search, tool, scope]);

  const standalone = visible.filter((entry) => !entry.plugin);
  const plugins = new Map<string, SkillEntry[]>();
  for (const entry of visible) {
    if (!entry.plugin) continue;
    const key = pluginGroupKey(entry);
    const group = plugins.get(key);
    if (group) group.push(entry);
    else plugins.set(key, [entry]);
  }
  const firstReadonly = standalone.findIndex((entry) => entry.readonly);
  const showGroups = firstReadonly > 0;

  const open = (key: string, members: SkillEntry[]) =>
    search ? (searchOpenPlugins[key] ?? true) : (openPlugins[key] ?? members.some((entry) => entryKey(entry) === selectedKey));
  const visibleRows = [
    ...standalone,
    ...Array.from(plugins).flatMap(([key, members]) => open(key, members) ? members : []),
  ];
  const tabKey = visibleRows.find((entry) => entryKey(entry) === selectedKey) ?? visibleRows[0];

  const renderRow = (entry: SkillEntry) => (
    <Row
      key={entryKey(entry)}
      entry={entry}
      selected={entryKey(entry) === selectedKey}
      update={updates.get(entry.id)}
      projectName={projectName}
      onSelect={() => onSelect(entry)}
      tabbable={entry === tabKey}
    />
  );

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    const items = Array.from(listRef.current?.querySelectorAll<HTMLElement>('[role="option"], [data-plugin-toggle]:not(:disabled)') ?? []);
    const i = items.indexOf(document.activeElement as HTMLElement);
    if (i < 0) return;
    e.preventDefault();
    items[Math.min(items.length - 1, Math.max(0, i + (e.key === "ArrowDown" ? 1 : -1)))]?.focus();
  };

  const canCheck = gitOk || entries.some((entry) => entry.plugin);
  const checkLabel = checking
    ? s.checking
    : checkedUpdates === null
      ? s.checkUpdates
      : fmt(s.checkUpdatesN, { n: checkedUpdates });

  return (
    <div className="flex h-full min-h-0 w-full flex-col">
      <div className="flex flex-col gap-2 border-b border-border p-3">
        <label className="flex h-[30px] items-center gap-1.5 rounded-md border border-border bg-card px-2.5 text-muted-foreground focus-within:ring-2 focus-within:ring-ring">
          <Search className="size-4 shrink-0" aria-hidden="true" />
          <Input
            type="search"
            role="searchbox"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setSearchOpenPlugins({});
            }}
            placeholder={s.searchPlaceholder}
            aria-label={s.searchLabel}
            className="h-auto min-w-0 flex-1 rounded-none border-0 bg-transparent p-0 text-sm text-foreground shadow-none focus-visible:ring-0 md:text-sm dark:bg-transparent [&::-webkit-search-cancel-button]:hidden"
          />
        </label>
        <div className="flex gap-2">
          <div className="relative flex min-w-0 flex-1">
            <SimpleSelect
              aria-label={s.toolFilter}
              value={tool}
              onValueChange={(v) => setTool(v as ToolFilter)}
              className={selectClass}
              options={[
                { value: "all", label: s.toolAll },
                { value: "claude", label: t.memory.toolClaude },
                { value: "codex", label: t.memory.toolCodex },
              ]}
            />
          </div>
          <div className="relative flex min-w-0 flex-1">
            <SimpleSelect
              aria-label={s.scopeFilter}
              value={scope}
              onValueChange={(v) => setScope(v as ScopeFilter)}
              className={selectClass}
              options={[
                { value: "all", label: s.scopeAll },
                { value: "global", label: s.scopeGlobal },
                { value: "project", label: s.scopeProject },
                { value: "readonly", label: s.scopeReadonly },
              ]}
            />
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <span className="flex min-w-fit flex-1" title={canCheck ? s.checkUpdatesScope : s.needGit}>
            <Button
              type="button"
              variant="outline"
              disabled={!canCheck || checking}
              onClick={onCheck}
              className="h-7 min-w-0 flex-1 cursor-pointer gap-1.5 bg-card px-3 text-xs hover:bg-muted"
            >
              <RefreshCw className={cn("size-3.5 shrink-0", checking && "animate-spin motion-reduce:animate-none")} aria-hidden="true" />
              <span className="truncate">{checkLabel}</span>
            </Button>
          </span>
          {onManifest ? (
            <Button
              type="button"
              variant="outline"
              onClick={onManifest}
              title={s.manifest.open}
              className="h-7 shrink-0 cursor-pointer gap-1.5 bg-card px-2.5 text-xs hover:bg-muted"
            >
              <ArrowLeftRight className="size-3.5" aria-hidden="true" />
              {s.manifest.open}
            </Button>
          ) : null}
          <Button type="button" onClick={onAdd} className="h-7 shrink-0 cursor-pointer gap-1.5 px-3 text-xs font-semibold">
            <Plus className="size-4" aria-hidden="true" />
            {s.addSkill}
          </Button>
        </div>
      </div>

      <div
        ref={listRef}
        role="listbox"
        aria-label={s.listLabel}
        onKeyDown={onKeyDown}
        className="min-h-0 flex-1 overflow-y-auto p-2"
      >
        {visible.length === 0 ? (
          <p className="px-2 py-6 text-center text-sm text-muted-foreground">{s.noMatches}</p>
        ) : (
          <>
            {standalone.map((entry, i) => (
              <React.Fragment key={entryKey(entry)}>
                {showGroups && i === 0 ? <GroupLabel>{s.writableGroup}</GroupLabel> : null}
                {showGroups && i === firstReadonly ? <GroupLabel>{s.readonlyGroup}</GroupLabel> : null}
                {renderRow(entry)}
              </React.Fragment>
            ))}
            {plugins.size > 0 && <GroupLabel>{s.pluginGroup}</GroupLabel>}
            {Array.from(plugins, ([key, members]) => {
              const entry = members[0];
              const name = (entry.plugin ?? "").split("@", 1)[0];
              const expanded = open(key, members);
              const isUpdating = updatingPlugin != null && pluginGroupKey(updatingPlugin) === key;
              const scopeName = entry.scope.type === "global" ? s.global : projectName(entry.scope.project_id);
              const metadata = [entry.plugin_version, fmt(s.pluginCount, { n: members.length }), scopeName].filter(Boolean).join(" · ");
              const toolName = entry.tool === "claude" ? t.memory.toolClaude : t.memory.toolCodex;
              return (
                <div key={key} className="mb-0.5">
                  <div className="flex items-center gap-1 rounded-md pr-1 hover:bg-muted/50 focus-within:bg-muted/50">
                    <button
                      type="button"
                      data-plugin-toggle
                      aria-label={`${name} ${toolName} · ${metadata}`}
                      aria-expanded={expanded}
                      onClick={() => {
                        const setExpanded = search ? setSearchOpenPlugins : setOpenPlugins;
                        setExpanded((previous) => ({ ...previous, [key]: !expanded }));
                      }}
                      className="flex min-w-0 flex-1 cursor-pointer items-start gap-1.5 rounded-md px-2 py-1.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      {expanded ? <ChevronDown className="mt-0.5 size-3.5 shrink-0" aria-hidden /> : <ChevronRight className="mt-0.5 size-3.5 shrink-0" aria-hidden />}
                      <span className="min-w-0 flex-1">
                        <span className="flex items-center gap-1.5">
                          <span className="truncate text-sm font-medium" title={entry.plugin ?? undefined}>{name}</span>
                          <ToolTag tool={entry.tool} />
                        </span>
                        <span className="block truncate text-xs text-muted-foreground">
                          {metadata}
                        </span>
                      </span>
                    </button>
                    {onUpdatePlugin && (
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        disabled={updatingPlugin != null}
                        aria-busy={isUpdating}
                        onClick={() => onUpdatePlugin(entry)}
                        aria-label={fmt(isUpdating ? s.updatingPluginNamed : s.updatePluginNamed, { name, tool: toolName })}
                        className="h-6 shrink-0 gap-1 px-1.5 text-xs font-normal text-muted-foreground hover:text-foreground"
                      >
                        <RefreshCw className={cn("size-3", isUpdating && "motion-safe:animate-spin")} aria-hidden="true" />
                        {isUpdating ? s.updating : s.updatePlugin}
                      </Button>
                    )}
                  </div>
                  {expanded && <div role="group" aria-label={name} className="mb-1 ml-[15px] border-l border-border/65 pl-2">{members.map(renderRow)}</div>}
                </div>
              );
            })}
          </>
        )}
      </div>

      {deleted.length > 0 ? (
        <div className="flex max-h-[45%] shrink-0 flex-col border-t border-border">
          <button
            type="button"
            aria-expanded={showDeleted}
            onClick={() => setDeletedOpen((v) => !v)}
            className="flex h-9 shrink-0 cursor-pointer items-center gap-1.5 px-3 text-left text-xs font-medium text-muted-foreground outline-none transition-colors duration-150 hover:bg-muted/60 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset motion-reduce:transition-none"
          >
            {showDeleted ? <ChevronDown className="size-3.5 shrink-0" aria-hidden="true" /> : <ChevronRight className="size-3.5 shrink-0" aria-hidden="true" />}
            <Trash2 className="size-3.5 shrink-0" aria-hidden="true" />
            <span className="truncate">{fmt(s.deletedGroup, { n: deleted.length })}</span>
          </button>
          {showDeleted ? (
            <div role="list" aria-label={s.deletedListLabel} className="min-h-0 overflow-y-auto px-2 pb-2">
              {deleted.map((d) => {
                const on = d.bucket === selectedDeleted;
                const latest = d.backups[0]?.created_at;
                return (
                  <button
                    key={d.bucket}
                    type="button"
                    role="listitem"
                    aria-current={on ? "true" : undefined}
                    onClick={() => onSelectDeleted?.(d)}
                    className={cn(
                      "mb-0.5 flex w-full cursor-pointer flex-col gap-0.5 rounded-md px-2.5 py-1.5 text-left text-foreground outline-none transition-colors duration-150 hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none",
                      on && "bg-muted shadow-[inset_2px_0_0_var(--primary)]",
                    )}
                  >
                    <span className="min-w-0 truncate font-mono text-[13px] font-medium" title={d.name}>
                      {d.name}
                    </span>
                    <span className="truncate text-xs text-muted-foreground">
                      {latest ? fmt(s.deletedLatest, { time: formatSnapshotTime(latest) }) : s.deletedNoBackups}
                    </span>
                  </button>
                );
              })}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function GroupLabel({ children }: { children: React.ReactNode }) {
  return <div className="px-2 pt-2 pb-1 text-xs font-medium text-muted-foreground first:pt-0">{children}</div>;
}

function Row({
  entry,
  selected,
  update,
  projectName,
  onSelect,
  tabbable,
}: {
  entry: SkillEntry;
  selected: boolean;
  update?: UpdateStatus;
  projectName: (id: string) => string;
  onSelect: () => void;
  tabbable: boolean;
}) {
  const { t } = useI18n();
  const s = t.skills;
  const scopeText = entry.scope.type === "global" ? s.global : projectName(entry.scope.project_id);
  return (
    <div
      role="option"
      aria-selected={selected}
      tabIndex={tabbable ? 0 : -1}
      onClick={onSelect}
      onKeyDown={(ev) => {
        if (ev.key === "Enter" || ev.key === " ") {
          ev.preventDefault();
          onSelect();
        }
      }}
      className={cn(
        "mb-0.5 flex cursor-pointer flex-col gap-1 rounded-md px-2.5 py-2 text-foreground transition-colors duration-150 outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none",
        entry.plugin && "gap-0.5 py-1.5",
        selected && "bg-muted shadow-[inset_2px_0_0_var(--primary)]",
        entry.readonly && !entry.plugin && "opacity-60",
      )}
    >
      <div className="flex min-w-0 items-center gap-1.5">
        <span data-testid="skill-name" className="min-w-0 truncate font-mono text-[13px] font-medium" title={entry.name}>
          {entry.name}
        </span>
        {update?.has_update ? (
          <span className="inline-flex h-[18px] shrink-0 items-center rounded-sm bg-primary/18 px-[5px] text-xs leading-[18px] font-semibold text-accent-text">
            {s.hasUpdate}
          </span>
        ) : null}
        {update?.local_modified ? (
          <span role="img" aria-label={s.localModified} title={s.localModified} className="size-[7px] shrink-0 rounded-full bg-warning" />
        ) : update?.local_state_unknown ? (
          <span
            role="img"
            aria-label={s.localUnknown}
            title={s.localUnknown}
            className="grid size-4 shrink-0 place-items-center rounded-full border border-border text-xs leading-none text-muted-foreground"
          >
            ?
          </span>
        ) : null}
      </div>
      <p className="truncate text-[12.5px] text-muted-foreground" title={entry.description}>
        {entry.description || " "}
      </p>
      {!entry.plugin && (
        <div className="flex flex-wrap items-center gap-1">
          <ToolTag tool={entry.tool} />
          <span className={cn(tagClass, "truncate")}>{scopeText}</span>
          <span className={cn(tagClass, "truncate")}>{sourceLabel(entry, s)}</span>
        </div>
      )}
    </div>
  );
}
