# 导航菜单模式（`menuMode`）— 能力契约

> **单一事实源**：本文件定义原型壳的导航菜单模式能力。
> **实现**：`.agents/skills/alioth-design/references/gateway-shell.tsx`（`MENU_MODES` / `MenuModePicker` / `MenuModeSwatch` / `NavPills` / `IconRail` / `SecondaryNavColumn` / `NavDrawer`）。
> **生产边界**：生产前端（`Gateway/frontend`、`Framework/frontend/components`）**未采用**本能力——`MainLayout` / `Navigation` / `ModuleLayout` 的导航形态仍为单一分组侧栏（`docs/specs/MODULE_SPEC.md` §6 / §11.11）。本能力服务于**原型表达与设计探索**；生产采用需另立 change（含 `app.json` schema 与 Gateway 渲染路径）。
>
> 层间适用性与各层承载方式见下「各层用法」。

## 各层用法

| 层 | 承载方式 |
| --- | --- |
| **Module**（主使用层） | `GatewayShell` 直传 `menuMode` / `collapsedShowText`；`embedded` 分支自行组合 `Navigation` / `IconRail` / `SecondaryNavColumn`，且**顶栏级导航**（`category` 分类 pills、`top` / `topDual` 导航项）由 Module 顶部 `nav[aria-label="主导航"]`（`h-11`）承载——embedded 组合没有自有 TopBar |
| **App**（TopBar-only 契约） | App 不传 `navGroups`、不渲染导航；以**受控透传**承载：App 持布局偏好状态，经 `menuMode` / `collapsedShowText` / `onMenuModeChange` / `onCollapsedShowTextChange` 下发 embedded Module，由 Module 页面内的 `MenuModePicker` 驱动变更（参考实现：`Pre-Proc/Alioth/Prototypes/Apps/shell-menu-modes/`） |
| **Block**（content-only） | 不适用：standalone 预览无 `navGroups` 且渲染选择器属「不出壳」红线；embedded 时随 Module 模式变化，**不得假设固定可用宽度** |

Module 的受控/非受控双形态：`menuMode` / `collapsedShowText` 传入即受控（变更经回调上抛），不传用内部状态——Module 独立预览自洽，App 组合可控（参考实现：`Pre-Proc/Alioth/Prototypes/Modules/shell-menu-modes/`）。

## 数据模型与 API

六种模式是**同一份导航数据的呈现方式**，不改变数据契约：

```tsx
<GatewayShell
  navGroups={NAV_GROUPS}        // NavGroup[]: { label, icon?, items: NavItemDef[] }
  activeId={activeId}           // 当前项 id
  onSelect={setActiveId}
  menuMode={menuMode}           // 可选，默认 'grouped'
  collapsedShowText={bool}      // 可选，默认 false
  collapsed={collapsed}         // 可选；与 onToggle 同传=受控，单独用=内部状态
  onToggle={toggleCollapsed}
/>
```

- `menuMode?: MenuMode` —— `'sidebar' | 'category' | 'dual' | 'top' | 'grouped' | 'topDual'`，默认 `'grouped'`（= 本能力引入前的导航形态，零回归）。
- `collapsedShowText?: boolean` —— 默认 `false`；仅作用于**二级导航列**（见下）。
- `NavGroup.icon?: string` —— 分类 pill / 图标栏 / 顶栏分类的图标；缺省回退该组首项 `icon`，再回退 `box`。
- **与 App/Module 模式正交**：App/Module 模式仍由 `moduleTabs` 存在性判定（`openspec/specs/shell-decomposition`），`menuMode` 不承担该语义、不引入 `mode` prop。
- `hideNavigation={true}` 优先级最高：任何 `menuMode` 都不渲染导航列 / 导航条 / 顶栏导航。

## 六种模式

