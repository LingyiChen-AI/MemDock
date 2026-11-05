# MemDock 计划 2：Skill 管理 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 MemDock 里统一管理 Claude Code 与 Codex 的本地 skill：列出、查看、新增（GitHub / 本地文件夹或 zip / 空白模板）、编辑、删除、从来源更新（更新前备份、可回滚）、两工具间同步；已有 skill 自动推断来源，推断不出可补登。

**Architecture:** Rust 后端新增 `skills` 模块族：扫描（发现 + frontmatter）、来源（推断 + 登记表）、git（调用本机 `git` 命令）、安装 / 更新 / 删除 / 同步（全部「临时目录准备 → 备份 → 原子替换」），通过 Tauri 命令暴露；前端新增 Skill 管理页（列表 + 详情 + 添加对话框 + 文件编辑）。登记表 `<appdata>/skills-registry.json` 是 MemDock 自己对「来源 + 安装位置」的记录，计划 4 的 GitHub 同步直接从它生成 `skills.json`。

**Tech Stack:** 沿用计划 1（Tauri 2、Rust、React + TS、Tailwind v4 + shadcn/ui、CodeMirror 6、Vitest）；新增 Rust crate：`yaml-rust2`（frontmatter）、`zip`（读取 zip）、`walkdir`（已有）、`tempfile`（运行时也用）。网络操作只通过本机 `git` 命令完成，不在应用里引入 HTTP 客户端。

**Spec:** `docs/superpowers/specs/2026-11-05-memdock-design.md` 第 5.4 节、5.8 节（已有 skill 的来源推断 / 补登、插件 skill 只读）；视觉见 `docs/design/memdock-design-system.md`；参考稿 `.superpowers/brainstorm/312870-1793859794/content/skills.html`（早期草图，视觉以设计规范为准）。

## Global Constraints

- 平台：macOS + Windows；CI 两平台都跑。
- 产品显示名 `MemDock`；代码 / 目录 / localStorage 键仍用小写 `memdock`。
- 视觉一律遵循 `docs/design/memdock-design-system.md`：语义 CSS 变量、无裸 hex、lucide 图标、无 emoji、最小 12px、焦点环 2px `--ring`；记忆 / skill 正文用 `--font-content`，代码与路径用等宽；中英双语。
- **网络**：只有用户主动发起的「从 GitHub 安装 / 检查更新 / 更新」会联网，且只通过本机 `git`（`git ls-remote`、`git clone --depth 1`、`git fetch`）访问用户给出的地址；不自动安装、不后台静默更新、不做遥测。没有 `git` 时这些入口置灰并提示「需要 git」。
- **只读来源**：插件自带（`~/.claude/plugins/...`）、Codex 系统内置（`~/.codex/skills/.system/`）、Claude 同步目录（`~/.claude/skills/synced/`）的 skill 只能查看，不能编辑 / 删除 / 更新 / 同步出去。
- **symlink**：删除只删链接本身，绝不删除链接目标；更新 / 编辑写到目标内容，不把链接替换成实体目录。
- **任何覆盖或删除前先备份**到 `<appdata>/skill-backups/<skill名>/<时间戳毫秒>/`（整目录复制），每个 skill 最多保留 10 份；替换使用「同级临时目录写好 → rename 原目录为 `.<name>.memdock-old-<ts>` → rename 新目录就位 → 删除旧目录」，任一步失败回滚到原状态。
- 解压 zip 必须防 zip-slip（拒绝绝对路径和含 `..` 的条目），单个 zip 解压后总大小上限 200 MB。
- 安装位置（本计划定的规则）：
  - Claude Code：全局 `~/.claude/skills/<name>/`，项目 `<project>/.claude/skills/<name>/`
  - Codex：全局 `~/.codex/skills/<name>/`，项目 `<project>/.agents/skills/<name>/`
- 提交信息不带 `Co-Authored-By`。

## Review Focus

