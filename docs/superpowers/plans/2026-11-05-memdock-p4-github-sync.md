# MemDock 计划 4：GitHub 同步（可选登录 + MemDockHub） Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用户可选地用 GitHub Device Flow 登录，MemDock 自动接上（或创建）私有仓库 `MemDockHub`；点「同步记忆」时先拉远端、三方比对、无冲突的远端改动落到本机（留快照）、有冲突的按文件三栏解决，然后扫描 secret 并推送本机改动；skill 只同步安装清单，另一台电脑上列出「清单有、本机没装」的 skill 供一键安装。

**Architecture:** Rust 新增 `sync` 模块族：仓库布局与本机文件映射（纯函数）、secret 扫描（与提取脱敏共用规则）、skill 清单合并（纯函数）、同步仓库的 git 层（系统 git，令牌经环境变量注入，不进命令行参数）、同步引擎（三方比对 → 计划 → 解决 → 落盘 → 提交 → 推送）、GitHub API 与 Device Flow（trait，可替换为假实现）、令牌存储（系统钥匙串，trait）。前端新增「同步」页（状态、项目对应、待安装 skill、历史）、冲突解决视图、登录对话框与侧栏账号位。

**Tech Stack:** 沿用计划 1–3；新增 Rust 依赖 `ureq`（HTTP，rustls）、`keyring`（Windows 凭据管理器 / macOS 钥匙串）、`base64`、`tauri-plugin-opener`（打开验证网址）。

**Spec:** `docs/superpowers/specs/2026-11-05-memdock-design.md` §5.5（GitHub 同步全文）、§5.8（删除不跨机器传播、Codex 记忆库不参与同步）、§2 反目标（不经确认不写记忆、secret 不上 GitHub、数据只去用户自己的仓库）；设计规范 `docs/design/memdock-design-system.md`（diff 展示规则复用于冲突视图）。

## Global Constraints

- 平台 macOS + Windows；显示名 `MemDock`；中英双语；视觉遵循设计规范（语义变量、tsx 无裸 hex、lucide、无 emoji、最小 12px、焦点环、记忆正文 `--font-content`）。
- 登录可选：未登录时所有本地功能照常可用，只有同步要求登录。
- OAuth：Device Flow，`client_id` 取构建期环境变量 `MEMDOCK_GITHUB_CLIENT_ID`，缺省 `Ov23liN2y64SfCPmQRZG`；scope 固定 `repo`；不使用 client secret。端点：`POST https://github.com/login/device/code`、`POST https://github.com/login/oauth/access_token`（`grant_type=urn:ietf:params:oauth:grant-type:device_code`）、REST `https://api.github.com`（`Accept: application/vnd.github+json`、`X-GitHub-Api-Version: 2022-11-28`、`User-Agent: MemDock`）。一期只支持 github.com。
- 令牌只存系统钥匙串（service `MemDock`，account `github`），不写任何配置文件、日志、错误信息、事件负载；退出登录删除令牌，不动本地数据与同步仓库克隆。
- git 访问令牌**只经环境变量**注入（`GIT_CONFIG_COUNT=1`、`GIT_CONFIG_KEY_0=http.https://github.com/.extraheader`、`GIT_CONFIG_VALUE_0=AUTHORIZATION: basic <base64("x-access-token:"+token)>`），并设 `GIT_TERMINAL_PROMPT=0`；绝不出现在命令行参数或仓库 `.git/config` 里。git 错误信息经 `skills::git::redact` 后才返回。
- 仓库默认名 `MemDockHub`，私有；标记文件 `memdock.json` = `{"format":1,"app":"MemDock"}`。
- 仓库布局（spec §5.5，根目录不再套 `memdock-sync/`）：`global/claude/CLAUDE.md`、`global/codex/AGENTS.md`、`projects/<key>/claude/CLAUDE.md`、`projects/<key>/claude/dot-claude/CLAUDE.md`（对应 `<项目>/.claude/CLAUDE.md`）、`projects/<key>/claude/CLAUDE.local.md`、`projects/<key>/claude/memory/<相对路径>.md`、`projects/<key>/codex/AGENTS.md`、`skills.json`、`memdock.json`。Codex 记忆库 `memories_1.sqlite` 不参与同步。
- 同步只在用户点「同步记忆」时发生；不在启动或编辑后自动同步。
- 文本比较与入库一律规范化：去 BOM、CRLF→LF；写回本机时沿用本机文件原有的 BOM / 换行风格（新文件按平台默认：Windows CRLF 无 BOM，macOS LF）。
- 本机记忆写入一律走 `memory::io::write_memory`（hash 检查 + 写前快照），并持有 `AppState.write_lock`；锁顺序 `sync_lock → write_lock → index`。
- 推送前对**将要推送的每个改动文件**做 secret 扫描，命中即整次同步拦截（不写本机、不提交、不推送），列出 文件 / 行号 / 规则名，不回显命中的原文。
- 删除不跨机器自动传播：远端删了而本机未改的文件，只提示「其他机器已删除，是否也删除本机文件」，默认保留。
- 测试与夜间开发期间**绝不访问真实 GitHub**：GitHub API、Device Flow、令牌存储全部经 trait 注入假实现；git 层用本地 bare 仓库（`file://`）测试；不在真实 `~/.claude`、`~/.codex` 写入。
- 数据文件（`<appdata>/sync/`）：`state.json`（账号 login、仓库 owner/name、默认分支、上次同步 commit 与时间、公开仓库确认、纳入的无远程项目）、`machine.json`（machine_id uuid + 主机名）、克隆目录 `repo/`；JSON 写入一律 `fsutil::save_json_atomic`，读取 `load_json_or_backup`。
- 提交信息不带 `Co-Authored-By`。

