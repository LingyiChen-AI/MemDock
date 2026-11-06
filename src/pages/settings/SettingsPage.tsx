import * as React from "react";
import {
  CircleCheck,
  CircleDashed,
  Copy,
  Download,
  FolderOpen,
  Gauge,
  GitBranch,
  Loader2,
  Monitor,
  Palette,
  RefreshCw,
  Settings as SettingsIcon,
  Sparkles,
  TerminalSquare,
  TriangleAlert,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { envDetect, envInstall, onInstallDone, onInstallLog, openDataDir } from "@/api/env";
import { getSettings } from "@/api/memory";
import { setClaudeQuotaEnabled } from "@/api/usage";
import type { EnvReport, EnvToolStatus } from "@/api/types";
import { AppearanceControls } from "@/components/appearance/AppearanceControls";
import { Button } from "@/components/ui/button";
import type { NavKey } from "@/layout/AppShell";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";
import { ToastRegion, useToasts } from "@/pages/skills/Toasts";
import { asCommandError, errorMessage } from "@/pages/skills/skillUtils";

const NODE_URL = "https://nodejs.org";

function Section({ icon: Icon, title, desc, children, action }: { icon: React.ComponentType<{ className?: string }>; title: string; desc?: string; children?: React.ReactNode; action?: React.ReactNode }) {
  return (
    <section className="flex flex-col gap-3 rounded-lg border border-border bg-card p-4">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-1">
          <h2 className="flex items-center gap-2 text-sm font-semibold">
            <Icon className="size-4 text-muted-foreground" aria-hidden="true" />
            {title}
          </h2>
          {desc ? <p className="text-xs text-muted-foreground">{desc}</p> : null}
        </div>
        {action}
      </header>
      {children}
    </section>
  );
}

function CopyButton({ text, label, copiedLabel }: { text: string; label: string; copiedLabel: string }) {
  const [copied, setCopied] = React.useState(false);
  return (
    <Button
      type="button"
      variant="ghost"
      size="xs"
      className="cursor-pointer text-muted-foreground"
      onClick={() => {
        void navigator.clipboard?.writeText(text).then(() => {
          setCopied(true);
          window.setTimeout(() => setCopied(false), 1500);
        });
      }}
    >
      <Copy aria-hidden="true" />
      {copied ? copiedLabel : label}
    </Button>
  );
}

function ToolRow({
  tool,
  npmOk,
  installing,
  onInstall,
}: {
  tool: EnvToolStatus;
  npmOk: boolean;
  installing: string | null;
  onInstall: (name: "claude" | "codex") => void;
}) {
  const { t } = useI18n();
  const e = t.settings.env;
  const installable = tool.name === "claude" || tool.name === "codex";
  const busy = installing === tool.name;
  return (
    <li className="flex flex-col gap-1.5 border-b border-border py-3 last:border-b-0">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span className="w-28 shrink-0 text-sm font-medium">{e.names[tool.name]}</span>
        {tool.installed ? (
          <span className="inline-flex items-center gap-1 text-xs text-accent-text">
            <CircleCheck className="size-3.5" aria-hidden="true" />
            {e.installed}
          </span>
        ) : (
          <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
            <CircleDashed className="size-3.5" aria-hidden="true" />
            {e.notInstalled}
          </span>
        )}
        {tool.version ? <span className="text-xs text-muted-foreground tabular-nums">{fmt(e.version, { v: tool.version })}</span> : null}
        {tool.logged_in !== null ? (
          <span className={cn("text-xs", tool.logged_in ? "text-accent-text" : "text-warning")}>{tool.logged_in ? e.loggedIn : e.notLoggedIn}</span>
        ) : null}
        <span className="flex-1" />
        {installable && !tool.installed ? (
          <Button type="button" variant="outline" size="sm" disabled={!npmOk || installing !== null} onClick={() => onInstall(tool.name as "claude" | "codex")} className="cursor-pointer">
            {busy ? <Loader2 className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Download aria-hidden="true" />}
            {busy ? e.installing : e.install}
          </Button>
        ) : null}
      </div>
      {tool.path ? (
        <p className="truncate pl-0 font-mono text-xs text-muted-foreground sm:pl-31" title={tool.path}>
          {tool.path}
        </p>
      ) : null}
      {tool.name === "codex" && !tool.installed && tool.desktop_app ? (
        <p className="flex items-center gap-1.5 text-xs text-warning sm:pl-31">
          <Monitor className="size-3.5 shrink-0" aria-hidden="true" />
          {e.desktopOnly}
        </p>
      ) : null}
      {installable && tool.installed && tool.logged_in === false ? (
        <p className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground sm:pl-31">
          <TerminalSquare className="size-3.5 shrink-0" aria-hidden="true" />
          {fmt(e.loginHint, { cmd: tool.name })}
          <CopyButton text={tool.name} label={e.copy} copiedLabel={e.copied} />
        </p>
      ) : null}
    </li>
  );
}

export interface SettingsPageProps {
  onNavigate?: (key: NavKey) => void;
}

export function SettingsPage({ onNavigate }: SettingsPageProps) {
  const { t } = useI18n();
  const s = t.settings;
  const [env, setEnv] = React.useState<EnvReport | null>(null);
  const [checking, setChecking] = React.useState(false);
  const [installing, setInstalling] = React.useState<string | null>(null);
  const [log, setLog] = React.useState<string[]>([]);
  const [quotaOn, setQuotaOn] = React.useState<boolean | null>(null);
  const { toasts, push, dismiss } = useToasts();
  const logRef = React.useRef<HTMLPreElement>(null);

  const detect = React.useCallback(async () => {
    setChecking(true);
    try {
      const r = await envDetect();
      setEnv(r);
      setInstalling(r.installing);
    } catch (e) {
      push("error", errorMessage(e));
    } finally {
      setChecking(false);
    }
  }, [push]);

  const nameOf = React.useCallback((tool: string) => (s.env.names as Record<string, string>)[tool] ?? tool, [s.env.names]);
  const nameRef = React.useRef(nameOf);
  nameRef.current = nameOf;
  const sRef = React.useRef(s);
  sRef.current = s;

  React.useEffect(() => {
    void detect();
    void getSettings()
      .then((x) => setQuotaOn(Boolean(x.claude_quota_enabled)))
      .catch(() => setQuotaOn(false));
    const offs: (() => void)[] = [];
    let alive = true;
    const keep = (p: Promise<() => void>) =>
      void p.then((f) => {
        if (alive) offs.push(f);
        else f();
      });
    keep(onInstallLog((ev) => setLog((l) => [...l.slice(-400), ev.line])));
    keep(
      onInstallDone((ev) => {
        setInstalling(null);
        const name = nameRef.current(ev.tool);
        if (ev.ok) push("success", fmt(sRef.current.env.installOk, { name }));
        else push("error", fmt(sRef.current.env.installFailed, { name, error: ev.error ?? `exit ${ev.code ?? "?"}` }));
        void detect();
      }),
    );
    return () => {
      alive = false;
      offs.forEach((f) => f());
    };
  }, [detect, push]);

  React.useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [log]);

  const install = async (tool: "claude" | "codex") => {
    setLog([]);
    setInstalling(tool);
    try {
      await envInstall(tool);
    } catch (e) {
      setInstalling(null);
      const code = asCommandError(e)?.code;
      push("error", code === "npm_missing" ? s.env.npmMissing : code === "busy" ? s.env.busy : errorMessage(e));
    }
  };

  const toggleQuota = async () => {
    const next = !quotaOn;
    try {
      const r = await setClaudeQuotaEnabled(next);
      setQuotaOn(Boolean(r.claude_quota_enabled));
    } catch (e) {
      push("error", errorMessage(e));
    }
  };

  const openData = async () => {
    try {
      await openDataDir();
    } catch (e) {
      push("error", fmt(s.data.openFailed, { error: errorMessage(e) }));
    }
  };

  const npmOk = env?.tools.some((x) => x.name === "npm" && x.installed) ?? false;

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex max-w-4xl flex-col gap-4 p-6">
        <h1 className="flex items-center gap-2 text-lg font-semibold">
          <SettingsIcon className="size-5 text-muted-foreground" aria-hidden="true" />
          {s.title}
        </h1>

        <Section
          icon={TerminalSquare}
          title={s.env.title}
          desc={s.env.desc}
          action={
            <Button type="button" variant="outline" size="sm" disabled={checking} onClick={() => void detect()} className="cursor-pointer">
              <RefreshCw className={cn(checking && "animate-spin motion-reduce:animate-none")} aria-hidden="true" />
              {checking ? s.env.checking : s.env.recheck}
            </Button>
          }
        >
          {env ? (
            <ul className="flex flex-col">
              {env.tools.map((tool) => (
                <ToolRow key={tool.name} tool={tool} npmOk={npmOk} installing={installing} onInstall={(n) => void install(n)} />
              ))}
            </ul>
          ) : (
            <div className="h-40 animate-pulse rounded-md bg-muted motion-reduce:animate-none" />
          )}
          {env && !npmOk ? (
            <p className="flex flex-wrap items-center gap-2 rounded-md bg-warning/15 px-3 py-2 text-sm">
              <TriangleAlert className="size-4 shrink-0 text-warning" aria-hidden="true" />
              {s.env.npmMissing}
              <button
                type="button"
                className="cursor-pointer font-mono text-xs text-primary underline underline-offset-2"
                onClick={() => void openUrl(NODE_URL).catch(() => undefined)}
              >
                {NODE_URL}
              </button>
              <CopyButton text={NODE_URL} label={s.env.copy} copiedLabel={s.env.copied} />
            </p>
          ) : null}
          {log.length > 0 ? (
            <div className="flex flex-col gap-1">
              <span className="text-xs text-muted-foreground">{s.env.log}</span>
              <pre ref={logRef} aria-live="polite" className="max-h-48 overflow-auto rounded-md bg-muted px-3 py-2 font-mono text-xs leading-5 whitespace-pre-wrap">
                {log.join("\n")}
              </pre>
            </div>
          ) : null}
        </Section>

        <Section icon={Palette} title={s.appearance.title}>
          <AppearanceControls />
        </Section>

        <Section icon={Gauge} title={s.quota.title} desc={t.usage.quota.explain}>
          <label className="flex cursor-pointer items-center gap-3 text-sm">
            <Switch checked={quotaOn === true} disabled={quotaOn === null} onCheckedChange={() => void toggleQuota()} />
            {s.quota.toggle}
          </label>
        </Section>

        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          <Section
            icon={Sparkles}
            title={s.extract.title}
            desc={s.extract.desc}
            action={
              <Button type="button" variant="outline" size="sm" onClick={() => onNavigate?.("extract")} className="cursor-pointer">
                {s.extract.open}
              </Button>
            }
          />
          <Section
            icon={GitBranch}
            title={s.sync.title}
            desc={s.sync.desc}
            action={
              <Button type="button" variant="outline" size="sm" onClick={() => onNavigate?.("sync")} className="cursor-pointer">
                {s.sync.open}
              </Button>
            }
          />
        </div>

        <Section
          icon={FolderOpen}
          title={s.data.title}
          desc={s.data.desc}
          action={
            <Button type="button" variant="outline" size="sm" onClick={() => void openData()} className="cursor-pointer">
              <FolderOpen aria-hidden="true" />
              {s.data.open}
            </Button>
          }
        >
          {env ? (
            <p className="flex min-w-0 items-center gap-1">
              <span className="truncate font-mono text-xs text-muted-foreground" title={env.data_dir}>
                {env.data_dir}
              </span>
              <CopyButton text={env.data_dir} label={s.data.copy} copiedLabel={s.env.copied} />
            </p>
          ) : null}
        </Section>
      </div>
      <ToastRegion toasts={toasts} onDismiss={dismiss} />
    </div>
  );
}
