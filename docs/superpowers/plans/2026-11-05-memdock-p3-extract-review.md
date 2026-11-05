# MemDock 计划 3：记忆更新提取 + 待审核 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 从 Claude Code / Codex 的本地会话记录里，调用用户本机的 `claude` 或 `codex` 命令行提炼记忆更新建议（新增 / 修改 / 过时），手动或定时触发；所有建议进入「待审核」页，逐条通过 / 编辑后通过 / 驳回，通过后才安全写入记忆文件。

**Architecture:** Rust 后端新增 `extract` 模块族：会话读取（两种日志格式 → 统一的对话轮次）、提示词构建与结果解析（纯函数）、CLI 调用器（trait，可替换为假实现）、建议与运行记录存储、任务编排与定时器、应用建议（复用计划 1 的 `memory::io` 冲突检测与快照）。前端新增「提取任务」页与「待审核」页，侧栏「待审核」显示数量，记忆树项目节点显示待审标记。

**Tech Stack:** 沿用计划 1/2；不新增网络依赖（模型调用全部经用户本机 CLI）。

**Spec:** `docs/superpowers/specs/2026-11-05-memdock-design.md` §5.2、§5.3、§5.8（提取写入目标范围、跨工具建议）；设计规范 `docs/design/memdock-design-system.md`（待审核卡片、diff 展示规则）。

## Global Constraints

- 平台 macOS + Windows；显示名 `MemDock`；中英双语；视觉遵循设计规范（语义变量、无裸 hex、lucide、无 emoji、最小 12px、`--font-content`）。
- **不自接模型 API**：提取只调用本机 CLI。Claude：`claude -p --output-format json --json-schema <schema> --tools "" --no-session-persistence --bare`，提示词经 stdin 传入；Codex：`codex exec --sandbox read-only --skip-git-repo-check --output-last-message <tmpfile> -`（提示词经 stdin，`-` 表示从 stdin 读）。CLI 只负责「读输入、给 JSON」，**不允许它改任何文件**；文件只由 MemDock 在用户通过建议后写入。
- **测试与开发期间绝不真实调用 `claude` / `codex`**（会消耗用户额度）：CLI 调用器通过 trait 注入，所有测试用假实现；真实调用器只做参数构造的单元测试。
- **所有建议必须人工审核**；没有任何自动写入路径。
- 提示词发出前**脱敏**：匹配 API key / token / 密码样式的串（`sk-…`、`ghp_…`、`github_pat_…`、`xox[bp]-…`、`AKIA…`、`-----BEGIN … PRIVATE KEY-----`、`password\s*[:=]\s*\S+` 等）替换为 `[REDACTED]`。
- 单次提取输入有上限：每个项目会话摘录最多 60 000 字符（超出时按时间从新到旧保留），每次运行最多 20 个项目；单次 CLI 调用超时 180 秒。
- 写入目标范围（spec §5.8）：`CLAUDE.md` 系列、Claude 自动记忆 `memory/*.md`、`AGENTS.md`；Codex 记忆库 `memories_1.sqlite` 只读，不作为目标。允许跨工具建议。
- 定时任务只在 MemDock 运行中（含托盘常驻）时生效；错过的时间点不补跑，等下一个时间点。
- 数据文件（`<appdata>/`）：`extract-config.json`、`suggestions.json`、`extract-runs.json`；写入一律「唯一临时文件 + sync_all + rename」，损坏时备份为 `.bak` 再报错，不静默清空（同计划 2 登记表的做法）。
- 提交信息不带 `Co-Authored-By`。

## Review Focus

1. **建议生成后用户又手动改了目标文件** → 通过时检测到 hash 变化：若 `old_text` 仍唯一存在则提示「文件已变，确认仍要应用？」；若已找不到则标记「已失效」，不写入。测试见 Task 6 `apply_detects_changed_target`。
2. **模型输出不是合法 JSON / 字段缺失 / 指向不存在或不允许的文件** → 该条丢弃并记入运行记录的 warnings，其余正常；整次运行不崩。测试见 Task 2 `parse_rejects_invalid_targets`。
3. **会话里含密钥** → 发给 CLI 的提示词里已脱敏。测试见 Task 2 `prompt_redacts_secrets`。
4. **同一条建议重复出现**（驳回后再次提取、或两次运行间未处理）→ 按指纹去重，驳回过的不再出现。测试见 Task 4 `rejected_fingerprint_suppressed`。
5. **提取进行中用户关闭窗口或再次点「立即运行」** → 同时只有一个运行；再次点击提示「正在运行」；进程退出时子进程被结束，运行记录标记为中断。测试见 Task 5 `second_run_rejected_while_running`、`orphan_running_marked_interrupted_on_start`。

