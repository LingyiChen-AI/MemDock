import { createContext, useCallback, useState } from "react";

export interface CodeLocation {
  path: string;
  line?: number;
  anchor?: string;
}

export const WorkspaceCodeContext = createContext<((location: CodeLocation) => boolean) | null>(null);

export function workspacePath(cwd: string, raw: string): string | null {
  const root = cwd.replace(/\\/g, "/").replace(/\/+$/, "");
  let path = raw.replace(/\\/g, "/").replace(/^\/([a-z]:\/)/i, "$1");
  const windows = /^[a-z]:/i.test(root) || root.startsWith("//");
  if (/^(?:\/|[a-z]:)/i.test(path)) {
    const prefix = root + "/";
    if (!(windows ? path.toLowerCase().startsWith(prefix.toLowerCase()) : path.startsWith(prefix))) return null;
    path = path.slice(prefix.length);
  }
  path = path.replace(/^(?:\.\/)+/, "");
  if (!path || path.split("/").some((part) => !part || part === "." || part === ".." || /^\.git[. ]*$/i.test(part) || part.includes(":"))) return null;
  return path;
}

export function messageCodeLocation(raw: string): CodeLocation | null {
  const match = raw.trim().match(/^(.+?)(?::(\d+)(?::\d+)?|#L(\d+)(?:-L\d+)?)?$/);
  if (!match || /[\r\n]/.test(raw)) return null;
  const [, path, line, hashLine] = match;
  if (/^(?:[a-z][a-z\d+.-]+:|\/\/)/i.test(path)) return null;
  if (!/[\\/]/.test(path) && !/^\.?[\w-]+\.[\w-]+$/.test(path)) return null;
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

export function useWorkspaceCode(cwd: string) {
  const [state, setState] = useState<{ cwd: string; shown: boolean; request: CodeLocation | null } | null>(null);
  const current = state?.cwd === cwd ? state : null;
  const open = useCallback((location: CodeLocation) => {
    const path = cwd && workspacePath(cwd, location.path);
    if (!path) return false;
    setState({ cwd, shown: true, request: { ...location, path } });
    return true;
  }, [cwd]);
  const toggle = () => setState({ cwd, shown: !current?.shown, request: current?.request ?? null });
  return { shown: current?.shown ?? false, mounted: current !== null, toggle, open, request: current?.request ?? null };
}
