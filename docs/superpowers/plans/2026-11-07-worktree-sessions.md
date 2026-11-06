# Worktree sessions ("独立工作区")

Approved by the user: entry via `/worktree` + status-line toggle (drafts only) AND the project "＋" menu (方案 1: one "＋" on project hover → 新建会话 / 在独立工作区新建会话 / 新建终端; replaces the two hover icons). Mockups: scratchpad `worktree-mockup.html` (sections 二/三) and `project-actions.html` (方案 1).
Starts after the session-isolation work is merged (same files).

## Behaviour
- Only for git repos (`git rev-parse --show-toplevel` succeeds). Default off.
- Create on the first send (or when terminal mode starts) of a draft with worktree on: branch `memdock/<slug>` (slug from the first message / title, ASCII, unique), path `<toplevel>/.memdock/worktrees/<slug>`, `git worktree add -b <branch> <path> HEAD`; base = the branch checked out in the main worktree at that moment (+ its commit). Add `.memdock/` to `.git/info/exclude` (never edit the user's .gitignore). The session then runs with cwd = worktree path.
- List: sessions whose cwd is a registered worktree are grouped under the parent project, with a purple branch badge (`🌿 <slug>`) and "改了 N 个文件" in the subtitle.
- Header bar (purple panel under the session header) for worktree sessions: `🌿 memdock/<slug> ← <base>` + path chip; changes summary (files, +/−, commits ahead); buttons 查看改动 (dialog: file list + diff; reuse the existing diff component from the review page), 打开文件夹, 合并到 <base> (primary), ⋯ → 放弃并删除工作区.
- Merge dialog: 压缩成一个提交合并 (default; message = session title, editable) / 保留全部提交合并 / 只保留分支，不合并; checkbox 合并后删除工作区和分支 (default on).
  - Dirty worktree → auto-commit first (`MemDock: <title>`), listed in the dialog.
  - Main worktree must be clean and on the base branch, else explain and stop.
  - Session running → merge disabled ("先停止当前回复").
  - Conflicts → abort the merge (main worktree left exactly as before), show the conflicting files, offer 「在终端处理」 (shell terminal in the worktree; user rebases there) and 「让 AI 解决冲突」 (sends the session a message asking it to rebase onto the latest base and resolve conflicts).
- Discard: confirm, `git worktree remove --force` + delete branch.
- Dependencies are not copied; note in the merge/info UI. (Linking node_modules is a later option.)

## Backend API (Rust, new module `src-tauri/src/worktree.rs`)
- `worktree_check(cwd) -> { repo_root: String, base_branch: String | null }` (error when not a git repo / detached HEAD message).
- `worktree_create(cwd, hint: String) -> WorktreeInfo { path, branch, slug, repo_root, base_branch, base_commit, created_at }`; persisted in `<data_dir>/worktrees.json`.
- `worktree_list() -> Vec<WorktreeInfo>` (drops entries whose path no longer exists).
- `worktree_status(path) -> { files: [{ path, status: "A"|"M"|"D"|"R", added, deleted }], ahead: u32, dirty: bool }` (committed changes vs base + uncommitted).
- `worktree_diff(path, file: Option<String>) -> String` (unified diff vs base incl. uncommitted).
- `worktree_merge(path, mode: "squash"|"merge"|"keep", message, delete_after: bool) -> { merged: bool, conflicts: Vec<String>, auto_committed: bool, commit: Option<String> }`.
- `worktree_remove(path, delete_branch: bool)`.
All git calls through the existing hidden-spawn helpers; no console windows; timeouts.

## Frontend
- `src/api/worktree.ts` for the above.
- Composer: `/worktree` (drafts only, git repos only; shows 开/关), status-line item `🌿 独立工作区` in violet when on; persisted per draft (not global).
- Project "＋" menu replacing the hover icons.
- First send / terminal start on a worktree draft: `worktree_create` → `startLive`/`ptyOpen` with cwd = path.
- List grouping + badge; header panel; changes dialog; merge dialog; discard confirm; conflict actions.
- Tests for each path with mocked APIs.
