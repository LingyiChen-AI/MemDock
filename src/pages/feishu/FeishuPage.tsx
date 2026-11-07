import * as React from "react";
import { Bot, Folder, Link2, LoaderCircle, Play, Plus, RefreshCw, Square } from "lucide-react";
import {
  listFeishuBots, listFeishuProjects, removeFeishuBot, saveFeishuBot,
  startFeishuBot, stopFeishuBot, testFeishuBot,
  type FeishuBot, type FeishuBotInput, type FeishuProject, type FeishuProjectGrant,
} from "@/api/feishu";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { SimpleSelect } from "@/components/ui/select";
import { useI18n } from "@/i18n/I18nProvider";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { fmt } from "@/pages/memory/paths";
import { errorMessage } from "@/pages/skills/skillUtils";
import { cn } from "@/lib/utils";

type Labels = ReturnType<typeof useI18n>["t"]["feishu"];
type Permission = Exclude<keyof FeishuProjectGrant, "path">;
type ConnectionAction = "start" | "stop" | "test" | "remove";
const permissions: Permission[] = ["read", "create", "edit", "delete"];
const checkClass = "size-4 shrink-0 cursor-pointer accent-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-default disabled:opacity-50";
const splitIds = (input: string) => [...new Set(input.split(/[\s,，]+/).filter(Boolean))];
const emptyInput: FeishuBotInput = {
  name: "", appId: "", tenantKey: "", allowedUsers: [], allowedGroups: [],
  allowPrivate: true, engine: "codex", projects: [],
};

