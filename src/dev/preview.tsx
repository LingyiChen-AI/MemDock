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
import "lxgw-wenkai-screen-webfont/lxgwwenkaiscreen.css";
import "../styles/globals.css";
import App from "../App";
import type { MemoryFile, MemoryIndex, Project, SkillEntry, UpdateStatus } from "../api/types";
import { mkSkill } from "../pages/skills/testUtils";

const params = new URLSearchParams(location.search);
const theme = params.get("theme") ?? "dark";
let previewAccent: string | null = params.get("accent");

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

截图路径 \`C:/Users/Admin/AppData/Local/Temp/claude/e290bfd9097cfe51/scratchpad/shots/gutter-fixed.png\`，参考 https://github.com/VoltAgent/awesome-design-md/blob/main/design-md/linear-app/DESIGN.md

\`\`\`bash
msedge --headless=new --force-device-scale-factor=2 --window-size=1600,1000 --screenshot=out.png file:///C:/devtools/diagrams/architecture.svg
\`\`\`

| 服务 | 更换前镜像版本 | 更换后镜像版本 |
|---|---|---|
| freeclaw-admin | registry.cn-hangzhou.aliyuncs.com/freecall/freeclaw-admin:v1.2.3-20261105 | registry.cn-hangzhou.aliyuncs.com/freecall/freeclaw-admin:v1.2.4-20261106 |
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


// Skill fixtures for the Skill 管理 page.
const skillsList: SkillEntry[] = [
  mkSkill({
    name: "fireworks-tech-graph",
    description: "Create precise SVG technical diagrams, export PNG or offline HTML, and animate supported semantic SVGs to GIF.",
    source: { url: "https://github.com/yizhiyanhua-ai/fireworks-tech-graph", git_ref: null, commit: "a1b2c3d4e5f6", skill_path: "", origin: "registry" },
    file_count: 14,
    size_bytes: 182_000,
  }),
  mkSkill({
    name: "fireworks-tech-graph",
    id: "c:/users/admin/.codex/skills/fireworks-tech-graph",
    tool: "codex",
    dir: "C:\\Users\\Admin\\.codex\\skills\\fireworks-tech-graph",
    description: "Create precise SVG technical diagrams, export PNG or offline HTML, and animate supported semantic SVGs to GIF.",
    is_symlink: true,
    symlink_target: "C:\\Users\\Admin\\.claude\\skills\\fireworks-tech-graph",
    source: { url: "https://github.com/yizhiyanhua-ai/fireworks-tech-graph", git_ref: null, commit: "a1b2c3d4e5f6", skill_path: "", origin: "registry" },
  }),
  mkSkill({ name: "freecall-tester", description: "Use when 需要根据 PRD 产出标准测试用例或冒烟测试用例、用 ego-browser 跑 AI 自动化测试和 UI 走查", file_count: 9 }),
  mkSkill({
    name: "bug-bot",
    description: "管理飞书群里的「查 bug 机器人」：给机器人加项目、加生产库、给新群配机器人、重启 bridge",
    scope: { type: "project", project_id: jev.id },
    source: { url: "https://git.example.com/team/bug-bot", git_ref: "main", commit: "9f8e7d6c5b4a", skill_path: "skills/bug-bot", origin: "manual" },
  }),
  mkSkill({ name: "sql-window", tool: "codex", description: "Use when 用户希望在 Codex 里通过飞书「研发SQL执行群」查询生产数据库", file_count: 4 }),
  mkSkill({ name: "legacy-notes", description: "没有来源记录的本地 skill，用来演示补登来源", file_count: 2, size_bytes: 900 }),
  mkSkill({
    name: "superpowers:brainstorming",
    id: "c:/users/admin/.claude/plugins/superpowers/skills/brainstorming",
    dir: "C:\\Users\\Admin\\.claude\\plugins\\superpowers\\skills\\brainstorming",
    kind: "plugin",
    plugin: "superpowers",
    readonly: true,
    description: "You MUST use this before any creative work - creating features, building components, adding functionality.",
  }),
  mkSkill({
    name: "imagegen",
    id: "c:/users/admin/.codex/skills/.system/imagegen",
    dir: "C:\\Users\\Admin\\.codex\\skills\\.system\\imagegen",
    tool: "codex",
    kind: "codex_system",
    readonly: true,
    description: "Generate and edit images with the built-in image tool.",
  }),
];
const skillUpdates: UpdateStatus[] = [
  { skill_id: skillsList[0].id, current: "a1b2c3d4e5f6", latest: "0f1e2d3c4b5a", has_update: true, local_modified: true, local_state_unknown: false, error: null },
  { skill_id: skillsList[3].id, current: "9f8e7d6c5b4a", latest: "9f8e7d6c5b4a", has_update: false, local_modified: false, local_state_unknown: true, error: null },
];
const skillFileList = {
  files: [
    { rel_path: "SKILL.md", size_bytes: 5200 },
    { rel_path: "scripts/fireworks.py", size_bytes: 20400 },
    { rel_path: "scripts/render.py", size_bytes: 3100 },
    { rel_path: "references/styles.md", size_bytes: 8800 },
    { rel_path: "assets/logo.png", size_bytes: 48000 },
  ],
  truncated: false,
};
const skillDoc = `---
name: fireworks-tech-graph
description: Create precise SVG technical diagrams, export PNG or offline HTML.
---

