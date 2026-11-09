import { createContext, useCallback, useEffect, useRef, useState } from "react";
import { openAbsoluteCode, resolveCodeLink } from "@/api/workspaceCode";

export interface CodeLocation {
  path: string;
  line?: number;
  anchor?: string;
}

export const WorkspaceCodeContext = createContext<((location: CodeLocation, fromMessage?: boolean) => boolean) | null>(null);

export function workspacePath(cwd: string, raw: string): string | null {
  const root = cwd.replace(/\\/g, "/").replace(/\/+$/, "");
  let path = raw.replace(/\\/g, "/").replace(/^\/([a-z]:\/)/i, "$1");
  if (path.startsWith("~/")) return null;
  const windows = /^[a-z]:/i.test(root) || root.startsWith("//");
  if (/^(?:\/|[a-z]:)/i.test(path)) {
    const prefix = root + "/";
    if (!(windows ? path.toLowerCase().startsWith(prefix.toLowerCase()) : path.startsWith(prefix))) return null;
    path = path.slice(prefix.length);
  }
  path = path.replace(/^(?:\.\/)+/, "").replace(/\/+$/, "");
  if (!path || path.split("/").some((part) => !part || part === "." || part === ".." || /^\.git[. ]*$/i.test(part) || part.includes(":"))) return null;
  return path;
}