1. **删除软链接型 skill**（如 `ego-browser -> ~/.local/share/ego/ego-skills`）→ 只删链接，目标目录完好。测试见 Task 6 `delete_symlink_keeps_target`。
2. **更新一个本地改过的 skill** → 先提示「本地有修改，更新会覆盖（已自动备份）」，确认后才更新；备份可一键回滚。测试见 Task 5 `update_detects_local_changes` 与 Task 9 `update with local changes asks confirmation`。
3. **更新过程中 git 失败 / 断网** → 原 skill 原样保留，无残留临时目录。测试见 Task 5 `update_failure_leaves_skill_intact`。
4. **恶意或损坏的 zip**（含 `../` 条目、超大）→ 拒绝并报错，不写出任何文件。测试见 Task 4 `zip_slip_rejected`、`zip_too_large_rejected`。
5. **一个仓库里有多个 skill / SKILL.md 不在根目录**（如 `skills/huashu-motion/SKILL.md`）→ 列出仓库内所有 skill 让用户选，按 `skill_path` 安装与更新。测试见 Task 3 `lists_multiple_skills_in_repo`、Task 4 `install_from_repo_subpath`。

---

## 文件结构

```
src-tauri/src/skills/
  mod.rs
  model.rs        SkillEntry、SkillSource、SkillKind、InstallTarget 等类型
  frontmatter.rs  解析 SKILL.md frontmatter（name、description）
  scan.rs         发现两工具全局 / 项目 / 插件 / 系统 / 同步目录下的 skill
  registry.rs     <appdata>/skills-registry.json 读写；读取 ~/.agents/.skill-lock.json
  source.rs       来源推断（.git、symlink 目标所在仓库、skill-lock、登记表）
  git.rs          本机 git 命令封装（可用性、ls-remote、浅克隆到缓存、列出仓库内 skill）
  fsops.rs        备份、原子替换、复制目录、zip 解压（防 zip-slip）
  ops.rs          install / update / delete / sync / rollback / 补登 的业务流程
src-tauri/src/commands_skills.rs   Skill 相关 #[tauri::command]
src/api/skills.ts, src/api/types.ts（追加类型）
src/pages/skills/SkillsPage.tsx, SkillList.tsx, SkillDetail.tsx, AddSkillDialog.tsx, SkillFiles.tsx, BackupsDrawer.tsx
```

---

### Task 1: Skill 类型、frontmatter 与发现扫描

**Files:**
- Create: `src-tauri/src/skills/mod.rs`、`model.rs`、`frontmatter.rs`、`scan.rs`
- Modify: `src-tauri/src/lib.rs`（`pub mod skills;`）、`Cargo.toml`（`yaml-rust2`）
- Test: 各文件内 `#[cfg(test)]`，用 `tempfile::TempDir` 伪造 `~/.claude`、`~/.codex`、`~/.agents`

**Interfaces:**
- Consumes: `paths::AgentPaths`（计划 1）、`model::{Tool, Project}`、`projects::normalize_path`；需要「用户 home」时用 `AppState.home`（计划 1 已有）。
- Produces（`skills/model.rs`，全部 `Serialize, Clone, Debug, PartialEq`，serde `snake_case`）：
  - `enum SkillKind { User, Plugin, CodexSystem, ClaudeSynced }`
  - `enum SkillScope { Global, Project { project_id: String } }`（`#[serde(tag="type")]`）
  - `struct SkillEntry { id: String /* normalize_path(dir) */, name: String /* frontmatter name，缺失时用目录名 */, description: String, tool: Tool, scope: SkillScope, kind: SkillKind, dir: String, is_symlink: bool, symlink_target: Option<String>, readonly: bool /* kind != User */, plugin: Option<String> /* "superpowers@claude-plugins-official" */, source: Option<SkillSource> /* Task 2 填 */, file_count: u32, size_bytes: u64, modified_at: Option<i64> }`
  - `fn parse_frontmatter(text: &str) -> Frontmatter { name: Option<String>, description: Option<String> }`：只解析开头 `---` 到下一个 `---` 之间的 YAML；支持 `>-`、`|` 多行与引号；无 frontmatter 返回两个 None。
  - `fn scan_skills(paths: &AgentPaths, home: &Path, projects: &[Project]) -> Vec<SkillEntry>`