---

## 文件结构

```
src-tauri/src/extract/
  mod.rs
  sessions.rs     读取 Claude / Codex 会话 → Turn；按项目、时间窗口过滤；上下文查询
  prompt.rs       构建提示词、脱敏、JSON Schema、解析与校验模型输出
  runner.rs       CliRunner trait + ClaudeCli / CodexCli 实现（参数构造、stdin、超时、杀进程树）
  store.rs        建议、运行记录、配置的持久化与去重指纹
  job.rs          一次提取运行的编排（项目选择、分批、调用、入库、水位线）+ 定时器
  apply.rs        通过 / 编辑后通过 / 驳回的落盘逻辑
src-tauri/src/commands_extract.rs
src/api/extract.ts（+ types.ts 追加）
src/pages/extract/ExtractPage.tsx, ExtractConfigForm.tsx, RunsTable.tsx
src/pages/review/ReviewPage.tsx, SuggestionCard.tsx, EvidenceDrawer.tsx
```

---

### Task 1: 会话读取

**Files:** Create `src-tauri/src/extract/mod.rs`、`sessions.rs`；Modify `lib.rs`（`pub mod extract;`）。Test：同文件，用临时目录伪造日志（Codex 的 `.jsonl.zst` 用 `zstd` crate 现场压缩）。

**Interfaces:**
- Consumes：`paths::AgentPaths`、`projects::{normalize_path, open_jsonl, claude_project_dirs}`。
- Produces（`Serialize, Clone, Debug, PartialEq`，snake_case）：
  - `struct Turn { tool: Tool, session_id: String, project_id: Option<String>, ts: i64 /* unix ms */, role: Role /* user | assistant */, text: String }`
  - `fn read_claude_turns(paths, since_ms: i64) -> Vec<Turn>`：遍历 `claude_home/projects/<enc>/*.jsonl`（跳过 mtime < since 的文件）；行 `type` 为 `user`/`assistant` 且 `message.content` 为字符串或含 `{type:"text", text}` 的数组时取文本；跳过 `tool_use` / `tool_result` / `thinking`、跳过 `isMeta: true`、跳过以 `<command-` / `<local-command-` / `<system-reminder>` 开头的文本；`project_id = normalize_path(cwd)`；`ts` 取行内 `timestamp`。
  - `fn read_codex_turns(paths, since_ms: i64) -> Vec<Turn>`：遍历 `codex_home/sessions/**/rollout-*.jsonl(.zst)` 与 `archived_sessions/`；首行 `session_meta.payload.{id, cwd}`；`response_item` 且 `payload.type == "message"`、`role ∈ {user, assistant}` 时取 `content[].text`（`input_text`/`output_text`）；跳过 role `developer`/`system`，跳过以 `<environment_context>` / `<user_instructions>` / `<permissions` 等尖括号标签开头的用户文本；`ts` 取行 `timestamp`。
  - `fn turns_for_project(turns: &[Turn], project_id: &str) -> Vec<&Turn>`
  - `fn context_around(paths, tool, session_id, ts, before: usize, after: usize) -> Vec<Turn>`（证据「查看原对话」用；最多各 6 轮）。
- 单行超过 1 MiB 跳过；单文件超过 200 MB 跳过并记录。

- [ ] Step 1 写失败测试：`claude_text_and_array_content`、`claude_skips_tool_and_meta_lines`、`codex_zst_messages_user_assistant_only`、`codex_skips_environment_context`、`since_filter_by_ts`、`context_around_returns_neighbors`、`oversized_line_skipped`。
- [ ] Step 2 运行，预期 FAIL；Step 3 实现；Step 4 预期 PASS。
- [ ] Step 5 Commit `feat(extract): read Claude and Codex session turns`

---

### Task 2: 提示词、脱敏与结果解析

**Files:** Create `src-tauri/src/extract/prompt.rs`。Test：同文件。

