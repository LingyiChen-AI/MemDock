# Memdock 设计文档

> 日期：2026-11-05　状态：待用户审阅
> 标注说明：**【用户定】** 是用户在讨论中明确的；**【我定】** 是设计者做的默认决定，用户可推翻；未标注的是对两者的展开。

## 1. 这是什么、为什么做

Memdock 是一个**开源、跨平台（macOS + Windows）的桌面客户端**，把 Claude Code 和 Codex 散落在本机各处的「记忆」和「skill」集中管理起来。

用户的真实痛点（**【用户定】**）：

- 项目一多，全局记忆不好管；每个项目的记忆各自躺在项目目录的 `CLAUDE.md` 里，没有一个地方能统一查看、统一管理。
- 记忆会过时。隔一段时间就需要人去发现、去改。其实这些更新可以从历史聊天记录里提炼出来。
- 多台电脑之间记忆和 skill 不同步。

竞品调研结论（2026-11-05）：GitHub 上没有同时管理 Claude Code + Codex 记忆的 GUI 客户端。最接近的是 markes76/claude-code-gui（只管 Claude 记忆）、cc-switch（15 万星，管供应商/MCP/Prompts/Skills，但 Prompts 是整份替换 `CLAUDE.md`）、AgentSwitch（开关 skills/hooks/MCP）。用量统计参考 steipete/CodexBar。

**第一版主打卖点（【用户定】）**：记忆管理 + Skill 管理。其余功能是配套。

## 2. 目标、非目标、反目标

### 目标（第一版）

1. 记忆总览：统一查看、搜索、编辑 Claude Code 与 Codex 的全局 / 项目记忆。
2. 记忆更新提取：调用本机 `claude` / `codex` CLI 读历史会话，提炼记忆更新建议；手动触发 + 定时触发。
3. 待审核：所有提炼出的建议必须人工审核后才写入。
4. Skill 管理：新增、编辑、删除、从来源更新、两工具间同步。
5. GitHub 同步：记忆 + skill 安装清单同步到用户自己的 GitHub 私有仓库，用于多机同步，顺带备份和版本历史。
6. 用量统计：订阅额度窗口 + 每日 / 每月 token 与估算金额，按工具（Claude Code / Codex）和模型拆分。
7. 环境检测：检测 Claude Code / Codex 是否安装、登录；未安装时一键安装。

### 非目标（明确不在第一版做）

- **在客户端里直接使用 Claude Code / Codex**（内嵌终端、会话界面）——**【用户定】** 记下来以后做。
- 同步本地创建的 skill（没有远程来源的）——**【用户定】** 一期排除。
- 用聊天记录提炼 skill 改进建议——**【用户定】** skill 更新就是从来源直接拉新版本，不碰记忆流程。
- 按 API 来源 / 中转站拆分用量——**【用户定】** 「渠道」就是指 Claude Code 和 Codex 两个工具。
- Linux 支持、团队共享记忆、云端服务。

### 反目标（做出来也算失败的情况）

- **未经用户确认就改写记忆文件**。提取结果一律经待审核；同步只在用户点「同步记忆」时发生，有冲突必须由用户解决后才落盘。
- **数据离开本机去任何非用户指定的地方**。不做遥测、不建服务器；唯一的外发是用户自己的 GitHub 仓库，以及用户本机 CLI 自身的模型调用。**【我定】**
- **破坏 Claude Code / Codex 自身的文件**：写坏 `CLAUDE.md` 格式、把 symlink 型 skill 变成实体目录、删掉插件目录里的东西。
- 把 secret（API key、密码、token）推到 GitHub。

## 3. 约束

- **平台**：macOS + Windows 都必须能用。**【用户定】**
- **开源**发布到 GitHub。**【用户定】**
- **技术栈**：Tauri 2 + React + TypeScript（后端 Rust）。**【用户定，采纳我的推荐】** 理由：安装包小、跨平台、读写本地文件和起子进程方便，cc-switch 同栈可参考。
- **界面语言**：中英双语，默认跟随系统。**【我定】**
- **许可证**：MIT。**【我定】**
- 提取任务使用用户本机 CLI 的登录态与额度（会消耗用户订阅用量），界面上要明示。

## 4. 本机数据位置（2026-11-05 在 Windows 本机核实）

