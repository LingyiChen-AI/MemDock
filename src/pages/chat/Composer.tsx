import * as React from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeftRight,
  ArrowUp,
  Brain,
  Check,
  ChevronDown,
  Cpu,
  File as FileIcon,
  Folder,
  FolderOpen,
  GitBranch,
  ImagePlus,
  Loader2,
  Paperclip,
  Puzzle,
  RefreshCw,
  Search,
  Shield,
  SlidersHorizontal,
  Square,
  SquareSlash,
  X,
} from "lucide-react";
import {
  liveCommands,
  liveModels,
  projectFiles,
  readImageDataUri,
  type CommandOption,
  type LiveAgent,
  type ModelOption,
  type PermissionMode,
  type ProjectFile,
} from "@/api/live";
import { ToolLogo } from "@/components/brand/ToolLogo";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverAnchor, PopoverContent } from "@/components/ui/popover";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import type { Messages } from "@/i18n";
import { fmt } from "@/pages/memory/paths";
import { errorMessage } from "@/pages/skills/skillUtils";
import { getConvUi, patchConvUi } from "./convUiState";
import { parseWorktreePath, useGitRepo } from "./worktreeStore";
import { MODEL_SUGGESTIONS, agentSettings, loadChatPrefs, parseExtraArgs, rememberAgentSettings, rememberLast, type AgentSettings } from "./chatPrefs";
import {
  IMAGE_EXTS,
  MAX_IMAGE_BYTES,
  baseName,
  fileToDataUri,
  findMenuToken,
  formatInsertPath,
  inRect,
  insertAtCursor,
  isImagePath,
  quotePath,
  replaceToken,
  type MenuToken,
} from "./composerUtils";

type C = Messages["chat"];
type K = C["composer"];

const MODES: PermissionMode[] = ["ask", "auto_edit", "plan", "bypass"];
const AGENTS: LiveAgent[] = ["claude", "codex"];
/** Effort levels offered when the model list could not be read. */
const FALLBACK_EFFORTS = ["low", "medium", "high"];

/** What a send hands to the caller. The settings matter only when the send (re)starts a process. */
export interface ComposerMessage {
  text: string;
  images: string[];
  permission: PermissionMode;
  model: string;
  effort: string;
  /** Extra start arguments, already split (Claude CLI args / Codex `key=value`). */
  args: string[];
}

export interface ProjectChoice {
  path: string;
  name: string;
}

export interface ComposerProps {
  c: C;
  /** Shown right after the model in the toolbar (context window / weekly quota meters). */
  meters?: React.ReactNode;
  agent: LiveAgent;
  /** Session folder: dropped files inside it are inserted as relative paths; `@` lists its files. */
  cwd: string;
  disabled?: boolean;
  placeholder?: string;
  suggestion?: string;
  /** Small line under the status line (e.g. "queued"). */
  hint?: React.ReactNode;
  /** A turn is running: shows the stop button next to send. */
  running?: boolean;
  onStop?: () => void;
  stopping?: boolean;
  stopLabel?: string;
  stopHint?: string;
  /**
   * Settings of the attached (running) process. Changes made meanwhile are remembered and
   * marked "next time" (they apply when the session reconnects).
   */
  attached?: { permission: PermissionMode; model: string | null } | null;
  /** A new chat that has not started yet: the agent and folder can still change (`/agent`, `/cwd`). */
  draft?: { onAgentChange: (agent: LiveAgent) => void; onCwdChange: (cwd: string) => void } | null;
  /** Folders `/cwd` offers. */
  projects?: ProjectChoice[];
  /**
   * Conversation key: unsent text / images (and, once the conversation started, its settings)
   * are kept per conversation under it and restored when the composer mounts again.
   */
  stateKey?: string | null;
  /** Resolving to `false` cancels the send without an error: the text stays. */
  onSend: (msg: ComposerMessage) => Promise<unknown>;
  onError: (e: string | null) => void;
}

/** `worktree` is a toggle (no picker). */
type PickerKind = "model" | "effort" | "permissions" | "agent" | "cwd" | "args" | "worktree";

function Tip({ tip, children }: { tip: React.ReactNode; children: React.ReactElement }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent side="top" className="max-w-64">
        {tip}
      </TooltipContent>
    </Tooltip>
  );
}

/** Model / command lists per agent, fetched once from the CLI (the backend caches them for an hour). */
const modelLists: Partial<Record<LiveAgent, Promise<ModelOption[]>>> = {};
const commandLists: Partial<Record<LiveAgent, Promise<CommandOption[]>>> = {};

function loadModels(agent: LiveAgent, force = false): Promise<ModelOption[]> {
  if (force || !modelLists[agent]) {
    modelLists[agent] = liveModels(agent, force).catch((e) => {
      delete modelLists[agent];
      throw e;
    });
  }
  return modelLists[agent]!;
}

function loadCommands(agent: LiveAgent): Promise<CommandOption[]> {
  if (!commandLists[agent]) {
    commandLists[agent] = liveCommands(agent).catch((e) => {
      delete commandLists[agent];
      throw e;
    });
  }
  return commandLists[agent]!;
}

/** Forgets the cached model / command lists (tests). */
export function resetComposerCaches(): void {
  for (const a of AGENTS) {
    delete modelLists[a];
    delete commandLists[a];
  }
}

/** Rows of the model picker: "" is the CLI's default model. */
interface ModelRow {
  value: string;
  name: string;
  description: string;
}

function toRows(agent: LiveAgent, models: ModelOption[] | null, c: C): ModelRow[] {
  if (!models) return [{ value: "", name: c.modelDefault, description: "" }, ...MODEL_SUGGESTIONS[agent].map((m) => ({ value: m, name: m, description: "" }))];
  const rows: ModelRow[] = [];
  const def = models.find((m) => m.is_default);
  // Claude lists its default as value "default"; Codex flags one model as the default.
  rows.push({ value: "", name: c.modelDefault, description: def ? (agent === "claude" ? def.description : def.display_name) : "" });
  for (const m of models) if (m.value !== "default") rows.push({ value: m.value, name: m.display_name, description: m.description });
  return rows;
}

/** "claude-opus-5-5" -> "Opus 5.5"; other ids unchanged. */
export function prettyModel(id: string): string {
  const m = /^claude-([a-z]+)-(\d+)(?:-(\d{1,2}))?(?:-\d{6,})?(\[1m\])?$/i.exec(id.trim());
  if (!m) return id;
  const family = m[1].charAt(0).toUpperCase() + m[1].slice(1);
  return `${family} ${m[2]}${m[3] ? `.${m[3]}` : ""}${m[4] ? " (1M)" : ""}`;
}

