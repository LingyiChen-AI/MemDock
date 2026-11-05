import type { MemoryFile, MemoryIndex } from "@/api/types";

/** Tree/selection key; the same file id can appear under several scopes. */
export function fileKey(file: MemoryFile): string {
  const scope = file.scope.type === "global" ? "global" : `project:${file.scope.project_id}`;
  return `${scope}:${file.id}`;
}

/** Windows paths in the index look like `C:\...` or `c:/...` (or UNC). */
export function indexIsWindows(index: MemoryIndex): boolean {
  const sample = [
    ...index.global.map((f) => f.path),
    ...index.projects.flatMap((p) => [p.project.path, ...p.files.map((f) => f.path)]),
  ];
  return sample.some((p) => /^[a-zA-Z]:[\\/]/.test(p) || p.startsWith("\\\\"));
}

/**
 * Mirrors the backend `normalize_path` (`\` -> `/`, no trailing `/`, lowercase on Windows)
 * and additionally collapses `.` and `..` segments, which import references may contain.
 */
export function normalizePath(p: string, windows: boolean): string {
  let s = p.replace(/\\/g, "/");
  const parts: string[] = [];
  const segs = s.split("/");
  segs.forEach((seg, i) => {
    if (seg === "." || (seg === "" && i > 0 && i < segs.length - 1)) return;
    if (seg === ".." && parts.length > 0 && parts[parts.length - 1] !== ".." && parts[parts.length - 1] !== "") {
      parts.pop();
      return;
    }
    parts.push(seg);
  });
  s = parts.join("/");
  while (s.length > 1 && s.endsWith("/")) s = s.slice(0, -1);
  return windows ? s.toLowerCase() : s;
}

export function basename(p: string): string {
  const s = p.replace(/\\/g, "/").replace(/\/+$/, "");
  return s.slice(s.lastIndexOf("/") + 1);
}

export function dirname(p: string): string {
  const s = p.replace(/\\/g, "/");
  const i = s.lastIndexOf("/");
  return i < 0 ? "" : s.slice(0, i + 1);
}

export function allFiles(index: MemoryIndex): MemoryFile[] {
  return [...index.global, ...index.projects.flatMap((p) => p.files)];
}

/** Finds the indexed file an import resolves to, if any. */
export function findByResolvedPath(index: MemoryIndex, resolved: string): MemoryFile | undefined {
  const win = indexIsWindows(index);
  const target = normalizePath(resolved, win);
  return allFiles(index).find((f) => f.kind !== "codex_db" && normalizePath(f.path, win) === target);
}

export function findByKey(index: MemoryIndex, key: string | undefined): MemoryFile | undefined {
  if (!key) return undefined;
  return allFiles(index).find((f) => fileKey(f) === key);
}

/** 4100 -> "4.1k", 800 -> "800". */
export function compactNumber(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}

export function fmt(template: string, vars: Record<string, string | number>): string {
  return template.replace(/\{(\w+)\}/g, (_, k: string) => String(vars[k] ?? ""));
}
