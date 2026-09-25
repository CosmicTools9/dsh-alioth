# Gateway 布局规范（参考）

> **参考用例**：`gateway-shell.tsx`（`alioth-design/references/`）—— 用于校准 Gateway Shell 原型的 DOM/CSS 结构。
> 生产权威源见 `docs/specs/MODULE_SPEC.md` §11.11。
> **注意**：下文 `gl-gateway-*` 类名仅描述参考用例自身的 DOM 标记；实际原型产物（a-/m-/b-v{N}.html）必须使用 `gateway-shell.tsx` 共享组件提供的 Tailwind 工具类实现，禁止输出 `gl-gateway-*` 类名或重复内联壳 CSS。

### 9.4 Gateway 布局规范（新增 — Gateway 原型必读）

Gateway 产品原型必须精确复刻生产 Gateway 的布局结构：

```
┌─ TopBar ─────────────────────────────────────────────┐
│ [Logo] [ModuleTabs] (仅 App 模式) [SearchSlot] [Actions] [User]│
├─────────────────────────┬────────────────────────────┤
│ Navigation              │ ContentArea               │
│ (sidebar, 可折叠)        │ (页面内容)                │
│                         │                           │
├─────────────────────────┤                           │
│ Footer                  │                           │
└─────────────────────────┴────────────────────────────┘
```

DOM 层次（对应 `gateway-shell.tsx`）——参考用例中的 `gl-gateway-*` 类名仅用于说明其自身结构；**实际原型产物须使用 Tailwind 工具类**（通过 `gateway-shell.tsx` 共享组件），禁止输出 `gl-gateway-*` 类名：

```
.root (flex-col h-screen overflow-hidden bg-background)
├── header.h-14.border-b (TopBar)
│   ├── left:  logo + breadcrumbs（Module 模式）/ logo + module-tabs + breadcrumbs（App 模式）
│   └── right: search-slot + actions + user-menu
└── .flex.flex-1.min-h-0.overflow-hidden (body)
    ├── .Navigation (sidebar, w-60 ↔ w-16)
    │   ├── MainNav (section-based grouped navigation)
    │   └── SidebarFoot (collapse toggle only)
    └── .flex.flex-col.min-w-0.overflow-hidden.flex-1 (main)
        ├── .h-[3px].w-full.bg-primary/15 (accent-bar)
        ├── main.flex-1.w-full.h-full.bg-muted/30.overflow-hidden (content)
        │   ├── .flex.flex-col.h-full (content-inner)
        │   │   ├── .flex-1.min-h-0.overflow-y-auto (block-scroll)
        │   │   └── Footer
        └── WorkspaceDock (w-80, conditional)
```

#### 搜索模式（TopBar 与 content-area 两种）

TopBar 全局搜索槽由 `gl-search-slot` 提供，每个 Module 原型**不应**再写自定义实现；content-area 内的局部搜索输入应使用统一结构：

```jsx
h(
  'div',
  { className: 'relative h-8' },
  h('span', {
    className:
      'absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground inline-flex w-3.5 h-3.5 items-center justify-center',
    dangerouslySetInnerHTML: { __html: ICONS.Search },
  }),
  h('input', {
    className:
      'pl-8 pr-3 h-8 text-sm border border-border rounded-lg bg-card text-foreground outline-none focus:border-primary/30 focus:shadow-[0_0_0_3px_hsl(var(--primary)/0.08)] min-w-[200px]',
    placeholder: '搜索...',
    value: search,
    onChange: function (e) {
      setSearch(e.target.value);
    },
  }),
);
```

要点：

- 父容器 `div.relative h-8` 提供 32px 高度，使 `top-1/2` 和 `-translate-y-1/2` 能正确居中。
- 图标 `left-3`（12px）与 TopBar 搜索槽图标左间距一致，不再贴边。
- `top-1/2` / `-translate-y-1/2` 在 utility CSS 中必须写为 `.top-1\\/2` / `.-translate-y-1\\/2`，否则浏览器会忽略居中规则。

#### TopBar 结构（精确复刻）

Gateway 的 `TopBar.tsx` 渲染顺序：

```
<FrameworkTopBar>
  logo:     [MenuButton] + [GatewayLogo SVG] + [App Name]
  tabs:     <ModuleTabs modules={appModules} /> (仅 AppPerspective)
  searchSlot: <SearchSlot /> (仅 hasApps)
  actions:  <ActionGroup />
  userMenu: <UserMenu />
</FrameworkTopBar>
```

原型实现（与 `gateway-shell.tsx` 一致）：

```jsx
h(
  'header',
  { className: 'gl-gateway-topbar' },
  // 左区：Logo + Breadcrumbs（Module 模式）或 Logo + ModuleTabs + Breadcrumbs（App 模式）
  h(
    'div',
    { className: 'gl-gateway-topbar-left' },
    h('a', { className: 'gl-gateway-logo' }, ...),
    // App 模式才渲染 ModuleTabs
    h('nav', { className: 'gl-module-tabs' }, ...),
    h('nav', { className: 'gl-breadcrumbs' }, ...),
  ),
  // 右区：搜索 + 操作 + 用户菜单
  h(
    'div',
    { className: 'gl-gateway-topbar-right' },
    h('div', { className: 'gl-search-slot' }, ...),
    h('div', { className: 'gl-action-group' }, ...),
    h('div', { className: 'gl-user-menu' }, ...),
  ),
);
```