- 扫描位置（每个子目录含 `SKILL.md` 才算一个 skill；顺序：全局 Claude → 全局 Codex → 各项目 → 插件 → 系统 → 同步）：
  - Claude 全局 `claude_home/skills/*/`（跳过 `synced`）；同步 `claude_home/skills/synced/**/`（kind ClaudeSynced，递归找 SKILL.md）；项目 `<p>/.claude/skills/*/`
  - 插件：读 `claude_home/plugins/installed_plugins.json`（`version: 2`，`plugins: { "<name>@<market>": [{ installPath, version, ... }] }`），对每个 `installPath/skills/*/` 记为 kind Plugin、`plugin = 键名`；文件缺失或格式不对则跳过插件部分
  - Codex 全局 `codex_home/skills/*/`（跳过 `.system`）与 `home/.agents/skills/*/`；系统 `codex_home/skills/.system/*/`（kind CodexSystem）；项目 `<p>/.agents/skills/*/` 与 `<p>/.codex/skills/*/`
  - 同一物理目录（规范化后路径相同，或 symlink 目标相同）出现在两个位置时各列一条，不去重（UI 会显示两处安装）。

- [ ] **Step 1: 写失败测试**：`frontmatter_folded_description`（`>-` 多行合并为一行）、`frontmatter_missing_returns_none`、`scans_claude_global_and_project`、`skips_dirs_without_skill_md`、`plugin_skills_readonly_from_installed_plugins`、`codex_system_and_agents_dirs`、`claude_synced_is_readonly`、`symlink_skill_flagged_with_target`（`#[cfg(unix)]`）。
- [ ] **Step 2:** `cargo test --manifest-path src-tauri/Cargo.toml skills::`，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS。
- [ ] **Step 5: Commit** `feat(skills): skill model, frontmatter parsing and discovery`

---

### Task 2: 来源推断与登记表

**Files:**
- Create: `src-tauri/src/skills/registry.rs`、`source.rs`
- Test: 同文件

**Interfaces:**
- Consumes: Task 1 `SkillEntry`；Task 3 的 `git::GitRunner`（本任务只用到 `remote_url(dir)`、`head_commit(dir)`、`repo_root(dir)`，用 trait 注入，测试用假实现）
- Produces:
  - `struct SkillSource { url: String /* 规范化：https://github.com/owner/repo（去 .git 与末尾 /） */, git_ref: Option<String> /* commit */, skill_path: String /* 仓库内相对路径，根目录为 "" */, origin: SourceOrigin }`，`enum SourceOrigin { Registry, GitDir, SymlinkRepo, SkillLock, Manual }`
  - 登记表 `struct Registry { version: u32 /* 1 */, skills: Vec<RegistryEntry> }`，`struct RegistryEntry { name: String, source: SkillSource, installs: Vec<InstallRecord { tool: Tool, scope: SkillScope, dir: String }>, content_hash: String /* 安装或更新后目录内容哈希 */, installed_at: i64, updated_at: i64 }`；`fn load_registry(path) -> Registry`（缺失 / 损坏 → 空表，损坏时先备份为 `.bak`，同计划 1 settings 做法）；`fn save_registry(path, &Registry) -> io::Result<()>`（临时文件 + rename）。
  - `fn read_skill_lock(home: &Path) -> Vec<(String /*name*/, SkillSource)>`：读 `home/.agents/.skill-lock.json`（`version: 3`，`skills: { name: { sourceUrl, skillPath, skillFolderHash } }`；`skillPath` 形如 `skills/x/SKILL.md` 时取其父目录）。
  - `fn dir_content_hash(dir: &Path) -> io::Result<String>`：按相对路径排序，对每个文件 `路径 + \0 + 内容` 做 sha256，忽略 `.git/` 与 `.DS_Store`。
  - `fn infer_source(entry: &SkillEntry, registry: &Registry, lock: &[(String, SkillSource)], git: &dyn GitRunner) -> Option<SkillSource>`，优先级：登记表中 installs 含该 dir → 该目录内有 `.git`（`skill_path` = dir 相对仓库根的路径）→ symlink 目标在某个 git 仓库内 → skill-lock 同名项 → None。
  - `fn register_manual(registry: &mut Registry, entry: &SkillEntry, url: &str, skill_path: &str) -> Result<(), String>`：补登，校验 url 为 `https://` 或 `git@` 形式，origin = Manual，记录当前 content_hash。

