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
  { skill_id: skillsList[0].id, current: "a1b2c3d4e5f6", latest: "0f1e2d3c4b5a", has_update: true, local_modified: true, local_state_unknown: false, not_updatable_subdir: false, repo_root: null, error: null },
  { skill_id: skillsList[3].id, current: "9f8e7d6c5b4a", latest: "9f8e7d6c5b4a", has_update: false, local_modified: false, local_state_unknown: true, not_updatable_subdir: false, repo_root: null, error: null },
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

const deletedSkills = [
  {
    bucket: "pdf-notes-1a2b3c4d",
    name: "pdf-notes",
    tool: "claude",
    scope: { type: "global" },
    dir: "C:\\Users\\Admin\\.claude\\skills\\pdf-notes",
    backups: [
      { id: "1730800000000", created_at: NOW - 3 * 3600_000, size_bytes: 24_500 },
      { id: "1730200000000", created_at: NOW - 7 * 24 * 3600_000, size_bytes: 21_900 },
    ],
  },
  {
    bucket: "deploy-helper-9e8d7c6b",
    name: "Deploy Helper",
    tool: "codex",
    scope: { type: "project", project_id: jev.id },
    dir: "C:\\work\\jev\\.agents\\skills\\Deploy Helper",
    backups: [{ id: "1730600000000", created_at: NOW - 36 * 3600_000, size_bytes: 8_800 }],
  },
];

// Extraction page fixtures: `?extract=running` starts with a run in progress.
const extractRunning = params.get("extract") === "running";
const hoursAgo = (h: number) => Date.now() - h * 3600_000;
const extractRuns = [
  ...(extractRunning
    ? [{ id: "r5", started_at: Date.now() - 90_000, finished_at: null, trigger: "manual", tool: "claude", status: "running", sessions_read: 0, projects: 0, suggestions_new: 0, warnings: [], error: null }]
    : []),
  { id: "r4", started_at: hoursAgo(5), finished_at: hoursAgo(5) + 240_000, trigger: "scheduled", tool: "claude", status: "failed", sessions_read: 9, projects: 3, suggestions_new: 0, warnings: ["synergy: 会话文件过大，已跳过 2 个", "omin-kube: 模型输出无法解析，重试 1 次后放弃"], error: "claude 退出码 1：Error: rate limit exceeded, retry after 3600s", error_code: "other" },
  { id: "r3", started_at: hoursAgo(29), finished_at: hoursAgo(29) + 410_000, trigger: "scheduled", tool: "claude", status: "succeeded", sessions_read: 31, projects: 5, suggestions_new: 7, warnings: [], error: null },
  { id: "r2", started_at: hoursAgo(53), finished_at: hoursAgo(53) + 20_000, trigger: "manual", tool: "codex", status: "cancelled", sessions_read: 2, projects: 1, suggestions_new: 0, warnings: [], error: null },
  { id: "r1", started_at: hoursAgo(77), finished_at: hoursAgo(77) + 60_000, trigger: "manual", tool: "claude", status: "interrupted", sessions_read: 4, projects: 2, suggestions_new: 1, warnings: [], error: null },
];
let extractConfig: Record<string, unknown> = {
  tool: "claude",
  schedule: { kind: "weekly", weekday: 1, hh: 9, mm: 30 },
  projects: { kind: "selected", ids: ["c:/work/jev-rpa", "c:/work/synergy"] },
  include_global: true,
  lookback_days: 14,
  lang: null,
};

