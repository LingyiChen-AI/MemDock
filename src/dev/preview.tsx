// Dev-only entry (see /preview.html): renders the real app against a mocked Tauri backend so the UI
// can be inspected and screenshotted in a normal browser. Never imported by the production entry.
import React from "react";
import ReactDOM from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import "@fontsource-variable/inter";
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
import "@fontsource-variable/jetbrains-mono";
import "../styles/globals.css";
import App from "../App";
import type { MemoryFile, MemoryIndex, Project } from "../api/types";

const params = new URLSearchParams(location.search);
const theme = params.get("theme") ?? "dark";

const file = (over: Partial<MemoryFile> & { id: string; path: string }): MemoryFile => ({
  tool: "claude",
  scope: { type: "global" },
  kind: "claude_instructions",
  exists: true,
  is_symlink: false,
  size_bytes: 4100,
  modified_at: Date.now(),
  ...over,
});
const project = (name: string, exists = true, sources: Project["sources"] = ["claude_sessions"]): Project => ({
  id: `c:/work/${name}`,
  path: `C:\\work\\${name}`,
  name,
  exists,
  sources,
});
const inProject = (p: Project) => ({ type: "project", project_id: p.id }) as const;

const jev = project("jev-rpa");
const autos = Array.from({ length: 12 }, (_, i) =>
  file({
    id: `c:/users/admin/.claude/projects/jev/memory/feedback_${i + 1}.md`,
    path: `C:\\Users\\Admin\\.claude\\projects\\jev\\memory\\feedback_${i + 1}.md`,
    scope: inProject(jev),
    kind: "claude_auto",
    size_bytes: 600,
  }),
);
const index: MemoryIndex = {
  global: [
    file({ id: "c:/users/admin/.claude/claude.md", path: "C:\\Users\\Admin\\.claude\\CLAUDE.md" }),
    file({
      id: "c:/users/admin/.codex/agents.md",
      path: "C:\\Users\\Admin\\.codex\\AGENTS.md",
      tool: "codex",
      kind: "codex_agents",
      exists: false,
      size_bytes: 0,
    }),
    file({ id: "codex-db", path: "codex-db", tool: "codex", kind: "codex_db", size_bytes: 0 }),
  ],
  projects: [
    {
      project: jev,
      files: [
        file({ id: "c:/work/jev-rpa/claude.md", path: "C:\\work\\jev-rpa\\CLAUDE.md", scope: inProject(jev), size_bytes: 2100 }),
        ...autos,
        file({
          id: "c:/work/jev-rpa/agents.md",
          path: "C:\\work\\jev-rpa\\AGENTS.md",
          scope: inProject(jev),
          tool: "codex",
          kind: "codex_agents",
          size_bytes: 800,
        }),
      ],
    },
    ...["synergy", "omin-kube", "freecall"].map((n) => ({ project: project(n), files: [] })),
    { project: project("cicada2win", false), files: [] },
    { project: project("cc-memory", true, ["manual"]), files: [] },
  ],
};

const TEXT = `# 全局规则（铁律）

## 0. 一律用中文回复

所有面向用户的输出一律用中文，代码标识符、命令、路径保持原样。

## 1. 数据库变更必须落 SQL 文件

放在项目约定目录，序号先看现有最大值再定。

## 8. 全局资源

- 本机数据库清单见 @C:/devtools/database/README.md
- 设计参考 @~/.claude/awesome-design-md/README.md

> 从旧机器迁移（2026-09-11）。

**Why**：用户为此已多次提醒，原话「铁律你也记不住是吧」，见 \`~/.claude/settings.json\`。

| 项目 | 端口 |
|---|---|
| MySQL | 3306 |
| Redis | 6379 |
`;

const content = (text: string, hash: string) => ({
  text,
  hash,
  has_bom: false,
  crlf: false,
  tokens: 4100,
  imports: [
    { raw: "C:/devtools/database/README.md", resolved: "C:/devtools/database/README.md", exists: true, line: 10 },
    { raw: "~/.claude/awesome-design-md/README.md", resolved: "C:/Users/Admin/.claude/awesome-design-md/README.md", exists: false, line: 11 },
  ],
});

// History fixtures: older variants of TEXT, so the diff shows word-level and whole-line changes.
const NOW = Date.now();
const snapshotTexts: Record<string, string> = {
  s3: TEXT.replace("序号先看现有最大值再定", "序号随便定").replace("多次提醒", "提醒过").replace("| Redis | 6379 |\n", "| Redis | 6380 |\n"),
  s2: TEXT.replace("## 8. 全局资源", "## 2. 资源").replace("一律用中文回复", "用中文回复"),
  s1: TEXT.split("\n").slice(0, 8).join("\n") + "\n",
};
const snapshots = [
  { id: "s3", created_at: NOW - 3 * 3600_000, size_bytes: 2150 },
  { id: "s2", created_at: NOW - 26 * 3600_000, size_bytes: 1980 },
  { id: "s1", created_at: NOW - 9 * 24 * 3600_000, size_bytes: 412 },
];
const searchHits = [
  { file_id: "c:/users/admin/.claude/claude.md", line: 24, preview: "| MySQL | 3306 |" },
  { file_id: "c:/users/admin/.claude/claude.md", line: 9, preview: "本机数据库清单见 @C:/devtools/database/README.md（MySQL 一主一从）" },
  { file_id: "c:/work/jev-rpa/claude.md", line: 12, preview: "本地 mysql 从库延迟 1 秒，写后立读必须读主库" },
  { file_id: "c:/work/jev-rpa/agents.md", line: 3, preview: "Run MySQL migrations before deploy" },
];

let version = 0;
mockIPC(
  (cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case "list_memories":
      case "refresh_memories":
      case "add_manual_project":
      case "remove_manual_project":
        return index;
      case "read_memory_file":
        return content(version === 0 ? TEXT : `${TEXT}\n- 外部新增的一行\n`, `h${version}`);
      case "write_memory_file":
        return content(String(a.text), `h${++version}`);
      case "search_memories":
        return String(a.query ?? "").trim() ? searchHits : [];
      case "list_snapshots":
        return snapshots;
      case "read_snapshot":
        return snapshotTexts[String(a.id)] ?? "";
      case "read_codex_db":
        return { status: "unavailable", reason: "preview" };
      case "get_settings":
        return { language: params.get("lang") ?? "zh", theme, manual_projects: [] };
      case "update_ui_prefs":
        return { language: params.get("lang") ?? "zh", theme, manual_projects: [] };
      case "plugin:dialog|open":
        return null;
      default:
        return undefined;
    }
  },
  { shouldMockEvents: true },
);

// Lets a scripted browser simulate the file watcher: `__previewExternalChange()`.
(window as unknown as Record<string, unknown>).__previewExternalChange = () => {
  version = 1;
  return emit("memory-changed", { paths: ["C:\\Users\\Admin\\.claude\\CLAUDE.md"] });
};

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
