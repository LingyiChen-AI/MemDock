# MemDock

MemDock 是一款源码公开的桌面应用（Windows / macOS，基于 Tauri 2 + React + TypeScript），把 **Claude Code** 和 **Codex** 的记忆、Skill、会话和用量放在一个地方管理。所有数据都在本机处理，只有你主动开启的功能（GitHub 同步、额度查询）才会联网。

> MemDock is a source-available desktop app (Windows / macOS, Tauri 2 + React + TypeScript) that manages Claude Code and Codex memories, skills, sessions and usage in one place. Everything runs locally; only the features you turn on (GitHub sync, quota lookup) go online.

## 功能

### 记忆
- **记忆总览**：全局与项目级的 `CLAUDE.md`、`CLAUDE.local.md`、自动记忆（`memory/*.md`）、`AGENTS.md`，以及只读的 Codex 记忆库，一棵树里浏览、搜索、编辑（Markdown 预览、分栏、版本历史与回滚）。不想看的项目可以隐藏。
- **记忆提取**：读取最近的 Claude Code / Codex 会话，调用本机的 CLI 提炼「反复出现或你明确要求记住」的内容，生成更新建议。支持手动和定时运行，提示词可自定义（安全条款与输出格式固定附加）。
- **待审核**：每条建议附原对话依据（聊天样式展示并高亮原话），可通过、编辑后通过、驳回，或批量处理；运行记录显示每次提取的处理进度，可只看某一次提取的建议。
- **从 GitHub 导入**：输入仓库链接，扫描其中的记忆文件和 Skill，记忆进入待审核，Skill 走安装流程。

### Skill
- 浏览、安装（GitHub / 本地目录 / zip）、更新、删除（带备份与回滚），在 Claude Code 与 Codex 之间同步。
- 安装清单 `skills.json` 随 GitHub 同步，新电脑可一键补装。

### 会话
- 按项目浏览两边的会话，原始对话按轮次展示：Markdown 渲染、工具调用胶囊、粘贴的截图。
- **会话互转**：把 Claude Code 会话转成 Codex 会话（或反过来），在另一个工具里 `resume` 接着做。

### 同步
- GitHub 私有仓库同步（Device Flow 登录，令牌存系统钥匙串）：**上传到云端**（先合并再推送）与 **同步到本地**（只拉不推，适合新电脑）。
- 三方合并，冲突逐块处理；疑似密钥的文件可跳过或坚持同步，跳过的文件会被记录并在记忆树中标出。

### 用量
- 从本机日志统计 Claude Code 与 Codex 的 token 用量和估算费用：指标卡、每日活跃热力图、按模型拆分、CSV 导出。
- 可选的实时额度查询（Claude / Codex，默认关闭，结果缓存 30 分钟）。

### 其他
- 环境检测与一键安装 Claude Code / Codex CLI；深浅主题与强调色；关闭窗口缩到托盘。

## 开发

需要 Node.js 24.15+（24 LTS）、pnpm、Rust（stable）和 Git。

```bash
pnpm install
pnpm tauri dev        # 启动开发版
pnpm test             # 前端测试（Vitest）
cargo test --manifest-path src-tauri/Cargo.toml   # 后端测试
pnpm tauri build      # 打包
```

### 发布与更新

- `main` 的 CI 运行测试并构建 macOS 和 Windows 安装包。安装包可从该次 Actions 运行的 Artifacts 下载。
- 正式发布时同步更新 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` 和 `src-tauri/tauri.conf.json` 的应用版本，再从已通过 CI 的 `main` 提交推送同版本的 `vX.Y.Z` 标签。Desktop Release 会打包 macOS（Apple Silicon / Intel）及 Windows（x64），确认更新清单包含三个平台的签名后发布 GitHub Release。
- 发布工作流使用 Actions Secrets 中的 `TAURI_SIGNING_PRIVATE_KEY` 和 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。请长期保存这对签名凭据。客户端使用配置中的公钥验签，下载 GitHub Release 不需要登录。GitHub 同步的设备登录使用公开的 OAuth Client ID，可在自行打包时通过 `MEMDOCK_GITHUB_CLIENT_ID` 覆盖。
- 安装版客户端启动后及运行期间每 6 小时检查一次更新。也可在「环境与设置 → 应用更新」手动检查并安装。已有版本首次接入时需要手动安装支持更新的版本。

目录结构：

```
src/                 前端（React + TS + Tailwind v4 + shadcn/Radix）
  pages/             各功能页面
  components/        通用组件（ui/、chat/ 等）
  i18n/              中英文文案
src-tauri/src/       后端（Rust）
  memory/ extract/   记忆索引、提取与审核
  skills/            Skill 管理
  convert/           会话解析与互转
  sync/              GitHub 同步引擎
  usage/             用量统计
```

## 隐私

- 记忆、会话、用量都只在本机读取和处理。
- 记忆提取调用的是你本机已登录的 Claude Code / Codex CLI，会话内容在发送前会去除疑似密钥。
- GitHub 同步只推送到你自己账号下的私有仓库；令牌只存在系统钥匙串里。
- 额度查询默认关闭，开启后才会读取本机 CLI 的登录凭据去查询。

## 许可协议 / License

本项目采用 [PolyForm Noncommercial License 1.0.0](LICENSE)，**源码公开，但不允许商业使用**：

- ✅ 个人学习、研究、自用，以及学校、公益组织、政府机构等非营利用途，可以免费使用、修改和分发（需保留 LICENSE 及其中的 `Required Notice`）。
- ❌ 任何商业用途（包括在公司内部用于营利业务、集成进收费产品或服务、二次打包销售等）都需要事先取得作者的书面授权。
- 商业授权请联系作者：https://github.com/LingyiChen-AI

> Licensed under the PolyForm Noncommercial License 1.0.0. Source-available; noncommercial use only. Commercial use requires a separate written license from the author — contact https://github.com/LingyiChen-AI.