## Review Focus

1. **两台电脑改了同一份全局 `CLAUDE.md` 的不同段落** → 自动三方合并成功，不报冲突；改了同一行 → 冲突，必须解决后才落盘。测试见 Task 5 `two_machines_merge_disjoint_edits`、`same_line_edit_is_conflict`。
2. **同步过程中用户在编辑器里保存了正在同步的文件** → 落盘前 hash 校验失败，整次同步中止并提示重新同步，不覆盖用户刚写的内容、不推送。测试见 Task 5 `local_edit_during_sync_aborts`。
3. **推送被拒（另一台电脑刚推过）** → 自动重新拉取并重算一次；仍失败则报「远端有新改动，请重新同步」，本机已落盘的内容不回滚但状态不前进（下次同步可重放）。测试见 Task 5 `push_race_retries_once`。
4. **新电脑第一次同步、本机已有与远端不同的全局记忆** → 没有共同基线，按冲突处理，不静默覆盖任何一边。测试见 Task 5 `first_sync_differing_global_is_conflict`。
5. **令牌失效 / 被撤销（401）** → 清除钥匙串里的令牌，界面回到未登录并提示「登录已失效，请重新登录」，本地数据不动。测试见 Task 6 `unauthorized_clears_token`。

---

## 文件结构

```
src-tauri/src/secrets.rs          共享 secret 规则：redact（提取用）+ scan（同步用）
src-tauri/src/sync/
  mod.rs
  layout.rs     项目 key（git 远程规范化）、本机文件 ↔ 仓库路径映射、文本规范化、machine.json
  manifest.rs   skills.json 结构、由登记表 + .skill-lock.json 生成本机条目、并集合并、待安装 / 他机已删列表
  repo.rs       SyncGit：clone / fetch / 读某 commit 的文件 / 写工作区 / commit / push / merge-file / log
  engine.rs     三方比对 → SyncPlan；解决 → 落盘 → secret 扫描 → 提交 → 推送；待同步计数；历史
  github.rs     GitHubApi trait + HttpGitHub（ureq）；Device Flow 轮询；ensure_repo
  token.rs      TokenStore trait + KeyringStore + MemoryTokenStore（测试）
  state.rs      state.json 读写
src-tauri/src/commands_sync.rs
src/api/sync.ts（+ types.ts 追加）
src/pages/sync/SyncPage.tsx, ConflictResolver.tsx, conflictBlocks.ts, ProjectMapping.tsx, MissingSkills.tsx, SyncHistory.tsx
src/components/account/AccountSlot.tsx, LoginDialog.tsx
```

---

### Task 1: 共享 secret 规则 + 仓库布局与文件映射

**Files:**
- Create: `src-tauri/src/secrets.rs`, `src-tauri/src/sync/mod.rs`, `src-tauri/src/sync/layout.rs`
- Modify: `src-tauri/src/extract/prompt.rs`（`redact` 改为调用 `crate::secrets::redact`，行为与现有测试不变）、`src-tauri/src/lib.rs`

**Interfaces:**
- Produces:
  - `secrets::redact(text: &str) -> String`（原 `extract::prompt::redact` 搬家，规则不变）
  - `secrets::scan(text: &str) -> Vec<SecretHit>`，`SecretHit { line: usize /*1 起*/, rule: &'static str }`；规则名：`private_key`、`password`、`password_zh`、`openai_key`、`anthropic_key`、`github_token`、`slack_token`、`aws_access_key` 等（与 redact 同一套正则，每条规则一个名字）
  - `layout::project_key(project_dir: &Path) -> Option<String>`：读 `<dir>/.git/config` 的 `[remote "origin"] url`（`.git` 是文件时跟随 `gitdir:`），规范化为 `host/owner/repo`（小写 host，去 scheme、`user@`、`:port`、`.git`、尾 `/`；scp 风格 `git@host:owner/repo.git` 同样处理）；无 origin → `None`
  - `layout::dir_key(project_dir: &Path) -> String` = `"local/" + 目录名`（无远程项目的 key）
  - `layout::safe_key(key: &str) -> Option<String>`：只允许 `[A-Za-z0-9._-]` 段、用 `/` 分隔、无 `..`、无空段，否则 `None`（该项目不参与同步）
  - `layout::map_files(index: &MemoryIndex, included_local: &[String]) -> Vec<SyncFile>`，`SyncFile { repo_path: String, local_path: PathBuf, file_id: String, project_key: Option<String> }`：全局 `ClaudeInstructions`→`global/claude/CLAUDE.md`，全局 `CodexAgents`→`global/codex/AGENTS.md`；项目内按 Global Constraints 的布局映射，`ClaudeAuto` 取相对 `memory/` 目录的路径；跳过 `CodexDb`、symlink 文件、`exists=false` 但路径无法确定的项；无远程项目只有其 `dir_key` 在 `included_local` 里才纳入
  - `layout::expected_local_path(repo_path: &str, projects: &[(String /*key*/, PathBuf /*dir*/)], paths: &AgentPaths) -> Option<PathBuf>`：远端有、本机还没有的文件该落在哪（项目不在本机 → `None`）
  - `layout::normalize_text(bytes: &[u8]) -> String`（去 BOM、CRLF→LF、非 UTF-8 → lossy）
  - `layout::Machine { id: String, name: String }`；`layout::load_or_create_machine(path: &Path) -> io::Result<Machine>`