/** The model the CLI uses when none is chosen, as a display name. */
function defaultModelName(models: ModelOption[] | null): string | null {
  const d = models?.find((m) => m.is_default);
  if (!d) return null;
  // Claude lists its default as "Default (recommended)" with the real model first in the description.
  if (d.value === "default") return d.description.split(/[·•]/)[0]?.trim() || null;
  return d.display_name;
}

/**
 * Label of the model button: the chosen model, else "默认 · <model>" with the model the running
 * session reports (`actual`) or the CLI's declared default.
 */
function modelName(models: ModelOption[] | null, value: string, c: C, actual?: string | null): string {
  if (value) return models?.find((m) => m.value === value)?.display_name ?? prettyModel(value);
  const real = actual ? (models?.find((m) => m.value === actual)?.display_name ?? prettyModel(actual)) : defaultModelName(models);
  return real ? fmt(c.composer.modelDefaultIs, { model: real }) : c.modelDefault;
}

/** The model whose effort levels apply: the chosen one, else the CLI's default. */
function effectiveModel(models: ModelOption[] | null, value: string): ModelOption | undefined {
  if (!models) return undefined;
  return value ? models.find((m) => m.value === value) : models.find((m) => m.is_default);
}

/** Arrow keys move between the options of a picker list. */
function listKeys(e: React.KeyboardEvent<HTMLElement>) {
  if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
  const opts = Array.from(e.currentTarget.querySelectorAll<HTMLElement>("[role=option]"));
  if (!opts.length) return;
  e.preventDefault();
  const i = opts.indexOf(document.activeElement as HTMLElement);
  const next = e.key === "ArrowDown" ? (i + 1) % opts.length : (i <= 0 ? opts.length : i) - 1;
  opts[next]?.focus();
}

function OptionButton({
  selected,
  onClick,
  warn,
  autoFocus,
  children,
}: {
  selected: boolean;
  onClick: () => void;
  warn?: boolean;
  autoFocus?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      role="option"
      aria-selected={selected}
      autoFocus={autoFocus}
      onClick={onClick}
      className={cn(
        "flex w-full cursor-pointer items-start gap-2 rounded-md px-2 py-1.5 text-left text-sm outline-none transition-colors hover:bg-muted focus-visible:bg-muted",
        selected && "bg-primary/8 hover:bg-primary/12",
        warn && "text-amber-700 dark:text-amber-300",
      )}
    >
      <span className="flex min-w-0 flex-1 flex-col">{children}</span>
      {selected ? <Check className="mt-0.5 size-4 shrink-0 text-primary" aria-hidden="true" /> : null}
    </button>
  );
}

function PickerTitle({ children }: { children: React.ReactNode }) {
  return <p className="px-2 pt-1.5 pb-1 text-xs font-semibold text-foreground">{children}</p>;
}

function ModelPicker({
  agent,
  value,
  models,
  loading,
  failed,
  onRefresh,
  onPick,
  c,
}: {
  agent: LiveAgent;
  value: string;
  models: ModelOption[] | null;
  loading: boolean;
  failed: boolean;
  onRefresh: () => void;
  onPick: (m: string) => void;
  c: C;
}) {
  const k = c.composer;
  const [query, setQuery] = React.useState("");
  const rows = toRows(agent, models, c);
  const q = query.trim().toLowerCase();
  const shown = q ? rows.filter((r) => `${r.value} ${r.name} ${r.description}`.toLowerCase().includes(q)) : rows;
  const custom = q && !rows.some((r) => r.value.toLowerCase() === q) ? query.trim() : null;
  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-1.5 border-b border-border px-2.5 py-2">
        <Search className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        <input
          autoFocus
          aria-label={k.modelSearch}
          value={query}
          placeholder={k.modelSearch}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key !== "Enter" || e.nativeEvent.isComposing) return;
            e.preventDefault();
            if (custom) onPick(custom);
            else if (shown[0]) onPick(shown[0].value);
          }}
          className="h-7 min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
        />
        <Tip tip={k.modelRefresh}>
          <button
            type="button"
            aria-label={k.modelRefresh}
            onClick={onRefresh}
            className="grid size-6 cursor-pointer place-items-center rounded-md text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
          >
            <RefreshCw className={cn("size-3.5", loading && "animate-spin motion-reduce:animate-none")} aria-hidden="true" />
          </button>
        </Tip>
      </div>
      <div role="listbox" aria-label={k.model} onKeyDown={listKeys} className="flex max-h-80 flex-col gap-0.5 overflow-y-auto p-1">
        {loading && !models ? (
          <div className="flex flex-col gap-1 p-1" aria-busy="true" aria-label={k.modelLoading}>
            {[0, 1, 2, 3].map((i) => (
              <div key={i} className="flex flex-col gap-1 rounded-md px-2 py-1.5">
                <div className="h-3.5 w-28 animate-pulse rounded bg-muted motion-reduce:animate-none" />
                <div className="h-3 w-44 animate-pulse rounded bg-muted/70 motion-reduce:animate-none" />
              </div>
            ))}
          </div>
        ) : (
          shown.map((r) => {
            const selected = value === r.value;
            return (
              <OptionButton key={r.value || "default"} selected={selected} onClick={() => onPick(r.value)}>
                <span className={cn("truncate text-foreground", selected && "font-semibold text-primary")}>{r.name}</span>
                {r.description || (r.value && r.value !== r.name) ? (
                  <span className="truncate text-[11px] text-muted-foreground">
                    {r.description}
                    {r.value && r.value !== r.name ? (
                      <span className="font-mono">
                        {r.description ? " · " : ""}
                        {r.value}
                      </span>
                    ) : null}
                  </span>
                ) : null}
              </OptionButton>
            );
          })
        )}
        {custom ? (
          <OptionButton selected={false} onClick={() => onPick(custom)}>
            {fmt(k.modelCustom, { model: custom })}
          </OptionButton>
        ) : null}
        {failed ? <p className="px-2 py-1.5 text-[11px] text-muted-foreground">{k.modelListFailed}</p> : null}
      </div>
    </div>
  );
}