export function FeishuPage() {
  const { t: { feishu: l } } = useI18n();
  const [bots, setBots] = React.useState<FeishuBot[]>([]);
  const [projects, setProjects] = React.useState<FeishuProject[]>([]);
  const [loaded, setLoaded] = React.useState(false);
  const [error, setError] = React.useState("");
  const [notice, setNotice] = React.useState("");
  const [editing, setEditing] = React.useState<FeishuBot | "new" | null>(null);
  const [removing, setRemoving] = React.useState<FeishuBot | null>(null);
  const [busy, setBusy] = React.useState<{ id: string; action: ConnectionAction } | null>(null);
  const request = React.useRef(0);
  const refreshing = React.useRef(false);

  const refresh = React.useCallback(async (includeProjects: boolean, silent = false) => {
    if (silent && refreshing.current) return;
    const version = ++request.current;
    refreshing.current = true;
    try {
      const [items, available] = await Promise.all([
        listFeishuBots(),
        includeProjects ? listFeishuProjects() : Promise.resolve(null),
      ]);
      if (request.current !== version) return;
      setBots(items);
      if (available) setProjects(available);
      setLoaded(true);
      if (!silent) setError("");
    } catch (e) {
      if (request.current === version && !silent) setError(errorMessage(e));
    } finally {
      if (request.current === version) refreshing.current = false;
    }
  }, []);

  React.useEffect(() => {
    void refresh(true);
    return () => { request.current++; };
  }, [refresh]);

  React.useEffect(() => {
    if (!loaded || busy || editing) return;
    const timer = window.setInterval(() => {
      if (!document.hidden) void refresh(false, true);
    }, 3500);
    return () => window.clearInterval(timer);
  }, [busy, editing, loaded, refresh]);

  const storeBot = (bot: FeishuBot) => {
    request.current++;
    refreshing.current = false;
    setBots((current) => current.some((item) => item.id === bot.id)
      ? current.map((item) => item.id === bot.id ? bot : item)
      : [...current, bot]);
  };

  const run = async (bot: FeishuBot, action: ConnectionAction) => {
    if (busy) return;
    request.current++;
    refreshing.current = false;
    setBusy({ id: bot.id, action });
    setError("");
    setNotice("");
    try {
      if (action === "remove") {
        await removeFeishuBot(bot.id);
        setBots((items) => items.filter((item) => item.id !== bot.id));
      } else {
        const call = { start: startFeishuBot, stop: stopFeishuBot, test: testFeishuBot }[action];
        storeBot(await call(bot.id));
        if (action === "test") setNotice(l.testOk);
      }
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex min-h-full w-full max-w-[1440px] flex-col gap-4 px-6 py-6">
        <header className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <h1 className="flex items-center gap-2 text-lg font-semibold"><Link2 className="size-5 text-muted-foreground" aria-hidden="true" />{l.title}</h1>
            <p className="mt-1 text-sm text-muted-foreground">{l.subtitle}</p>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" type="button" size="sm" onClick={() => void refresh(true)} disabled={busy !== null}><RefreshCw aria-hidden="true" />{l.refresh}</Button>
            <Button type="button" size="sm" disabled={!loaded || busy !== null} onClick={() => setEditing("new")}><Plus aria-hidden="true" />{l.add}</Button>
          </div>
        </header>

        {error && <p role="alert" className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">{error}</p>}
        {notice && <p role="status" className="rounded-md bg-primary/10 px-3 py-2 text-sm text-primary">{notice}</p>}
        <section aria-labelledby="feishu-connections" className="flex flex-1 flex-col gap-3">
          <div className="flex items-center gap-2">
            <h2 id="feishu-connections" className="text-sm font-semibold">{l.connections}</h2>
            {loaded && <span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground tabular-nums">{bots.length}</span>}
          </div>
          {!loaded && !error && (
            <p role="status" className="flex items-center gap-2 py-3 text-sm text-muted-foreground">
              <LoaderCircle className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />{l.loading}
            </p>
          )}
          {loaded && !bots.length && (
            <div className="grid flex-1 place-items-center">
              <div className="flex w-[22rem] max-w-full flex-col items-center gap-2 rounded-lg border border-dashed border-border px-4 py-8 text-center">
                <span className="grid size-9 place-items-center rounded-md bg-muted text-muted-foreground"><Bot className="size-4" aria-hidden="true" /></span>
                <p className="text-sm font-semibold">{l.emptyTitle}</p>
                <p className="text-sm text-muted-foreground">{l.empty}</p>
              </div>
            </div>
          )}
          {bots.length > 0 && (
            <ul className="flex flex-wrap items-start gap-4">
              {bots.map((bot) => <BotCard key={bot.id} bot={bot} projects={projects} l={l} busy={busy}
                onRun={(action) => void run(bot, action)} onEdit={() => setEditing(bot)} onRemove={() => setRemoving(bot)} />)}
            </ul>
          )}
        </section>
      </div>
      {editing && (
        <BotForm bot={editing === "new" ? null : editing} projects={projects} l={l} onClose={() => setEditing(null)} onSave={(bot) => {
          storeBot(bot);
          setEditing(null);
          setNotice(l.saved);
        }} />
      )}
      <ConfirmDialog open={removing !== null} title={l.remove} body={fmt(l.removeBody, { name: removing?.name ?? "" })}
        confirmLabel={l.remove} cancelLabel={l.cancel} destructive
        onCancel={() => setRemoving(null)} onConfirm={() => {
          if (removing) void run(removing, "remove");
          setRemoving(null);
        }} />
    </div>
  );
}

function BotCard({ bot, projects, l, busy, onRun, onEdit, onRemove }: {
  bot: FeishuBot; projects: FeishuProject[]; l: Labels;
  busy: { id: string; action: ConnectionAction } | null;
  onRun: (action: Exclude<ConnectionAction, "remove">) => void; onEdit: () => void; onRemove: () => void;
}) {
  const running = bot.status !== "stopped";
  const activeAction = busy?.id === bot.id ? busy.action : null;
  const indicator = {
    connected: "bg-primary/10 text-primary",
    connecting: "bg-warning/15 text-warning",
    error: "bg-destructive/10 text-destructive",
    stopped: "bg-muted text-muted-foreground",
  }[bot.status];
  return (
    <li className="w-[22rem] max-w-full min-w-0 rounded-lg border border-border bg-card" aria-busy={activeAction !== null}>
      <div className="flex flex-wrap items-start justify-between gap-3 px-3.5 py-3">
        <div className="flex min-w-0 items-start gap-3">
          <span className="grid size-9 shrink-0 place-items-center rounded-md bg-muted text-muted-foreground"><Bot className="size-4" aria-hidden="true" /></span>
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
              <h3 className="break-words text-sm font-semibold">{bot.name}</h3>
              <span className={cn("inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium", indicator)}>
                <span className="size-1.5 rounded-full bg-current" aria-hidden="true" />{l.status[bot.status]}
              </span>
            </div>
            <p className="mt-1 break-all text-xs text-muted-foreground">
              {bot.engine === "codex" ? "Codex" : "Claude Code"} <span aria-hidden="true">·</span> <code>{bot.appId}</code>
            </p>
          </div>
        </div>
        <Button size="sm" type="button" variant={running ? "outline" : "default"} disabled={busy !== null} onClick={() => onRun(running ? "stop" : "start")}>
          {activeAction === "start" || activeAction === "stop" ? <LoaderCircle className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
            : running ? <Square aria-hidden="true" /> : <Play aria-hidden="true" />}
          {running ? l.stop : l.start}
        </Button>
      </div>
      {bot.error && <p role="status" className="mx-3.5 mb-3 break-words rounded-md bg-destructive/10 px-2.5 py-2 text-xs text-destructive">{bot.error}</p>}
      {bot.projects.length > 0 && (
        <div className="mx-3.5 mb-3 rounded-md border border-border">
          <p className="border-b border-border bg-muted/45 px-2.5 py-1.5 text-xs font-medium text-muted-foreground">{l.projectPermissions}</p>
          <ul className="divide-y divide-border">
            {bot.projects.map((grant) => (
              <li key={grant.path} title={grant.path} className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 px-2.5 py-1.5">
                <span className="flex min-w-0 items-center gap-2 text-sm">
                  <Folder className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                  <span className="truncate">{projects.find((project) => project.path === grant.path)?.name ?? grant.path}</span>
                </span>
                <span className="text-xs text-muted-foreground">{permissions.filter((permission) => grant[permission]).map((permission) => l.permissions[permission]).join(" · ") || l.noPermissions}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
      <div className="flex flex-wrap gap-2 border-t border-border px-3.5 py-2.5">
        <Button size="sm" type="button" variant="outline" disabled={busy !== null} onClick={() => onRun("test")}>
          {activeAction === "test" && <LoaderCircle className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}{l.test}
        </Button>
        <Button size="sm" type="button" variant="outline" disabled={busy !== null || running} onClick={onEdit}>{l.edit}</Button>
        <Button size="sm" type="button" variant="ghost" disabled={busy !== null || running} onClick={onRemove}>{l.remove}</Button>
      </div>
    </li>
  );
}

function BotForm({ bot, projects, l, onClose, onSave }: {
  bot: FeishuBot | null; projects: FeishuProject[]; l: Labels;
  onClose: () => void; onSave: (bot: FeishuBot) => void;
}) {
  const [input, setInput] = React.useState<FeishuBotInput>(() => bot ? {
    id: bot.id, name: bot.name, appId: bot.appId, tenantKey: bot.tenantKey, allowedUsers: bot.allowedUsers,
    allowedGroups: bot.allowedGroups, allowPrivate: bot.allowPrivate, engine: bot.engine, projects: bot.projects,
  } : emptyInput);
  const [userIds, setUserIds] = React.useState(bot?.allowedUsers.join("\n") ?? "");
  const [groupIds, setGroupIds] = React.useState(bot?.allowedGroups.join("\n") ?? "");
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState("");

  const update = <K extends keyof FeishuBotInput>(key: K, value: FeishuBotInput[K]) => {
    setInput((current) => ({ ...current, [key]: value }));
  };
  const toggleProject = (path: string) => {
    setInput((current) => ({ ...current, projects: current.projects.some((grant) => grant.path === path)
      ? current.projects.filter((grant) => grant.path !== path)
      : [...current.projects, { path, read: true, create: false, edit: false, delete: false }] }));
  };
  const setPermission = (path: string, permission: Permission, checked: boolean) => {
    setInput((current) => ({ ...current, projects: current.projects.map((grant) =>
      grant.path === path ? { ...grant, [permission]: checked } : grant) }));
  };
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (saving) return;
    setSaving(true);
    setError("");
    try {
      onSave(await saveFeishuBot({
        ...input,
        name: input.name.trim(),
        appId: input.appId.trim(),
        appSecret: input.appSecret?.trim() || undefined,
        tenantKey: input.tenantKey.trim(),
        allowedUsers: splitIds(userIds),
        allowedGroups: splitIds(groupIds),
      }));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };
  const unavailable = input.projects.filter((grant) => !projects.some((project) => project.path === grant.path));
  const valid = !unavailable.length && input.name.trim() && input.appId.trim() && input.tenantKey.trim() && splitIds(userIds).length &&
    input.projects.length && (input.appSecret?.trim() || (bot?.hasSecret && bot.appId === input.appId.trim()));
  const available = [...projects, ...unavailable.map(({ path }) => ({ path, name: path }))];

  return (
    <Dialog open onOpenChange={(open) => { if (!open && !saving) onClose(); }}>
      <DialogContent className="flex max-h-[88vh] w-[680px] flex-col overflow-hidden p-0" showCloseButton={!saving}>
        <header className="border-b border-border px-5 py-4 pr-12">
          <DialogTitle>{bot ? l.edit : l.add}</DialogTitle>
          <DialogDescription className="mt-1">{l.formHint}</DialogDescription>
        </header>
        <form className="flex min-h-0 flex-col" onSubmit={(event) => void submit(event)}>
          <div className="grid min-h-0 gap-5 overflow-y-auto px-5 py-4">
            <fieldset disabled={saving} className="min-w-0">
              <legend className="mb-2.5 text-sm font-semibold">{l.formCredentials}</legend>
              <div className="grid gap-3 sm:grid-cols-2">
                <label className="grid gap-1 text-xs">{l.name}<Input value={input.name} onChange={(event) => update("name", event.target.value)} required /></label>
                <div className="grid gap-1 text-xs">
                  <label htmlFor="feishu-engine">{l.engine}</label>
                  <SimpleSelect
                    id="feishu-engine"
                    className="w-full"
                    value={input.engine}
                    disabled={saving}
                    onValueChange={(value) => update("engine", value === "claude" ? "claude" : "codex")}
                    options={[
                      { value: "codex", label: "Codex" },
                      { value: "claude", label: "Claude Code" },
                    ]}
                  />
                </div>
                <label className="grid gap-1 text-xs">{l.appId}<Input autoComplete="off" value={input.appId} onChange={(event) => update("appId", event.target.value)} required /></label>
                <label className="grid gap-1 text-xs">{l.appSecret}<Input type="password" autoComplete="new-password" placeholder={bot?.hasSecret ? l.keepSecret : undefined}
                  value={input.appSecret ?? ""} onChange={(event) => update("appSecret", event.target.value)} /></label>
              </div>
            </fieldset>
            <fieldset disabled={saving} className="min-w-0 border-t border-border pt-3">
              <legend className="pr-2 text-sm font-semibold">{l.formAccess}</legend>
              <div className="grid gap-3 sm:grid-cols-2">
                <label className="grid gap-1 text-xs">{l.tenant}<Input autoComplete="off" value={input.tenantKey} onChange={(event) => update("tenantKey", event.target.value)} required /></label>
                <label className="grid gap-1 text-xs">{l.users}<Input autoComplete="off" placeholder="ou_…" value={userIds} onChange={(event) => setUserIds(event.target.value)} required /></label>
                <label className="grid gap-1 text-xs">{l.groups}<Input autoComplete="off" placeholder="oc_…" value={groupIds} onChange={(event) => setGroupIds(event.target.value)} /></label>
                <p className="self-end pb-1 text-xs leading-5 text-muted-foreground">{l.private}</p>
              </div>
            </fieldset>
            <fieldset disabled={saving} className="min-w-0 border-t border-border pt-3">
              <legend className="pr-2 text-sm font-semibold">{l.projectPermissions}</legend>
              <p className="mb-2 text-xs text-muted-foreground">{l.projectHint}</p>
              {unavailable.length > 0 && <p className="mb-2 text-xs text-destructive">{l.unavailable}</p>}
              {available.length ? (
                <div className="overflow-hidden rounded-md border border-border">
                  <table className="w-full text-xs">
                    <thead className="bg-muted text-muted-foreground"><tr>
                      <th scope="col" className="p-2 text-left">{l.project}</th>
                      {permissions.map((permission) => <th key={permission} scope="col" className="px-2 py-1">{l.permissions[permission]}</th>)}
                    </tr></thead>
                    <tbody>{available.map((project) => {
                      const grant = input.projects.find((item) => item.path === project.path);
                      return (
                        <tr key={project.path} className="border-t border-border">
                          <td className="min-w-0 p-2"><label className="flex items-center gap-2">
                            <input type="checkbox" className={checkClass} aria-label={fmt(l.bind, { project: project.name })}
                              checked={!!grant} onChange={() => toggleProject(project.path)} />
                            <span className="min-w-0 truncate" title={project.path}>{project.name}</span>
                            {unavailable.some(({ path }) => path === project.path) && <span className="shrink-0 text-destructive">{l.unavailableShort}</span>}
                          </label></td>
                          {permissions.map((permission) => (
                            <td className="p-1 text-center" key={permission}>
                              <input type="checkbox" className={checkClass}
                                aria-label={fmt(l.permissionLabels[permission], { project: project.name })}
                                disabled={!grant} checked={grant?.[permission] ?? false}
                                onChange={(event) => setPermission(project.path, permission, event.target.checked)} />
                            </td>
                          ))}
                        </tr>
                      );
                    })}</tbody>
                  </table>
                </div>
              ) : <p className="text-xs text-muted-foreground">{l.noProjects}</p>}
            </fieldset>
            {error && <p role="alert" className="break-words text-sm text-destructive">{error}</p>}
          </div>
          <DialogFooter className="mt-0 border-t border-border px-5 py-3">
            <Button type="button" variant="outline" disabled={saving} onClick={onClose}>{l.cancel}</Button>
            <Button type="submit" disabled={!valid || saving}>
              {saving && <LoaderCircle className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}{l.save}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
