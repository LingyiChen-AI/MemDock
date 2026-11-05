# Memdock 计划 1：基础框架 + 记忆总览 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 搭出可在 macOS 和 Windows 上运行的 Memdock 桌面应用骨架，并完成「记忆总览」：自动发现项目，统一列出、搜索、编辑 Claude Code 与 Codex 的全局 / 项目记忆，写入前留快照、可回滚，外部改动实时刷新。

**Architecture:** Tauri 2 应用。Rust 后端负责一切文件系统操作（发现项目、扫描记忆、读写、快照、搜索、文件监听、只读读取 Codex sqlite），通过 Tauri command 暴露给前端；React 前端只做展示和编辑，不直接碰文件。Rust 侧所有「家目录」都经 `AgentPaths` 注入，测试用临时目录伪造 `~/.claude`、`~/.codex`。

**Tech Stack:** Tauri 2、Rust（serde、serde_json、rusqlite[bundled]、notify、walkdir、sha2、chrono、dirs、tempfile[dev]）、React 18 + TypeScript + Vite、pnpm、i18next + react-i18next、@uiw/react-codemirror + @codemirror/lang-markdown、react-markdown、diff、@tauri-apps/plugin-dialog（+ Rust 侧 tauri-plugin-dialog）、Vitest + @testing-library/react。

**Spec:** `docs/superpowers/specs/2026-11-05-memdock-design.md`（本计划覆盖第 3、4 节和 5.1 节；5.8 中「Codex 记忆库只读」一条）

## Global Constraints

- 平台：macOS + Windows 都必须能用；CI 两个平台都跑。
- 技术栈：Tauri 2 + React + TypeScript（后端 Rust）。
- 开源，许可证 MIT。
- 界面中英双语，默认跟随系统语言。
- 不做遥测，不发任何网络请求（本计划范围内应用零外发）。
- 不破坏 Claude Code / Codex 自身文件：写入保持原有编码细节（BOM、换行符）；symlink 不被替换成实体文件；`~/.codex/memories_1.sqlite` 只读打开，绝不写。
- 本机数据位置（spec 第 4 节）：
  - Claude 全局：`~/.claude/CLAUDE.md`；项目：`<项目>/CLAUDE.md`、`<项目>/.claude/CLAUDE.md`、`<项目>/CLAUDE.local.md`；自动记忆：`~/.claude/projects/<编码>/memory/`（`MEMORY.md` + 每条 `.md`）。
  - Codex 全局：`~/.codex/AGENTS.md`；项目：`<项目>/AGENTS.md`；记忆库：`~/.codex/memories_1.sqlite` 表 `stage1_outputs`（列 `thread_id, raw_memory, rollout_summary, generated_at`）。
  - Claude 会话：`~/.claude/projects/<编码>/*.jsonl`，行内有 `"cwd"`；Codex 会话：`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`，首个 `session_meta` 行的 `payload.cwd`。
- **界面视觉一律遵循 `docs/design/memdock-design-system.md`**：Tailwind CSS v4 + shadcn/ui；颜色只用该文件第 3 节的语义变量（`:root` 亮色、`.dark` 暗色），组件里不得出现裸 hex；默认暗色，支持亮色 / 跟随系统；图标只用 lucide-react，禁止 emoji 当图标；字体用 @fontsource 本地打包（Inter Variable / IBM Plex Sans / JetBrains Mono Variable），不引用任何 CDN；正文 13px、最小 12px；焦点环 2px `--ring`；动效 150–200ms 且尊重 `prefers-reduced-motion`。视觉参考稿：`.superpowers/brainstorm/1459-1793865330/content/styled-memdock.html`。
- 应用数据目录：Tauri `app_data_dir()`（以下记作 `<appdata>`）；设置 `<appdata>/settings.json`，快照 `<appdata>/snapshots/`。

## Review Focus

1. **编辑中文件被外部改了**（CLI 或别的编辑器写了同一个 `CLAUDE.md`）→ 保存必须失败并提示「文件已在外部修改」，不能静默覆盖。测试见 Task 6 `write_rejects_when_disk_changed`。
2. **Windows 路径大小写 / 分隔符不同的同一项目**（Claude 记录 `C:\codes\x`、Codex 记录 `c:/codes/x`）→ 树里只出现一个项目。测试见 Task 3 `dedupes_same_project_different_forms`。
3. **项目目录已删除或移走**，但 `~/.claude/projects` 里还有记录 → 项目显示为「目录不存在」，扫描不报错。测试见 Task 4 `missing_project_dir_is_flagged`。
4. **带 BOM 或 CRLF 的记忆文件**→ 编辑保存后 BOM 和换行风格保持不变。测试见 Task 6 `write_preserves_bom_and_crlf`。
5. **Codex 正在运行、sqlite 被占用或库文件不存在**→ Codex 记忆节点显示「暂不可用」，其余功能正常。测试见 Task 8 `missing_db_returns_unavailable`。

