import { render } from "@testing-library/react";
import { vi } from "vitest";
import type * as React from "react";
import { AppProviders } from "@/providers";
import type { MemoryFile, MemoryIndex, Project } from "@/api/types";

/** Renders inside all app providers with the UI language forced to Chinese. */
export function renderApp(ui: React.ReactElement) {
  vi.spyOn(navigator, "language", "get").mockReturnValue("zh-CN");
  return render(<AppProviders>{ui}</AppProviders>);
}

export function mkFile(over: Partial<MemoryFile> & { id: string }): MemoryFile {
  return {
    tool: "claude",
    scope: { type: "global" },
    kind: "claude_instructions",
    path: `/${over.id}`,
    exists: true,
    is_symlink: false,
    size_bytes: 4100,
    modified_at: 1_700_000_000_000,
    ...over,
  };
}

export function mkProject(over: Partial<Project> & { id: string; name: string }): Project {
  return {
    path: `/work/${over.name}`,
    exists: true,
    sources: ["claude_sessions"],
    ...over,
  };
}

export const globalClaude = mkFile({ id: "/home/u/.claude/claude.md", path: "/home/u/.claude/CLAUDE.md" });
export const globalAgents = mkFile({
  id: "/home/u/.codex/agents.md",
  path: "/home/u/.codex/AGENTS.md",
  tool: "codex",
  kind: "codex_agents",
  exists: false,
  size_bytes: 0,
  modified_at: null,
});
export const globalDb = mkFile({
  id: "codex-db",
  path: "codex-db",
  tool: "codex",
  kind: "codex_db",
  size_bytes: 0,
});

const proj = mkProject({ id: "/work/jev-rpa", name: "jev-rpa", path: "/work/jev-rpa" });
const scope = { type: "project", project_id: proj.id } as const;
export const projClaude = mkFile({ id: "/work/jev-rpa/claude.md", path: "/work/jev-rpa/CLAUDE.md", scope, size_bytes: 2100 });
export const projAutos = [1, 2, 3].map((n) =>
  mkFile({
    id: `/home/u/.claude/projects/jev/memory/m${n}.md`,
    path: `/home/u/.claude/projects/jev/memory/m${n}.md`,
    scope,
    kind: "claude_auto",
    size_bytes: 300,
  }),
);

export const missingProject = mkProject({
  id: "/work/gone",
  name: "gone",
  path: "/work/gone",
  exists: false,
  sources: ["manual"],
});

export const sampleIndex: MemoryIndex = {
  global: [globalClaude, globalAgents, globalDb],
  projects: [
    { project: proj, files: [projClaude, ...projAutos] },
    { project: missingProject, files: [] },
  ],
};
