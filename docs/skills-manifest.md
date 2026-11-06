# Skill 安装清单（skills.json）

MemDock 不同步 skill 文件，只同步“每个 skill 是怎么装的”。清单有两种出现方式：

- 登录 GitHub 后，每次上传记忆时合并写入同步仓库根目录的 `skills.json`；
- 在「Skill 管理 → Skill 同步」里导出 / 导入的清单文件（格式相同，多一个 `description` 字段）。

## 格式（format 1）

```json
{
  "description": "MemDock skill install manifest: ...",
  "format": 1,
  "skills": [
    {
      "name": "huashu-motion",
      "url": "https://github.com/alchaincyf/huashu-motion",
      "git_ref": null,
      "commit": "3f2a9c1e0b7d4a6e8c5f1b2d3e4f5a6b7c8d9e0f",
      "skill_path": "skills/huashu-motion",
      "tool": "claude",
      "scope": { "type": "global" },
      "installed_on": ["5b0c1d2e-...-machine-a"],
      "removed_on": []
    },
    {
      "name": "my-notes",
      "url": "",
      "git_ref": null,
      "commit": null,
      "skill_path": "",
      "tool": "codex",
      "scope": { "type": "project", "key": "github.com/acme/app" },
      "installed_on": ["5b0c1d2e-...-machine-a"],
      "removed_on": [],
      "local_only": true
    }
  ]
}
```

| 字段 | 说明 |
|---|---|
| `description` | 可选，仅导出文件带，说明文件用途；读取时忽略 |
| `format` | 格式版本，目前为 1；读到更大的版本会报错，不会当成空清单 |
| `name` | skill 名称（SKILL.md frontmatter 的 name，缺省为目录名） |
| `url` | 来源仓库地址，统一成 `https://<host>/<owner>/<repo>`；无来源时为 `""` |
| `git_ref` | 跟踪的分支 / tag；`null` 表示远端默认分支 |
| `commit` | 本机装的提交号（已知时）；安装时按它固定版本 |
| `skill_path` | skill 在仓库里的子目录，`""` 表示仓库根目录 |
| `tool` | `claude` 或 `codex` |
| `scope` | `{"type":"global"}` 或 `{"type":"project","key":"<项目 key>"}`，项目 key 与记忆同步相同（git 远程地址或目录 key） |
| `installed_on` | 当前装有它的机器 id |
| `removed_on` | 主动卸载过它的机器 id（不会再提示这些机器安装） |
| `local_only` | 可选，`true` 表示找不到来源（手动放进去的 skill），只用于统计，不能自动安装，需要手动拷贝 |

## 收录范围

扫描到的所有可写 skill（Claude / Codex，全局和项目级），来源按以下顺序推断：

1. MemDock 安装记录（registry）
2. skill 目录自身是 git 仓库：取 `origin` 远程地址，`skill_path` 为相对仓库根的路径
3. skill 是软链：目标所在 git 仓库的远程地址
4. `~/.agents/.skill-lock.json`（`npx skills` 安装）

都推断不出来的写成 `local_only`。插件、Codex 内置、synced 等只读 skill 不收录；项目级 skill 所在项目没有同步 key 时不收录。

## 比对规则

身份键：`url + skill_path + tool + scope`；`local_only` 条目用 `name + tool + scope`。

- **云端有、本机缺少**：本机没有同身份的 skill，本机没有主动卸载过（`removed_on` 不含本机），且仍有别的机器装着（`installed_on` 含其他机器；导入文件里两者都为空时也算）。
- **本机有、云端没有**：下次上传时写入清单。
- **两边都有**：仅显示数量。

## 兼容性

`local_only`、`description` 是新增的可选字段，旧版本读取时忽略未知字段，格式号保持 1。旧版本会把 `local_only` 条目当成普通条目显示在“缺少的 skill”里，安装会失败（url 为空）。