- [ ] **Step 1: 写失败测试**：`registry_roundtrip`、`corrupt_registry_backed_up`、`reads_skill_lock_v3`、`infer_prefers_registry`、`infer_from_git_dir_with_subpath`、`infer_from_symlink_repo`、`infer_from_skill_lock`、`infer_none_for_local`、`content_hash_ignores_git_dir_and_is_order_independent`、`register_manual_rejects_bad_url`。
- [ ] **Step 2:** 运行，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS。
- [ ] **Step 5: Commit** `feat(skills): source inference and registry`

---

### Task 3: git 封装

**Files:**
- Create: `src-tauri/src/skills/git.rs`
- Test: 同文件；需要真实 git 的测试用本地裸仓库（`git init --bare` + 本地 clone 推送）做远端，不访问网络；`#[ignore]` 不允许

**Interfaces:**
- Produces:
  - `trait GitRunner: Send + Sync { fn available(&self) -> bool; fn ls_remote_head(&self, url: &str, git_ref: Option<&str>) -> Result<String, GitError>; fn shallow_clone(&self, url: &str, git_ref: Option<&str>, dest: &Path) -> Result<String /*commit*/, GitError>; fn remote_url(&self, dir: &Path) -> Option<String>; fn head_commit(&self, dir: &Path) -> Option<String>; fn repo_root(&self, dir: &Path) -> Option<PathBuf>; }`
  - `struct SystemGit { timeout: Duration /* 60s */ }` 实现：调用 `git`（`GIT_TERMINAL_PROMPT=0` 防卡住在密码输入；Windows 用 `CREATE_NO_WINDOW` 不弹黑框）；超时杀进程返回 `GitError::Timeout`。
  - `enum GitError { NotInstalled, Timeout, Failed { stderr: String } }`（Serialize，供前端显示）
  - `fn parse_github_input(input: &str) -> Result<ParsedSource, String>`：接受 `owner/repo`、`https://github.com/owner/repo(.git)`、`https://github.com/owner/repo/tree/<ref>/<path>`、`git@github.com:owner/repo.git`，以及其他 `https://` git 地址；输出 `{ url, git_ref: Option, skill_path: Option }`。
  - `fn list_repo_skills(repo_dir: &Path) -> Vec<RepoSkill { skill_path: String, name: String, description: String }>`：在克隆下来的仓库里找所有 `SKILL.md`（跳过 `.git`、`node_modules`），按路径排序。
  - 缓存目录：`<appdata>/skill-sources/<sha256(url) 前 16 位>-<commit 前 8 位>/`，同一 commit 复用；每次 install / update 结束后清理 7 天前的缓存。

- [ ] **Step 1: 写失败测试**：`parse_owner_repo`、`parse_tree_url_with_ref_and_path`、`parse_ssh_url`、`parse_rejects_garbage`、`lists_multiple_skills_in_repo`、`shallow_clone_from_local_bare_repo`、`ls_remote_head_from_local_bare_repo`、`not_installed_when_git_missing`（把 PATH 指向空目录的 runner 构造参数）。
- [ ] **Step 2:** 运行，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS。
- [ ] **Step 5: Commit** `feat(skills): git runner, source parsing and repo skill listing`

---

