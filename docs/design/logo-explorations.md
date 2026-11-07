# Memdock 图标探索记录

## 第 1 版（2026-11-05）

草图：`.superpowers/brainstorm/1459-1793865330/content/logo-options.html`（本地，未入库）

| 方案 | 思路 |
|---|---|
| A 记忆卡停靠 | 记忆卡片落进坞形托槽，背后压一张卡 |
| B M 字母 + 坞线 | 绿底 M 立在底座上，最简洁，16px 最清楚 |
| C 书签锚点 | 书签 = 记住，圆点竖线像锚 = 停靠 |
| D 记忆网络 | 节点网落在底座上，技术感，小尺寸会糊 |
| E 叠层卡片 | 三层浅到深绿色卡片，浅底 |
| F 对话提炼 | 对话气泡 + 三个渐淡点 + 坞线 |

用户反馈：先记下这一版，再设计一版。

## 第 2 版（2026-11-05）

草图：`.superpowers/brainstorm/1459-1793865330/content/logo-options-v2.html`

第 2 版用户选 H（绳结），要求改成 Claude 配色 → H1–H4 变体（`logo-h-claude.html`），倾向 H2 整块陶土橙；
随后要求从「记忆管理的 M + admin 的 A」出发做字母组合 → MA1–MA6（`logo-ma.html`）。

## 当前选定（2026-11-05，暂定）

**MA6 · 一笔连写**：近黑底 `#141413`，陶土橙 `#D97757` 丝带一笔写出 M 并滑成 A 的右腿，奶油白 `#FAF9F5` 横杠与落点补成 A。
源文件：`assets/brand/memdock-logo.svg`。产品名写法同时改为驼峰 **MemDock**。

## macOS 图标要留边（2026-11-07）

第一版 `icon.icns` 直接拿满画布的 logo 去生成，结果 Dock 和 ⌘Tab 切换器里比相邻 app
大一圈。实测系统图标（备忘录 / 邮件 / 提醒事项）的圆角方块只占画布 **80.5%**——
1024 画布里是 824，四边各留 100px。

所以 macOS 单独一份源：`assets/brand/memdock-icon-macos.svg`，画一样的图但按这个栅格留边，
用 `scripts/gen-macos-icns.sh` 生成 `src-tauri/icons/icon.icns`（10 个 representation）。

**只有 `.icns` 留边**，其余照旧占满画布：

| 文件 | 谁在用 | 留边 |
|---|---|---|
| `icons/icon.icns` | macOS Dock、⌘Tab、访达 | 是，80.5% |
| `icons/32x32.png` | 托盘图标（`default_window_icon`，见 `src-tauri/src/tray.rs`） | 否，留边会让菜单栏里的图标缩水 |
| `icons/icon.ico` + `Square*Logo.png` | Windows | 否，Windows 自己有一套边距规则 |
| `icons/icon.png` | 跑 `pnpm tauri icon` 时的母版 | 否，留边会被二次叠加 |

`bundle.icon` 里谁被选中是 `tauri-codegen` 定的：Dock/⌘Tab 取第一个 `.icns`，
`default_window_icon` 取第一个 `.png`。改这个列表的顺序会连带换掉上面两者的来源。