- [ ] **Step 1: 写失败测试**（`secrets.rs`、`layout.rs` 内 `#[cfg(test)]`）
  - `scan_reports_line_and_rule`：`"a\nkey sk-ant-api03-xxxxxxxxxxxxxxxxxxxx\n"` → `[SecretHit{line:2, rule:"anthropic_key"}]`；`"password: hunter2"` 命中 `password`；普通文本 → 空
  - `redact_unchanged_after_move`：现有 `extract::prompt` 的脱敏测试全部仍通过
  - `project_key_variants`：`https://github.com/Foo/Bar.git`、`git@github.com:Foo/Bar.git`、`ssh://git@github.com:22/Foo/Bar`、`https://user:tok@GitHub.com/Foo/Bar/` → 全部为 `github.com/Foo/Bar`；无 origin → `None`；`.git` 为 `gitdir:` 文件时能跟随
  - `safe_key_rejects_traversal`：`github.com/../x`、`a//b`、`a/b c` → `None`
  - `map_files_layout`：构造含全局两份 + 一个有远程项目（CLAUDE.md、.claude/CLAUDE.md、CLAUDE.local.md、memory/a.md、memory/sub/b.md、AGENTS.md、CodexDb）的 `MemoryIndex`，断言 repo_path 全部符合布局、CodexDb 与 symlink 被跳过
  - `no_remote_project_needs_inclusion`：无远程项目不在 `included_local` 时不出现，在时以 `projects/local/<目录名>/...` 出现
  - `normalize_text_strips_bom_and_crlf`
  - `machine_id_stable`：两次 `load_or_create_machine` 同一路径返回同一 id

- [ ] **Step 2: 运行测试确认失败**：`cargo test --manifest-path src-tauri/Cargo.toml sync::layout secrets -- --test-threads=4` → 编译失败 / FAIL
- [ ] **Step 3: 实现**（`secrets` 规则保留现有正则，`scan` 逐行匹配；layout 全部为纯函数 + 小量文件读取）
- [ ] **Step 4: 运行全部 Rust 测试**：`cargo test --manifest-path src-tauri/Cargo.toml -- --test-threads=4` → 全部 PASS
- [ ] **Step 5: 提交** `feat(sync): shared secret rules and repo layout mapping`

---

### Task 2: skill 安装清单（skills.json）

**Files:**
- Create: `src-tauri/src/sync/manifest.rs`

**Interfaces:**
- Consumes: `skills::registry::{Registry, RegistryEntry}`、`skills::source::read_skill_lock`、`skills::model::{SkillSource, SkillScope}`、`layout::{project_key, Machine}`
- Produces:
  - `Manifest { format: u32 /*=1*/, skills: Vec<ManifestSkill> }`
  - `ManifestSkill { name, url, git_ref: Option<String>, commit: Option<String>, skill_path, tool: Tool, scope: ManifestScope, installed_on: BTreeSet<String /*machine id*/>, removed_on: BTreeSet<String> }`；`ManifestScope = Global | Project { key: String }`（项目用 `project_key`，没有远程的项目内 skill 不进清单）；条目身份 = `(url, skill_path, tool, scope)`
  - `local_entries(registry: &Registry, skill_lock: &[(String, SkillSource)], projects: &[(String /*project_id*/, Option<String> /*key*/)]) -> Vec<ManifestSkill>`（本机安装条目，不含 machine 集合）；插件 skill、本地创建（无来源）的 skill 不进
  - `merge(base_remote: &Manifest, local: &[ManifestSkill], machine: &str) -> Manifest`：对远端每条：本机有 → `installed_on` 加入本机、`removed_on` 去掉本机、commit 取本机；本机没有但 `installed_on` 含本机 → 视为本机删除：从 `installed_on` 去掉、`removed_on` 加入本机。本机有而远端没有 → 新增条目。结果按身份排序，序列化稳定（两空格缩进 + 尾换行）。
  - `missing_here(m: &Manifest, local: &[ManifestSkill], machine: &str) -> Vec<ManifestSkill>`：清单里 `installed_on` 非空、本机没装、且 `removed_on` 不含本机
  - `removed_elsewhere(m: &Manifest, local: &[ManifestSkill], machine: &str) -> Vec<ManifestSkill>`：本机装着、`removed_on` 含其他机器、`installed_on` 只剩本机