// Review page fixtures (`?review=empty` starts without suggestions).
const evidenceOf = (quote: string, ts: number) => ({ tool: "claude", session_id: "9f3c2a1e-77b4-4d0e-a1c5-3b8e6d2f4a10", ts, quote });
let pendingList: Record<string, unknown>[] =
  params.get("review") === "empty"
    ? []
    : [
        {
          id: "sg1", fingerprint: "f1", kind: "modify", file_id: "c:/work/jev-rpa/claude.md", path: "C:\\work\\jev-rpa\\CLAUDE.md", tool_of_file: "claude",
          old_text: "本地 MySQL 也是一主一从（3306 主库写、3307 从库只读），\n从库已设 SOURCE_DELAY=2，实测写后约 4 秒可见\n本地开发与自测必须在「写主读从」配置下跑通。",
          new_text: "本地 MySQL 也是一主一从（3306 主库写、3307 从库只读），\n从库已设 SOURCE_DELAY=1，实测写后约 3 秒可见\n本地开发与自测必须在「写主读从」配置下跑通。",
          reason: "用户把从库延迟从 2 秒调到 1 秒，记忆里的数字需要同步。",
          evidence: [evidenceOf("把从库延迟调小到 1 秒，缩短测试耗时", NOW - 6 * 3600_000)], source_tool: "claude", created_at: NOW - 2 * 3600_000, base_hash: "h1", status: "pending",
        },
        {
          id: "sg2", fingerprint: "f2", kind: "add", file_id: "c:/users/admin/.claude/claude.md", path: "C:\\Users\\Admin\\.claude\\CLAUDE.md", tool_of_file: "claude",
          old_text: null, new_text: "## 15. ego-browser 只在用户明确要求时才调用\n\n用户没有点名要用 ego-browser，就不调用它。",
          reason: "用户纠正过一次：没有明确要求不要自行调用。",
          evidence: [evidenceOf("你为啥老是自己去调研 ego-browser？必须有明确要求的时候才调", NOW - 30 * 3600_000), evidenceOf("记忆力加强一下", NOW - 30 * 3600_000)], source_tool: "claude", created_at: NOW - 3 * 3600_000, base_hash: "h2", status: "pending",
        },
        {
          id: "sg3", fingerprint: "f3", kind: "obsolete", file_id: "c:/work/jev-rpa/agents.md", path: "C:\\work\\jev-rpa\\AGENTS.md", tool_of_file: "codex",
          old_text: "部署前先在测试环境手动跑一遍 Run MySQL migrations", new_text: null,
          reason: "迁移已改为流水线自动执行，这条手动步骤不再适用。",
          evidence: [{ tool: "codex", session_id: "c0de-5e55-1011", ts: NOW - 50 * 3600_000, quote: "迁移现在由流水线自动跑，不用手动了" }], source_tool: "codex", created_at: NOW - 5 * 3600_000, base_hash: "h3", status: "pending",
        },
        {
          id: "sg-stale", fingerprint: "f4", kind: "modify", file_id: "c:/work/jev-rpa/claude.md", path: "C:\\work\\jev-rpa\\CLAUDE.md", tool_of_file: "claude",
          old_text: "Redis 端口 6379", new_text: "Redis 端口 6380",
          reason: "用户提到 Redis 换了端口。",
          evidence: [evidenceOf("Redis 现在是 6380", NOW - 20 * 3600_000)], source_tool: "claude", created_at: NOW - 8 * 3600_000, base_hash: "h4", status: "pending",
        },
      ];
const projectIdOfPending = (s: Record<string, unknown>) => (String(s.file_id).startsWith("c:/work/jev-rpa") ? jev.id : "__global__");
const pendingSummaryNow = () => {
  const by: Record<string, number> = {};
  for (const x of pendingList) by[projectIdOfPending(x)] = (by[projectIdOfPending(x)] ?? 0) + 1;
  return { total: pendingList.length, by_project: by };
};
const dropPending = (id: string) => {
  pendingList = pendingList.filter((x) => x.id !== id);
  void emit("suggestions-changed", pendingSummaryNow());
};
const contextFor = (id: string) => {
  const sg = pendingList.find((x) => x.id === id);
  const evs = ((sg?.evidence as { tool: string; session_id: string; ts: number; quote: string }[]) ?? []).slice(0, 1);
  return evs.map((evd) => ({
    evidence: evd,
    turns: [
      { tool: evd.tool, session_id: evd.session_id, project_id: null, ts: evd.ts - 240_000, role: "user", text: "先看下现在的测试环境配置" },
      { tool: evd.tool, session_id: evd.session_id, project_id: null, ts: evd.ts - 180_000, role: "assistant", text: "当前是从库延迟 2 秒，每次写后要等约 4 秒才能读到。" },
      { tool: evd.tool, session_id: evd.session_id, project_id: null, ts: evd.ts - 60_000, role: "user", text: `太慢了，${evd.quote}` },
      { tool: evd.tool, session_id: evd.session_id, project_id: null, ts: evd.ts, role: "assistant", text: "已经改好，从库延迟现在是 1 秒。" },
    ],
  }));
};