---

## 文件结构

```
memdock/  (仓库根 = C:\codes\cc-memory)
  LICENSE, README.md, package.json, vite.config.ts, tsconfig.json, index.html
  .github/workflows/ci.yml
  src-tauri/
    Cargo.toml, tauri.conf.json, build.rs, capabilities/default.json
    src/
      main.rs            入口，调 memdock_lib::run()
      lib.rs             Tauri builder、注册 command、启动 watcher
      paths.rs           AgentPaths：各家目录解析，可注入
      settings.rs        Settings 读写
      model.rs           共享类型：Tool、Scope、MemoryKind、MemoryFile、Project
      projects.rs        项目发现与去重
      memory/
        mod.rs
        scan.rs          列出全局 / 项目记忆文件
        analyze.rs       token 估算、@import 解析
        io.rs            读写、编码保持、冲突检测
        snapshot.rs      快照与回滚
        search.rs        全文搜索
        codex_db.rs      Codex 记忆库只读
      watcher.rs         文件监听 → 前端事件
      commands.rs        全部 #[tauri::command]
  src/
    main.tsx, App.tsx
    styles/globals.css, theme/ThemeProvider.tsx, components/ui/  (shadcn 生成)
    i18n/index.ts, i18n/zh.json, i18n/en.json
    api/types.ts, api/memory.ts
    layout/AppShell.tsx
    pages/memory/MemoryPage.tsx, MemoryTree.tsx, MemoryEditor.tsx, SearchPanel.tsx, HistoryDrawer.tsx
    pages/Placeholder.tsx
    test/setup.ts
```

---

### Task 1: 工程脚手架、许可证、CI

**Files:**
- Create: 上面文件结构中的根配置、`src-tauri/` 骨架、`src/main.tsx`、`src/App.tsx`、`LICENSE`（MIT，版权人「Memdock contributors」）、`README.md`（中英两段简介）、`.gitignore`（含 `node_modules/`、`src-tauri/target/`、`.superpowers/`）、`.github/workflows/ci.yml`
- Test: `src/App.test.tsx`

**Interfaces:**
- Produces: `memdock_lib::run()`；crate 名 `memdock_lib`；Tauri `identifier` = `dev.memdock.app`，`productName` = `Memdock`；前端测试命令 `pnpm test`（= `vitest run`），Rust 测试命令 `cargo test --manifest-path src-tauri/Cargo.toml`。

- [ ] **Step 1:** 在仓库根 `git init`，用 `pnpm create tauri-app`（React + TypeScript 模板）生成到当前目录，按上述名称改 `tauri.conf.json` 与 `Cargo.toml`。
- [ ] **Step 1b:** 接入 Tailwind CSS v4（`@tailwindcss/vite`）与 shadcn/ui（`pnpm dlx shadcn@latest init`，base=radix）；按设计规范第 3 节在 `src/styles/globals.css` 写全套 `:root` / `.dark` 语义变量（含 `--sidebar`、`--accent-text`、`--warning`、`--tool-claude`、`--tool-codex`）并用 `@theme inline` 暴露；按第 4 节设置字体栈与 13px 正文；安装 `lucide-react`、`@fontsource-variable/inter`、`@fontsource/ibm-plex-sans`、`@fontsource-variable/jetbrains-mono` 并在 `main.tsx` 引入。
- [ ] **Step 2: 写失败测试** `src/App.test.tsx`：渲染 `<App />`，断言 `screen.getByText("Memdock")` 存在。配置 Vitest（jsdom、`src/test/setup.ts` 引入 `@testing-library/jest-dom`）。
- [ ] **Step 3:** 运行 `pnpm test`，预期 FAIL（找不到 “Memdock”）。
- [ ] **Step 4:** `App.tsx` 暂时渲染标题 `Memdock`。
- [ ] **Step 5:** 运行 `pnpm test` 与 `cargo test --manifest-path src-tauri/Cargo.toml`，预期均 PASS；`pnpm tauri dev` 能打开窗口。
- [ ] **Step 6:** `ci.yml`：`strategy.matrix.os: [macos-latest, windows-latest]`，步骤为 checkout → setup pnpm/node 20 → setup Rust stable → `pnpm install` → `pnpm test` → `cargo test --manifest-path src-tauri/Cargo.toml` → `pnpm tauri build --debug`。
- [ ] **Step 7: Commit** `chore: scaffold Memdock (Tauri 2 + React + TS), MIT license, CI`

---

### Task 2: 路径解析与设置