- [ ] **Step 1: 写失败测试**
  - `local_entries_skip_plugin_and_local`
  - `merge_unions_two_machines`：A 装 x，B 装 y → 合并后 x、y 都在，`installed_on` 分别为 {A}、{B}
  - `uninstall_records_removal_not_delete`：清单 x 的 `installed_on={A,B}`，A 本机没有 x → `installed_on={B}`、`removed_on={A}`，条目仍在
  - `missing_here_lists_others`、`removed_elsewhere_prompts_only_when_last`
  - `merge_is_deterministic`：同一输入两次序列化字节相同
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 全部 Rust 测试通过**
- [ ] **Step 5: 提交** `feat(sync): skills manifest build and merge`

---

### Task 3: 同步仓库的 git 层

**Files:**
- Create: `src-tauri/src/sync/repo.rs`

**Interfaces:**
- Consumes: `crate::proc::{spawn_hidden, wait_with_timeout, kill_tree}`、`skills::git::redact`
- Produces:
  - `SyncGit { program: PathBuf, timeout: Duration /*默认 60s*/ }`；`Auth = Option<String /*token*/>`（`None` 用于本地 `file://` 测试）
  - `clone(&self, url: &str, dest: &Path, auth: &Auth) -> Result<(), SyncGitError>`（克隆到临时兄弟目录再 rename；空仓库也要能克隆）
  - `fetch(&self, repo: &Path, auth: &Auth) -> Result<Option<String /*origin/<branch> commit*/>, _>`（远端空分支 → `None`）
  - `default_branch(&self, repo: &Path) -> Result<String, _>`（读 `refs/remotes/origin/HEAD`，缺省 `main`）
  - `read_at(&self, repo: &Path, commit: &str, path: &str) -> Result<Option<Vec<u8>>, _>`、`list_at(&self, repo: &Path, commit: &str) -> Result<Vec<String>, _>`
  - `commit_tree(&self, repo: &Path, parent: Option<&str>, files: &BTreeMap<String, Vec<u8>> /*完整树*/, message: &str, author: (&str, &str)) -> Result<String /*new commit*/, _>`：用 `GIT_INDEX_FILE` 临时索引 + `hash-object -w --stdin` + `update-index --index-info` + `write-tree` + `commit-tree`，不碰工作区；作者固定 `MemDock <memdock@users.noreply.github.com>` 形式、`commit.gpgsign=false`
  - `push(&self, repo: &Path, commit: &str, branch: &str, auth: &Auth) -> Result<(), SyncGitError>`；非快进被拒 → `SyncGitError::Rejected`
  - `merge3(&self, base: &str, ours: &str, theirs: &str) -> Result<Merge3, _>`，`Merge3 { text: String, conflicts: usize }`（`git merge-file -p --diff3 -L local -L base -L remote`，临时文件放系统临时目录并清理）
  - `log(&self, repo: &Path, branch_ref: &str, limit: usize) -> Result<Vec<CommitInfo>, _>`，`CommitInfo { id, message, time: i64 /*ms*/ , files: Vec<String> }`
  - `SyncGitError::{NotInstalled, Timeout, Rejected, Unauthorized /*stderr 含 401/403/Authentication failed*/, Failed { stderr /*已 redact*/ }}`
  - 令牌注入严格按 Global Constraints；所有路径 / ref 参数前加 `--` 或校验不以 `-` 开头

- [ ] **Step 1: 写失败测试**（`tempfile` + 本地 bare 仓库，复用 `skills::git` 测试辅助的思路）
  - `clone_empty_and_push_first_commit`：空 bare 仓库克隆 → `commit_tree(None, {memdock.json})` → `push` → 另一个克隆 `fetch` 能读到
  - `push_non_fast_forward_is_rejected`
  - `read_and_list_at_commit`
  - `merge3_clean_and_conflict`：改不同行 → `conflicts==0` 且两处都在；改同一行 → `conflicts>0` 且文本含 `<<<<<<< local`、`||||||| base`、`>>>>>>> remote`
  - `token_never_in_args`：把 `program` 换成测试脚本（Windows `.cmd` / Unix sh，参照计划 3 假 CLI 写法）记录 argv 与环境，断言 argv 不含令牌、环境含 `GIT_CONFIG_VALUE_0` 与 `GIT_TERMINAL_PROMPT=0`
  - `error_text_is_redacted`
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 全部 Rust 测试通过**
- [ ] **Step 5: 提交** `feat(sync): git layer for the sync repository`

---

### Task 4: 同步状态与 GitHub 接入（令牌、Device Flow、仓库接上）

**Files:**
- Create: `src-tauri/src/sync/state.rs`, `src-tauri/src/sync/token.rs`, `src-tauri/src/sync/github.rs`
- Modify: `src-tauri/Cargo.toml`（`ureq`、`keyring` 开 `apple-native` + `windows-native`、`base64`）