#### 窄屏布局不变量（TopBar 左簇与右簇）

TopBar 左簇（`flex-1 min-w-0`）内的**品牌块 MUST 可压缩**：`gateway-shell.tsx` 的品牌链接用
`min-w-0`（MUST NOT 用 `shrink-0`），块级 App 模式额外带 `w-60`（与 240px 侧栏同宽，仅作**基准宽度**）——
空间不足时按 flex 收缩，品牌标签自身 `hidden sm:inline` + `truncate`，窄屏只剩图标即不再吃宽度。

判据（`visual-verify` 的窄屏机械检查）：375px 档不得出现品牌链接盒 × 右簇动作按钮的 `overlap`。
反例（2026-09-22 实测，修复前）：`shrink-0` + `w-60` 使品牌块在 375px 恒定占 240px，`Δoverlap=12`，
并伴随品牌文字被挤至零宽——表现为「窄屏丢品牌信息」。

右簇亦然：`ActionGroup` 的 workspace triggers 在 `<md` 隐藏（`hidden md:flex`）。触发项是 `w-9` 固定宽，
其 min-content 之和使右簇在 375px 溢出——实测**右簇 390px > 375px** ⇒ 左簇被压为 0、移动端菜单按钮
与外溢右簇几何重叠、尾部被 `header` 的 `overflow-hidden` 裁掉。移动端保留 搜索 / 主题 / 语言 / 用户。

> **同款反例模式（已修 2026-09-22）**：`activity-shell.tsx` 的品牌链接原带 `shrink-0`，已改为
> `min-w-0`（对齐本节不变量）；预览壳族（`shells/{block,module,app}-shell.tsx`）同期一并对齐，
> 并把三个断点契约固化到壳源与 `prototype-base.css`：
> ① 品牌链接 `min-w-0` + 品牌字 `hidden sm:inline`；
> ② 顶栏搜索槽（`w-72`=288px，`previewTopBarRight`）`hidden lg:block`——`.lg\:block` 需在基座中定义，
>    否则 `lg:block` 无定义 ⇒ 搜索框永不显示；
> ③ 模块标签条 `hidden sm:flex`、标签 `min-w-0 max-w-[42vw]`、标题 `whitespace-nowrap truncate min-w-0`
>    （截断链每一环 MUST `min-w-0`，缺一环即溢出并压覆右簇）。
> 同类改动后 MUST 重建受影响原型（`bun scripts/prototype-tool.js build <llm-tsx/module.tsx>`，构建会级联
> 重建所属 App 原型并同步 `Sources/`），再跑 `visual-verify`；每个原型目录保留最近 3 个版本。

#### Navigation 结构

Gateway 的 Navigation 组件渲染：

- App 视角：`AppNavContent`（模块导航）
- Gateway 视角：`GatewayNavContent`（App 列表）
- 底部：折叠按钮
  （品牌 block 已整体移除，品牌由 Gateway TopBar 统一管理）

原型实现使用 `.gl-gateway-sidebar` + `.gl-main-nav` + `.gl-nav-section` + `.gl-nav-item` + `.gl-sidebar-foot` + `.gl-collapse-btn`，与 `gateway-shell.tsx` 保持一致。

#### 导航菜单模式变体（原型壳扩展）

`gateway-shell.tsx` 的 `GatewayShell` 另提供 `menuMode`（`sidebar` / `category` / `dual` / `top` / `grouped` / `topDual`，默认 `grouped` = 上文单列分组侧栏）与 `collapsedShowText`（仅作用于二级导航列），并附 `MenuModePicker` 设置页选择器。各模式的 DOM / 尺寸 / 折叠 / 选择器契约统一见 [menu-modes.md](menu-modes.md)（能力单一事实源）。

- 上文描述的单列分组侧栏 = `menuMode="grouped"`（默认，零回归）；`hideNavigation={true}` 对所有模式优先。
- 模式只改变导航**呈现方式**，不改变 `navGroups` 数据契约；`top` / `topDual` 无导航列（项内联顶栏 / 顶栏第二行 `h-11` 导航条）。
- 主侧栏折叠尺寸仍受 `docs/specs/MODULE_SPEC.md` §11.11.3 Token 约束（240px ↔ 64px 仅图标）；本变体不松动该 Token。
- **生产未采用**：生产 Gateway 仍为单列侧栏；本变体服务于原型表达与设计探索。

#### WorkspaceDock 结构

Gateway 的右侧面板在工作区激活时展开：

```
<div className="flex flex-1 min-h-0 overflow-hidden">
  <ContentArea />       ← 左：导航 + 内容
  <WorkspaceDock />     ← 右：工作区（条件渲染，w-80）
</div>
```

原型必须包含此结构，即使 WorkspaceDock 仅作占位。

**适用范围**：所有将多个 Block 内联到 Module 原型（`Pre-Proc/{ns}/Prototypes/Modules/{name}/v{N}.html`）的场景。Single-scene 模式同样适用。