| 对象 | Claude Code | Codex |
|---|---|---|
| 全局指令记忆 | `~/.claude/CLAUDE.md` | `~/.codex/AGENTS.md`（本机未创建，按官方约定） |
| 项目指令记忆 | `<项目>/CLAUDE.md`、`<项目>/.claude/CLAUDE.md`、`CLAUDE.local.md`，支持 `@import` | `<项目>/AGENTS.md`（可逐级向上） |
| 自动记忆 | `~/.claude/projects/<路径编码>/memory/`：`MEMORY.md` 索引 + 每条一个 `.md`（带 frontmatter）。路径编码规则如 `C:\codes\cc-memory` → `C--codes-cc-memory` | `~/.codex/memories_1.sqlite`（表 `stage1_outputs.raw_memory` 等，sqlx 迁移管理）。**本机为空，产物格式需构建时再核实** |
| Skill | `~/.claude/skills/<name>/SKILL.md`、`<项目>/.claude/skills/`；插件自带的在 `~/.claude/plugins/cache/...`。**存在 symlink**（如 `ego-browser -> ~/.local/share/ego/ego-skills`） | `~/.codex/skills/`、`~/.agents/skills/`、`<项目>/.agents/skills/` |
| 已有安装清单 | — | `~/.agents/.skill-lock.json`（`npx skills` 工具产生，version 3，记录 `source/sourceUrl/skillPath/skillFolderHash`） |
| 会话日志 | `~/.claude/projects/<编码>/<session>.jsonl`，assistant 消息带 `model` 与 `usage{input_tokens, cache_creation_input_tokens, cache_read_input_tokens, output_tokens}` | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`，`token_count` 事件带 `total/last_token_usage` 与 `rate_limits{used_percent, window_minutes, resets_at}` |
| 额度窗口 | 本地日志不含；需走 OAuth 用量接口（参考 CodexBar 的 Claude provider） | 直接从最新 `token_count.rate_limits` 读 |

注意：本机 `codex` 不在 PATH（装的是 Codex 桌面版）。环境检测要能识别「装了桌面版但没有 CLI」的情况。

## 5. 功能设计

界面为左侧导航 + 右侧内容。导航：记忆总览 / 待审核 / 提取任务 / Skill 管理 / 用量统计 / 环境与设置。草图见 `.superpowers/brainstorm/312870-1793859794/content/`（overview.html、usage-v2.html、skills.html）。

### 5.1 记忆总览

- 左侧树：「全局」+ 每个项目，每个节点下列出该项目的 Claude 指令记忆、Claude 自动记忆（条数）、Codex `AGENTS.md`、Codex 记忆；有待审核建议的项目显示角标。
- **项目从哪来**：自动发现 = `~/.claude/projects/` 下的目录（反解路径）+ Codex 会话里出现过的 `cwd`；另支持用户手动添加目录。**【我定】**
- 右侧编辑器：Markdown 编辑 + 预览，显示估算 token 数；`@import` 可点击跳转。
- 全局搜索：跨全部记忆全文搜索。
- 历史版本：每次通过 Memdock 写入前留本地快照，可查看、回滚（开了 GitHub 同步时以仓库历史为准）。
- 外部改动：用文件监听感知 CLI 或编辑器对记忆的修改，界面实时刷新。

### 5.2 记忆更新提取

- **执行方式**：不自接模型 API，后台以无交互模式调用本机 `claude -p` 或 `codex exec`，把「目标记忆现状 + 时间范围内的会话摘录」交给它，要求按固定 JSON 结构输出建议。**【用户定：用本机 CLI】**
- **触发**：手动按钮「立即运行」；定时（每天某时 / 每周某天）。定时只在 Memdock 运行中（含托盘常驻）时生效。**【我定】**
- **可配置项**：用哪个 CLI 提取、定时频率、扫描项目范围、会话时间范围（默认最近 7 天，只处理上次成功之后的新会话）。
- **建议结构**：类型（新增 / 修改 / 过时）、目标文件、改动前后内容、依据（会话 id、时间、原话片段）。
- **运行记录**：每次运行的时间、触发方式、工具、读取会话数、产出建议数、状态与错误（如 CLI 未登录）。
- 安全：调用 CLI 时限制为只读、不允许它直接改文件——文件只由 Memdock 在审核通过后写。

### 5.3 待审核

- 卡片列表，可按项目、类型筛选。
- 每张卡片：类型标签、目标文件、diff、依据（可点开看原对话上下文）。
- 操作：通过 / 编辑后通过 / 驳回；支持批量通过。
- 写入前检查目标文件自建议生成后是否被改过，改过则提示重新确认。
- 被驳回的建议记下来，后续提取避免重复提出。**【我定】**

### 5.4 Skill 管理

- 统一列表：两工具、全局 / 项目范围的全部 skill，标注工具、范围、来源（GitHub / Git / 本地创建 / 插件）；可搜索筛选。
- **新增**：GitHub 链接 / 本地文件夹或 zip / 空白模板新建；安装时选择装给 Claude Code、Codex 或两者，选全局或某项目。
- **更新**：有远程来源的 skill 检查新版本（对比 commit / tag），点一下更新；更新前自动备份，可一键回滚。**【用户定：直接从来源更新】**
- **编辑 / 删除**：直接编辑 `SKILL.md` 及附属文件；删除前确认。
- **两工具同步**：两边都是 `SKILL.md` 格式，可一键复制到另一工具。
- **插件自带 skill 只读**，随插件更新。**【我定】**
- **symlink 型 skill**：识别并显示真实位置；更新 / 删除只动链接本身或明确提示，不擅自把链接变成实体目录。

### 5.5 GitHub 同步

用途：**多台电脑之间同步，顺带备份**。**【用户定】**

- **登录与绑定（2026-11-05 用户改定）**：
  - 登录可选：不登录时本地功能全部可用，只有同步需要登录。入口：侧栏底部头像位（未登录显示「登录 GitHub」）+ 设置页。
  - GitHub OAuth **Device Flow**：显示 8 位验证码并打开 `github.com/login/device`；无 client secret；Client ID 由项目方注册 OAuth App 后配置（构建期配置项）。申请 `repo` scope（自动建私有仓库所需），登录页写明用途。**【我定】**
  - 登录后自动处理同步仓库 **`MemDockHub`**：不存在 → 创建为私有仓库并写入标记文件 `memdock.json`；存在且有标记 → 直接接上（第二台电脑登录即接上）；存在但无标记 → 不动它，提示换名；存在但公开 → 明确警告，确认后才同步。**【用户定：默认创建名为 MemDockHub 的私有仓库】**
  - 令牌存系统钥匙串（Windows 凭据管理器 / macOS 钥匙串），不落配置文件；退出登录删除令牌，不动本地数据。**【我定】**
  - 一期只支持 github.com。**【我定】**
- **同步内容**：全局记忆、各项目记忆、**skill 安装清单**。
- **skill 只同步安装方式，不同步 skill 文件本身**；清单每条记录 名称、来源地址、版本（commit / tag）、装给哪个工具、全局或哪个项目。本地创建的 skill 不进清单（一期）。**【用户定】** 插件 skill 也不进清单。**【我定】**
- **清单格式**：自定义 `memdock/skills.json`；能读入 `~/.agents/.skill-lock.json` 作为来源之一，避免与 `npx skills` 打架。**【我定】**
- **另一台电脑上**：拉下清单与本机对比，列出「清单有、本机没装」的 skill，用户一键全装或挑选安装；不自动静默安装。**【我定】**
- **项目对应**：按项目 git 远程地址（规范化后）识别同一项目，所以不同电脑路径不同也能对上；没有远程的项目按目录名并提示用户确认。本机没有的项目，其记忆留在仓库里，等本机出现该项目再落盘。
- **同步范围包括已提交在项目 git 里的 `CLAUDE.md` / `AGENTS.md`**（**【用户定】**）：Memdock 仓库里的记忆是独立管理的一份。同事通过项目 git 改了这些文件、本机 pull 项目代码后，Memdock 要把本机的新内容当作一次本地改动，下次同步时推到自己的仓库里。
- **同步时机：用户点「同步记忆」触发**（**【用户定】**）：先从远端 pull，再把本机改动 push 上去。不在启动时或编辑后自动同步。**【我定】** 界面上显示「有 N 份记忆待同步」，提醒用户点。
- **冲突**：像解决代码冲突一样处理（**【用户定】**）。按文件列出冲突，三栏展示「本机 / 远端 / 合并结果」，支持逐块选用本机或远端、手动编辑合并结果，全部解决后才能完成同步。没有冲突的远端改动直接写入本机，写入前留本地快照。
- **全局记忆在各台机器上保持完全一致**，暂不区分本机专属部分。**【用户定】**
- **推送前扫描 secret**（API key、密码、token 等模式），命中则拦截并提示。
- 每次同步是一次 commit，可看历史、一键恢复。

仓库目录结构（**【我定】**）：

```
memdock-sync/
  global/
    claude/CLAUDE.md
    claude/memory/...
    codex/AGENTS.md
  projects/
    <规范化的 git 远程地址 或 目录名>/
      claude/CLAUDE.md
      claude/memory/...
      codex/AGENTS.md
  skills.json
  memdock.json        # 仓库格式版本等元信息