**Interfaces:**
- Produces:
  - `SyncState { login: Option<String>, repo_owner: Option<String>, repo_name: String /*默认 MemDockHub*/, branch: Option<String>, last_sync_commit: Option<String>, last_sync_at: Option<i64>, public_ack: bool, included_local: Vec<String>, synced_projects: Vec<String> }`；`load_state(path) -> Result<SyncState,String>`、`save_state(path, &SyncState) -> io::Result<()>`
  - `trait TokenStore: Send + Sync { fn get(&self) -> Result<Option<String>, String>; fn set(&self, t: &str) -> Result<(), String>; fn clear(&self) -> Result<(), String>; }`；`KeyringStore`（service `MemDock`、account `github`）、`MemoryTokenStore`（测试）
  - `trait GitHubApi: Send + Sync`：
    - `device_code(&self, client_id: &str) -> Result<DeviceCode, ApiError>`，`DeviceCode { device_code, user_code, verification_uri, interval: u64, expires_in: u64 }`
    - `poll_token(&self, client_id: &str, device_code: &str) -> Result<PollResult, ApiError>`，`PollResult = Pending | SlowDown | Token(String) | Expired | Denied`
    - `user(&self, token: &str) -> Result<GhUser { login }, ApiError>`
    - `get_repo(&self, token, owner, name) -> Result<Option<GhRepo { private: bool, default_branch: String, clone_url: String }>, ApiError>`
    - `create_repo(&self, token, name) -> Result<GhRepo, ApiError>`（`POST /user/repos` `{name, private:true, auto_init:true, description:"MemDock memory sync"}`）
    - `get_file(&self, token, owner, name, path) -> Result<Option<Vec<u8>>, ApiError>`、`put_file(&self, token, owner, name, path, bytes, message) -> Result<(), ApiError>`
    - `ApiError::{Unauthorized, Network(String), Http { status: u16, message: String }}`
  - `HttpGitHub`（ureq，超时 20s，错误信息不含令牌）
  - `device_login(api: &dyn GitHubApi, client_id: &str, code: &DeviceCode, cancel: &AtomicBool, sleep: &dyn Fn(Duration)) -> Result<String /*token*/, LoginError>`：按 `interval` 轮询，`SlowDown` 加 5 秒，`Expired`/`Denied`/取消各自报错；总时长不超过 `expires_in`
  - `ensure_repo(api, token, owner: &str, name: &str, public_ack: bool) -> Result<RepoOutcome, ApiError>`，`RepoOutcome = Ready(GhRepo) | Created(GhRepo) | NotOurs | PublicNeedsAck(GhRepo)`：不存在 → 创建 + `put_file("memdock.json")` → `Created`；存在且有标记 → 私有 `Ready`，公开且未确认 `PublicNeedsAck`；存在无标记 → `NotOurs`（不写任何东西）
  - `CLIENT_ID: &str = option_env!("MEMDOCK_GITHUB_CLIENT_ID")` 缺省 `"Ov23liN2y64SfCPmQRZG"`

- [ ] **Step 1: 写失败测试**（`FakeGitHub` 记录调用、按脚本返回）
  - `device_login_handles_pending_slowdown_then_token`：脚本 Pending、SlowDown、Token → 返回令牌，记录到的等待序列为 `[i, i+5]`
  - `device_login_expired_and_cancel`
  - `ensure_repo_creates_private_with_marker`、`ensure_repo_attaches_existing`、`ensure_repo_not_ours_writes_nothing`、`ensure_repo_public_needs_ack`（确认后 `Ready`）
  - `state_roundtrip_and_corrupt_backup`
  - `memory_token_store_roundtrip`（不测真实钥匙串）
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 全部 Rust 测试通过**
- [ ] **Step 5: 提交** `feat(sync): github device flow, token store and repo bootstrap`

---

### Task 5: 同步引擎

**Files:**
- Create: `src-tauri/src/sync/engine.rs`

