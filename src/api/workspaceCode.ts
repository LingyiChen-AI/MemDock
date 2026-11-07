import { invoke } from "@tauri-apps/api/core";
import type { WorktreeInfo } from "./worktree";

export interface CodeEntry {
  name: string;
  path: string;
  directory: boolean;
}

export type CodeFile =
  | { kind: "text"; text: string; revision: string; baseline: string | null }
  | { kind: "image"; data: string };

export interface CodeMatch {
  path: string;
  line: number | null;
  text: string | null;
}

export const trashCodeFile = (cwd: string, path: string) => invoke<void>("code_trash_file", { cwd, path });
export const createCodeFile = (cwd: string, directory: string, name: string, text?: string) => invoke<string>("code_create_file", { cwd, directory, name, text });
export const codeEntries = (cwd: string, path: string) => invoke<CodeEntry[]>("code_entries", { cwd, path });
export const readCode = (cwd: string, path: string, base: WorktreeInfo | null) => invoke<CodeFile>("code_read", { cwd, path, base });
export const saveCode = (cwd: string, path: string, revision: string, text: string) => invoke<string>("code_save", { cwd, path, revision, text });
export const codeStatus = (cwd: string) => invoke<Record<string, string>>("code_status", { cwd });
export const searchCode = (cwd: string, query: string, content: boolean) => invoke<CodeMatch[]>("code_search", { cwd, query, content });
export const openCodeInVSCode = (cwd: string, path: string, line?: number) => invoke<void>("code_open_vscode", { cwd, path, line });