```

### 5.6 用量统计

参考 CodexBar。**【用户定：参考 CodexBar，除当天外支持看每天的总量】**

- **顶部额度窗口**：Claude Code 与 Codex 各一张卡片，显示 5 小时会话、本周的已用百分比和重置倒计时。
- **视图切换**：今天 / 每日 / 每月；时间范围、工具、模型筛选；导出 CSV。
- **汇总卡片**：区间金额、区间 tokens、日均、最高一天、缓存命中率。
- **每日金额柱状图**：按工具堆叠，点柱子跳到当天明细。
- **每日明细表**：一天一行（输入、输出、缓存读、缓存写、总 tokens、金额），可展开为「工具 × 模型」。
- **金额口径**：本地日志 token × 公开单价估算；未知价格的模型不按 0 算，标「含未知价格模型」；页面注明订阅用户实际不按此扣费。单价表随应用发布，可在线更新。**【我定】**

### 5.7 环境与设置

- 检测 Claude Code、Codex CLI、Node.js：是否安装、版本、是否登录；识别「只装了 Codex 桌面版没有 CLI」。
- 未安装时**一键安装**（**【用户定】**），按官方推荐方式（如 npm 全局安装或官方安装脚本），装完复检；登录需要用户交互时，引导用户在终端完成。
- 其他设置：语言、开机自启 / 托盘常驻、提取任务配置入口、GitHub 绑定、数据目录。

### 5.8 审阅后补充的决定（【我定】）

- **Codex 记忆库 `memories_1.sqlite` 只读**：只展示，不编辑、不作为提取的写入目标、不参与同步，避免和 Codex 的迁移与运行时锁冲突。
- **提取建议能写入的文件**：`CLAUDE.md` 系列、Claude 自动记忆 `memory/*.md`、`AGENTS.md`。允许跨工具：Claude 会话也能产出改 `AGENTS.md` 的建议，反之亦然，建议卡片上标明来源工具。
- **Claude 额度窗口**：需要读 Claude Code 的 OAuth 凭据去调非公开用量接口（CodexBar 也是这么做的）。做成设置里可关闭的开关，并在开关旁说明原理；接口失效时卡片降级为「暂不可用」，不影响 token 统计。Codex 额度直接读本地日志，无此问题。
- **删除不跨机器自动传播**：skill 清单是各机器安装过的合集；某台机器卸载 skill 或删除记忆后，其他机器同步时只提示「其他机器已删除，是否也删除」，不自动删。清单里为删除操作记删除标记。
- **已有的、没有来源记录的 skill**：首次导入时自动推断来源（skill 目录里有 `.git` 就读远程地址和 commit；symlink 指向 git 仓库同理；在 `.skill-lock.json` 里的直接用）。推断不出来的，提供「补登来源」，填 GitHub 地址后即可更新、可进清单；不补登的才按「本地创建」处理。

## 6. 留给构建者决定的

- Rust 侧与前端的模块划分、状态管理方案、UI 组件库。
- 提取任务给 CLI 的具体 prompt 与 JSON schema；会话摘录如何裁剪以控制 token。
- 文件监听、定时调度、GitHub API 调用（直接 REST 还是 git 命令）的具体实现。
- secret 扫描规则集。
- 价格表的数据来源与更新方式。
- 自动更新、代码签名、安装包分发（GitHub Releases、Homebrew cask、winget）。

## 7. 以后再做

- 在客户端里直接使用 Claude Code / Codex。
- 同步本地创建的 skill。
- 更多工具（Gemini CLI、OpenCode、Cursor 等）。