**Interfaces:**
- Consumes: Task 1–4 全部；`memory::io::{read_memory, write_memory}`、`SnapshotStore`、`AppState.write_lock`
- Produces:
  - `SyncCtx<'a> { git: &SyncGit, repo_dir: &Path, auth: Auth, branch: String, base_commit: Option<String>, files: Vec<SyncFile>, projects: Vec<(String /*key*/, PathBuf)>, paths: &AgentPaths, machine: &Machine, registry/skill_lock 输入, snapshots: &SnapshotStore, write_lock: &Mutex<()> }`
  - `prepare(ctx) -> Result<SyncPlan, SyncError>`：fetch → 远端 commit R、基线 B=`last_sync_commit`；对 `union(B 中路径 ∩ 本机存在的项目/全局, 本机文件, R 中路径 ∩ 本机存在的项目/全局)` 每个 repo 路径取 b/l/r（规范化文本，缺失为 None），分类：
    - `l==r` → 无事；`l==b` → `TakeRemote`（r 为 None 且 b 非 None → `RemoteDeleted` 待用户选，默认保留）；`r==b` → `PushLocal`（l 为 None → 推送删除）；
    - 两边都变：b 存在 → `merge3`，无冲突 → `Merged(text)`，有冲突 → `Conflict{ local, remote, base, merged_with_markers }`；b 不存在 → `Conflict`（首次同步差异）
    - **基线只对上次同步时本机已有的项目有效**：`SyncState.synced_projects`（上次成功同步时本机存在并参与的项目 key）之外的项目，b 一律视为 None——否则本机新出现的项目会被当成「本机删除了全部文件」而把远端记忆删掉。成功同步后更新 `synced_projects`
    - `skills.json`：总是 `manifest::merge`，不产生冲突
    - 记录每个本机文件的规范化前 hash（落盘前校验用）
    - 对所有将推送的内容（PushLocal、Merged 的结果）先做 `secrets::scan`，有命中 → `SyncPlan.secret_hits` 非空
  - `SyncPlan { id: String, remote_commit: Option<String>, take_remote: Vec<String>, push_local: Vec<String>, merged: Vec<String>, conflicts: Vec<ConflictFile { repo_path, local_path, local: Option<String>, remote: Option<String>, merged: String, conflicts: usize }>, remote_deleted: Vec<String>, secret_hits: Vec<FileSecretHit { repo_path, line, rule }>, missing_skills: Vec<ManifestSkill>, removed_elsewhere: Vec<ManifestSkill> }`
  - `finish(ctx, plan: &SyncPlan, resolutions: &BTreeMap<String, Resolution>) -> Result<SyncSummary, SyncError>`，`Resolution = Text(String) | DeleteLocal | KeepLocal`：
    1. 每个冲突必须有 `Text`（且不含冲突标记 `<<<<<<<`/`>>>>>>>` 行）、每个 `remote_deleted` 有 `DeleteLocal|KeepLocal`（缺省 KeepLocal），否则 `SyncError::Unresolved`
    2. 对解决后的文本再做 secret 扫描，命中 → `SyncError::Secrets(hits)`，什么都不写
    3. 持 `write_lock`：校验所有涉及的本机文件 hash 未变（变了 → `SyncError::LocalChanged{paths}`，什么都不写）；写 TakeRemote / Merged / 冲突解决结果到本机（`write_memory`，含快照；新文件建父目录；保留本机换行风格）；DeleteLocal 先快照再删
    4. 组装完整树 = R 的树（无 R 时空）应用本机变更，`commit_tree(parent=R, message="sync from <machine name>")`；无任何变化则不提交
    5. `push`；`Rejected` → 重新 `prepare` 一次并在无新冲突时自动 `finish`（只重试一次），否则 `SyncError::RemoteMoved`
    6. 成功后返回 `SyncSummary { commit: Option<String>, pulled: usize, pushed: usize, merged: usize }`，调用方更新 `last_sync_commit/last_sync_at`
  - `pending_local(ctx_without_fetch) -> Result<usize, SyncError>`：对比本机与 `base_commit` 的差异文件数（未同步过 = 本机文件总数）
  - `history(ctx, limit) -> Result<Vec<CommitInfo>, _>`、`file_at(ctx, commit, repo_path) -> Result<Option<String>, _>`

- [ ] **Step 1: 写失败测试**（两个临时 home + 两个克隆共享一个本地 bare 仓库，模拟两台电脑 A、B；`Auth=None`）
  - `first_sync_pushes_everything`：A 首次同步 → bare 里有 `global/claude/CLAUDE.md`、项目文件、`skills.json`、`memdock.json` 未被删
  - `second_machine_pulls_into_local`：B 首次同步、本机无该文件 → 写入本机，且有快照目录记录（新文件无快照，存在的被覆盖前有快照）
  - `two_machines_merge_disjoint_edits`、`same_line_edit_is_conflict`（`finish` 带 `Text` 解决后两边一致）
  - `first_sync_differing_global_is_conflict`
  - `remote_deletion_prompts_default_keep`：A 删除项目记忆并同步；B 同步 → `remote_deleted` 含该文件；`KeepLocal` → 文件仍在且被重新推回
  - `secret_blocks_everything`：本机文件含 `ghp_` 令牌 → `prepare` 的 `secret_hits` 非空；`finish` 返回 `Secrets`，bare 仓库无新提交、本机无写入
  - `local_edit_during_sync_aborts`：`prepare` 后改本机文件，`finish` → `LocalChanged`，不推送
  - `push_race_retries_once`：A `prepare` 后 B 推送一次不冲突的改动，A `finish` 自动重试成功，最终 bare 含两边改动
  - `unresolved_conflict_rejected`、`marker_text_rejected`
  - `project_absent_here_left_untouched`：远端有本机不存在的项目 → 不出现在计划里，推送后其文件仍在远端
  - `project_appearing_later_pulls_not_deletes`：B 同步时还没有项目 P；之后 B 出现 P（本机无记忆文件）再同步 → P 的记忆从远端写入本机，远端不被删除
  - `line_endings_preserved`：本机 CRLF 文件取远端改动后仍为 CRLF
  - `pending_local_counts_changes`
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: 全部 Rust 测试通过**
- [ ] **Step 5: 提交** `feat(sync): three-way sync engine`