**Files:**
- Create: `src-tauri/src/paths.rs`、`src-tauri/src/settings.rs`、`src-tauri/src/model.rs`
- Test: 同文件内 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
  - `pub struct AgentPaths { pub claude_home: PathBuf, pub codex_home: PathBuf }`
  - `impl AgentPaths { pub fn from_home(home: &Path) -> Self; pub fn detect() -> Self }`（`claude_home = home/.claude`，`codex_home = home/.codex`；`detect` 用 `dirs::home_dir()`，并尊重环境变量 `CLAUDE_CONFIG_DIR`、`CODEX_HOME` 的覆盖）
  - `#[derive(Serialize, Deserialize, Default)] pub struct Settings { pub language: Option<String>, pub theme: Option<String> /* "dark" | "light" | "system"，空 = "dark" */, pub manual_projects: Vec<String> }`
  - `pub fn load_settings(path: &Path) -> Settings`（文件不存在或损坏返回 `Default`）；`pub fn save_settings(path: &Path, s: &Settings) -> io::Result<()>`（先写临时文件再 rename）
  - `model.rs`：`#[derive(Serialize, Clone, Copy, PartialEq, Eq, Hash, Debug)] #[serde(rename_all="snake_case")] pub enum Tool { Claude, Codex }`

- [ ] **Step 1: 写失败测试**：`from_home_builds_expected_dirs`（断言两个子路径）；`detect_respects_env_overrides`（设 `CODEX_HOME` 为临时目录，断言 `codex_home` 等于它）；`load_settings_missing_or_corrupt_returns_default`；`save_then_load_roundtrip`（`manual_projects == ["/a"]`）。
- [ ] **Step 2:** `cargo test --manifest-path src-tauri/Cargo.toml paths settings`，预期 FAIL（未定义）。
- [ ] **Step 3:** 实现上述签名。
- [ ] **Step 4:** 同命令，预期 PASS。
- [ ] **Step 5: Commit** `feat: agent home paths and settings store`

---

### Task 3: 项目发现与去重

**Files:**
- Create: `src-tauri/src/projects.rs`；`model.rs` 增加 `Project`
- Test: `projects.rs` 内测试，fixture 用 `tempfile::TempDir` 现场造 jsonl

**Interfaces:**
- Consumes: `AgentPaths`、`Settings.manual_projects`
- Produces:
  - `#[derive(Serialize, Clone, Debug)] pub struct Project { pub id: String, pub path: String, pub name: String, pub exists: bool, pub sources: Vec<ProjectSource> }`，`#[serde(rename_all="snake_case")] pub enum ProjectSource { ClaudeSessions, CodexSessions, Manual }`
  - `pub fn normalize_path(p: &str) -> String`：统一分隔符为 `/`；Windows 上（`cfg!(windows)`）整体转小写；去掉末尾 `/`。`Project.id = normalize_path(path)`。
  - `pub fn discover_projects(paths: &AgentPaths, manual: &[String]) -> Vec<Project>`：按 `name` 排序。
- 规则：
  - Claude：遍历 `claude_home/projects/*/`，在该目录的 `*.jsonl` 里逐行找第一个含 `"cwd"` 字段的 JSON 行，取其 `cwd`；**每个文件最多读前 200 行**；取不到 cwd 的目录跳过（不要反解目录名，编码有损）。同时记录 `encoded_dir`（供 Task 4 定位自动记忆），放在 `pub fn claude_project_dirs(paths: &AgentPaths) -> HashMap<String /*project id*/, PathBuf /*encoded dir*/>` 中。
  - Codex：遍历 `codex_home/sessions/**/rollout-*.jsonl`，读首行 `{"type":"session_meta","payload":{"cwd":...}}`。
  - 合并：同一 `id` 合并 `sources`；`path` 取首个出现的原始写法；`name` = 路径最后一段；`exists = Path::new(path).is_dir()`。

- [ ] **Step 1: 写失败测试**：
  - `discovers_from_claude_jsonl_cwd`：造 `projects/C--x-a/s1.jsonl` 首行无 cwd、第二行 `{"cwd":"<tmp>/a"}`，断言得到 1 个项目，`name=="a"`，`sources==[ClaudeSessions]`。
  - `discovers_from_codex_session_meta`。
  - `dedupes_same_project_different_forms`：Claude 记 `C:\\codes\\x`、Codex 记 `c:/codes/x`，在 Windows 下断言只有 1 个项目且 `sources` 含两者（非 Windows 用 `#[cfg(windows)]` 跳过）；另加一个全平台用例：`/a/b/` 与 `/a/b` 合并为 1 个。
  - `manual_projects_included`。
  - `reads_at_most_200_lines`：cwd 只出现在第 201 行的文件不产生项目。
- [ ] **Step 2:** 运行 `cargo test ... projects`，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS。
- [ ] **Step 5: Commit** `feat: discover projects from Claude/Codex sessions with dedupe`

