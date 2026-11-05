# Memdock 设计系统

> 来源：ui-ux-pro-max `--design-system`（查询 "developer tool desktop productivity minimal dark mode"，density 8 / motion 3 / variance 3），结合桌面客户端、离线、中英双语的约束调整。2026-11-05。
> 所有前端任务以本文件为准；组件里不得出现裸 hex，一律用语义变量。

## 1. 风格

- **Minimalism & Swiss Style**：克制、网格对齐、高对比、靠字重和留白分层级，不靠阴影和渐变。
- **开发者工具气质**：代码与路径用等宽字体；绿色强调色（「运行 / 通过」的语义）。
- **高密度**：这是天天开着的工具，不是营销页。列表行高紧凑，间距用 4 的倍数小步长。
- **默认暗色**，同时完整支持亮色和「跟随系统」。
- 图标只用 **lucide-react**（SVG），**禁止用 emoji 当图标**。

## 2. 技术落地

- Tailwind CSS v4 + **shadcn/ui**（Radix 底座）。颜色以 OKLCH/hex 语义变量定义在 `src/styles/globals.css` 的 `:root`（亮）与 `.dark`（暗），通过 `@theme inline` 暴露给 Tailwind。
- 主导航使用 shadcn `Sidebar` 组件，不手写侧栏。
- 图表：**Recharts**。
- **字体随包打包，不走 Google Fonts CDN**（应用承诺零外发）：`@fontsource-variable/inter`、`@fontsource/ibm-plex-sans`（400/500/600）、`@fontsource-variable/jetbrains-mono`。

## 3. 颜色（语义变量）

| 变量 | 暗色（默认） | 亮色 | 用途 |
|---|---|---|---|
| `--background` | `#0F172A` | `#F8FAFC` | 窗口背景 |
| `--foreground` | `#F8FAFC` | `#0F172A` | 正文 |
| `--card` | `#1B2336` | `#FFFFFF` | 卡片、编辑器面板 |
| `--card-foreground` | `#F8FAFC` | `#0F172A` | |
| `--sidebar` | `#0B1222` | `#F1F5F9` | 左侧导航底色 |
| `--muted` | `#272F42` | `#F1F5F9` | 次级底色、hover 行 |
| `--muted-foreground` | `#94A3B8` | `#475569` | 次要文字（亮色用 #475569 保证 4.5:1） |
| `--border` | `#334155` | `#E2E8F0` | 分隔线、卡片边框 |
| `--primary` | `#22C55E` | `#16A34A` | 主按钮底色、选中态、焦点环 |
| `--primary-foreground` | `#0F172A` | `#FFFFFF` | 主按钮文字 |
| `--accent-text` | `#4ADE80` | `#15803D` | 绿色文字/链接（满足 4.5:1） |
| `--destructive` | `#EF4444` | `#DC2626` | 删除、驳回、错误 |
| `--warning` | `#F59E0B` | `#B45309` | 外部修改、未知价格、待同步 |
| `--ring` | `#4ADE80` | `#16A34A` | 键盘焦点环 |

**工具标识色**（标签、图表系列；必须同时配文字标签，不能只靠颜色）：

| 变量 | 暗色 | 亮色 | 标签文字 |
|---|---|---|---|
| `--tool-claude` | `#F59E0B` | `#D97706` | `CC` / Claude Code |
| `--tool-codex` | `#38BDF8` | `#0284C7` | `Codex` |

**变更类型色**（待审核卡片、diff）：新增 `--primary`；修改 `--warning`；过时 `--destructive`。diff 行底色用对应颜色 15% 透明度。

## 4. 字体

| 用途 | 字体栈 | 字号 / 行高 / 字重 |
|---|---|---|
| 界面正文 | `"Inter Variable", "IBM Plex Sans", "PingFang SC", "Microsoft YaHei UI", system-ui, sans-serif` | 13px / 20px / 400 |
| 次要说明 | 同上 | 12px / 16px / 400（最小字号，不再更小） |
| 页面标题 | 同上 | 18px / 26px / 600 |
| 区块标题 | 同上 | 14px / 20px / 600 |
| 数字指标 | 同上，`font-variant-numeric: tabular-nums` | 22px / 28px / 600 |
| 代码、路径、编辑器 | `"JetBrains Mono Variable", "Cascadia Code", Menlo, Consolas, monospace` | 12.5px / 20px |