---

### Task 6: Tauri 命令、事件与前端 API

**Files:**
- Create: `src-tauri/src/commands_sync.rs`, `src/api/sync.ts`, `src/api/sync.test.ts`
- Modify: `src-tauri/src/commands.rs`（AppState 加 `sync: SyncRuntime { lock: Mutex<()>, plans: Mutex<HashMap<String, SyncPlan>>, login_cancel: AtomicBool, api: Arc<dyn GitHubApi>, tokens: Arc<dyn TokenStore>, git: SyncGit }`）、`src-tauri/src/lib.rs`（注册命令、`tauri-plugin-opener`）、`src-tauri/capabilities/default.json`（`opener:allow-open-url`，只允许 `https://github.com/*`）、`src-tauri/Cargo.toml`、`package.json`（`@tauri-apps/plugin-opener`）、`src/api/types.ts`

**Interfaces:**
- 命令（全部 async，`*_impl` 可单测；`CommandError.code` 取值如下）：
  - `sync_status() -> SyncStatusDto { logged_in, login, repo: Option<{owner,name,url,private}>, last_sync_at, pending: usize, needs: Option<"public_ack"|"not_ours"|"relogin"> }`
  - `login_start() -> DeviceCodeDto { user_code, verification_uri, expires_in }`；后台线程轮询，成功后存令牌、取 login、`ensure_repo`、首次克隆，发事件 `sync://login`（`{ status: "ok"|"expired"|"denied"|"cancelled"|"error", needs?: ... , message? }`）
  - `login_cancel()`、`logout()`（清令牌与 `state.login`，保留克隆与 `last_sync_commit`）
  - `set_repo_name(name)`（`NotOurs` 后换名；名字限 `[A-Za-z0-9._-]{1,100}`）、`ack_public_repo()`
  - `open_verification_url()`（只打开 `https://github.com/login/device`）
  - `sync_start() -> SyncStartDto = { kind:"done", summary } | { kind:"needs_resolution", plan: SyncPlanDto } | { kind:"blocked", secret_hits }`：无冲突、无远端删除、无 secret → 直接 `finish` 返回 `done`
  - `sync_finish(plan_id, resolutions) -> SyncSummaryDto`、`sync_discard(plan_id)`
  - `set_included_local(keys: Vec<String>)`、`sync_projects() -> Vec<{ project_id, name, key: Option<String>, dir_key, included }>`
  - `sync_history(limit) -> Vec<CommitInfo>`、`sync_file_at(commit, repo_path) -> Option<String>`、`restore_from_history(commit, repo_path) -> ()`（把该版本写到本机对应文件，走 `write_memory` + 快照，成为一次本机改动）
  - 错误码：`not_logged_in`、`unauthorized`（同时清令牌、发 `sync://login {status:"relogin"}`）、`network`、`git_missing`、`sync_busy`、`unresolved`、`secrets`、`local_changed`、`remote_moved`、`repo_not_ours`、`public_not_acked`
- 事件：`sync://login`、`sync://status-changed`（同步完成、记忆变化后 `pending` 变化时发，防抖 1s，挂在已有 memory watcher 上）
- 前端 `src/api/sync.ts`：上述每个命令一个函数 + `onLoginEvent`、`onSyncStatusChanged`

- [ ] **Step 1: 写失败测试**：`commands_sync` 内 `*_impl` 测试用 `FakeGitHub` + `MemoryTokenStore` + 本地 bare 仓库（`clone_url` 指向 `file://`）：`login_flow_creates_repo_and_clones`、`unauthorized_clears_token`、`sync_start_done_without_conflicts`、`sync_start_needs_resolution_then_finish`、`second_sync_while_busy_rejected`、`restore_from_history_writes_snapshot`；`src/api/sync.test.ts` 用 mockIPC 断言命令名与参数
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: `cargo test … -- --test-threads=4`、`pnpm test`、`pnpm build` 全部通过**
- [ ] **Step 5: 提交** `feat(sync): tauri commands, events and frontend api`

---

### Task 7: 登录对话框与侧栏账号位

**Files:**
- Create: `src/components/account/AccountSlot.tsx`, `src/components/account/LoginDialog.tsx`, 对应 `.test.tsx`
- Modify: `src/App.tsx`（`status` 区放 AccountSlot；`sync` 页导航）、`src/layout/AppShell.tsx`（NavKey 加 `"sync"`，图标 `RefreshCw`，位于 `skills` 之后）、`src/i18n/{zh,en}.json`

