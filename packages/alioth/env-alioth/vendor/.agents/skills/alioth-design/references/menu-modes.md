# 导航菜单模式（`menuMode`）— 能力契约

> **单一事实源**：本文件定义原型壳的导航菜单模式能力。
> **实现**：`.agents/skills/alioth-design/references/gateway-shell.tsx`（`MENU_MODES` / `MenuModePicker` / `MenuModeSwatch` / `NavPills` / `IconRail` / `SecondaryNavColumn` / `NavDrawer`）。
> **生产边界**：生产 Gateway **已采用**本能力（原「未采用」陈述已由 `adopt-navigation-menu-modes` 落地推翻）——App 视角六模式实现于 `Gateway/frontend/src/atoms/navigation-mode-atoms.ts` + `src/components/layout/{Navigation,NavBars}.tsx`，规约正本 = `openspec/specs/gateway-navigation-modes`，设置入口 = 个人中心「导航」tab（`?tab=navigation`），偏好为 per-browser localStorage（MUST NOT 写 `app.json` 或 DB）。仍在原型面之外的只有**模块级壳**：生产 `Framework/frontend/composables/src/layout/ModuleLayout.tsx` + `Framework/frontend/components/src/components/layout/MainNav.tsx` 不接收 `menuMode`——模块恒以 `embedded` 形式经 `setModuleSidebar` 把 `navItems` 推给 Gateway 渲染，其独立预览形态仍为单一分组侧栏（`docs/specs/MODULE_SPEC.md` §6 / §11.11）。
>
> 层间适用性与各层承载方式见下「各层用法」。

## 各层用法

| 层 | 承载方式 |
| --- | --- |
| **Module**（主使用层） | `GatewayShell` 直传 `menuMode` / `collapsedShowText`；`embedded` 分支自行组合 `Navigation` / `IconRail` / `SecondaryNavColumn`，且**顶栏级导航**（`top` 的导航项 pills / `topDual` 第二行；`category` 的切换控件在分类列内）由 Module 顶部 `nav[aria-label="主导航"]`（`h-11`）承载——embedded 组合没有自有 TopBar |
| **App**（壳承载导航，契约 change `revise-app-prototype-nav-hosting`） | **App 壳渲染导航**（与生产同构：Gateway 壳渲染模块推送的导航槽）——App 持 `navGroups` + `activeId` / `onSelect` 交给 `GatewayShell`，壳按 `menuMode` 渲染顶栏内联丸 / `topDual` 第二行 / 导航列；MUST NOT 用 `hideNavigation` 关闭、MUST NOT 渲染第二套导航。被承载 Module 以 `embedded` 只渲染页面内容（MUST NOT 渲染 `NavPills` / `IconRail` / `SecondaryNavColumn` / `<Navigation>` / `<Footer>`）。布局偏好与导航数据均受控透传，由 Module 页面内的 `MenuModePicker` 驱动变更（参考实现：`Pre-Proc/Alioth/Prototypes/Apps/shell-menu-modes/`；红线由 `prototype-tool.js#preCheckApp/preCheckModule` 校验） |
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
- `NavGroup.icon?: string` —— 图标栏 / 分类列分组头 / 分组标题的图标；缺省回退该组首项 `icon`，再回退 `box`。
- **与 App/Module 模式正交**：App/Module 模式仍由 `moduleTabs` 存在性判定（`openspec/specs/shell-decomposition`），`menuMode` 不承担该语义、不引入 `mode` prop。
- `hideNavigation={true}` 优先级最高：任何 `menuMode` 都不渲染导航列 / 导航条 / 顶栏导航。

## 六种模式

| `menuMode` | 中文 | 结构 | 导航列宽度 | 折叠按钮 |
| --- | --- | --- | --- | --- |
| `sidebar` | 单栏菜单 | 顶栏 + 单列侧栏（**扁平**项，无分组标题） | `w-60` | 侧栏底部 |
| `grouped`（默认） | 分组菜单 | 顶栏 + 单列侧栏（**分组标题**） | `w-60` | 侧栏底部 |
| `category` | 分类导航 | **分类列（单开手风琴）**：列内分组头可点击、同一次只展开一组、仅展开组列出项（**顶栏无分类丸**） | `w-60` | 分类列底部 |
| `dual` | 双栏菜单 | **图标栏**（分组图标；仅图标 `w-16` / 展开 `w-56` 常驻）+ 二级列（当前分类的项） | 二级列 `w-60` | **图标栏**底部 |
| `top` | 顶部菜单 | 顶栏内联可视项（无侧栏）+ **「更多」菜单**承载溢出（Priority+） | — | 无 |
| `topDual` | 顶栏双行 | 顶栏（品牌/搜索/操作）+ 第二行导航条 `h-11`（无侧栏） | — | 无 |