---

### Task 4: 记忆文件扫描

**Files:**
- Create: `src-tauri/src/memory/mod.rs`、`memory/scan.rs`；`model.rs` 增加类型
- Test: `scan.rs` 内测试

**Interfaces:**
- Consumes: `AgentPaths`、`Project`、`claude_project_dirs`
- Produces（`model.rs`）：
  - `#[serde(tag="type", rename_all="snake_case")] pub enum Scope { Global, Project { project_id: String } }`
  - `#[serde(rename_all="snake_case")] pub enum MemoryKind { ClaudeInstructions, ClaudeLocal, ClaudeAuto, CodexAgents, CodexDb }`
  - `pub struct MemoryFile { pub id: String /* = normalize_path(path)，CodexDb 为 "codex-db" */, pub tool: Tool, pub scope: Scope, pub kind: MemoryKind, pub path: String, pub exists: bool, pub is_symlink: bool, pub size_bytes: u64, pub modified_at: Option<i64> /*unix ms*/ }`
  - `pub struct ProjectMemories { pub project: Project, pub files: Vec<MemoryFile> }`
  - `pub struct MemoryIndex { pub global: Vec<MemoryFile>, pub projects: Vec<ProjectMemories> }`
- 函数：`pub fn scan_all(paths: &AgentPaths, projects: &[Project]) -> MemoryIndex`
- 规则：
  - 全局：`claude_home/CLAUDE.md`（ClaudeInstructions）、`codex_home/AGENTS.md`（CodexAgents）——**不存在也列出**，`exists=false`，便于用户新建；`codex_home/memories_1.sqlite` 存在时列一条 CodexDb。
  - 项目：`CLAUDE.md`、`.claude/CLAUDE.md`（ClaudeInstructions）、`CLAUDE.local.md`（ClaudeLocal）、`AGENTS.md`（CodexAgents）只列存在的；项目的 Claude 自动记忆目录 `<encoded_dir>/memory/*.md` 每个文件一条 ClaudeAuto（`MEMORY.md` 排第一，其余按文件名）。
  - 项目目录不存在时：`files` 只含自动记忆（它在 `~/.claude` 下仍可读），`project.exists=false`，不报错。
  - symlink 用 `symlink_metadata` 判断，`is_symlink=true`，`path` 保留链接路径。

- [ ] **Step 1: 写失败测试**：`lists_global_even_if_missing`；`lists_project_instruction_files`（造 `CLAUDE.md`、`CLAUDE.local.md`、`AGENTS.md`，断言 3 条且 kind 正确）；`lists_auto_memory_with_index_first`；`missing_project_dir_is_flagged`；`codex_db_listed_when_present`；`symlink_is_flagged`（`#[cfg(unix)]`）。
- [ ] **Step 2:** `cargo test ... memory::scan`，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS。
- [ ] **Step 5: Commit** `feat: scan global and project memory files`

---

### Task 5: 内容分析（token 估算、@import）

**Files:**
- Create: `src-tauri/src/memory/analyze.rs`
- Test: 同文件

