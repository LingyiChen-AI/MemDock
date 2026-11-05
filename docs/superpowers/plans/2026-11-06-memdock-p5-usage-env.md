# MemDock 计划 5：用量统计 + 环境检测与设置（快速模式）

**Goal:** 用量页（今天 / 每日 / 每月，按工具 Claude Code / Codex 和模型拆分，token 与估算金额，额度窗口，CSV 导出）；设置页（环境检测与一键安装、语言 / 主题、提取任务与 GitHub 入口、数据目录）。

**Spec:** `docs/superpowers/specs/2026-11-05-memdock-design.md` §5.6、§5.7、§5.8（Claude 额度开关）。视觉遵循 `docs/design/memdock-design-system.md`。

**模式（用户 2026-11-06 要求）：** 快速开发，不做逐任务审查；每个模块写核心单测（解析、计算），跑一次测试即可，开发完用户配合真机测试。

## 约束

- 中英双语；语义 CSS 变量、tsx 无裸 hex（有测试强制）；lucide；无 emoji；最小 12px。
- 不真实调用 `claude` / `codex` 的模型请求；不在开发期间真实执行安装命令（安装命令走可注入的执行器，测试用假实现）。
- 提交信息不带 `Co-Authored-By`。
- 只读本机日志；唯一的网络请求是可关闭的 Claude 额度接口（默认关闭，开关旁说明原理）。

## Task 1：用量采集（Rust `src-tauri/src/usage/`）

- `claude.rs`：遍历 `~/.claude/projects/**/*.jsonl`，取 `type=="assistant"` 且有 `message.usage` 的行：`model`、`input_tokens`、`output_tokens`、`cache_creation_input_tokens`、`cache_read_input_tokens`、`timestamp`；按 `message.id + requestId` 去重（同一条消息会写多行）。跳过 `model == "<synthetic>"`。
- `codex.rs`：遍历 `~/.codex/sessions/**/rollout-*.jsonl(.zst)` 与 `archived_sessions`（用 `projects::open_jsonl`）；`turn_context` 行给出当前 `model`；`event_msg` 且 `payload.type=="token_count"` 的 `info.last_token_usage`（`input_tokens`、`cached_input_tokens`、`output_tokens`、`reasoning_output_tokens`）作为一次增量，`info` 为 null 跳过；最新一条的 `rate_limits`（`primary` / `secondary`：`used_percent`、`window_minutes`、`resets_at`/`resets_in_seconds`）作为 Codex 额度窗口。
- `cache.rs`：按文件 `(path, size, mtime)` 缓存解析结果到 `<appdata>/usage-cache.json`，只重解析变化的文件；首次全量在后台线程。
- `pricing.rs`：内置单价表 `src-tauri/resources/pricing.json`（每百万 token：input / output / cache_write / cache_read；Claude Opus/Sonnet/Haiku 各代、OpenAI gpt-5 系列与 codex 模型），模型名前缀匹配；未知模型金额记 `None` 并在汇总标「含未知价格模型」。
- `aggregate.rs`：按本地时区日期聚合 `(date, tool, model)` → tokens 各项 + cost；提供今天（按小时）、每日、每月三种视图与区间汇总（总金额、总 tokens、日均、最高一天、缓存命中率 = cache_read / (input + cache_read)）。
- `claude_quota.rs`：开关打开时读 Claude OAuth 凭据（Windows/Linux `~/.claude/.credentials.json` 的 `claudeAiOauth.accessToken`；macOS 钥匙串 `Claude Code-credentials`），`GET https://api.anthropic.com/api/oauth/usage`（`Authorization: Bearer`、`anthropic-beta: oauth-2025-04-20`），取 `five_hour` / `seven_day` 的 `utilization` 与 `resets_at`；失败 → 「暂不可用」，不影响其他统计。令牌不进日志、错误、事件。
- 单测：两种日志格式样例解析、去重、增量、单价匹配与未知模型、聚合与时区边界。

## Task 2：用量命令与页面

- 命令：`usage_summary(view: today|daily|monthly, from, to, tools[], models[]) -> UsageReport`、`usage_quota() -> QuotaDto`（Codex 来自日志；Claude 依开关）、`usage_export_csv(view, from, to, tools, models, path)`（用 dialog 选保存路径）、`set_claude_quota_enabled(bool)`（存 settings）、事件 `usage://updated`（后台解析完成、文件变化后防抖 5s）。
- 页面 `src/pages/usage/`：顶部两张额度卡（Claude Code / Codex：5 小时、本周已用百分比 + 重置倒计时；Claude 开关关闭时显示开关与说明）；视图切换 今天 / 每日 / 每月 + 时间范围 + 工具 / 模型筛选 + 导出 CSV；汇总卡片（区间金额、tokens、日均、最高一天、缓存命中率）；Recharts 每日金额柱状图按工具堆叠（点柱跳当天）；明细表一天一行，可展开「工具 × 模型」。金额旁注明「按公开单价估算，订阅用户实际不按此扣费」；含未知价格模型时显示提示。
- 依赖：`recharts`。图表颜色用设计规范的语义变量（Claude Code 与 Codex 固定两色）。

## Task 3：环境检测、一键安装与设置页

- Rust `src-tauri/src/env/`：检测 `claude`、`codex`、`node`、`npm`、`git`：是否在 PATH（复用 `extract::runner` 的 PATH 解析，含 macOS 登录 shell PATH）、`--version`（2s 超时）；登录状态：Claude 有 `~/.claude/.credentials.json` 或（macOS）钥匙串项存在，Codex 有 `~/.codex/auth.json`；识别「装了 Codex 桌面版但没有 CLI」（Windows：`%LOCALAPPDATA%\Programs` 或开始菜单 / WindowsApps 中的 Codex；macOS：`/Applications/Codex.app`）。
- 一键安装：`npm install -g @anthropic-ai/claude-code` / `npm install -g @openai/codex`（无 npm 时提示先装 Node.js 并给官网链接）；执行器可注入，后台运行、逐行事件 `env://install-log`、完成后复检；登录需交互时提示「在终端运行 `claude` / `codex` 完成登录」并提供复制命令按钮。
- 设置页 `src/pages/settings/`：环境卡片（每个工具：状态、版本、登录、安装按钮）；外观（语言 / 主题 / 主题色，复用现有 AppearanceControls）；提取任务配置入口（跳转提取页）；GitHub 同步入口（跳转同步页，计划 4 合并后生效，这里先放占位按钮并在合并时接上）；Claude 额度开关；数据目录（显示路径 + 打开文件夹）。
- 单测：版本输出解析、桌面版识别（构造目录）、安装执行器假实现的事件流。