> **无回落（2026-10-05 用户裁决）**：六模式的呈现**只由 `menuMode` 偏好决定**，数据形态（分组数 / 项数）MUST NOT 改变生效模式、MUST NOT 回落为 `grouped`。边界形态 MUST 由组件与样式承载（契约 = `openspec/specs/shell-navigation-modes#menu-mode-boundary-data-shapes`；生产落地同 `gateway-navigation-modes#production-six-mode-rendering`）：
>
> - **单分组**：`category` 仍渲染分类列（列内含恰 1 个分组头，同一次展开该组）；`dual` 仍渲染图标栏 + 二级列（图标栏常驻并承载折叠钮）——MUST NOT 隐藏结构或改渲染为 `grouped`。
> - **多项目（溢出承载 = Priority+「更多」菜单）**：`top` 内联槽放得下的项内联、**放不下的进「更多」下拉**（点击触发、`aria-haspopup="menu"`、当前项被折叠时触发器呈激活态）；MUST NOT 以横向滚动条承载内联溢出、MUST NOT 截断丢项（「可视项 + 菜单项 = 全部项」）。`topDual` 的第二行为**整行**承载，可横向滚动且滚动可见。判定数据/依据见基准 `docs/specs/NAV_MENU_BENCHMARK.md`（Ant Design `overflowedIndicator` / PatternFly overflow / Salesforce More）。
> - **图标栏两态（`dual`，对齐 M3 expanded rail）**：仅图标态 `w-16` / 展开态 `w-56`（图标 + 标签，标签 MUST NOT 截断）——展开由 `collapsedShowText` 偏好驱动；条目命中区高度 MUST ≥ `40px`。
> - **命中区**：顶栏导航丸高度 MUST ≥ `36px`。
> - **面包屑**（同排不可变形且不可溢出，两者同时成立）：MUST 单行（`flex-nowrap` + `whitespace-nowrap`）、MUST 可收缩并在超长时**省略**末段（`min-w-0` + `truncate`）——MUST NOT 换行成竖排，也 MUST NOT 把右侧簇挤出视口。
> - **判定数据**：`ct-wms-menu-modes` 画廊提供 1/3 分组 × 9/33 项**边界压力位**（`?shape=` 深链）——真实 ct 模块只有 1–2 分组 / 1–3 项，压不出边界形态。

- **当前分类派生**：由 `activeId` 反查所属 `navGroups[i]`；点击分类列分组头 / 图标栏按钮时以点击值为准并 `onSelect(该组首项 id)`，`activeId` 变化即复位。
- **移动端**：`<md` 恒以抽屉呈现（`NavDrawer`，含分组标题），模式只影响 `≥md` 的呈现。
- **顶栏内联槽**：`TopBar` 的 `navInline`（仅 `top` 传全部项，含 Priority+「更多」折叠；`category` **不**经顶栏——其切换在分类列内）；`topDual` 的导航条是 `TopBar` 的兄弟节点（`nav[aria-label="主导航"]`，`h-11`）。

## 折叠语义（`collapsedShowText`）

| 列 | 展开 | 折叠（`false`） | 折叠（`true`） |
| --- | --- | --- | --- |
| **主侧栏**（`sidebar` / `grouped`） | `w-60` | `w-16` 仅图标 | `w-16` 仅图标（**不受开关影响**） |
| **二级列**（`category` 分类列 / `dual` 二级列） | `w-60` | `w-16` 仅图标 | `w-56` 图标 + 文字 |
| `dual` 图标栏 | `w-16` 仅图标 | `w-16`（不受折叠影响） | `w-56`（图标 + 标签） |

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