**Interfaces:**
- Produces:
  - `pub fn estimate_tokens(text: &str) -> u32`：ASCII 字符数 / 4（向上取整）+ 非 ASCII 字符数 × 1。
  - `pub struct ImportRef { pub raw: String, pub resolved: String, pub exists: bool, pub line: u32 /*1-based*/ }`
  - `pub fn find_imports(text: &str, base_dir: &Path, home: &Path) -> Vec<ImportRef>`：匹配行内 `@` 开头、后接以 `/`、`./`、`../`、`~/` 开头或含 `.md` 的路径 token；跳过 ``` 代码块内的内容和行内反引号内的内容；`~/` 用 `home` 展开，相对路径相对 `base_dir`。

- [ ] **Step 1: 写失败测试**：`estimate_ascii`（`"abcdefgh"` → 2）；`estimate_cjk`（`"中文"` → 2）；`finds_relative_and_home_imports`（`"see @./docs/a.md and @~/x.md"` → 2 条，resolved 正确）；`ignores_imports_in_code_blocks`；`ignores_email_like_text`（`"mail a@b.com"` → 0 条）。
- [ ] **Step 2:** `cargo test ... analyze`，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS。
- [ ] **Step 5: Commit** `feat: token estimate and @import detection`

---

### Task 6: 读写与快照

**Files:**
- Create: `src-tauri/src/memory/io.rs`、`memory/snapshot.rs`
- Test: 同文件

**Interfaces:**
- Produces:
  - `pub struct FileContent { pub text: String /*去掉 BOM，换行统一为 \n*/, pub hash: String /*原始字节 sha256 hex；文件不存在为 ""*/, pub has_bom: bool, pub crlf: bool }`
  - `pub fn read_memory(path: &Path) -> io::Result<FileContent>`（不存在返回空 text、hash ""）
  - `#[derive(Debug)] pub enum WriteError { DiskChanged { current_hash: String }, Io(io::Error) }`
  - `pub fn write_memory(path: &Path, text: &str, expected_hash: &str, snapshots: &SnapshotStore) -> Result<FileContent, WriteError>`：步骤 = 重新计算磁盘 hash，与 `expected_hash` 不同则 `DiskChanged`；文件存在时先 `snapshots.save(path, 原始字节)`；按原文件的 `has_bom`/`crlf` 还原编码细节后写入；**symlink 时写到链接目标**（`fs::write` 对链接路径本身即写目标，禁止先删后建）；非 symlink 用「同目录临时文件 + rename」原子写；父目录不存在则创建。
  - `pub struct SnapshotStore { root: PathBuf }`，`pub struct SnapshotInfo { pub id: String /*时间戳毫秒*/, pub created_at: i64, pub size_bytes: u64 }`
  - `impl SnapshotStore { pub fn new(root: PathBuf) -> Self; pub fn save(&self, path: &Path, bytes: &[u8]) -> io::Result<SnapshotInfo>; pub fn list(&self, path: &Path) -> io::Result<Vec<SnapshotInfo>> /*新→旧*/; pub fn read(&self, path: &Path, id: &str) -> io::Result<Vec<u8>> }`
  - 存储布局：`root/<sha256(normalize_path(path)) 前 16 位>/<id>.bak`，同目录 `meta.json` 记原路径；每个文件最多保留 50 份，超出删最旧。
  - 回滚不单独提供函数：前端读取快照内容后走 `write_memory`（因此回滚前的状态也会被快照）。

- [ ] **Step 1: 写失败测试**：`read_strips_bom_and_normalizes_crlf`；`write_preserves_bom_and_crlf`（原文件 BOM+CRLF，写 `"a\nb"`，磁盘字节为 `EF BB BF 61 0D 0A 62`）；`write_rejects_when_disk_changed`（读后外部改写，再写 → `DiskChanged`，磁盘内容未变）；`write_creates_snapshot_of_previous`（写后 `list` 有 1 条，`read` 等于旧内容）；`write_new_file_creates_parent_dirs_no_snapshot`；`snapshot_keeps_at_most_50`；`write_through_symlink_keeps_link`（`#[cfg(unix)]`，写后 `symlink_metadata` 仍是链接且目标内容已更新）。
- [ ] **Step 2:** `cargo test ... memory::io memory::snapshot`，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS。
- [ ] **Step 5: Commit** `feat: safe memory read/write with conflict check and snapshots`

---

### Task 7: 全文搜索

**Files:**
- Create: `src-tauri/src/memory/search.rs`
- Test: 同文件

**Interfaces:**
- Consumes: `MemoryIndex`、`read_memory`
- Produces:
  - `pub struct SearchHit { pub file_id: String, pub line: u32, pub preview: String /*该行，超 200 字符截断*/ }`
  - `pub fn search(index: &MemoryIndex, query: &str, limit: usize) -> Vec<SearchHit>`：大小写不敏感子串匹配；跳过 `exists=false` 与 `CodexDb`；空白 query 返回空；达到 `limit` 即停。

- [ ] **Step 1: 写失败测试**：`finds_case_insensitive`；`finds_cjk`；`respects_limit`；`blank_query_returns_empty`。
- [ ] **Step 2:** `cargo test ... memory::search`，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS。
- [ ] **Step 5: Commit** `feat: full-text search across memories`

---

### Task 8: Codex 记忆库只读

**Files:**
- Create: `src-tauri/src/memory/codex_db.rs`
- Test: 同文件（测试里用 rusqlite 现场建同结构的库）

**Interfaces:**
- Produces:
  - `pub struct CodexMemoryEntry { pub thread_id: String, pub raw_memory: String, pub rollout_summary: String, pub generated_at: i64 }`
  - `#[serde(tag="status", rename_all="snake_case")] pub enum CodexDbResult { Ok { entries: Vec<CodexMemoryEntry> }, Unavailable { reason: String } }`
  - `pub fn read_codex_memories(db_path: &Path) -> CodexDbResult`：以 URI `file:<path>?mode=ro` + `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_URI` 打开，`busy_timeout` 1 秒；按 `generated_at DESC` 读 `stage1_outputs`；文件不存在、表不存在、被锁超时都返回 `Unavailable`，不 panic。

- [ ] **Step 1: 写失败测试**：`reads_entries_newest_first`；`missing_db_returns_unavailable`；`missing_table_returns_unavailable`；`opening_does_not_modify_file`（读前后文件字节与 mtime 不变）。
- [ ] **Step 2:** `cargo test ... codex_db`，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS。
- [ ] **Step 5: Commit** `feat: read-only Codex memory db`