**Interfaces:**
- Produces：
  - `fn redact(text: &str) -> String`（Global Constraints 的模式；保留前 4 个字符便于辨认，如 `sk-a…[REDACTED]`）。
  - `struct MemoryTarget { file_id: String, path: String, tool: Tool, kind: MemoryKind, content: String }`
  - `fn build_prompt(project_name: &str, targets: &[MemoryTarget], turns: &[&Turn], lang: &str /* "zh" | "en" */) -> String`：中文 / 英文指令 + 每个目标文件全文（带 `file_id`）+ 按时间排序的对话摘录（`[tool session ts role] text`，已脱敏，总长按 Global Constraints 裁剪）。指令要求：只提炼「长期有效的偏好、约定、事实、纠正」，忽略一次性任务细节；输出严格 JSON。
  - `fn output_schema() -> serde_json::Value`：`{ suggestions: [ { kind: "add"|"modify"|"obsolete", file_id, old_text?: string, new_text?: string, reason: string, evidence: [{ session_id, ts, quote }] } ] }`（modify/obsolete 必须有 `old_text`；add/modify 必须有 `new_text`；obsolete 的 `new_text` 可省略表示删除该段）。
  - `struct Suggestion { id: String /* uuid v4 */, fingerprint: String, kind, file_id, path, tool_of_file: Tool, old_text: Option<String>, new_text: Option<String>, reason: String, evidence: Vec<Evidence { tool: Tool, session_id, ts, quote }>, source_tool: Tool /* 跑提取的 CLI */, created_at: i64, base_hash: String /* 生成时目标文件 hash */, status: Pending | Accepted | Rejected | Stale }`
  - `fn parse_output(raw: &str, targets: &[MemoryTarget], source_tool: Tool, turns: &[&Turn]) -> (Vec<Suggestion>, Vec<String> /* warnings */)`：容忍外层 ```json 代码块与 CLI 的 JSON 包装（Claude `--output-format json` 结果在 `result` 或 `structured_output` 字段）；逐条校验：`file_id` 必须在 targets 中；modify/obsolete 的 `old_text` 必须在目标内容中**恰好出现一次**；add 的 `new_text` 非空；evidence 的 `session_id` 必须出现在输入 turns 中（否则丢弃该证据，若全部无效则丢弃该建议）；`fingerprint = sha256(file_id + kind + normalize_ws(old_text) + normalize_ws(new_text))`。

- [ ] Step 1 写失败测试：`prompt_redacts_secrets`、`prompt_truncates_oldest_first`、`prompt_includes_targets_with_ids`、`parse_accepts_fenced_and_wrapped_json`、`parse_rejects_invalid_targets`（不存在的 file_id、old_text 出现 0 次 / 2 次、Codex 记忆库目标）、`parse_drops_unknown_evidence`、`fingerprint_ignores_whitespace`。
- [ ] Step 2–4 RED → 实现 → GREEN。
- [ ] Step 5 Commit `feat(extract): prompt building, redaction and output parsing`

---

### Task 3: CLI 调用器

**Files:** Create `src-tauri/src/extract/runner.rs`。Test：同文件（只测参数构造与假程序；**不调用真实 claude / codex**）。

**Interfaces:**
- Produces：
  - `trait CliRunner: Send + Sync { fn tool(&self) -> Tool; fn available(&self) -> bool; fn run(&self, prompt: &str, schema: &serde_json::Value, timeout: Duration, cancel: &AtomicBool) -> Result<String /* 原始输出 */, RunError>; }`
  - `enum RunError { NotInstalled, NotLoggedIn(String), Timeout, Cancelled, Failed { code: Option<i32>, stderr: String } }`（Serialize，stderr 脱敏并截断 2000 字符）
  - `struct ClaudeCli { program: PathBuf }`：`claude -p --output-format json --json-schema <schema-json> --tools "" --no-session-persistence --bare`，stdin 写入提示词；输出 JSON 取 `structured_output` 优先、否则 `result`；`is_error: true` 或 stderr 含登录提示时返回 `NotLoggedIn`。
  - `struct CodexCli { program: PathBuf }`：`codex exec --sandbox read-only --skip-git-repo-check --output-schema <tmp schema file> --output-last-message <tmp out file> -`，stdin 写入提示词，结果读 out 文件；工作目录设为一个空临时目录。
  - `fn args_for(tool) -> Vec<String>` 可单测；程序路径可注入（`with_program`）；Windows `CREATE_NO_WINDOW`，超时 / 取消时杀进程树（复用计划 2 `skills::git` 的 kill_tree 思路，抽到 `crate::proc` 公共模块）。
  - `fn detect_runners(paths) -> Vec<Arc<dyn CliRunner>>`：`which claude` / `which codex`；Windows 额外识别 `.cmd` 包装。

- [ ] Step 1 写失败测试：`claude_args_disable_tools_and_persistence`、`codex_args_read_only_sandbox`、`fake_program_output_returned`（用测试脚本模拟 CLI 回显 stdin）、`timeout_kills_process_tree`、`cancel_flag_stops_run`、`not_logged_in_detected`、`missing_program_not_installed`。
- [ ] Step 2–4 RED → 实现 → GREEN。
- [ ] Step 5 Commit `feat(extract): CLI runners for claude and codex`

---

### Task 4: 存储（配置、建议、运行记录）

**Files:** Create `src-tauri/src/extract/store.rs`。Test：同文件。

**Interfaces:**
- Produces：
  - `struct ExtractConfig { tool: Option<Tool> /* None = 自动：优先 claude */, schedule: Schedule /* Off | Daily{hh,mm} | Weekly{weekday 0-6, hh, mm} */, projects: ProjectFilter /* All | Selected(Vec<project_id>) */, include_global: bool /* 默认 true */, lookback_days: u32 /* 默认 7，1..=90 */, lang: Option<String> }`，默认 `Off`。
  - `struct RunRecord { id, started_at, finished_at: Option<i64>, trigger: Manual | Scheduled, tool: Tool, status: Running | Succeeded | Failed | Cancelled | Interrupted, sessions_read: u32, projects: u32, suggestions_new: u32, warnings: Vec<String>, error: Option<String> }`
  - `struct Store { … }`：`load(appdata) -> Result<Store, String>`、`save()`；`suggestions()`、`add_suggestions(Vec<Suggestion>) -> u32 /* 实际新增数 */`（指纹去重：已 Pending / Accepted / Rejected 的同指纹不再加入）、`set_status(id, status)`、`pending_count()`、`pending_by_project() -> HashMap<project_id, u32>`；运行记录最多保留 100 条；水位线 `watermark(project_id, tool) -> i64` / `set_watermark`（只处理水位线之后、且在 lookback 窗口内的会话）。
  - 文件写入规则与损坏处理见 Global Constraints。

- [ ] Step 1 写失败测试：`config_defaults_off`、`config_roundtrip`、`rejected_fingerprint_suppressed`、`pending_duplicate_not_added`、`runs_capped_at_100`、`watermark_roundtrip`、`corrupt_store_backed_up_and_errors`。
- [ ] Step 2–4 RED → 实现 → GREEN。
- [ ] Step 5 Commit `feat(extract): config, suggestions and run store`

---

### Task 5: 运行编排与定时器

**Files:** Create `src-tauri/src/extract/job.rs`。Test：同文件（假 CliRunner）。

**Interfaces:**
- Consumes：Task 1–4、计划 1 的 `memory::scan`（MemoryIndex）、`memory::io::read_memory`。
- Produces：
  - `fn run_extraction(ctx: &ExtractCtx, trigger, cancel: &AtomicBool, progress: &dyn Fn(Progress)) -> RunRecord`：
    1. 选 runner（config.tool 或自动），不可用 → 记录 Failed（NotInstalled）。
    2. 读两工具 turns（since = max(now - lookback, 各项目水位线的最小值)）。
    3. 按项目分组（无 project_id 的会话只用于「全局」）；跳过没有新 turns 的项目；最多 20 个项目，按最近活动排序。
    4. 每个项目：目标 = 该项目的可写记忆文件 + （include_global 时）全局可写记忆；`build_prompt` → `runner.run` → `parse_output` → `store.add_suggestions`；成功后推进该项目水位线；单项目失败记 warning 继续。
    5. 取消标志置位时尽快停止，状态 Cancelled；结束写 RunRecord。
  - `struct Progress { project: String, index: u32, total: u32 }`
  - `struct Scheduler`：后台线程每 30 秒检查一次；到达配置时间点（当天该分钟、未在该分钟运行过）触发 Scheduled 运行；运行中不重复触发。
  - 单例运行锁：同一时间只允许一个运行；`start` 时若已有运行返回 `AlreadyRunning`。
  - 启动时把遗留的 `Running` 记录标记为 `Interrupted`。

- [ ] Step 1 写失败测试：`runs_per_project_and_stores_suggestions`、`skips_projects_without_new_turns`、`failed_project_recorded_as_warning`、`watermark_advanced_only_on_success`、`cancel_marks_cancelled`、`second_run_rejected_while_running`、`orphan_running_marked_interrupted_on_start`、`scheduler_fires_once_per_slot`（注入时钟）、`runner_missing_fails_run`。
- [ ] Step 2–4 RED → 实现 → GREEN。
- [ ] Step 5 Commit `feat(extract): extraction job orchestration and scheduler`

---

### Task 6: 应用建议

**Files:** Create `src-tauri/src/extract/apply.rs`。Test：同文件。

**Interfaces:**
- Consumes：`memory::io::{read_memory, write_memory}`（含快照）、Task 4 Store。
- Produces：
  - `enum ApplyOutcome { Applied { hash: String }, NeedsConfirm { reason: String }, Stale }`
  - `fn apply(ctx, suggestion_id, edited_new_text: Option<String>, confirm_changed: bool) -> Result<ApplyOutcome, ApplyError>`：
    - 读目标当前内容；当前 hash == base_hash → 直接应用；
    - hash 变了：`old_text`（modify/obsolete）仍恰好出现一次 → `!confirm_changed` 时返回 `NeedsConfirm`，确认后应用；找不到或多次出现 → 标记 Stale 返回 `Stale`；add 类型 hash 变了 → `NeedsConfirm`。
    - modify：替换唯一出现的 `old_text` 为 `new_text`（或编辑后的文本）；obsolete：删除 `old_text`（连同其后一个空行）或替换为 `new_text`；add：追加到文件末尾（确保前面有空行；文件不存在则创建，父目录必须存在）。
    - 写入走 `write_memory`（期望 hash = 刚读到的 hash，冲突则返回 `NeedsConfirm{reason:"disk_changed"}`）；成功后状态 Accepted。
  - `fn reject(ctx, id)`；`fn accept_many(ctx, ids) -> Vec<(id, Result<ApplyOutcome, ApplyError>)>`（逐条，不因一条失败中断；`NeedsConfirm` 的条目跳过并返回）。

- [ ] Step 1 写失败测试：`apply_modify_replaces_unique_old_text`、`apply_obsolete_removes_block`、`apply_add_appends_with_blank_line`、`apply_detects_changed_target`（NeedsConfirm / Stale 两种）、`apply_with_edited_text`、`apply_creates_missing_file_only_if_parent_exists`、`reject_sets_status`、`accept_many_continues_after_failure`、`apply_preserves_bom_and_crlf`。
- [ ] Step 2–4 RED → 实现 → GREEN。
- [ ] Step 5 Commit `feat(extract): apply, edit-apply and reject suggestions`

---

### Task 7: Tauri 命令、事件与前端 API

**Files:** Create `src-tauri/src/commands_extract.rs`、`src/api/extract.ts`；Modify `lib.rs`（注册命令、启动 Scheduler、启动时标记 Interrupted）、`src/api/types.ts`。Test：`commands_extract.rs` 内 `*_impl` 测试；`src/api/extract.test.ts`。

**Interfaces（全部 `#[tauri::command(async)]`）：**
- `get_extract_config() / set_extract_config(config)`（校验 lookback 1..=90、时间合法、Selected 项目非空）
- `extract_runners() -> Vec<{ tool, available, program? }>`
- `start_extraction() -> RunRecord`（Running 记录立即返回，后台线程执行）；`cancel_extraction()`；`list_extract_runs()`
- `list_suggestions(status?: Status) -> Vec<Suggestion>`；`pending_summary() -> { total, by_project }`
- `apply_suggestion(id, editedNewText?, confirmChanged) -> ApplyOutcome`；`reject_suggestion(id)`；`accept_suggestions(ids) -> results`
- `suggestion_context(id) -> Vec<{ evidence, turns: Vec<Turn> }>`（每条证据前后各 4 轮）
- 事件：`extract-progress`（Progress）、`extract-finished`（RunRecord）、`suggestions-changed`（pending_summary）。应用建议写入记忆后照常触发计划 1 的 `memory-changed`。
- 错误统一走 CommandError，新增 code：`already_running`、`not_installed`、`not_logged_in`、`stale`。
- TS 类型与 Rust serde 名称逐字对应（snake_case 字段）。