// Sync fixtures: `?sync=out` starts logged out; `?sync=resolve` makes 同步记忆 return a plan with conflicts.
const syncMode = params.get("sync") ?? "in";
const syncStatusNow = () =>
  syncMode === "out"
    ? { logged_in: false, login: null, repo: null, last_sync_at: null, pending: 0, needs: null }
    : {
        logged_in: true,
        login: "chenhao",
        repo: { owner: "chenhao", name: "MemDockHub", url: "https://github.com/chenhao/MemDockHub", private: true },
        last_sync_at: Date.now() - 3 * 3600_000,
        pending: 3,
        needs: null,
      };
const syncPlan = {
  id: "plan-1",
  counts: { pull: 2, push: 1, merged: 1, conflicts: 2, remote_deleted: 1, local_all_deleted: 1, skipped: 1 },
  conflicts: [
    {
      repo_path: "global/claude/CLAUDE.md",
      local_path: "C:\\Users\\Admin\\.claude\\CLAUDE.md",
      local: "# 全局规则\n\n## 数据库\n从库延迟设为 1 秒，写后立读走主库。\n",
      remote: "# 全局规则\n\n## 数据库\n从库延迟设为 2 秒，写后立读必须走主库。\n",
      base: "# 全局规则\n\n## 数据库\n从库延迟设为 2 秒。\n",
      merged:
        "# 全局规则\n\n## 数据库\n<<<<<<< local\n从库延迟设为 1 秒，写后立读走主库。\n||||||| base\n从库延迟设为 2 秒。\n=======\n从库延迟设为 2 秒，写后立读必须走主库。\n>>>>>>> remote\n",
      conflicts: 1,
    },
    {
      repo_path: "projects/jev-rpa/claude/memory/feedback_1.md",
      local_path: "C:\\Users\\Admin\\.claude\\projects\\jev\\memory\\feedback_1.md",
      local: null,
      remote: "每个应用都走同一套主流程。\n",
      base: "每个应用走同一套流程。\n",
      merged: "",
      conflicts: 1,
    },
  ],
  remote_deleted: [{ repo_path: "global/codex/AGENTS.md", local_path: "C:\\Users\\Admin\\.codex\\AGENTS.md" }],
  local_all_deleted: [{ project_key: "github.com/chenhao/old-tool", files: [{ repo_path: "projects/old-tool/claude/CLAUDE.md", local_path: null }] }],
  skipped: [{ repo_path: "global/claude/notes-gbk.md", reason: "not_utf8_local" }],
  secret_hits: [],
  missing_skills: [],
  removed_elsewhere: [],
};
const syncProjectsList = [
  { project_id: "c:/work/jev-rpa", name: "jev-rpa", key: "github.com/chenhao/jev-rpa", dir_key: "jev-rpa", included: true },
  { project_id: "c:/work/synergy", name: "synergy", key: "github.com/freecall/synergy", dir_key: "synergy", included: true },
  { project_id: "c:/work/scratch", name: "scratch", key: null, dir_key: "scratch", included: false },
];
const syncCommits = [
  { id: "c3", message: "sync from DESKTOP-OFFICE", time: Date.now() - 3 * 3600_000, files: ["global/claude/CLAUDE.md", "projects/jev-rpa/claude/CLAUDE.md"] },
  { id: "c2", message: "sync from LAPTOP-HOME", time: Date.now() - 26 * 3600_000, files: ["global/codex/AGENTS.md"] },
];
const syncSummary = { commit: "c4", committed: true, pulled: 2, pushed: 1, merged: 1 };

// Usage page fixtures: deterministic fake numbers for any range.
const usageTotals = (input: number, output: number, read: number, write: number, cost: number, unpriced = 0, requests = Math.round((input + output) / 900)) => ({
  input, output, cache_read: read, cache_write: write, total_tokens: input + output + read + write, requests, cost, unpriced_tokens: unpriced,
});
const addT = (a: ReturnType<typeof usageTotals>, b: ReturnType<typeof usageTotals>) =>
  usageTotals(a.input + b.input, a.output + b.output, a.cache_read + b.cache_read, a.cache_write + b.cache_write, a.cost + b.cost, a.unpriced_tokens + b.unpriced_tokens, a.requests + b.requests);