---

### Task 9: Tauri 命令、文件监听、前端 API

**Files:**
- Create: `src-tauri/src/commands.rs`、`src-tauri/src/watcher.rs`、`src/api/types.ts`、`src/api/memory.ts`
- Modify: `src-tauri/src/lib.rs`
- Test: `commands.rs` 内测试（直接调用内部 `*_impl` 函数）；`src/api/memory.test.ts`（mock `@tauri-apps/api/core` 的 `invoke`）

**Interfaces:**
- Consumes: Task 2–8 全部
- Produces（命令名即前端调用名，参数 camelCase）：
  - `list_memories() -> MemoryIndex`
  - `read_memory_file(path: String) -> FileContentDto { text, hash, tokens: u32, imports: Vec<ImportRef> }`
  - `write_memory_file(path: String, text: String, expectedHash: String) -> Result<FileContentDto, CommandError>`；`CommandError` 序列化为 `{ code: "disk_changed" | "io" | "forbidden", message: String }`
  - `search_memories(query: String) -> Vec<SearchHit>`（limit 200）
  - `list_snapshots(path: String) -> Vec<SnapshotInfo>`；`read_snapshot(path: String, id: String) -> String`
  - `read_codex_db() -> CodexDbResult`
  - `add_manual_project(path: String) -> MemoryIndex`；`remove_manual_project(path: String) -> MemoryIndex`
  - 事件 `memory-changed`，payload `{ paths: string[] }`
- 安全规则：`read_memory_file`/`write_memory_file`/快照命令只接受**当前 `MemoryIndex` 中出现过的路径**（`id` 比对），否则返回 `forbidden`；`write_memory_file` 拒绝 `CodexDb`。
- watcher：用 `notify` 递归监听 `claude_home`（只关心 `CLAUDE.md` 与 `projects/*/memory/`）、`codex_home/AGENTS.md`、每个存在的项目根目录（**非递归**）及其 `.claude/`；300ms 去抖后发 `memory-changed`。
- `src/api/types.ts` 手写与 Rust 一一对应的 TS 类型；`src/api/memory.ts` 每个命令一个函数，如 `listMemories(): Promise<MemoryIndex>`、`writeMemoryFile(path, text, expectedHash): Promise<FileContentDto>`（失败时抛出 `CommandError` 对象）、`onMemoryChanged(cb): Promise<UnlistenFn>`。

- [ ] **Step 1: 写失败测试**（Rust）：`write_outside_index_is_forbidden`；`write_codex_db_is_forbidden`；`manual_project_add_remove_persists`。（TS）`writeMemoryFile passes camelCase args`；`writeMemoryFile rethrows CommandError`。
- [ ] **Step 2:** 运行 `cargo test ... commands` 与 `pnpm test src/api`，预期 FAIL。
- [ ] **Step 3:** 实现命令、watcher、TS API；`lib.rs` 中 `manage` 一个 `AppState { paths, settings_path, snapshots, index: Mutex<MemoryIndex> }`，启动时扫描一次，`list_memories` 与手动项目增删时重扫。
- [ ] **Step 4:** 两个测试命令预期 PASS；`pnpm tauri dev` 下在 devtools 执行 `__TAURI__.core.invoke('list_memories')` 能返回本机真实索引。
- [ ] **Step 5: Commit** `feat: memory commands, file watcher, typed frontend API`

---

### Task 10: 应用外壳与中英双语

**Files:**
- Create: `src/i18n/index.ts`、`src/i18n/zh.json`、`src/i18n/en.json`、`src/layout/AppShell.tsx`、`src/pages/Placeholder.tsx`
- Modify: `src/App.tsx`、`src/App.test.tsx`
- Test: `src/layout/AppShell.test.tsx`

**Interfaces:**
- Produces:
  - 导航项 key 顺序固定：`memory`、`review`、`extract`、`skills`、`usage`、`settings`；中文文案「记忆总览 / 待审核 / 提取任务 / Skill 管理 / 用量统计 / 环境与设置」，英文「Memories / Review / Extraction / Skills / Usage / Settings」。
  - `<AppShell current: NavKey; onNavigate(key: NavKey) => void>`；除 `memory` 外的页面渲染 `<Placeholder />`，文案「即将推出 / Coming soon」。
  - 语言：`settings.language` 为空时用 `navigator.language` 以 `zh` 开头判为中文，否则英文。
  - 侧栏用 shadcn `Sidebar`：顶部品牌（绿色方块 logo + 「Memdock」），导航项配 lucide 图标（BookOpen、ListChecks、Sparkles、Package、BarChart3、Settings），选中态 = `--muted` 底 + 左侧 2px `--primary` 竖条；宽 208px，可折叠为 52px 图标栏（折叠后图标带 tooltip）；底部预留状态区（计划 4 放「待同步」）。
  - 主题：`<ThemeProvider>`（`src/theme/ThemeProvider.tsx`）在 `<html>` 上切换 `.dark`；`settings.theme` 为空按 `dark`；`system` 时监听 `prefers-color-scheme`。Placeholder 页提供语言与主题切换控件，直到计划 5 的设置页接手。
  - 最小窗口 1024×640（`tauri.conf.json`）。