function EffortPicker({ value, model, modelsLoaded, onPick, c }: { value: string; model: ModelOption | undefined; modelsLoaded: boolean; onPick: (e: string) => void; c: C }) {
  const k = c.composer;
  const levels = modelsLoaded ? [...(model?.efforts ?? [])] : [...FALLBACK_EFFORTS];
  if (value && !levels.includes(value)) levels.push(value);
  return (
    <div role="listbox" aria-label={k.effortTitle} onKeyDown={listKeys} className="flex flex-col gap-0.5 p-1">
      <PickerTitle>{k.effortTitle}</PickerTitle>
      <OptionButton selected={value === ""} autoFocus={value === ""} onClick={() => onPick("")}>
        <span>{k.effortDefault}</span>
        {model?.default_effort ? <span className="text-[11px] text-muted-foreground">{fmt(k.effortModelDefault, { level: model.default_effort })}</span> : null}
      </OptionButton>
      {levels.map((l) => (
        <OptionButton key={l} selected={value === l} autoFocus={value === l} onClick={() => onPick(l)}>
          <span className="font-mono">{l}</span>
        </OptionButton>
      ))}
      {modelsLoaded && levels.length === 0 ? <p className="px-2 py-1.5 text-[11px] text-muted-foreground">{k.effortUnsupported}</p> : null}
    </div>
  );
}

function PermissionPicker({ value, onPick, c }: { value: PermissionMode; onPick: (m: PermissionMode) => void; c: C }) {
  return (
    <div role="listbox" aria-label={c.composer.permissionTitle} onKeyDown={listKeys} className="flex flex-col gap-0.5 p-1">
      <PickerTitle>{c.composer.permissionTitle}</PickerTitle>
      {MODES.map((m) => (
        <OptionButton key={m} selected={value === m} autoFocus={value === m} warn={m === "bypass"} onClick={() => onPick(m)}>
          <span>{c.permissions[m]}</span>
          <span className="text-[11px] text-muted-foreground">{c.permissionHints[m]}</span>
        </OptionButton>
      ))}
    </div>
  );
}

function AgentPicker({ value, onPick, c }: { value: LiveAgent; onPick: (a: LiveAgent) => void; c: C }) {
  return (
    <div role="listbox" aria-label={c.composer.agentTitle} onKeyDown={listKeys} className="flex flex-col gap-0.5 p-1">
      <PickerTitle>{c.composer.agentTitle}</PickerTitle>
      {AGENTS.map((a) => (
        <OptionButton key={a} selected={value === a} autoFocus={value === a} onClick={() => onPick(a)}>
          <span className="flex items-center gap-2">
            <ToolLogo tool={a} className="size-4" aria-hidden="true" />
            {c.agents[a]}
          </span>
        </OptionButton>
      ))}
    </div>
  );
}

function CwdPicker({ value, projects, onPick, onBrowse, c }: { value: string; projects: ProjectChoice[]; onPick: (p: string) => void; onBrowse: () => void; c: C }) {
  const k = c.composer;
  const rows = value && !projects.some((p) => p.path === value) ? [{ path: value, name: baseName(value) }, ...projects] : projects;
  return (
    <div className="flex flex-col p-1">
      <PickerTitle>{k.cwdTitle}</PickerTitle>
      <div role="listbox" aria-label={k.cwdRecent} onKeyDown={listKeys} className="flex max-h-72 flex-col gap-0.5 overflow-y-auto">
        {rows.length === 0 ? <p className="px-2 py-1.5 text-[11px] text-muted-foreground">{k.cwdEmpty}</p> : null}
        {rows.map((p, i) => (
          <OptionButton key={p.path} selected={p.path === value} autoFocus={p.path === value || (!value && i === 0)} onClick={() => onPick(p.path)}>
            <span className="flex items-center gap-1.5 truncate">
              <Folder className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
              {p.name}
            </span>
            <span className="truncate font-mono text-[11px] text-muted-foreground">{p.path}</span>
          </OptionButton>
        ))}
      </div>
      <div className="mt-1 border-t border-border pt-1">
        <button
          type="button"
          onClick={onBrowse}
          className="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm outline-none hover:bg-muted focus-visible:bg-muted"
        >
          <FolderOpen className="size-4 text-muted-foreground" aria-hidden="true" />
          {k.cwdBrowse}
        </button>
      </div>
    </div>
  );
}

function ArgsPicker({ agent, value, onSave, c }: { agent: LiveAgent; value: string; onSave: (v: string) => void; c: C }) {
  const k = c.composer;
  const [text, setText] = React.useState(value);
  const save = () => onSave(text.trim());
  return (
    <form
      className="flex flex-col gap-2 p-2"
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
    >
      <label htmlFor="composer-args" className="text-xs font-semibold">
        {k.argsTitle}
      </label>
      <div className="flex items-center gap-1.5">
        <Input
          id="composer-args"
          autoFocus
          value={text}
          placeholder={agent === "claude" ? k.argsClaudePlaceholder : k.argsCodexPlaceholder}
          onChange={(e) => setText(e.target.value)}
          className="h-8 font-mono text-xs"
        />
        <Button type="submit" size="sm" className="cursor-pointer">
          {k.argsSave}
        </Button>
      </div>
      <p className="text-[11px] leading-relaxed text-muted-foreground">{agent === "claude" ? k.argsClaudeHint : k.argsCodexHint}</p>
    </form>
  );
}

/** An entry of the `/` or `@` menu. */
type MenuItem =
  | { id: string; type: "setting"; picker: PickerKind; name: string; description: string; value: string; icon: React.ReactNode }
  | { id: string; type: "cli"; cmd: CommandOption }
  | { id: string; type: "file"; file: ProjectFile };

const SETTING_ICONS: Record<PickerKind, React.ReactNode> = {
  model: <Cpu aria-hidden="true" />,
  effort: <Brain aria-hidden="true" />,
  permissions: <Shield aria-hidden="true" />,
  agent: <ArrowLeftRight aria-hidden="true" />,
  cwd: <Folder aria-hidden="true" />,
  args: <SlidersHorizontal aria-hidden="true" />,
  worktree: <GitBranch aria-hidden="true" />,
};

const SETTING_NAMES: Record<PickerKind, string> = {
  model: "model",
  effort: "effort",
  permissions: "permissions",
  agent: "agent",
  cwd: "cwd",
  args: "args",
  worktree: "worktree",
};

function settingDescription(p: PickerKind, c: C): string {
  const k = c.composer;
  return { model: k.cmdModel, effort: k.cmdEffort, permissions: k.cmdPermissions, agent: k.cmdAgent, cwd: k.cmdCwd, args: k.cmdArgs, worktree: c.worktree.cmd }[p];
}

/** Commands whose name starts with the query first, then those containing it. */
function filterCommands(list: CommandOption[], q: string): CommandOption[] {
  const ql = q.toLowerCase();
  if (!ql) return list;
  const starts = list.filter((x) => x.name.toLowerCase().startsWith(ql));
  const has = list.filter((x) => !x.name.toLowerCase().startsWith(ql) && x.name.toLowerCase().includes(ql));
  return [...starts, ...has];
}