| `menuMode` | 中文 | 结构 | 导航列宽度 | 折叠按钮 |
| --- | --- | --- | --- | --- |
| `sidebar` | 单栏菜单 | 顶栏 + 单列侧栏（**扁平**项，无分组标题） | `w-60` | 侧栏底部 |
| `grouped`（默认） | 分组菜单 | 顶栏 + 单列侧栏（**分组标题**） | `w-60` | 侧栏底部 |
| `category` | 分类导航 | 顶栏承载**分类** pills + 分类列（当前分类的项） | `w-60` | 分类列底部 |
| `dual` | 双栏菜单 | **图标栏**（分组图标，`w-16` 常驻）+ 二级列（当前分类的项） | 二级列 `w-60` | **图标栏**底部 |
| `top` | 顶部菜单 | 顶栏内联**全部项**（无侧栏） | — | 无 |
| `topDual` | 顶栏双行 | 顶栏（品牌/搜索/操作）+ 第二行导航条 `h-11`（无侧栏） | — | 无 |

- **当前分类派生**：由 `activeId` 反查所属 `navGroups[i]`；点击分类 pill / 图标栏按钮时以点击值为准并 `onSelect(该组首项 id)`，`activeId` 变化即复位。
- **移动端**：`<md` 恒以抽屉呈现（`NavDrawer`，含分组标题），模式只影响 `≥md` 的呈现。
- **顶栏内联槽**：`TopBar` 的 `navInline`（`top` 传全部项、`category` 传分类）；`topDual` 的导航条是 `TopBar` 的兄弟节点（`nav[aria-label="主导航"]`，`h-11`）。

## 折叠语义（`collapsedShowText`）

| 列 | 展开 | 折叠（`false`） | 折叠（`true`） |
| --- | --- | --- | --- |
| **主侧栏**（`sidebar` / `grouped`） | `w-60` | `w-16` 仅图标 | `w-16` 仅图标（**不受开关影响**） |
| **二级列**（`category` 分类列 / `dual` 二级列） | `w-60` | `w-16` 仅图标 | `w-56` 图标 + 文字 |
| `dual` 图标栏 | `w-16` | `w-16`（常驻不折叠） | `w-16` |

主侧栏不受开关影响是**硬约束**：`docs/specs/MODULE_SPEC.md` §11.11.3 的 Sidebar Token（展开 240px / 折叠 64px 仅图标 / `PanelLeft`↔`PanelRight` 图标语义）明文「同时约束 React 组件实现与 HTML 原型设计」。让主侧栏「折叠保留文字」须先修订该 Token（另案），不得在本能力内松动。

## 选择器（设置页控件）

```tsx
<MenuModePicker
  value={menuMode}
  onChange={setMenuMode}
  collapsedShowText={collapsedShowText}
  onCollapsedShowTextChange={setCollapsedShowText}
  title="菜单模式"          // 默认
  hint="…"                 // 可选副标题
  modes={MENU_MODES}       // 可选，默认全量六项
/>
```

- 布局：`flex gap-2` 两行 × 3 张卡片，卡片 `flex-1 min-w-0`；顺序 = `MENU_MODES` 顺序。
- 每张卡片：`role="radio"` + `aria-checked`（容器 `role="radiogroup"`），内容 = swatch + 中文标签。
- 选中态：外层 `p-1 rounded-lg` 在选中时 `bg-primary/15`（未选中 `bg-transparent hover:bg-accent`），标签 `text-primary font-semibold`。**不硬编码参考图的 `#3b82f6`** —— 按 `token-rules.md` 用 `--primary` token。
- 底部开关行：`role="switch"` + `aria-checked`（关 `bg-border`，开 `bg-primary justify-end`）。

### swatch 同构判据（缩略图必须能反推模式）

| 模式 | swatch 结构（深色块 = 导航面 `bg-foreground`；内容面 `bg-muted/30`） |
| --- | --- |
| `sidebar` | 左侧深色列（3 条等宽条）+ 内容 |
| `grouped` | 同左列，但条数更多且含**更短**的分组标题条 |
| `category` | **顶部深色横条** + 左深色列（3 条）+ 内容 |
| `dual` | 更窄深色图标栏（方块 + 2 条）+ **中间浅列**（4 条 `bg-border`）+ 内容 |
| `top` | **顶部深色横条**（主色 pill + 3 点）+ 全宽内容 |
| `topDual` | 顶部深色横条 + **紧邻浅色第二行**（3 条）+ 全宽内容 |