- [ ] **Step 1: 写失败测试**：`renders six nav items in order (zh)`；`switches language to en`；`clicking nav calls onNavigate`；`non-memory pages show placeholder`；`theme defaults to dark class on html`；`system theme follows prefers-color-scheme`。
- [ ] **Step 2:** `pnpm test src/layout`，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS。
- [ ] **Step 5: Commit** `feat: app shell with zh/en i18n`

---

### Task 11: 记忆树与编辑器

**Files:**
- Create: `src/pages/memory/MemoryPage.tsx`、`MemoryTree.tsx`、`MemoryEditor.tsx`
- Test: `MemoryTree.test.tsx`、`MemoryEditor.test.tsx`（mock `src/api/memory.ts`）

**Interfaces:**
- Consumes: Task 9 的 API 与类型
- Produces:
  - `<MemoryTree index: MemoryIndex; selectedId?: string; onSelect(file: MemoryFile) => void>`：根节点「全局」+ 每个项目（`name`，悬停显示 `path`）；项目 `exists=false` 时名称后显示「目录不存在」；文件节点显示工具标签（CC / Codex）与文件名，ClaudeAuto 归在「自动记忆（N 条）」子节点下；`exists=false` 的全局文件显示「未创建」。
  - `<MemoryEditor file: MemoryFile>`：加载 `readMemoryFile`；顶栏显示路径、约 N tokens、「保存」「历史版本」按钮；编辑区 CodeMirror（markdown），右侧可切换预览（react-markdown）；`imports` 在编辑器下方列出，`exists=false` 的标红；保存调用 `writeMemoryFile(path, text, hash)`，成功后更新 hash；`code==="disk_changed"` 时弹出提示「文件已在外部修改」，提供「重新加载（丢弃我的修改）」与「取消」两个按钮；有未保存修改时切换文件先确认。
  - 树顶部「添加项目目录」按钮：用 `@tauri-apps/plugin-dialog` 的 `open({ directory: true })` 选目录后调 `addManualProject`；来源仅为 Manual 的项目右键可「移除」（`removeManualProject`）。
  - `imports` 列表中 `exists=true` 且在当前索引里的路径可点击打开；不在索引里的只显示，不可打开。
  - CodexDb 节点打开只读列表（`readCodexDb`），`Unavailable` 时显示「暂不可用：{reason}」。
  - 视觉按设计规范第 7 节与参考稿：树节点 28px、工具标签（CC / Codex，工具色 18% 底）、记忆树左栏 280px 可拖动 220–420px；编辑器顶栏 40px（等宽路径、token 胶囊、「编辑 / 分栏 / 预览」分段控件、历史版本次按钮、保存主按钮、未保存圆点）；外部修改用 `--warning` 横幅；空状态居中 lucide 图标 + 一句说明 + 一个主操作；CodeMirror 主题取语义变量，亮暗随主题切换。树支持方向键导航。
  - 收到 `memory-changed` 时：重新 `listMemories`；若当前文件在 `paths` 中且无未保存修改，自动重新加载；有未保存修改则显示横幅「文件已在外部修改」。

- [ ] **Step 1: 写失败测试**：`tree shows global and projects`；`missing project shows flag`；`auto memories grouped with count`；`editor saves with expected hash`；`disk_changed shows conflict dialog and keeps user text`；`external change reloads when clean`；`codex db unavailable shows reason`；`add project button calls addManualProject with picked dir`。
- [ ] **Step 2:** `pnpm test src/pages/memory`，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS；`pnpm tauri dev` 手动核对：本机项目都出现、编辑全局 `CLAUDE.md` 保存成功、另开编辑器改同一文件后界面自动刷新。
- [ ] **Step 5: Commit** `feat: memory tree and editor`

---

### Task 12: 搜索面板与历史版本

**Files:**
- Create: `src/pages/memory/SearchPanel.tsx`、`HistoryDrawer.tsx`
- Modify: `MemoryPage.tsx`、`MemoryEditor.tsx`
- Test: `SearchPanel.test.tsx`、`HistoryDrawer.test.tsx`