**Interfaces:**
- AccountSlot：未登录 → 按钮「登录 GitHub」（图标 `Github` 不可用时用 `LogIn`）；已登录 → `login` 名 + 「N 份待同步」小字（N>0 时）；点击 → 跳「同步」页。折叠侧栏时只显示图标 + tooltip。
- LoginDialog（Radix Dialog）：说明文案「MemDock 会在你的 GitHub 账号下创建私有仓库 MemDockHub，用来在多台电脑间同步记忆。需要 repo 权限，令牌只保存在本机系统钥匙串。」→ 「开始登录」→ 大号等宽显示 `user_code`（带复制按钮）+「打开 GitHub」按钮（`open_verification_url`）+ 等待中提示 + 「取消」；收到 `sync://login`：`ok` 关闭并 toast；`expired` 显示「验证码已过期」+ 重试；`denied` 显示「已在 GitHub 拒绝授权」；`needs: not_ours` 显示「你的账号下已有同名仓库 MemDockHub 但不是 MemDock 创建的」+ 输入新仓库名 → `set_repo_name`；`needs: public_ack` 显示警告（醒目 destructive 样式）「仓库 X 是公开的，记忆会被所有人看到」+ 「我了解，继续使用」/「取消」。

- [ ] **Step 1: 写失败测试**（mockIPC + 模拟事件）：`shows_code_and_opens_github`、`ok_event_closes_and_toasts`、`expired_offers_retry`、`not_ours_asks_new_name`、`public_requires_ack`、`account_slot_shows_pending_count`、`collapsed_sidebar_icon_only`
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: `pnpm test`、`pnpm build` 通过；`src/dev/preview.tsx` 增加对应 mock，Edge 无头截图看未登录 / 验证码 / 已登录三态（亮、暗各一张），对照设计规范**
- [ ] **Step 5: 提交** `feat(sync): login dialog and sidebar account slot`

---

### Task 8: 「同步」页与冲突解决

**Files:**
- Create: `src/pages/sync/{SyncPage,ConflictResolver,ProjectMapping,MissingSkills,SyncHistory}.tsx`, `src/pages/sync/conflictBlocks.ts`, 对应测试
- Modify: `src/App.tsx`（`pages.sync`）、`src/i18n/{zh,en}.json`

**Interfaces:**
- `conflictBlocks.parse(text: string) -> Block[]`，`Block = { kind: "text", text } | { kind: "conflict", local: string, base: string, remote: string }`（解析 `<<<<<<< local` / `||||||| base` / `=======` / `>>>>>>> remote`）；`conflictBlocks.render(blocks, choices: Record<number, "local"|"remote"|"both"|{custom:string}>) -> string | null`（有未选块 → `null`）
- SyncPage：
  - 未登录：说明 + 「登录 GitHub」按钮（打开 LoginDialog）
  - 已登录：状态卡（仓库链接 `owner/name`、私有/公开标记、上次同步时间、「N 份记忆待同步」、主按钮「同步记忆」，同步中禁用并显示进度文案）；「退出登录」次要按钮（确认对话框）
  - `sync_start` 结果：`done` → toast「已同步：拉取 a、推送 b、自动合并 c」；`blocked` → 在页内列出 secret 命中（文件、行号、规则名），提示「请先移除这些内容再同步」；`needs_resolution` → 进入解决视图
  - 解决视图：左侧文件列表（冲突 / 远端已删除，未解决的带标记）；右侧 ConflictResolver；底部「完成同步」（全部解决才可点）+ 「放弃本次同步」（`sync_discard`）
  - ConflictResolver：每个冲突块用 `DiffView`（左右分栏、词级高亮）展示本机 vs 远端，按钮「用本机」「用远端」「两者都保留」；切换「手动编辑合并结果」后用 CodeMirror（`--font-content`）编辑整份文本（编辑后以文本为准，保存前校验不含冲突标记）；远端删除项：「删除本机文件」/「保留」两按钮
  - ProjectMapping：项目列表，显示对应 key（git 远程）或「无远程，按目录名对应」+ 开关（纳入后才同步）
  - MissingSkills：`missing_skills` 列表（名称、来源、装给哪个工具、范围），「全部安装」/逐条「安装」→ 调计划 2 的 `installSkill`（`from: {type:"git", url, clone_url:url, git_ref, skill_path, commit}`），项目范围映射到本机对应项目，找不到本机项目的条目置灰；`removed_elsewhere` 列表：「其他机器已删除」+「也删除本机」（调 `deleteSkill`，先确认）/「保留」
  - SyncHistory：最近 30 次同步（时间、来源机器、文件数），展开看文件，单文件「查看」（只读 Markdown）与「恢复到本机」（确认后 `restore_from_history`）

- [ ] **Step 1: 写失败测试**：`parse_and_render_blocks`（含无冲突文本、多块、`both`、custom、未选返回 null）；SyncPage：`not_logged_in_shows_login`、`sync_done_toast`、`blocked_lists_hits_without_secret_text`、`resolve_all_then_finish_sends_texts`、`finish_disabled_until_resolved`、`manual_edit_with_markers_rejected`、`remote_deleted_keep_or_delete`、`install_missing_skill_calls_install`、`history_restore_confirms`
- [ ] **Step 2: 运行确认失败**
- [ ] **Step 3: 实现**
- [ ] **Step 4: `pnpm test` 连续跑 3 次全部通过、`pnpm build` 通过；dev preview 增加 mock，Edge 无头截图看：已登录状态卡、解决视图（亮 / 暗），对照设计规范逐张检查文字截断、对比度**
- [ ] **Step 5: 提交** `feat(sync): sync page and conflict resolver`