export function messageCodeLocation(raw: string, explicitLink = false): CodeLocation | null {
  const match = raw.trim().match(/^(.+?)(?::(\d+)(?::\d+)?|#L(\d+)(?:-L\d+)?)?$/);
  if (!match || /[\r\n]/.test(raw)) return null;
  const [, path, line, hashLine] = match;
  if (path.startsWith("#")) return null;
  if (!/^[a-z]:[\\/]/i.test(path) && /^(?:[a-z][a-z\d+.-]+:|\/\/)/i.test(path)) return null;
  if (!/[\\/]/.test(path) && !explicitLink && !/^[\p{L}\p{N}_. -]+\.[\p{L}\p{N}_-]+$/u.test(path)) return null;
  return { path, line: Number(line || hashLine) || undefined };
}

export function toolCodeLocation(call: { name: string; input: string; output: string | null }): CodeLocation | null {
  const name = call.name.toLowerCase();
  if (name === "apply_patch") {
    const diff = call.input.match(/^--- (.+) \((?:update|add|move)\)\r?\n([\s\S]*)/);
    if (diff) {
      const hunk = diff[2].match(/^@@ .* \+(\d+)/m);
      return { path: diff[1], line: hunk ? Math.max(1, Number(hunk[1])) : 1 };
    }
    const patch = call.input.match(/^\*\*\* (?:Add|Update) File: (.+)$/m);
    return patch ? { path: patch[1].trim(), line: 1 } : null;
  }
  if (name === "view_image") return { path: call.input };
  if (!["read", "edit", "write", "multiedit", "read_file", "write_file", "str_replace"].includes(name)) return null;
  try {
    const input = JSON.parse(call.input);
    const path = input?.file_path ?? input?.path;
    if (typeof path !== "string") return null;
    const offset = input.offset ?? input.line;
    const numbered = name === "read" ? call.output?.match(/^\s*(\d+)\s*(?:→|\t|\|)/) : null;
    const line = Number.isSafeInteger(offset) ? Math.max(1, offset) : numbered ? Number(numbered[1]) : undefined;
    const candidate = input.new_string ?? input.edits?.[0]?.new_string ?? input.old_string;
    const anchor = typeof candidate === "string" ? candidate : undefined;
    return { path, line, ...(anchor ? { anchor } : {}) };
  } catch {
    return null;
  }
}

function absolutePath(cwd: string, path: string): string | null {
  if (/^~[\\/]/.test(path)) return path.replace(/\\/g, "/");
  const windows = /^[a-z]:[\\/]/i.test(cwd);
  if (windows) {
    const shellDrive = path.match(/^\/([a-z])\/(.+)$/i);
    if (shellDrive) return `${shellDrive[1]}:/${shellDrive[2]}`;
    return /^[a-z]:[\\/]/i.test(path) ? path : null;
  }
  return path.startsWith("/") ? path : null;
}

type RecordedTool = Parameters<typeof toolCodeLocation>[0];

export function recordedFilePaths(cwd: string, name: string, calls: readonly RecordedTool[]): string[] {
  const matches = new Map<string, string>();
  const windows = /^[a-z]:[\\/]/i.test(cwd);
  const key = (value: string) => windows ? value.toLowerCase() : value;
  const add = (candidate: string) => {
    const path = absolutePath(cwd, candidate)?.replace(/\\/g, "/");
    if (path && key(path.slice(path.lastIndexOf("/") + 1)) === key(name)) matches.set(key(path), path);
  };
  for (const call of calls) {
    const location = toolCodeLocation(call);
    if (location) add(location.path);
    if (!["bash", "shell", "exec_command"].includes(call.name.toLowerCase())) continue;
    let command = call.input;
    try {
      const input = JSON.parse(command);
      command = input.command ?? input.cmd ?? "";
    } catch { /* Some history stores the shell command directly. */ }
    if (typeof command !== "string" || !key(command).includes(key(name))) continue;
    for (const [, variable, doubleQuoted, singleQuoted, plain] of command.matchAll(/(?:^|[;\s])([A-Za-z_]\w*)=(?:"([^"]+)"|'([^']+)'|([^;\s"'&|]+))/g)) {
      if (key(command).includes(key(`$${variable}/${name}`)) || key(command).includes(key(`\${${variable}}/${name}`))) {
        add(`${doubleQuoted ?? singleQuoted ?? plain}/${name}`);
      }
    }
  }
  return [...matches.values()];
}

export function useWorkspaceCode(cwd: string, onLinkError: (path: string | null, ambiguous?: boolean, absolute?: boolean) => void, recordedTools: readonly RecordedTool[]) {
  const [state, setState] = useState<{ cwd: string; shown: boolean; request: CodeLocation | null } | null>(null);
  const requestId = useRef(0);
  useEffect(() => () => { requestId.current++; }, [cwd]);
  const current = state?.cwd === cwd ? state : null;
  const open = useCallback((location: CodeLocation, fromMessage = false) => {
    const id = ++requestId.current;
    const path = cwd && workspacePath(cwd, location.path);
    const absolute = absolutePath(cwd, location.path);
    if (!fromMessage && !(absolute && !path)) {
      if (!path) return false;
      setState({ cwd, shown: true, request: { ...location, path } });
      return true;
    }
    const bareName = !!path && !/[\\/]/.test(location.path);
    const candidates = absolute ? [absolute] : bareName ? recordedFilePaths(cwd, location.path, recordedTools) : [];
    if (candidates.length > 1) {
      onLinkError(location.path, true);
      return true;
    }
    const target = candidates[0];
    const relative = target ? workspacePath(cwd, target) : path;
    if (!target && !relative) {
      onLinkError(location.path);
      return false;
    }
    onLinkError(null);
    const reportMissing = () => {
      if (id === requestId.current) onLinkError(target ?? location.path, false, !!target);
    };
    const openLocal = (absoluteTarget: string) => openAbsoluteCode(absoluteTarget, location.line).then((opened) => {
      if (!opened) reportMissing();
    });
    if (relative) {
      void resolveCodeLink(cwd, relative, bareName && !target).then((matches) => {
        if (id !== requestId.current) return;
        if (matches.length === 1) {
          const [match] = matches;
          if (match.endsWith("/")) return openLocal(`${cwd.replace(/[\\/]+$/, "")}/${match}`);
          setState({ cwd, shown: true, request: { ...location, path: match } });
        } else {
          onLinkError(target ?? location.path, matches.length > 1, !!target);
        }
      }).catch(reportMissing);
    } else if (target) {
      void openLocal(target).catch(reportMissing);
    }
    return true;
  }, [cwd, onLinkError, recordedTools]);
  const toggle = () => {
    requestId.current++;
    setState({ cwd, shown: !current?.shown, request: current?.request ?? null });
  };
  return { shown: current?.shown ?? false, mounted: current !== null, toggle, open, request: current?.request ?? null };
}