> 桌面工具 13px 正文是 macOS/VS Code 的惯例；可在设置里提供「界面缩放 90% / 100% / 110%」。

## 5. 间距、圆角、阴影

- 间距刻度（density 8）：`4 / 8 / 12 / 16 / 24 / 32` px。列表行高 32px，树节点 28px，表格行 36px。
- 圆角：控件 6px，卡片 8px，标签 4px。
- 阴影：仅弹层（Dialog、Popover、Dropdown）使用一层柔和阴影；卡片只用 1px 边框。
- 窗口布局：侧栏宽 208px（可折叠为 52px 图标栏）；内容区左右内边距 20px；双栏页面（记忆树 + 编辑器）左栏 280px，可拖动 220–420px。
- 最小窗口尺寸 1024×640。

## 6. 动效

- 交互反馈 150ms，面板展开 / 抽屉 200ms，`ease-out`；退出比进入快（120ms）。
- 只动画 `opacity` / `transform`，不动画宽高。
- 尊重 `prefers-reduced-motion`：关闭非必要动画。
- 不使用 GSAP 和滚动动画（工具型界面不需要）。

## 7. 组件规范

- **按钮**：主按钮（`--primary` 底）每屏最多一个；次按钮为描边；危险操作用 `--destructive` 文字按钮并二次确认。图标按钮必须有 `aria-label` 和 tooltip。
- **工具标签**：圆角 4px、高 18px、11px 字、字重 600，底色为工具色 18% 透明度，文字为工具色。
- **记忆树**：节点 28px 高，选中态为 `--muted` 底 + 左侧 2px `--primary` 竖条；不存在 / 未创建的条目用 `--muted-foreground` 加删除线或「未创建」小字。
- **编辑器**：CodeMirror 主题跟随 `--card` / `--foreground`，行号 `--muted-foreground`；顶栏 40px，放路径（等宽、可复制）、token 数、保存 / 历史按钮；未保存时标题前显示圆点。
- **待审核卡片**：左侧 3px 变更类型色条；diff 用等宽字体；依据区为 `--muted` 底的引用块。
- **diff 展示**（用户 2026-11-05 确认）：
  - 默认**左右分栏**：左「当前」、右「建议改为」，行号对齐；一侧没有对应行时用斜纹空行占位。
  - 卡片右上角「上下 / 左右」分段切换，选择记在本地设置里，下次沿用（同 GitHub Split / Unified）。
  - 卡片可用宽度小于 760px 时自动改为上下（单列）显示。
  - 两种布局都做**词级高亮**：整行用对应色 12% 底，真正变化的字词再叠 45% 底。
  - 显示改动前后各 1 行上下文（灰色）。
  - 同一套 diff 组件也用于历史版本对比和同步冲突处理。
  - 参考稿：`.superpowers/brainstorm/1459-1793865330/content/review-diff-v2.html`。
- **空状态**：居中 lucide 图标（32px，`--muted-foreground`）+ 一句说明 + 一个主操作。
- **横幅提示**（外部修改、未安装工具）：`--warning` 15% 底 + 左侧图标 + 行内操作按钮。
- **图表**：每日金额用按工具堆叠的柱状图（Recharts `BarChart`），工具色 + 图例文字；悬停 tooltip 显示两工具分项与合计；图下必须有同数据的表格（可访问性兜底）。

## 8. 可访问性与交付检查

- 文字对比度 ≥ 4.5:1（亮暗两套都要测）。
- 全部可键盘操作：Tab 顺序合理，焦点环清晰（2px `--ring`，2px offset）；树支持方向键。
- 可点击元素 `cursor: pointer`，hover 有 150ms 过渡。
- 不靠颜色单独表达含义（工具、变更类型都带文字）。
- 中英两种语言下检查文字溢出（英文通常更长），按钮宽度不写死。