机器可判判据（`< 缩略图根>` 的深色块数 / 子元素数）：`sidebar`(1,2) · `grouped`(1,2) · `category`(2,2) · `dual`(1,3) · `top`(1,2) · `topDual`(1,3)。
（`sidebar` vs `grouped` 与 `top` vs `topDual` 各有一维相同——前者靠导航列内**细条数**区分（`sidebar` 3 条 vs `grouped` 含分组标题条共 5 条），后者靠**是否存在第二行**区分（`topDual` 有 `nav[aria-label="主导航"]` 的 `h-11` 第二行，`top` 无）。）

## 词表约束（硬约束，新增类名必读）

`prototype-base.css` 是**预编译**产物（`scripts/generate-prototype-base-css.mjs`；工具类来自 `tailwind-utilities.css`），**原型内没有 Tailwind JIT** —— 未出现在该文件中的类名等于**没有样式**。

本能力实测**不存在于词表**的常见类（勿使用，它们从未被定义）：`grid-cols-*`、`w-40/44/52/64`、`h-12`、`border-primary`、`ring-2`/`ring-primary/20`、`bg-primary/5`、`items-stretch`、`space-y-2`、`translate-x-*`（正向）、`top-0.5`、`px-0.5`、`rounded-xl`、`aspect-*`、任意 `shadow-[…]`。

与之**不同**的一类问题是选择器转义：类选择器中的 `.` `/` `[` `]` MUST 用**单反斜杠**转义（`.text-\[11px\]`、`.h-0\.5`、`.bg-muted\/30`）；双反斜杠转义（`.text-\\[11px\\]`）与类名内未转义的点（`.h-0.5`）会让规则被解析成别的选择器而**永不命中**（且无任何门禁报警）。生成器 `scripts/generate-prototype-base-css.mjs` 已对此 **fail-closed 自校验**（产出前结构判定，命中即中止），故这类缺陷不会再进入产物。

- 新增样式需求 → 优先用**已存在的类**重新表达（本能力的选中态、swatch、开关全部如此）。
- 确需新类 → 改 `.agents/skills/alioth-design/references/tailwind-utilities.css` 并**重新生成** `prototype-base.css`；代价是**全仓原型内嵌 CSS 全部陈旧**（`check-stale-embedded-css.mjs` 阻断 + `prototype-tool.js refresh-base-css` 全量改写产物）——除非收益明确，**不要走这条路**。
- 自检：用 `css-tree` 解析真实文件比对类名存在性（不要用正则/文本猜测）。

## 验证协议

```bash
# 1) 构建（产出 m-v{N}.html / b-v{N}.html / a-v{N}.html）
bun scripts/prototype-tool.js build <llm-tsx/<module|block|app>.tsx>

# 2) 静态评分（结构 / token / 禁止模式 / 构建元数据）—— 目标 ≥90
bun scripts/eval/evaluate-prototype-reference.ts <产物.html> --human

# 3) 视觉 + 交互验证（三档 viewport 截图 + console 错误 + 报告 + 门禁）
bun scripts/visual-verify.ts verify <产物.html>

# 4) 六模式交互回归（数据驱动断言：结构/宽度/顶栏导航丸/折叠语义/swatch 同构；期望值从页面派生）
bun .agents/skills/alioth-design/scripts/verify-menu-modes.ts <产物.html>   # 退出码 0=全过
```

DOM 断言（ego-browser）要点：

- **宽度断言前必须关闭 CSS 过渡**：壳的导航列带 `transition-all duration-300`，页面不绘制时过渡会节流，`getBoundingClientRect()` 会读到**插值中间值**（实测：折叠后读到 `240px` 或 `69px`，而 `class` 已正确变为 `w-16`）。注入 `*{transition:none !important}` 后再测，稳态值为 `64px`/`224px`/`240px`。
- 结构断言用 `aria-label`（`nav[aria-label="主导航"]`）与 `role`（`radio`/`switch`）而非视觉类名；列数断言用 `document.querySelectorAll('aside')`。
- 回归基线原型：`Pre-Proc/Alioth/Prototypes/Modules/shell-menu-modes/`（六模式画廊 + 选择器，交互切换）。