/** Clickable item of the status line. */
function StatusItem({
  label,
  onClick,
  warn,
  next,
  violet,
  title,
  k,
  children,
}: {
  label: string;
  onClick?: () => void;
  warn?: boolean;
  next?: boolean;
  violet?: boolean;
  title?: string;
  k: K;
  children: React.ReactNode;
}) {
  const body = (
    <>
      {children}
      {next ? <span className="ml-1 rounded bg-amber-500/12 px-1 text-[10px] text-amber-700 dark:text-amber-300">{k.nextTime}</span> : null}
    </>
  );
  if (!onClick) return <span className={cn("truncate", warn && "text-amber-700 dark:text-amber-300")}>{body}</span>;
  return (
    <button
      type="button"
      aria-label={label}
      title={title ?? (next ? k.nextTimeHint : undefined)}
      onClick={onClick}
      className={cn(
        "max-w-48 cursor-pointer truncate rounded-sm border-b border-dashed border-muted-foreground/40 outline-none transition-colors hover:border-foreground/60 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring",
        warn && "border-amber-500/60 text-amber-700 hover:text-amber-800 dark:text-amber-300 dark:hover:text-amber-200",
        violet && "border-violet-500/60 font-medium text-violet-700 hover:text-violet-800 dark:text-violet-300 dark:hover:text-violet-200",
      )}
    >
      {body}
    </button>
  );
}

/**
 * Card composer: attachments, auto-growing text, file/image buttons, model label, send / stop.
 * Settings are changed with `/` commands (or the status line under the card); `@` inserts project files.
 */