# Fireworks Tech Graph

Use this skill for architecture, UML, agent, cloud or workflow diagrams.

## Workflow

1. Write the JSON spec
2. Run \`fireworks.py validate\`
3. Render and check the result
`;

const inspectResult = {
  url: "https://github.com/anthropics/skills",
  clone_url: "https://github.com/anthropics/skills.git",
  git_ref: null,
  commit: "c9d8e7f6a5b4",
  preselected: null,
  skills: [
    { skill_path: "skills/pdf", name: "pdf", description: "Use this skill whenever the user wants to do anything with PDF files: read, merge, split, fill forms." },
    { skill_path: "skills/xlsx", name: "xlsx", description: "Create, read, edit spreadsheets (.xlsx, .csv) with formulas and formatting." },
    { skill_path: "skills/skill-creator", name: "skill-creator", description: "Create new skills, modify and improve existing skills, and measure skill performance." },
    { skill_path: "document-skills/docx", name: "docx", description: "" },
  ],
};
const skillBackups = [
  { id: "1730800000000", created_at: NOW - 2 * 3600_000, size_bytes: 182_000 },
  { id: "1730700000000", created_at: NOW - 30 * 3600_000, size_bytes: 176_400 },
  { id: "1730100000000", created_at: NOW - 8 * 24 * 3600_000, size_bytes: 91_300 },
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
        return { language: params.get("lang") ?? "zh", theme, accent: previewAccent, manual_projects: [] };
      case "update_ui_prefs":
        if (a.accent) previewAccent = String(a.accent);
        return { language: params.get("lang") ?? "zh", theme, accent: previewAccent, manual_projects: [] };
      case "list_skills":
        return skillsList;
      case "git_available":
        return true;
      case "check_skill_updates":
        return skillUpdates;
      case "list_skill_files":
        return skillFileList;
      case "read_skill_file":
        if (String(a.relPath).endsWith(".png")) return { text: "", hash: "p", has_bom: false, crlf: false, tokens: 0, imports: [], binary: true, too_large: false };
        return { text: String(a.relPath) === "SKILL.md" ? skillDoc : "print('hello')", hash: "sk1", has_bom: false, crlf: false, tokens: 120, imports: [], binary: false, too_large: false };
      case "register_skill_source":
        return skillsList[2];
      case "inspect_git_source":
        return new Promise((resolve) => setTimeout(() => resolve(inspectResult), 150));
      case "install_skill":
        return [];
      case "update_skill":
        if (!a.acceptLocalChanges) return Promise.reject({ code: "local_changes", message: "local changes" });
        return { ...skillsList[0], source: { ...skillsList[0].source, commit: "0f1e2d3c4b5a" } };
      case "delete_skill":
        return { id: "1730800000000", created_at: Date.now(), size_bytes: 182_000 };
      case "sync_skill":
        return skillsList[0];
      case "list_skill_backups":
        return skillBackups;
      case "rollback_skill":
        return skillsList[0];
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

// `?page=环境与设置` opens that nav item once the shell is rendered (screenshots of non-memory pages).
const page = params.get("page");
if (page) {
  const timer = setInterval(() => {
    const btn = Array.from(document.querySelectorAll("button")).find((b) => b.textContent?.trim() === page);
    if (btn) {
      clearInterval(timer);
      btn.click();
    }
  }, 100);
}

// `?open=appearance` opens the sidebar appearance popover (headless screenshots cannot click).
if (params.get("open") === "appearance") {
  const timer = setInterval(() => {
    const btn = document.querySelector<HTMLButtonElement>('button[aria-label="外观 / Appearance"], button[aria-label="Appearance"]');
    if (btn) {
      clearInterval(timer);
      btn.click();
    }
  }, 100);
}

// `?skilltab=files` switches the opened skill to its file editor; `?select=<name>` opens a skill row;
// `?check=1` runs the update check.
const selectName = params.get("select");
if (selectName || params.get("skilltab") === "files" || params.get("check")) {
  let tries = 0;
  const timer = setInterval(() => {
    tries++;
    if (tries > 100) clearInterval(timer);
    const rows = Array.from(document.querySelectorAll<HTMLElement>('[role="option"]'));
    if (rows.length === 0) return;
    clearInterval(timer);
    if (params.get("check")) Array.from(document.querySelectorAll("button")).find((b) => b.textContent?.startsWith("检查更新"))?.click();
    if (selectName) rows.find((r) => r.textContent?.includes(selectName))?.click();
    if (params.get("skilltab") === "files") {
      setTimeout(() => document.querySelector<HTMLElement>('[role="radio"]:not([aria-checked="true"])')?.click(), 300);
    }
  }, 100);
}

// `?flow=add-gh|add-gh-pick|add-template|update|sync|delete|backups` drives the skill flows for screenshots.
const flow = params.get("flow");
if (flow) {
  const wait = <T,>(find: () => T | null | undefined | false, then: (v: T) => void) => {
    let tries = 0;
    const timer = setInterval(() => {
      const v = find();
      if (v) {
        clearInterval(timer);
        then(v);
      } else if (++tries > 100) clearInterval(timer);
    }, 100);
  };
  const btn = (text: string) => () => Array.from(document.querySelectorAll<HTMLElement>("button")).find((b) => b.textContent?.trim() === text);
  const setValue = (el: HTMLInputElement, value: string) => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  };
  wait(() => document.querySelector('[role="option"]'), () => {
    if (flow.startsWith("add")) {
      wait(btn("添加 skill"), (b) => {
        b.click();
        if (flow === "add-template") {
          wait(() => Array.from(document.querySelectorAll<HTMLElement>('[role="tab"]')).find((t) => t.textContent === "空白模板"), (t) => {
            t.click();
            setTimeout(() => {
              const n = document.getElementById("add-tpl-name") as HTMLInputElement | null;
              if (n) setValue(n, "My Skill");
            }, 150);
          });
          return;
        }
        wait(() => document.getElementById("add-git-input") as HTMLInputElement | null, (input) => {
          setValue(input, "anthropics/skills");
          setTimeout(() => btn("解析")()?.click(), 100);
          if (flow === "add-gh-pick") {
            wait(() => document.querySelector<HTMLInputElement>('ul input[type="checkbox"]'), () => {
              const boxes = Array.from(document.querySelectorAll<HTMLInputElement>('ul input[type="checkbox"]'));
              boxes[0].click();
              boxes[2].click();
            });
          }
        });
      });
      return;
    }
    if (flow === "delete") {
      const row = Array.from(document.querySelectorAll<HTMLElement>('[role="option"]')).find((r) => r.textContent?.includes("fireworks-tech-graph") && r.textContent.includes("Codex"));
      row?.click();
    }
    if (flow === "update") wait(btn("更新"), (b) => b.click());
    if (flow === "sync") wait(btn("同步到…"), (b) => b.click());
    if (flow === "delete") setTimeout(() => btn("删除")()?.click(), 400);
    if (flow === "backups") wait(() => document.querySelector<HTMLElement>('button[aria-label="备份与回滚"]'), (b) => b.click());
  });
}