### Task 4: 文件操作与安装

**Files:**
- Create: `src-tauri/src/skills/fsops.rs`、`ops.rs`（本任务只实现 install 相关）
- Modify: `Cargo.toml`（`zip`）
- Test: 同文件

**Interfaces:**
- Consumes: Task 1–3
- Produces（`fsops.rs`）：
  - `fn copy_dir(src, dst) -> io::Result<()>`（跳过 `.git`；保留文件内容，不复制 symlink 本身而是复制其指向内容）
  - `fn backup_dir(src: &Path, backups_root: &Path, name: &str) -> io::Result<BackupInfo { id: String, created_at: i64, size_bytes: u64 }>`（最多保留 10 份）、`fn list_backups(backups_root, name) -> Vec<BackupInfo>`（新→旧）
  - `fn atomic_replace_dir(prepared: &Path, target: &Path) -> io::Result<()>`：按 Global Constraints 的 rename 流程；target 是 symlink 时改为「清空链接目标目录内容后把 prepared 内容复制进去」，链接本身不动。
  - `fn extract_zip(zip_path, dest, max_total_bytes: u64 /*200 MiB*/) -> Result<(), String>`：防 zip-slip；zip 内若只有一个顶层目录则去掉这一层。
- Produces（`ops.rs`）：
  - `enum InstallFrom { Git { url: String, git_ref: Option<String>, skill_path: String }, Folder { path: String }, Zip { path: String }, Template { name: String, description: String } }`
  - `struct InstallTarget { tool: Tool, scope: SkillScope }`
  - `fn target_dir(t: &InstallTarget, paths: &AgentPaths, home: &Path, projects: &[Project], name: &str) -> Result<PathBuf, String>`（按 Global Constraints 的安装位置）
  - `fn install(ctx: &OpsCtx, from: InstallFrom, targets: &[InstallTarget], overwrite: bool) -> Result<Vec<SkillEntry>, OpsError>`：准备内容到临时目录 → 校验含 `SKILL.md` 且 name 合法（`^[a-z0-9][a-z0-9-_.]{0,63}$`，取 frontmatter name，模板用输入名）→ 对每个 target：已存在且 `!overwrite` 返回 `OpsError::Exists { dir }`；存在且 overwrite 先 backup 再 atomic_replace；不存在则 rename 就位 → Git 来源写登记表（installs、content_hash、commit）。Template 生成的 `SKILL.md`：frontmatter `name`、`description` + 一段中文说明占位。
  - `OpsCtx { paths, home, projects, appdata: PathBuf, git: Arc<dyn GitRunner>, registry_path }`；`enum OpsError { Exists { dir: String }, ReadOnly, GitUnavailable, Git(GitError), Invalid(String), Io(String) }`（Serialize）

- [ ] **Step 1: 写失败测试**：`zip_slip_rejected`、`zip_too_large_rejected`、`zip_single_top_dir_flattened`、`install_from_folder_to_claude_global`、`install_from_repo_subpath`（本地裸仓库，`skill_path = "skills/x"`）、`install_template_creates_skill_md`、`install_existing_without_overwrite_errors`、`install_overwrite_backs_up_first`、`install_both_tools_records_two_installs`、`invalid_name_rejected`。
- [ ] **Step 2:** 运行，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS。
- [ ] **Step 5: Commit** `feat(skills): safe file ops and install`

---

### Task 5: 检查更新、更新与回滚

**Files:**
- Modify: `src-tauri/src/skills/ops.rs`
- Test: 同文件