export function ChatComposer({
  c,
  meters,
  agent,
  cwd,
  disabled = false,
  placeholder,
  suggestion,
  hint,
  running = false,
  stopping = false,
  stopLabel,
  stopHint,
  onStop,
  attached = null,
  draft = null,
  projects = [],
  stateKey = null,
  onSend,
  onError,
}: ComposerProps) {
  const k = c.composer;
  const [text, setText] = React.useState(() => getConvUi(stateKey).text);
  const [caret, setCaret] = React.useState(0);
  const [images, setImages] = React.useState<string[]>(() => getConvUi(stateKey).images);
  const [sending, setSending] = React.useState(false);
  const [dismissedSuggestion, setDismissedSuggestion] = React.useState<string>();
  React.useEffect(() => setDismissedSuggestion(undefined), [suggestion]);
  const [dragging, setDragging] = React.useState(false);
  const [prefs, setPrefs] = React.useState(loadChatPrefs);
  const [picker, setPicker] = React.useState<PickerKind | null>(null);
  /** A started conversation keeps its own settings; a draft follows (and updates) the global prefs. */
  const perConv = !!stateKey && !draft;
  const [convSettings, setConvSettings] = React.useState<Partial<AgentSettings> | null>(() => (perConv ? getConvUi(stateKey).settings : null));
  /** Settings changed while attached to a running process: they apply next time. */
  const [changed, setChanged] = React.useState<Set<keyof AgentSettings>>(() => {
    const out = new Set<keyof AgentSettings>();
    if (attached && convSettings) {
      if (convSettings.permission !== undefined && convSettings.permission !== attached.permission) out.add("permission");
      if (convSettings.model !== undefined && convSettings.model !== (attached.model ?? "")) out.add("model");
    }
    return out;
  });
  // Unsent text / images survive switching conversations.
  React.useEffect(() => {
    patchConvUi(stateKey, { text, images });
  }, [stateKey, text, images]);
  const ref = React.useRef<HTMLTextAreaElement>(null);
  const rootRef = React.useRef<HTMLDivElement>(null);
  const textRef = React.useRef(text);
  textRef.current = text;

  // Models (labels, effort levels, the /model picker).
  const [models, setModels] = React.useState<ModelOption[] | null>(null);
  const [modelsLoading, setModelsLoading] = React.useState(false);
  const [modelsFailed, setModelsFailed] = React.useState(false);
  const fetchModels = React.useCallback(
    (force = false) => {
      setModelsLoading(true);
      setModelsFailed(false);
      loadModels(agent, force)
        .then((m) => setModels(m))
        .catch(() => setModelsFailed(true))
        .finally(() => setModelsLoading(false));
    },
    [agent],
  );
  React.useEffect(() => {
    setModels(null);
    fetchModels();
  }, [fetchModels]);

  const mine: AgentSettings = { ...agentSettings(prefs, agent), ...(convSettings ?? {}) };
  const permission = attached && !changed.has("permission") ? attached.permission : mine.permission;
  const model = attached && !changed.has("model") ? (attached.model ?? "") : mine.model;
  const effort = mine.effort;
  const args = mine.args;
  const argList = parseExtraArgs(agent, args);
  const curModel = effectiveModel(models, model);

  // `/worktree`: drafts in a git repo only (a folder that already is a worktree has nothing to toggle).
  const w = c.worktree;
  const wtAvail = useGitRepo(cwd, !!draft && !!cwd && !parseWorktreePath(cwd)) === true;
  const [wtOn, setWtOn] = React.useState(() => !!getConvUi(stateKey).worktree);
  const worktreeOn = wtAvail && wtOn;
  const toggleWorktree = () => {
    const v = !wtOn;
    setWtOn(v);
    patchConvUi(stateKey, { worktree: v });
  };

  const setSetting = (patch: Partial<AgentSettings>) => {
    if (perConv) {
      const next = { ...(convSettings ?? {}), ...patch };
      setConvSettings(next);
      patchConvUi(stateKey, { settings: next });
    } else {
      setPrefs(rememberAgentSettings(agent, patch));
    }
    if (attached) setChanged((s) => new Set([...s, ...(Object.keys(patch) as (keyof AgentSettings)[])]));
  };

  const focusText = (pos?: number) =>
    requestAnimationFrame(() => {
      const el = ref.current;
      if (!el) return;
      el.focus();
      if (pos !== undefined) {
        el.setSelectionRange(pos, pos);
        setCaret(pos);
      }
    });

  const closePicker = () => {
    setPicker(null);
    focusText();
  };

  const browseCwd = async () => {
    try {
      const picked = await openDialog({ directory: true, multiple: false, title: k.cwdBrowseTitle, defaultPath: cwd || undefined });
      if (typeof picked === "string" && picked) pickCwd(picked);
    } catch (e) {
      onError(fmt(k.pickFailed, { error: errorMessage(e) }));
    }
  };
  const pickCwd = (p: string) => {
    draft?.onCwdChange(p);
    setPrefs(rememberLast({ cwd: p }));
    closePicker();
  };
  const pickAgent = (a: LiveAgent) => {
    draft?.onAgentChange(a);
    setPrefs(rememberLast({ agent: a }));
    closePicker();
  };

  // ----- `/` and `@` menu -----
  const token: MenuToken | null = disabled || picker ? null : findMenuToken(text, caret);
  const [dismissed, setDismissed] = React.useState<string | null>(null);
  // Esc hides the menu until the word changes.
  const tokenId = token ? `${token.kind}:${token.start}:${token.query}` : null;
  React.useEffect(() => {
    if (!tokenId) setDismissed(null);
  }, [tokenId]);
  const menuKind = token && tokenId !== dismissed ? token.kind : null;

  const [cmds, setCmds] = React.useState<{ agent: LiveAgent; list: CommandOption[] | null; failed: boolean } | null>(null);
  const cmdsAgent = React.useRef<LiveAgent | null>(null);
  React.useEffect(() => {
    if (menuKind !== "slash" || cmdsAgent.current === agent) return;
    cmdsAgent.current = agent;
    setCmds({ agent, list: null, failed: false });
    loadCommands(agent)
      .then((list) => cmdsAgent.current === agent && setCmds({ agent, list, failed: false }))
      .catch(() => {
        if (cmdsAgent.current !== agent) return;
        // Asked again the next time the menu opens.
        cmdsAgent.current = null;
        setCmds({ agent, list: [], failed: true });
      });
  }, [menuKind, agent]);

  const fileQuery = menuKind === "at" ? token!.query : null;
  const [files, setFiles] = React.useState<{ query: string; list: ProjectFile[] | null; error: string | null }>({ query: "", list: null, error: null });
  React.useEffect(() => {
    if (fileQuery === null || !cwd) return;
    let alive = true;
    const t = setTimeout(() => {
      projectFiles(cwd, fileQuery)
        .then((list) => alive && setFiles({ query: fileQuery, list, error: null }))
        .catch((e) => alive && setFiles({ query: fileQuery, list: [], error: errorMessage(e) }));
    }, 120);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [fileQuery, cwd]);

  const settingValue = (p: PickerKind): string => {
    switch (p) {
      case "model":
        return modelName(models, model, c);
      case "effort":
        return effort || k.effortDefault;
      case "permissions":
        return c.permissions[permission];
      case "agent":
        return c.agents[agent];
      case "cwd":
        return cwd ? baseName(cwd) : k.cwdNone;
      case "args":
        return args || k.argsNone;
      case "worktree":
        return wtOn ? w.on : w.off;
    }
  };

  const items: MenuItem[] = React.useMemo(() => {
    if (!menuKind || !token) return [];
    if (menuKind === "at") {
      return (files.list ?? []).map((f) => ({ id: `f-${f.path}`, type: "file" as const, file: f }));
    }
    const q = token.query.toLowerCase();
    const kinds: PickerKind[] = draft
      ? ["model", "effort", "permissions", "agent", "cwd", ...(wtAvail ? (["worktree"] as const) : []), "args"]
      : ["model", "effort", "permissions", "args"];
    const settings: MenuItem[] = kinds
      .filter((p) => SETTING_NAMES[p].includes(q))
      .map((p) => ({ id: `s-${p}`, type: "setting", picker: p, name: SETTING_NAMES[p], description: settingDescription(p, c), value: settingValue(p), icon: SETTING_ICONS[p] }));
    const cli: MenuItem[] = filterCommands(cmds?.agent === agent ? (cmds.list ?? []) : [], token.query)
      .slice(0, 100)
      .map((x) => ({ id: `c-${x.name}`, type: "cli", cmd: x }));
    return [...settings, ...cli];
    // settingValue reads the current settings, all listed below.
  }, [menuKind, token?.query, files.list, cmds, agent, draft, model, effort, permission, args, cwd, models, k, c, wtAvail, wtOn]);

  const [active, setActive] = React.useState(0);
  React.useEffect(() => setActive(0), [menuKind, token?.query]);
  const activeIndex = Math.min(active, Math.max(0, items.length - 1));
  const activeId = items[activeIndex] ? `composer-menu-${activeIndex}` : undefined;
  React.useEffect(() => {
    if (activeId) document.getElementById(activeId)?.scrollIntoView?.({ block: "nearest" });
  }, [activeId]);

  const choose = (item: MenuItem) => {
    if (!token) return;
    if (item.type === "setting") {
      const r = replaceToken(text, token, "");
      setText(r.text);
      if (item.picker === "worktree") {
        toggleWorktree();
        focusText(r.caret);
        return;
      }
      setCaret(r.caret);
      setPicker(item.picker);
      return;
    }
    const insert = item.type === "cli" ? `/${item.cmd.name} ` : `${quotePath(item.file.path)} `;
    const r = replaceToken(text, token, insert);
    setText(r.text);
    focusText(r.caret);
  };

  const addImages = React.useCallback((uris: string[]) => {
    if (uris.length) setImages((xs) => [...xs, ...uris]);
  }, []);

  /** Inserts paths at the caret (relative to `cwd` when inside it). */
  const insertPaths = React.useCallback(
    (paths: string[]) => {
      if (!paths.length) return;
      const el = ref.current;
      const cur = textRef.current;
      const start = el?.selectionStart ?? cur.length;
      const end = el?.selectionEnd ?? cur.length;
      const r = insertAtCursor(cur, start, end, paths.map((p) => formatInsertPath(p, cwd)).join(" "));
      setText(r.text);
      requestAnimationFrame(() => {
        if (!ref.current) return;
        ref.current.focus();
        ref.current.setSelectionRange(r.caret, r.caret);
        setCaret(r.caret);
      });
    },
    [cwd],
  );

  const loadImageFiles = React.useCallback(
    async (paths: string[]) => {
      const out: string[] = [];
      for (const p of paths) {
        try {
          out.push(await readImageDataUri(p));
        } catch (e) {
          onError(fmt(k.imageFailed, { error: errorMessage(e) }));
        }
      }
      addImages(out);
    },
    [addImages, onError, k.imageFailed],
  );

  /** Dropped / picked paths: images become attachments, everything else is inserted as a path. */
  const handlePaths = React.useCallback(
    (paths: string[]) => {
      const imgs = paths.filter(isImagePath);
      insertPaths(paths.filter((p) => !isImagePath(p)));
      if (imgs.length) void loadImageFiles(imgs);
    },
    [insertPaths, loadImageFiles],
  );

  // Tauri hands dropped files to the webview as paths (HTML5 drop has none). Only react while the
  // pointer is over this composer's pane (hidden pages have an empty rect).
  const disabledRef = React.useRef(disabled);
  disabledRef.current = disabled;
  const handlePathsRef = React.useRef(handlePaths);
  handlePathsRef.current = handlePaths;
  React.useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    const over = (pos: { x: number; y: number }) => {
      const zone = rootRef.current?.parentElement ?? rootRef.current;
      if (!zone || disabledRef.current) return false;
      const r = zone.getBoundingClientRect();
      return r.width > 0 && r.height > 0 && inRect(pos, r, window.devicePixelRatio);
    };
    try {
      getCurrentWebview()
        .onDragDropEvent(({ payload }) => {
          if (payload.type === "leave") setDragging(false);
          else if (payload.type === "enter" || payload.type === "over") setDragging(over(payload.position));
          else if (payload.type === "drop") {
            setDragging(false);
            if (over(payload.position)) handlePathsRef.current(payload.paths);
          }
        })
        .then((f) => {
          if (alive) unlisten = f;
          else f();
        })
        .catch(() => {});
    } catch {
      // not running inside Tauri (tests, plain browser)
    }
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  const onPaste = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(e.clipboardData?.files ?? []).filter((f) => f.type.startsWith("image/"));
    if (!files.length) return;
    e.preventDefault();
    void (async () => {
      const out: string[] = [];
      for (const f of files) {
        if (f.size > MAX_IMAGE_BYTES) {
          onError(fmt(k.imageTooLarge, { name: f.name || "image" }));
          continue;
        }
        try {
          out.push(await fileToDataUri(f));
        } catch (err) {
          onError(fmt(k.imageFailed, { error: errorMessage(err) }));
        }
      }
      addImages(out);
    })();
  };

  const pickFiles = async (wantImages: boolean) => {
    try {
      const r = await openDialog(
        wantImages ? { multiple: true, filters: [{ name: k.imageFilter, extensions: IMAGE_EXTS }] } : { multiple: true, defaultPath: cwd || undefined },
      );
      const paths = r === null ? [] : Array.isArray(r) ? r : [r];
      if (wantImages) void loadImageFiles(paths);
      else handlePaths(paths);
    } catch (e) {
      onError(fmt(k.pickFailed, { error: errorMessage(e) }));
    }
  };

  const canSend = !disabled && !sending && !!cwd && (text.trim().length > 0 || images.length > 0);
  const send = async () => {
    if (!canSend) return;
    setSending(true);
    onError(null);
    // The key may move to the started conversation during the send: clear it first, restore on failure.
    const snap = { text, images };
    const settings: AgentSettings = { permission, model: model.trim(), effort, args };
    patchConvUi(stateKey, { text: "", images: [], settings: perConv ? (convSettings ?? settings) : settings });
    try {
      const r = await onSend({ text: text.trim(), images, permission, model: model.trim(), effort, args: argList });
      if (r === false) {
        patchConvUi(stateKey, snap);
        return;
      }
      setText("");
      setCaret(0);
      setImages([]);
    } catch (e) {
      patchConvUi(stateKey, snap);
      onError(fmt(c.sendFailed, { error: errorMessage(e) }));
    } finally {
      setSending(false);
      ref.current?.focus();
    }
  };

  const predicted = !disabled && !running && !sending && !picker && !text && !images.length && suggestion !== dismissedSuggestion ? suggestion : undefined;
  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // IME composition (e.g. Chinese input) uses Enter to pick a candidate.
    const composing = e.nativeEvent.isComposing || e.keyCode === 229;
    if (menuKind && !composing) {
      if (e.key === "Escape") {
        e.preventDefault();
        setDismissed(tokenId);
        return;
      }
      if (items.length && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
        e.preventDefault();
        setActive((i) => (e.key === "ArrowDown" ? (i + 1) % items.length : (i - 1 + items.length) % items.length));
        return;
      }
      if (items.length && ((e.key === "Enter" && !e.shiftKey) || e.key === "Tab")) {
        e.preventDefault();
        choose(items[activeIndex]);
        return;
      }
    }
    if (predicted && !composing && !e.shiftKey && !e.altKey && !e.ctrlKey && !e.metaKey) {
      if (e.key === "Tab" || e.key === "Escape") {
        e.preventDefault();
        setDismissedSuggestion(predicted);
        if (e.key === "Tab") {
          setText(predicted);
          setCaret(predicted.length);
        }
        return;
      }
    }
    if (e.key !== "Enter" || e.shiftKey || composing) return;
    e.preventDefault();
    void send();
  };

  const syncCaret = (e: React.SyntheticEvent<HTMLTextAreaElement>) => setCaret(e.currentTarget.selectionStart ?? 0);

  const menu = menuKind ? (
    <div className="absolute bottom-full left-0 z-40 mb-2 w-[400px] max-w-full overflow-hidden rounded-lg border border-border bg-popover text-popover-foreground shadow-lg">
      <div id="composer-menu" role="listbox" aria-label={menuKind === "slash" ? k.menuLabel : k.filesLabel} className="flex max-h-80 flex-col overflow-y-auto p-1">
        {menuKind === "slash" ? (
          <>
            {items.some((x) => x.type === "setting") ? <p className="px-2 pt-1 pb-0.5 text-[11px] text-muted-foreground">{k.menuSettings}</p> : null}
            {items.map((item, i) =>
              item.type === "setting" ? (
                <MenuRow key={item.id} index={i} active={i === activeIndex} onHover={setActive} onChoose={() => choose(item)}>
                  <span className="mt-0.5 text-muted-foreground [&_svg]:size-4">{item.icon}</span>
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="font-mono text-[13px] font-semibold">/{item.name}</span>
                    <span className="truncate text-[11px] text-muted-foreground">{item.description}</span>
                  </span>
                  <span className={cn("max-w-32 shrink-0 truncate text-[11px] text-muted-foreground", item.picker === "permissions" && permission === "bypass" && "text-amber-700 dark:text-amber-300")}>
                    {item.value}
                  </span>
                </MenuRow>
              ) : null,
            )}
            <p className="px-2 pt-1.5 pb-0.5 text-[11px] text-muted-foreground">{fmt(k.menuCli, { agent: c.agents[agent] })}</p>
            {items.map((item, i) =>
              item.type === "cli" ? (
                <MenuRow key={item.id} index={i} active={i === activeIndex} onHover={setActive} onChoose={() => choose(item)}>
                  <span className="mt-0.5 text-muted-foreground [&_svg]:size-4">{item.cmd.kind === "skill" ? <Puzzle aria-hidden="true" /> : <SquareSlash aria-hidden="true" />}</span>
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="truncate font-mono text-[13px] font-semibold">
                      /{item.cmd.name}
                      {item.cmd.argument_hint ? <span className="ml-1.5 font-normal text-muted-foreground">{item.cmd.argument_hint}</span> : null}
                    </span>
                    {item.cmd.description ? <span className="truncate text-[11px] text-muted-foreground">{item.cmd.description}</span> : null}
                  </span>
                  <span className="shrink-0 text-[10px] text-muted-foreground">{item.cmd.kind === "skill" ? k.kindSkill : k.kindCommand}</span>
                </MenuRow>
              ) : null,
            )}
            {cmds?.agent !== agent || cmds.list === null ? (
              <p className="flex items-center gap-1.5 px-2 py-1.5 text-[11px] text-muted-foreground">
                <Loader2 className="size-3 animate-spin motion-reduce:animate-none" aria-hidden="true" />
                {k.menuLoading}
              </p>
            ) : cmds.failed ? (
              <p className="px-2 py-1.5 text-[11px] text-muted-foreground">{k.menuFailed}</p>
            ) : !items.some((x) => x.type === "cli") ? (
              <p className="px-2 py-1.5 text-[11px] text-muted-foreground">{k.menuEmpty}</p>
            ) : null}
          </>
        ) : !cwd ? (
          <p className="px-2 py-1.5 text-[11px] text-muted-foreground">{k.cwdMissing}</p>
        ) : (
          <>
            {items.map((item, i) =>
              item.type === "file" ? (
                <MenuRow key={item.id} index={i} active={i === activeIndex} onHover={setActive} onChoose={() => choose(item)}>
                  <span className="mt-0.5 text-muted-foreground [&_svg]:size-4">{item.file.is_dir ? <Folder aria-hidden="true" /> : <FileIcon aria-hidden="true" />}</span>
                  <span className="min-w-0 flex-1 truncate font-mono text-[12px]">{item.file.path}</span>
                </MenuRow>
              ) : null,
            )}
            {files.list === null || files.query !== fileQuery ? (
              items.length === 0 ? (
                <p className="flex items-center gap-1.5 px-2 py-1.5 text-[11px] text-muted-foreground">
                  <Loader2 className="size-3 animate-spin motion-reduce:animate-none" aria-hidden="true" />
                  {k.filesLoading}
                </p>
              ) : null
            ) : files.error ? (
              <p className="px-2 py-1.5 text-[11px] text-destructive">{fmt(k.filesFailed, { error: files.error })}</p>
            ) : items.length === 0 ? (
              <p className="px-2 py-1.5 text-[11px] text-muted-foreground">{k.filesEmpty}</p>
            ) : null}
          </>
        )}
      </div>
    </div>
  ) : null;

  const pickerBody = (() => {
    switch (picker) {
      case "model":
        return (
          <ModelPicker
            agent={agent}
            value={model}
            models={models}
            loading={modelsLoading}
            failed={modelsFailed}
            onRefresh={() => fetchModels(true)}
            onPick={(m) => {
              setSetting({ model: m.trim() });
              closePicker();
            }}
            c={c}
          />
        );
      case "effort":
        return (
          <EffortPicker
            value={effort}
            model={curModel}
            modelsLoaded={models !== null}
            onPick={(v) => {
              setSetting({ effort: v });
              closePicker();
            }}
            c={c}
          />
        );
      case "permissions":
        return (
          <PermissionPicker
            value={permission}
            onPick={(m) => {
              setSetting({ permission: m });
              closePicker();
            }}
            c={c}
          />
        );
      case "agent":
        return <AgentPicker value={agent} onPick={pickAgent} c={c} />;
      case "cwd":
        return <CwdPicker value={cwd} projects={projects} onPick={pickCwd} onBrowse={() => void browseCwd()} c={c} />;
      case "args":
        return (
          <ArgsPicker
            agent={agent}
            value={args}
            onSave={(v) => {
              setSetting({ args: v });
              closePicker();
            }}
            c={c}
          />
        );
      default:
        return null;
    }
  })();

  const modelLabel = modelName(models, model, c, attached?.model ?? null);

  return (
    <div ref={rootRef} className="flex flex-col gap-1.5 px-4 pt-2 pb-3">
      <Popover open={picker !== null} onOpenChange={(o) => !o && closePicker()}>
        <PopoverAnchor asChild>
          <div className="relative">
            {menu}
            <div
              className={cn(
                "relative rounded-[14px] border border-border bg-card shadow-sm transition-[border-color,box-shadow] duration-150 motion-reduce:transition-none",
                "focus-within:border-primary focus-within:ring-3 focus-within:ring-primary/15",
                disabled && "opacity-70",
              )}
            >
              {!cwd ? (
                <div className="flex items-center gap-2 border-b border-border/70 px-3.5 py-2 text-xs text-muted-foreground">
                  <FolderOpen className="size-3.5 shrink-0" aria-hidden="true" />
                  <span className="min-w-0 flex-1">{k.cwdMissing}</span>
                  {draft ? (
                    <Button type="button" size="xs" variant="outline" className="cursor-pointer" onClick={() => setPicker("cwd")}>
                      {k.cwdNone}
                    </Button>
                  ) : null}
                </div>
              ) : null}
              {images.length ? (
                <ul aria-label={k.attachImage} className="flex flex-wrap gap-2 px-3.5 pt-3">
                  {images.map((src, i) => (
                    <li key={i} className="relative">
                      <img src={src} alt={fmt(k.imageAlt, { n: i + 1 })} className="block size-14 rounded-lg border border-border object-cover" />
                      <button
                        type="button"
                        aria-label={fmt(k.removeImage, { n: i + 1 })}
                        onClick={() => setImages((xs) => xs.filter((_, j) => j !== i))}
                        className="absolute -top-1.5 -right-1.5 grid size-[18px] cursor-pointer place-items-center rounded-full bg-foreground text-background shadow outline-none hover:opacity-85 focus-visible:ring-2 focus-visible:ring-ring"
                      >
                        <X className="size-3" aria-hidden="true" />
                      </button>
                    </li>
                  ))}
                </ul>
              ) : null}
              <Textarea
                ref={ref}
                aria-label={c.inputLabel}
                aria-controls={menuKind ? "composer-menu" : undefined}
                aria-activedescendant={menuKind ? activeId : undefined}
                value={text}
                disabled={disabled}
                rows={3}
                placeholder={predicted ?? placeholder ?? fmt(k.placeholder, { agent: c.agents[agent] })}
                onChange={(e) => {
                  if (predicted) setDismissedSuggestion(predicted);
                  setText(e.target.value);
                  setCaret(e.target.selectionStart ?? e.target.value.length);
                }}
                onSelect={syncCaret}
                onClick={syncCaret}
                onPaste={onPaste}
                onKeyDown={onKeyDown}
                onBlur={() => setDismissed(tokenId)}
                onFocus={() => setDismissed(null)}
                className="max-h-[280px] min-h-[84px] resize-none overflow-y-auto rounded-none border-0 bg-transparent px-3.5 pt-3 pb-1 leading-5 shadow-none focus-visible:ring-0 disabled:bg-transparent dark:bg-transparent dark:disabled:bg-transparent"
              />
              <div className="flex min-w-0 items-center gap-1 px-2 pt-1 pb-2">
                {predicted ? <kbd className="mx-1 rounded border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground">Tab</kbd> : null}
                <Tip tip={k.attachFile}>
                  <Button type="button" variant="ghost" size="icon-sm" className="cursor-pointer text-muted-foreground" aria-label={k.attachFile} disabled={disabled} onClick={() => void pickFiles(false)}>
                    <Paperclip aria-hidden="true" />
                  </Button>
                </Tip>
                <Tip tip={k.attachImage}>
                  <Button type="button" variant="ghost" size="icon-sm" className="cursor-pointer text-muted-foreground" aria-label={k.attachImage} disabled={disabled} onClick={() => void pickFiles(true)}>
                    <ImagePlus aria-hidden="true" />
                  </Button>
                </Tip>
                <button
                  type="button"
                  aria-label={k.model}
                  title={changed.has("model") ? k.nextTimeHint : k.cmdModel}
                  aria-expanded={picker === "model"}
                  onClick={() => setPicker(picker === "model" ? null : "model")}
                  className={cn(
                    "inline-flex h-7 max-w-44 min-w-0 cursor-pointer items-center gap-1 rounded-md px-1.5 text-xs text-muted-foreground outline-none transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring",
                    picker === "model" && "bg-muted text-foreground",
                  )}
                >
                  <span className="truncate">{modelLabel}</span>
                  {changed.has("model") ? <span className="shrink-0 rounded bg-amber-500/12 px-1 text-[10px] text-amber-700 dark:text-amber-300">{k.nextTime}</span> : null}
                  <ChevronDown className="size-3 shrink-0 opacity-60" aria-hidden="true" />
                </button>
                {meters}
                <span className="flex-1" />
                {running && onStop ? (
                  <Tip tip={stopHint ?? c.stopHint}>
                    <Button type="button" variant="destructive" size="icon-sm" className="cursor-pointer rounded-full" aria-label={stopLabel ?? c.stop} disabled={stopping} onClick={onStop}>
                      {stopping ? <Loader2 className="size-3 animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Square className="size-3 fill-current" aria-hidden="true" />}
                    </Button>
                  </Tip>
                ) : null}
                <Button type="button" size="icon-sm" className="cursor-pointer rounded-full disabled:opacity-40" aria-label={c.send} disabled={!canSend} onClick={() => void send()}>
                  {sending ? <Loader2 className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <ArrowUp aria-hidden="true" />}
                </Button>
              </div>
              {dragging ? (
                <div
                  aria-hidden="true"
                  className="pointer-events-none absolute inset-0 grid place-items-center rounded-[14px] border-2 border-dashed border-primary bg-primary/10 text-center backdrop-blur-[1px]"
                >
                  <div className="flex flex-col gap-0.5 rounded-lg bg-card/85 px-3 py-1.5">
                    <span className="text-sm font-semibold text-primary">{k.dropTitle}</span>
                    <span className="text-[11px] text-muted-foreground">{k.dropHint}</span>
                  </div>
                </div>
              ) : null}
            </div>
          </div>
        </PopoverAnchor>
        <PopoverContent
          side="top"
          align="start"
          className={cn("flex flex-col gap-0 overflow-hidden p-0", picker === "model" || picker === "cwd" ? "w-80" : picker === "args" ? "w-96" : "w-72")}
          onOpenAutoFocus={(e) => {
            // Pickers focus their own selected option / input.
            e.preventDefault();
          }}
          onCloseAutoFocus={(e) => e.preventDefault()}
        >
          {pickerBody}
        </PopoverContent>
      </Popover>
      <div aria-label={k.statusLabel} className="flex min-w-0 items-center gap-1.5 px-1 text-[11px] text-muted-foreground">
        <StatusItem label={`${k.agentTitle}: ${c.agents[agent]}`} onClick={draft ? () => setPicker("agent") : undefined} k={k}>
          {c.agents[agent]}
        </StatusItem>
        <span aria-hidden="true">·</span>
        <StatusItem label={`${k.cwdTitle}: ${cwd || k.cwdNone}`} onClick={draft ? () => setPicker("cwd") : undefined} warn={!cwd} k={k}>
          <span title={cwd || undefined}>{cwd ? baseName(cwd) : k.cwdNone}</span>
        </StatusItem>
        <span aria-hidden="true">·</span>
        <StatusItem label={`${k.effortTitle}: ${effort || k.effortDefault}`} onClick={() => setPicker("effort")} next={changed.has("effort")} k={k}>
          {fmt(k.effortStatus, { level: effort || k.effortDefault })}
        </StatusItem>
        <span aria-hidden="true">·</span>
        <StatusItem label={`${k.permissionTitle}: ${c.permissions[permission]}`} onClick={() => setPicker("permissions")} warn={permission === "bypass"} next={changed.has("permission") && running && permission !== attached?.permission} title={k.nextTurnPermission} k={k}>
          {c.permissions[permission]}
        </StatusItem>
        {worktreeOn ? (
          <>
            <span aria-hidden="true">·</span>
            <StatusItem label={`${w.cmd}: ${w.on}`} title={w.statusHint} onClick={toggleWorktree} violet k={k}>
              {w.status}
            </StatusItem>
          </>
        ) : null}
        {argList.length ? (
          <>
            <span aria-hidden="true">·</span>
            <StatusItem label={`${k.argsTitle}: ${args}`} onClick={() => setPicker("args")} next={changed.has("args")} k={k}>
              <span title={args}>{fmt(k.argsStatus, { n: argList.length })}</span>
            </StatusItem>
          </>
        ) : null}
        <span className="flex-1" />
        <span className="hidden shrink-0 text-muted-foreground/80 sm:inline">{k.keysHint}</span>
      </div>
      {hint ? <p className="px-1 text-[11px] text-muted-foreground">{hint}</p> : null}
    </div>
  );
}

function MenuRow({ index, active, onHover, onChoose, children }: { index: number; active: boolean; onHover: (i: number) => void; onChoose: () => void; children: React.ReactNode }) {
  return (
    <div
      id={`composer-menu-${index}`}
      role="option"
      aria-selected={active}
      onMouseDown={(e) => e.preventDefault()}
      onMouseEnter={() => onHover(index)}
      onClick={onChoose}
      className={cn("flex cursor-pointer items-start gap-2 rounded-md px-2 py-1.5 text-sm", active && "bg-muted")}
    >
      {children}
    </div>
  );
}