**Interfaces:**
- Consumes: `searchMemories`、`listSnapshots`、`readSnapshot`、`writeMemoryFile`
- Produces:
  - `<SearchPanel onOpen(fileId: string, line: number) => void>`：输入框 300ms 去抖调用 `searchMemories`；结果按文件分组，显示行号与预览，命中词高亮；点击打开文件并滚动到该行。
  - `<HistoryDrawer path: string; currentText: string; onRestored(text: string) => void>`：列出快照（时间、大小）；选中显示与当前内容的 diff（用下面的 `DiffView`）；「恢复此版本」= `readSnapshot` 后以当前 hash 调 `writeMemoryFile`。
  - 可复用组件 `src/components/diff/DiffView.tsx`：`<DiffView oldText: string; newText: string; oldLabel?: string; newLabel?: string; context?: number /*默认 1*/>`，按设计规范第 7 节「diff 展示」实现：默认左右分栏、可切换上下、偏好存 `localStorage` 键 `memdock.diffLayout`、容器宽度 < 760px 自动上下、行级（`diffLines`）+ 词级（`diffWordsWithSpace`）高亮、空行斜纹占位。计划 3 的待审核与计划 4 的冲突处理复用它。

- [ ] **Step 1: 写失败测试**：`search debounces and groups by file`；`clicking hit calls onOpen with line`；`history lists snapshots newest first`；`restore writes snapshot content with current hash`；`DiffView defaults to split and remembers unified choice`；`DiffView falls back to unified when narrow`；`DiffView highlights changed words only`。
- [ ] **Step 2:** `pnpm test src/pages/memory`，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS；`pnpm test` 与 `cargo test --manifest-path src-tauri/Cargo.toml` 全部 PASS；CI 在 macOS 与 Windows 上均绿。
- [ ] **Step 5: Commit** `feat: memory search and snapshot history`

---

### Task 13: 主题色切换与首次启动窗口尺寸（用户 2026-11-05 追加）

**Files:**
- Modify: `src-tauri/src/settings.rs`（`Settings` 增加 `accent: Option<String>`）、`src-tauri/src/commands.rs`（`update_ui_prefs` 增加 `accent` 参数）、`src-tauri/src/lib.rs`（启动时设置窗口尺寸）、`src-tauri/tauri.conf.json`、`src/styles/globals.css`、`src/prefs/PrefsProvider.tsx`、`src/theme/ThemeProvider.tsx`、`src/pages/Placeholder.tsx`、`src/api/memory.ts`、`src/api/types.ts`、i18n 文件
- Test: Rust `commands.rs` 内测试；`src/theme/ThemeProvider.test.tsx`、`src/prefs/PrefsProvider.test.tsx`

**Interfaces:**
- Consumes: Task 9 的 `update_ui_prefs`、Task 10 的 PrefsProvider / ThemeProvider
- Produces:
  - `Settings.accent`：取值 `"green" | "blue" | "violet" | "orange" | "teal"`，空 = `"green"`；`update_ui_prefs(language, theme, accent)`，非法值返回 `invalid`；TS `updateUiPrefs(language?, theme?, accent?)`。
  - `globals.css` 按设计规范第 3 节「主题色」表，用 `html[data-accent="…"]` 与 `html.dark[data-accent="…"]` 覆盖 `--primary`、`--primary-foreground`、`--accent-text`、`--ring`；green 即现有默认值。
  - ThemeProvider 在 `<html>` 上设置 `data-accent`；PrefsProvider 增加 `accent` 字段，沿用按字段采纳与失败回滚。
  - Placeholder 页（计划 5 设置页接手前）增加「主题色」选择：5 个 24px 圆形色块（`aria-label` 为颜色名，选中带 2px `--ring` 外环）。
  - 窗口：`tauri.conf.json` 主窗口 `visible: false`、`center: true`；`setup` 中若无已保存的窗口状态，按当前显示器工作区宽 70%、高 75%（不小于 1024×640）设置尺寸并居中，然后 `show()`；用 `tauri-plugin-window-state` 记住之后的大小与位置。

- [ ] **Step 1: 写失败测试**：Rust `update_ui_prefs_accent_roundtrip`、`update_ui_prefs_rejects_unknown_accent`；TS `accent defaults to green data attribute`、`choosing accent sets data-accent and persists`、`accent failure rolls back`；纯函数 `initial_window_size(work_area_w, work_area_h) -> (w, h)` 的测试 `initial_window_is_70pct_width_75pct_height` 与 `initial_window_respects_minimum`。
- [ ] **Step 2:** 运行测试，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** `pnpm test`、`pnpm build`、`cargo test --manifest-path src-tauri/Cargo.toml` 全部 PASS；切换 5 种主题色在亮 / 暗下各截图一张核对对比度。
- [ ] **Step 5: Commit** `feat: accent color themes and initial window size`