**Interfaces:**
- Produces:
  - `struct UpdateStatus { skill_id: String, current: Option<String>, latest: Option<String>, has_update: bool, local_modified: bool, error: Option<String> }`
  - `fn check_updates(ctx, entries: &[SkillEntry]) -> Vec<UpdateStatus>`：只对有 `source` 且非只读的 skill；同一 url+ref 只 `ls_remote` 一次；`local_modified` = 当前 `dir_content_hash` ≠ 登记表 content_hash（无登记表记录时为 false）。
  - `fn update(ctx, skill_id: &str, accept_local_changes: bool) -> Result<SkillEntry, OpsError>`：`local_modified && !accept_local_changes` → `OpsError::LocalChanges`；浅克隆最新 → 取 `skill_path` 子目录到临时目录 → backup → atomic_replace → 更新登记表 commit / content_hash / updated_at。目录自身是 git 仓库（`.git` 在 skill 目录里）时改为在该目录执行 `git pull --ff-only`，失败原样返回。任一步失败：原目录不变，临时目录清理。
  - `fn rollback(ctx, skill_id: &str, backup_id: &str) -> Result<SkillEntry, OpsError>`：先备份当前再恢复所选备份。

- [ ] **Step 1: 写失败测试**：`check_reports_update_when_remote_moved`、`check_groups_ls_remote_by_url`、`update_detects_local_changes`、`update_applies_and_records_commit`、`update_failure_leaves_skill_intact`（假 GitRunner 在 clone 时报错）、`update_git_dir_skill_uses_pull`、`rollback_restores_backup_and_backs_up_current`。
- [ ] **Step 2:** 运行，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS。
- [ ] **Step 5: Commit** `feat(skills): check updates, update with backup, rollback`

---

### Task 6: 删除、两工具同步、补登

**Files:**
- Modify: `src-tauri/src/skills/ops.rs`
- Test: 同文件

**Interfaces:**
- Produces:
  - `fn delete(ctx, skill_id: &str) -> Result<BackupInfo, OpsError>`：只读 → `ReadOnly`；先 backup；symlink 只删链接（Windows 目录联接 / 符号链接用 `remove_dir` 而非 `remove_dir_all`），否则 `remove_dir_all`；从登记表 installs 里移除该 dir（installs 为空则删条目）。
  - `fn sync_to(ctx, skill_id: &str, target: InstallTarget, overwrite: bool) -> Result<SkillEntry, OpsError>`：把该 skill 复制到另一工具的对应位置（等价于以 Folder 来源 install，但沿用原 source 写登记表）；只读 skill 也允许「复制一份到可写位置」，新副本 kind = User。
  - `fn register_source(ctx, skill_id, url, skill_path) -> Result<SkillEntry, OpsError>`：调用 Task 2 `register_manual`。

- [ ] **Step 1: 写失败测试**：`delete_backs_up_and_removes`、`delete_symlink_keeps_target`（`#[cfg(unix)]`；Windows 再加 `#[cfg(windows)]` 版本用 junction 或 `std::os::windows::fs::symlink_dir`，权限不足时跳过并在报告说明）、`delete_readonly_rejected`、`delete_updates_registry`、`sync_claude_to_codex_global`、`sync_existing_without_overwrite_errors`、`sync_readonly_plugin_skill_creates_user_copy`、`register_source_then_check_updates_works`。
- [ ] **Step 2:** 运行，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS。
- [ ] **Step 5: Commit** `feat(skills): delete, cross-tool sync, manual source registration`

---

### Task 7: Tauri 命令与前端 API

**Files:**
- Create: `src-tauri/src/commands_skills.rs`、`src/api/skills.ts`
- Modify: `src-tauri/src/lib.rs`（注册命令、`AppState` 增加 `skills_lock: Mutex<()>`、`git: Arc<dyn GitRunner>`）、`src/api/types.ts`
- Test: `commands_skills.rs` 内 `*_impl` 测试；`src/api/skills.test.ts`（mock invoke）