- [ ] Step 1 写失败测试：Rust `start_twice_returns_already_running`、`apply_command_updates_summary`、`config_validation`、`context_for_suggestion`；TS `startExtraction`、`applySuggestion passes confirmChanged`。
- [ ] Step 2–4 RED → 实现 → GREEN；`cargo test`、`pnpm test`、`pnpm build`。
- [ ] Step 5 Commit `feat(extract): commands, events and typed API`

---

### Task 8: 「提取任务」页

**Files:** Create `src/pages/extract/ExtractPage.tsx`、`ExtractConfigForm.tsx`、`RunsTable.tsx`；Modify `App.tsx`（导航 `extract`）、i18n。Test：对应 `*.test.tsx`。

- 配置卡片：用哪个 CLI（Claude Code / Codex 分段，不可用的置灰并提示「未检测到」），定时（关闭 / 每天 HH:MM / 每周 X HH:MM），项目范围（全部 / 选择项目，多选列表来自 listMemories），包含全局记忆开关，回看天数（1–90），保存按钮（有改动才可用）。
- 卡片底部提示：「提取会调用你本机的 Claude Code / Codex，消耗你的订阅用量；会话内容在发送前会去除密钥。」
- 「立即运行」主按钮：运行中变为「运行中… 第 i / N 个项目」+「取消」；`already_running` 时提示。
- 运行记录表：时间、触发（手动 / 定时）、工具、读取会话数、项目数、新建议数、状态（成功 / 失败 / 已取消 / 中断，带图标与颜色）；失败行可展开看错误与 warnings；「新建议数」可点击跳转待审核页。
- 空状态与加载状态遵循设计规范。