const usageRow = (key: string, seed: number, scale: number) => {
  const wave = (k: number) => Math.abs(Math.sin(seed * 1.7 + k)) * scale;
  const c = usageTotals(Math.round(wave(1) * 9000), Math.round(wave(2) * 60000), Math.round(wave(3) * 3_000_000), Math.round(wave(4) * 200000), wave(5) * 9);
  const x0 = usageTotals(Math.round(wave(6) * 12000), Math.round(wave(7) * 40000), Math.round(wave(8) * 900000), 0, wave(9) * 3);
  const x1 = usageTotals(Math.round(wave(10) * 3000), Math.round(wave(11) * 8000), Math.round(wave(12) * 200000), 0, 0, Math.round(wave(12) * 211000));
  const codex = addT(x0, x1);
  return {
    key, claude: c, codex, totals: addT(c, codex),
    breakdown: [
      { tool: "claude", model: "claude-opus-5-5", totals: c },
      { tool: "codex", model: "gpt-5.2-codex", totals: x0 },
      { tool: "codex", model: "gpt-6.1-sol", totals: x1 },
    ],
  };
};
const usageReport = (a: Record<string, unknown>) => {
  const view = String(a.view);
  const from = String(a.from);
  const to = String(a.to);
  const keys: string[] = [];
  if (view === "today") for (let h = 0; h < 24; h++) keys.push(String(h).padStart(2, "0"));
  else {
    const d = new Date(`${from}T00:00:00`);
    const end = new Date(`${to}T00:00:00`);
    while (d <= end) {
      const k = view === "monthly" ? `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}` : `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
      if (!keys.includes(k)) keys.push(k);
      d.setDate(d.getDate() + 1);
    }
  }
  const scale = view === "monthly" ? 25 : view === "today" ? 0.15 : 1;
  const rows = keys.map((k, i) => (view === "today" && (i < 8 || i > 16) ? usageRow(k, 0, 0) : usageRow(k, i + 1, scale)));
  const totals = rows.reduce((acc, r) => addT(acc, r.totals), usageTotals(0, 0, 0, 0, 0));
  const days = view === "today" ? 1 : Math.max(1, Math.round((new Date(to).getTime() - new Date(from).getTime()) / 86_400_000) + 1);
  const peak = rows.reduce((p, r) => (r.totals.cost > p.totals.cost ? r : p), rows[0]);
  return {
    view, from, to, rows,
    summary: {
      totals, days, daily_avg_cost: totals.cost / days, daily_avg_tokens: totals.total_tokens / days,
      peak: peak ? { key: peak.key, cost: peak.totals.cost, tokens: peak.totals.total_tokens } : null,
      cache_hit_rate: totals.cache_read / Math.max(1, totals.input + totals.cache_read + totals.cache_write),
    },
    models: ["claude-opus-5-5", "gpt-5.2-codex", "gpt-6.1-sol"],
    unknown_models: ["gpt-6.1-sol"],
    pricing_updated: "2026-11-06",
    scanning: false,
  };
};
let claudeQuotaOn = params.get("quota") === "on";
const usageQuotaNow = () => ({
  claude: claudeQuotaOn
    ? { status: "ok", five_hour: { used_percent: 37.5, resets_at: Date.now() + 2.4 * 3600_000, window_minutes: 300 }, weekly: { used_percent: 84, resets_at: Date.now() + 3.2 * 86_400_000, window_minutes: 10080 }, updated_at: Date.now() }
    : { status: "disabled", five_hour: null, weekly: null, updated_at: null },
  codex: { status: "ok", five_hour: { used_percent: 12, resets_at: Date.now() + 47 * 60_000, window_minutes: 300 }, weekly: { used_percent: 46, resets_at: Date.now() + 5 * 86_400_000, window_minutes: 10080 }, updated_at: Date.now() - 600_000 },
});
const envReport = () => ({
  tools: [
    { name: "claude", installed: true, path: "C:\\Users\\Admin\\AppData\\Roaming\\npm\\claude.cmd", version: "2.1.318", logged_in: true, desktop_app: null },
    { name: "codex", installed: false, path: null, version: null, logged_in: true, desktop_app: "C:\\Users\\Admin\\AppData\\Local\\Microsoft\\WindowsApps\\OpenAI.Codex_2p2nqsd0c76g0" },
    { name: "node", installed: true, path: "C:\\Program Files\\nodejs\\node.exe", version: "24.22.1", logged_in: null, desktop_app: null },
    { name: "npm", installed: true, path: "C:\\Program Files\\nodejs\\npm.cmd", version: "11.6.2", logged_in: null, desktop_app: null },
    { name: "git", installed: true, path: "C:\\Program Files\\Git\\cmd\\git.exe", version: "2.51.0.windows.1", logged_in: null, desktop_app: null },
  ],
  data_dir: "C:\\Users\\Admin\\AppData\\Roaming\\com.memdock.app",
  installing: null,
});

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
        return { language: params.get("lang") ?? "zh", theme, accent: previewAccent, manual_projects: [], claude_quota_enabled: claudeQuotaOn };
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
        if (!a.overwrite)
          return Promise.reject({
            code: "exists",
            message: "already exists",
            dir: "C:\\Users\\Admin\\.claude\\skills\\demo",
            dirs: ["C:\\Users\\Admin\\.claude\\skills\\demo", "C:\\Users\\Admin\\.codex\\skills\\demo"],
          });
        return [];
      case "read_skill_backup_file":
        return { text: skillDoc.replace("Use this skill for", "Use this for").replace("3. Render and check the result", "3. Render"), hash: "bk", has_bom: false, crlf: false, tokens: 100, imports: [], binary: false, too_large: false };
      case "update_skill":
        if (!a.acceptLocalChanges) return Promise.reject({ code: "local_changes", message: "local changes" });
        return { ...skillsList[0], source: { ...skillsList[0].source, commit: "0f1e2d3c4b5a" } };
      case "delete_skill":
        return { id: "1730800000000", created_at: Date.now(), size_bytes: 182_000 };
      case "sync_skill":
        return skillsList[0];
      case "list_skill_backups":
        return skillBackups;
      case "list_deleted_skill_backups":
        return params.get("deleted") === "0" ? [] : deletedSkills;
      case "reveal_backup_folder":
        return `C:\\Users\\Admin\\AppData\\Roaming\\MemDock\\skill-backups\\${String(a.bucket)}`;
      case "restore_deleted_skill":
        return "C:\\Users\\Admin\\.claude\\skills\\pdf-notes";
      case "rollback_skill":
        return skillsList[0];
      case "get_extract_config":
        return extractConfig;
      case "set_extract_config":
        extractConfig = a.config as Record<string, unknown>;
        return extractConfig;
      case "extract_runners":
        return [{ tool: "claude", available: true, program: "claude" }, { tool: "codex", available: false }];
      case "list_extract_runs":
        return extractRuns;
      case "start_extraction":
        return extractRuns[0];
      case "cancel_extraction":
        return null;
      case "reset_extract_data":
        return null;
      case "list_suggestions":
        return pendingList;
      case "pending_summary":
        return pendingSummaryNow();
      case "apply_suggestion":
        if (a.id === "sg-stale") return { kind: "stale" };
        dropPending(String(a.id));
        return { kind: "applied", hash: "hx", saved: true };
      case "reject_suggestion":
        dropPending(String(a.id));
        return null;
      case "accept_suggestions":
        return (a.ids as string[]).map((id) => {
          if (id === "sg-stale") return { id, outcome: { kind: "stale" } };
          dropPending(id);
          return { id, outcome: { kind: "applied", hash: "hx", saved: true } };
        });
      case "suggestion_context":
        return contextFor(String(a.id));
      case "sync_status":
      case "set_repo_name":
      case "ack_public_repo":
        return syncStatusNow();
      case "logout":
        return { ...syncStatusNow(), logged_in: false, login: null, repo: null };
      case "login_start":
        return { user_code: "WDJB-MJHT", verification_uri: "https://github.com/login/device", expires_in: 900 };
      case "login_cancel":
      case "open_verification_url":
      case "sync_discard":
      case "restore_from_history":
        return null;
      case "sync_start":
        return new Promise((resolve) =>
          setTimeout(
            () =>
              resolve(
                syncMode === "resolve"
                  ? { kind: "needs_resolution", plan: syncPlan }
                  : { kind: "done", summary: syncSummary, skipped: syncPlan.skipped, missing_skills: [], removed_elsewhere: [] },
              ),
            600,
          ),
        );
      case "sync_finish":
        return syncSummary;
      case "sync_projects":
      case "set_included_local":
        return syncProjectsList;
      case "sync_history":
        return syncCommits;
      case "sync_file_at":
        return TEXT;
      case "usage_summary":
        return usageReport(a);
      case "usage_quota":
        return usageQuotaNow();
      case "set_claude_quota_enabled":
        claudeQuotaOn = Boolean(a.enabled);
        return { language: params.get("lang") ?? "zh", theme, accent: previewAccent, manual_projects: [], claude_quota_enabled: claudeQuotaOn };
      case "usage_rescan":
        return false;
      case "usage_export_csv":
        return null;
      case "plugin:dialog|save":
        return null;
      case "env_detect":
        return envReport();
      case "env_install":
        setTimeout(() => {
          void emit("env://install-log", { kind: "log", tool: String(a.tool), line: "$ npm install -g @openai/codex" });
          void emit("env://install-log", { kind: "log", tool: String(a.tool), line: "added 1 package in 3s" });
          void emit("env://install-done", { kind: "done", tool: String(a.tool), ok: true, code: 0, error: null });
        }, 400);
        return null;
      case "open_data_dir":
        return envReport().data_dir;
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
    if (flow === "add-exists") {
      wait(btn("添加 skill"), (b) => {
        b.click();
        wait(() => Array.from(document.querySelectorAll<HTMLElement>('[role="tab"]')).find((t) => t.textContent === "空白模板"), (t) => {
          t.click();
          setTimeout(() => {
            const n = document.getElementById("add-tpl-name") as HTMLInputElement | null;
            if (n) setValue(n, "demo");
            const boxes = Array.from(document.querySelectorAll<HTMLInputElement>('input[type="checkbox"]'));
            boxes.find((x) => x.parentElement?.textContent === "Codex")?.click();
            setTimeout(() => btn("安装")()?.click(), 200);
          }, 150);
        });
      });
      return;
    }
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

// `?extract=running` emits a progress event once the page listens; `?expand=1` opens the first expandable run row.
if (extractRunning) {
  setTimeout(() => void emit("extract-progress", { project: "synergy", index: 2, total: 5 }), 1200);
}
if (params.get("expand")) {
  let tries = 0;
  const timer = setInterval(() => {
    const b = document.querySelector<HTMLElement>('button[aria-expanded="false"][aria-label="展开详情"], button[aria-expanded="false"][aria-label="Show details"]');
    if (b) {
      clearInterval(timer);
      b.click();
    } else if (++tries > 100) clearInterval(timer);
  }, 100);
}

// `?dirty=1` toggles "包含全局记忆" and clicks 立即运行 to show the unsaved-changes confirmation.
if (params.get("dirty")) {
  let tries = 0;
  const timer = setInterval(() => {
    const box = document.getElementById("extract-global") as HTMLInputElement | null;
    if (box) {
      clearInterval(timer);
      box.click();
      setTimeout(() => Array.from(document.querySelectorAll("button")).find((b) => b.textContent?.trim() === "立即运行")?.click(), 300);
    } else if (++tries > 100) clearInterval(timer);
  }, 100);
}

// `?review=edit|evidence|stale` drives the review page for screenshots (headless cannot click).
const reviewState = params.get("review");
if (reviewState === "edit" || reviewState === "evidence" || reviewState === "stale") {
  let tries = 0;
  const timer = setInterval(() => {
    tries++;
    if (tries > 150) clearInterval(timer);
    const cards = Array.from(document.querySelectorAll<HTMLElement>("article"));
    if (cards.length === 0) return;
    clearInterval(timer);
    const btn = (card: HTMLElement | undefined, label: string) =>
      Array.from(card?.querySelectorAll("button") ?? []).find((b) => b.textContent?.trim() === label);
    if (reviewState === "edit") btn(cards[0], "编辑后通过")?.click();
    if (reviewState === "evidence") btn(cards[0], "查看原对话")?.click();
    if (reviewState === "stale") btn(cards[cards.length - 1], "通过")?.click();
  }, 100);
}

// `?login=code` opens the login dialog and requests a code; `?sync=resolve&page=同步` clicks 同步记忆.
const clickWhen = (label: string, then?: () => void) => {
  let tries = 0;
  const timer = setInterval(() => {
    const btn = Array.from(document.querySelectorAll("button")).find((b) => b.textContent?.trim() === label);
    if (btn && !btn.disabled) {
      clearInterval(timer);
      btn.click();
      then?.();
    } else if (++tries > 100) clearInterval(timer);
  }, 100);
};
if (params.get("login") === "code") clickWhen("登录 GitHub", () => clickWhen("开始登录"));
else if (params.get("login") === "intro") clickWhen("登录 GitHub");
if (syncMode === "resolve") clickWhen("同步记忆");