**Interfaces:**
- Produces（全部 `#[tauri::command(async)]`，写操作持 `skills_lock` 串行）：
  - `list_skills() -> Vec<SkillEntry>`（每次调用重新扫描 + 推断来源）
  - `git_available() -> bool`
  - `inspect_git_source(input: String) -> { url, git_ref, commit, skills: Vec<RepoSkill> }`（浅克隆到缓存后列出仓库内 skill）
  - `install_skill(from: InstallFrom, targets: Vec<InstallTarget>, overwrite: bool) -> Vec<SkillEntry>`
  - `check_skill_updates() -> Vec<UpdateStatus>`
  - `update_skill(skillId, acceptLocalChanges) -> SkillEntry`；`rollback_skill(skillId, backupId) -> SkillEntry`；`list_skill_backups(skillId) -> Vec<BackupInfo>`
  - `delete_skill(skillId) -> BackupInfo`；`sync_skill(skillId, target, overwrite) -> SkillEntry`；`register_skill_source(skillId, url, skillPath) -> SkillEntry`
  - 文件：`list_skill_files(skillId) -> Vec<{ rel_path, size_bytes }>`、`read_skill_file(skillId, relPath) -> FileContentDto`、`write_skill_file(skillId, relPath, text, expectedHash) -> FileContentDto`（复用计划 1 `memory::io` 的读写与冲突检测；`relPath` 规范化后不得含 `..` 或为绝对路径；只读 skill 拒绝写；单文件大于 1 MB 或非 UTF-8 只读显示「二进制 / 过大文件」）
  - 错误统一序列化为 `{ code, message, dir? }`，code ∈ `exists | read_only | git_unavailable | git_failed | local_changes | invalid | io`。
  - 事件：写操作完成后发 `skills-changed`；文件监听：沿用计划 1 watcher，追加监听各 skills 根目录（非递归 + 一层子目录），去抖后发 `skills-changed`。
- TS：`src/api/skills.ts` 每个命令一个函数（camelCase 参数），`onSkillsChanged(cb)`；类型与 Rust 一一对应。

- [ ] **Step 1: 写失败测试**：Rust `rel_path_traversal_rejected`、`write_readonly_skill_rejected`、`install_then_list_includes_new_skill`、`error_codes_serialize`；TS `installSkill passes camelCase`、`updateSkill rethrows local_changes`。
- [ ] **Step 2:** 运行，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** `cargo test`、`pnpm test`、`pnpm build` 预期全部 PASS。
- [ ] **Step 5: Commit** `feat(skills): tauri commands, watcher and typed API`

---

### Task 8: Skill 管理页——列表与详情

**Files:**
- Create: `src/pages/skills/SkillsPage.tsx`、`SkillList.tsx`、`SkillDetail.tsx`、`SkillFiles.tsx`
- Modify: `src/App.tsx`（导航 `skills` 渲染 SkillsPage）、i18n
- Test: `SkillList.test.tsx`、`SkillDetail.test.tsx`、`SkillFiles.test.tsx`

**Interfaces:**
- Consumes: Task 7 API；计划 1 的 CodeMirror 主题（`src/pages/memory/cmTheme.ts`）、`DiffView`、确认对话框
- Produces:
  - 布局：左列表 340px（可拖动 280–480，存 `memdock.skillListWidth`），右详情。
  - 列表顶栏：搜索（名称 + 描述，大小写不敏感）、工具筛选（全部 / Claude Code / Codex）、范围筛选（全部 / 全局 / 项目 / 只读来源）、「检查更新（N）」按钮（无 git 时置灰 + tooltip「需要安装 git」）、「添加 skill」主按钮。
  - 列表项：名称（等宽）、描述单行省略、标签（工具标签同记忆页；范围：全局 / 项目名；来源：GitHub / Git / 本地 / 插件名 / 系统 / 同步；有更新时 `--primary` 标签「有更新」；本地改动 `--warning` 点）。按「可写 → 只读」，再按名称排序；只读项整体 60% 不透明度。
  - 详情：标题行（名称 + 标签 + 操作按钮：更新 / 编辑文件 / 同步到… / 删除；只读时只显示「复制到可写位置」）；信息区（来源 URL 可复制、当前 commit 前 7 位、安装位置列表（同一 skill 多处安装都列出，等宽路径）、symlink 显示「→ 目标」、文件数 / 大小、更新时间）；无来源时显示「来源未知」+「补登来源」按钮（输入 GitHub 地址与仓库内路径）。
  - `SkillFiles`：左侧文件树、右侧 CodeMirror 编辑（`SKILL.md` 默认打开）、保存走 `write_skill_file`（disk_changed 处理同记忆编辑器）。
  - 空状态：没有任何 skill 时显示 lucide `Package` + 「还没有 skill」+「添加 skill」按钮。