- [ ] Step 1 写失败测试：`unavailable runner disabled`、`schedule weekly saves`、`run now shows progress and cancel`、`already running notice`、`failed run expands error`。
- [ ] Step 2–4 RED → 实现 → GREEN；开发预览截图（暗 / 亮）自查。
- [ ] Step 5 Commit `feat(extract): extraction page`

---

### Task 9: 「待审核」页、侧栏数量与记忆树标记

**Files:** Create `src/pages/review/ReviewPage.tsx`、`SuggestionCard.tsx`、`EvidenceDrawer.tsx`；Modify `App.tsx`、`src/layout/AppShell.tsx`（侧栏「待审核」数量徽标）、`src/pages/memory/MemoryTree.tsx`（项目节点待审小圆点，计划 1 设计稿已有）、i18n。Test：对应测试文件。

- 顶部：标题 + 「N 条建议 · 来自 <最近一次运行时间>」+ 类型筛选（全部 / 新增 / 修改 / 过时）+ 项目筛选 + 「全部通过」（二次确认，列出数量）。
- `SuggestionCard`：左侧 3px 类型色条；标题行（类型标签、工具标签、`项目 / 文件名`、时间）；diff 用计划 1 的 `DiffView`（modify：old_text → new_text；obsolete：old_text → 空 / new_text；add：空 → new_text），默认左右分栏、可切上下；理由一行；证据区（`--muted` 底，引号包裹原话，「查看原对话」打开 `EvidenceDrawer`）；操作：通过（主按钮）/ 编辑后通过（展开 CodeMirror 编辑 new_text，`--font-content`）/ 驳回。
- 通过返回 `NeedsConfirm` 时弹确认（显示原因），确认后以 `confirmChanged=true` 重试；`Stale` 时卡片变灰显示「目标内容已变化，建议已失效」并提供「驳回」。
- `EvidenceDrawer`：右侧抽屉，显示证据所在会话前后各 4 轮，命中句高亮，标明工具、会话 id、时间。
- 空状态：「没有待审核的建议」+ 去「提取任务」页的按钮。
- 侧栏「待审核」数量徽标（`suggestions-changed` 实时更新）；记忆树中有待审建议的项目显示 `--warning` 小圆点（tooltip「N 条待审核」）。

- [ ] Step 1 写失败测试：`cards render diff per kind`、`accept calls apply then removes card`、`needs confirm then retry with flag`、`stale card shows invalid state`、`edit then accept sends edited text`、`reject removes card`、`accept all confirms count`、`sidebar badge updates on event`、`memory tree shows pending dot`。
- [ ] Step 2–4 RED → 实现 → GREEN；`pnpm test`（两遍）、`pnpm build`、`cargo test`；开发预览截图（暗 / 亮：卡片列表、编辑态、证据抽屉、失效卡片）自查。
- [ ] Step 5 Commit `feat(review): review page, sidebar badge and tree markers`