- [ ] **Step 1: 写失败测试**：`list groups writable before readonly`、`search filters by name and description`、`tool and scope filters`、`update badge shown from check results`、`readonly detail hides edit and delete`、`unknown source shows register button`、`register source calls API with url and path`、`files editor saves with expected hash`。
- [ ] **Step 2:** `pnpm test src/pages/skills`，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS；开发预览截图（暗 / 亮）对照设计规范自查。
- [ ] **Step 5: Commit** `feat(skills): skills page list, detail and file editor`

---

### Task 9: 添加、更新、同步、删除、回滚的交互

**Files:**
- Create: `src/pages/skills/AddSkillDialog.tsx`、`BackupsDrawer.tsx`、`SyncDialog.tsx`
- Modify: `SkillsPage.tsx`、`SkillDetail.tsx`、i18n
- Test: `AddSkillDialog.test.tsx`、`SkillActions.test.tsx`、`BackupsDrawer.test.tsx`

**Interfaces:**
- Produces:
  - `AddSkillDialog`（shadcn Dialog，三个页签）：
    - 「从 GitHub」：输入框（支持 owner/repo、仓库 URL、tree 子路径 URL）→「解析」调用 `inspectGitSource`，显示进度与错误（git 未安装 / 失败的 stderr 摘要）；仓库内多个 skill 时列出复选（名称 + 描述 + 路径），单个时默认选中。
    - 「本地文件夹 / zip」：`@tauri-apps/plugin-dialog` 选目录或 `.zip`。
    - 「空白模板」：名称（实时校验格式）+ 描述。
    - 底部共用「安装到」：Claude Code / Codex 复选（至少一个）+ 范围（全局 / 选择项目，项目列表来自 `listMemories` 的项目）→「安装」。遇 `exists` 错误弹确认「已存在，覆盖（会先备份）？」→ 以 `overwrite=true` 重试。
  - 更新：详情「更新」→ `updateSkill(id, false)`；返回 `local_changes` 时弹确认「本地有修改，更新会覆盖（已自动备份），继续？」→ `updateSkill(id, true)`；成功 toast「已更新到 abc1234」。
  - 「检查更新」：调用 `checkSkillUpdates`，结果写入列表标签；单个失败显示在该项 tooltip。
  - 同步：`SyncDialog` 选目标工具 + 范围；已存在时同上确认覆盖。
  - 删除：确认对话框写明「会先备份，可在『备份』里恢复」；symlink 额外说明「只删除链接，不会删除 → 目标」。
  - `BackupsDrawer`：列出备份（时间、大小），选中显示该备份 `SKILL.md` 与当前的 `DiffView`；「恢复此备份」→ `rollbackSkill`。
  - 所有长操作按钮显示 loading，期间禁止重复点击。

- [ ] **Step 1: 写失败测试**：`github tab inspects and lists multiple skills`、`git unavailable disables github tab`、`template name validation`、`install exists asks overwrite then retries`、`update with local changes asks confirmation`、`delete symlink shows link-only notice`、`sync dialog installs to other tool`、`backups drawer restores selected backup`。
- [ ] **Step 2:** `pnpm test src/pages/skills`，预期 FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 预期 PASS；`pnpm test`（两遍）、`pnpm build`、`cargo test --manifest-path src-tauri/Cargo.toml`、`pnpm tauri build --debug` 全部通过；用真实数据启动核对（只读浏览，不在真实目录里安装 / 删除）。
- [ ] **Step 5: Commit** `feat(skills): add, update, sync, delete and rollback flows`
