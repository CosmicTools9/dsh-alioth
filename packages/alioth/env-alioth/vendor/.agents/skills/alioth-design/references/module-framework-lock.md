# ModuleLayout 框架锁定（参考）

> **权威源**：`docs/specs/MODULE_SPEC.md` §11.11（ModuleLayout 统一布局）
> **参考实现**：`Pre-Proc/AVIC-CAASEC/Sources/Apps/Modules/system-dev/frontend/src/App.tsx`（稳定工作示例）

本技能产出的 Module 原型 MUST 匹配以下布局框架，禁止自行发明不同的 Shell 结构。

#### 11.1 双模式（Two Modes）

| 模式                              | 触发条件                | Shell 提供方                                              | Module 渲染内容                                  |
| --------------------------------- | ----------------------- | --------------------------------------------------------- | ------------------------------------------------ |
| **集成模式**（`embedded: true`）  | Gateway App 视角下运行  | Gateway `MainLayout`（TopBar + Navigation + ContentArea） | `<ContentArea>` 包裹 `<Routes>` — 仅页面内容     |
| **独立模式**（`embedded: false`） | 直接访问模块 URL 或预览 | 模块自身 `ModuleLayout`                                   | 完整 Shell：Sidebar + TopBar + Accent bar + Main |

#### 11.2 布局 Token（独立模式 Shell）

所有原型设计和前端实现 MUST 使用以下固定值，禁止偏离：

| Token            | 值            | Tailwind    | 说明                                   |
| ---------------- | ------------- | ----------- | -------------------------------------- |
| TopBar 高度      | 56px          | `h-14`      | 统一，禁止 64px（`h-16`，仅 Meta 用）  |
| Sidebar 展开宽度 | 240px         | `w-60`      | 导航 + 品牌区                          |
| Sidebar 折叠宽度 | 64px          | `w-16`      | 仅图标                                 |
| Accent bar       | 3px           | `h-[3px]`   | 主题色实色条，位于 TopBar 与 Main 之间 |
| Main padding     | 20px 24px     | `px-6 py-5` | 内容区间距                             |
| Main 背景        | `bg-muted/30` | —           | 统一背景色                             |
| 右侧 Dock        | 320px         | `w-80`      | WorkspaceDock，md+ 可见                |

#### 11.3 Shell 结构（独立模式 DOM 层次）

与原型模板 `gateway-shell.tsx` 对齐：

```
div.flex.h-screen.flex-col.overflow-hidden.bg-background
├── header.h-14.border-b (TopBar)
│   ├── div.flex.items-center.gap-2 (logo + breadcrumbs（Module 模式）)
│   └── div.flex.items-center.gap-3 (search + action-group + user-menu)
└── div.flex.flex-1.min-h-0.overflow-hidden (body)
    ├── aside.w-60↔w-16.border-r.bg-secondary (Navigation)
    │   ├── nav.flex.flex-col (MainNav groups)
    │   └── div.border-t (SidebarFoot collapse)
    └── main.flex.flex-col.min-w-0.overflow-hidden.flex-1
        ├── div.h-\[3px\].bg-primary\/15 (accent-bar, optional)
        ├── div.flex-1.w-full.h-full.bg-muted\/30.overflow-hidden (content)
        │   └── div.flex-col.h-full
        │       ├── div.flex-1.min-h-0.overflow-y-auto (block-scroll)
        │       └── footer.hidden.md\:flex.h-10.border-t (Footer)
        └── div.w-80.border-l.bg-card (WorkspaceDock, conditional)
```

> 注：原型模板 `gateway-shell.tsx` 不含 accent bar；生产 `createModuleLayout` 若需提供主题色强调条，应在 TopBar 与 body 之间独立插入，不得改变上述 DOM 层次。

#### 11.4 品牌归属（Gateway 主责）

**品牌由 Gateway 统一管理，模块 Sidebar 不渲染品牌块。**

- **App 视角（生产环境）**：Sidebar 品牌组件已整体移除。品牌（应用名）由 Gateway `TopBar` 的 `<GatewayLogo showAppName={currentApp.name}>` 统一展示。
- **Gateway 视角**：Gateway TopBar 展示平台品牌名（`brand.name`），Sidebar 不展示品牌。
- **独立模式（开发预览）**：Sidebar 顶部不渲染品牌 block。
- **`accentBarColor`**：MUST 提供 hex 值，用于 Gateway `AppContentArea` 同步 `--primary` CSS 变量实现整站换色。

> 模块的 `title` / `subtitle` / `icon` 字段通过 `setModuleSidebar()` 推送给 Gateway，用于 TopBar 同步和 accentBarColor 关联。模块本身不持有品牌渲染逻辑。

#### 11.5 Block 内联约束（原型侧）

Module 原型内联 Block 时，**禁止**嵌入 Block 自带的 Shell/Sidebar/TopBar：

- 仅提取 Block 内容组件 / helper / mock data / ICONS
- 用轻量 wrapper 包装（如 `Block__{id}SceneContent`）
- 通过 ESM `import` 引入 Block 内容组件，渲染到 `GatewayShell.children`
- Shell 由 ModuleLayout / `GatewayShell` 统一提供（仅一份）

#### 11.6 工厂函数

**生产代码 MUST 使用** `createModuleLayout` 工厂（`@alioth/composables`），参数：

```tsx
createModuleLayout({
  moduleName: "{name}",
  getModuleConfig: (t) => ({ ..., hideBrand: true, accentBarColor: "#...", accentBarStyle: "solid" }),
  useNavItems: () => [...],
  embedded: true,  // Gateway 集成模式
})
```

> **禁止**模块内手写 `ModuleLayout`（除非过渡期）。
> 当前 system-dev `App.tsx` 为 hand-rolled 过渡版本，后续应迁移到 `createModuleLayout`。

#### 11.7 修订历史

| 日期       | 来源                                         | 说明                                  |
| ---------- | -------------------------------------------- | ------------------------------------- |
| 2026-06-21 | system-dev `App.tsx` + MODULE_SPEC.md §11.11 | 首次框架锁定，固化双模式 + 布局 Token |

#### 11.8 App.tsx 标准模式（参考：system-approve）

**权威参考**：`Pre-Proc/Alioth/Sources/Apps/Modules/system-approve/frontend/src/App.tsx`

```tsx
import { Routes, Route, Navigate } from 'react-router';
import { ModuleI18nShell } from '@alioth/i18n';
import { componentsZhCN, componentsEn } from '@alioth/components';
import { createModuleLayout } from '@alioth/composables';
import type { MainNavItem } from '@alioth/components';
import { useT } from '@alioth/i18n';
import zhCNDict from './locales/zh-CN.json';
import enDict from './locales/en.json';
import { PageA } from './pages/PageA';
import { PageB } from './pages/PageB';

const dictionaries = {
  'zh-CN': { ...componentsZhCN, ...zhCNDict },
  en: { ...componentsEn, ...enDict },
};

function useNavItems(): MainNavItem[] {
  const t = useT();
  return [
    {
      id: 'page-a',
      label: t('{module}.nav.pageA'),
      section: t('{module}.nav.group-name'),
      href: '/page-a',
      icon: 'Xxx',
    },
  ];
}

const ModuleLayout = createModuleLayout({
  moduleName: '{module}',
  getModuleConfig: (t) => ({
    title: t('{module}.module.title'),
    subtitle: t('{module}.module.subtitle'),
    icon: 'Xxx',
    hideBrand: true,
    accentBarColor: '#xxxxxx',
    accentBarStyle: 'solid',
  }),
  useNavItems,
});

export default function App() {
  return (
    <ModuleI18nShell dictionaries={dictionaries}>
      <Routes>
        <Route element={<ModuleLayout />}>
          <Route index element={<Navigate to="page-a" replace />} />
          <Route path="page-a" element={<PageA />} />
        </Route>
        <Route path="*" element={<Navigate to="." replace />} />
      </Routes>
    </ModuleI18nShell>
  );
}
```

**关键要素**：

| 要素             | 规则                                                  | 来源                         |
| ---------------- | ----------------------------------------------------- | ---------------------------- |
| Layout 工厂      | `createModuleLayout` 单次调用                         | 消除 hand-rolled 复制粘贴    |
| i18n             | `ModuleI18nShell` + dict merge（组件字典 + 模块字典） | 统一 locale 加载             |
| Routes           | `<Route element={<Layout />}>` 父路由包裹子页面       | 工厂内部用 `<Outlet />`      |
| `hideBrand`      | 建议 `true`（保留作为约定声明）                       | Gateway 统一品牌             |
| `accentBarColor` | MUST 提供 hex 色值                                    | Gateway `--primary` 同步换色 |
| `accentBarStyle` | `"solid"`                                             | 3px 实色条                   |

#### 11.9 NavItem 与 i18n 键结构模式（参考：system-approve locale）

**权威参考**：`Pre-Proc/Alioth/Sources/Apps/Modules/system-approve/frontend/src/locales/{zh-CN,en}.json`

```json
{
  "{module}.module.title": "英文标题",
  "{module}.module.subtitle": "中文描述",
  "{module}.nav.{blockId}": "导航标签",
  "{module}.nav.group-{name}": "导航分组名",
  "{module}.page.{block-id}.h2": "页面标题",
  "{module}.page.{block-id}.searchPlaceholder": "...",
  "{module}.page.{block-id}.col{Field}": "列名",
  "{module}.common.new": "新建",
  "{module}.common.edit": "编辑",
  "{module}.common.save": "保存",
  "{module}.common.cancel": "取消",
  "{module}.common.search": "搜索",
  "{module}.common.loading": "加载中…",
  "{module}.common.empty": "暂无数据",
  "{module}.common.confirmDeleteTitle": "确认删除 {name}？",
  "{module}.common.confirmDeleteMessage": "此操作不可撤销。"
}
```

> **2026-10-11 校准**：键为**扁平点号**形态（键名自带 `{module}.` 前缀），**不是**嵌套对象。实测全 61 个 ns 模块 `locales/zh-CN.json` 均为扁平（0 嵌套）；第二段家族第一梯队 = `page`（2971 键）/ `common`（646）/ `nav`（171），其余（`list`/`detail`/`action`/`status`…）按模块自定。旧文档的嵌套写法与 `scene` 家族（全仓 0 命中）均已无实例。

**键结构规则**：

| 层级   | 模式                                                   | 说明                               |
| ------ | ------------------------------------------------------ | ---------------------------------- |
| module | `{module}.module.{title,subtitle}`                     | 模块元数据，用于 `getModuleConfig` |
| nav    | `{module}.nav.{blockId}` / `{module}.nav.group-{name}` | 导航项标签 + 分组名                |
| page   | `{module}.page.{block-id}.{key}`                       | 页面级文案（标题、列名、占位符）   |
| common | `{module}.common.{key}`                                | 通用操作按钮、提示                 |

#### 11.10 主题色注入模式（参考：system-approve theme.css）

**权威参考**：`Pre-Proc/Alioth/Sources/Apps/Modules/system-approve/frontend/src/theme.css`

模块的自有主题色通过**独立原始 CSS 文件**声明，不经过 Tailwind：

```tsx
// App.tsx — 模块入口处普通载入（Vite 打成 CSS chunk；壳侧 MicroFrontendLoader 动态 import 注入）
import './theme.css';
```

> **2026-10-11 校准**：旧通道 `import themeCss from './theme.css.txt?raw'` + `document.createElement('style')` 手写注入**已退役**——全 ns 模块前端实测 **0 处** `?raw` 代码引用（部分 ns 的 `env.d.ts` 仍留 `*.css?raw` / `*.css.txt?raw` 遗留声明，勿再新增）。

**CSS scoping 规则**（根类 = `.mod-{module}`；实测 ns 模块 theme.css：`.mod-*` 1272 处 vs 旧 `.{module}-page` 30 处）：

```css
/* 所有规则用 .mod-{module} 根类隔离，不泄漏到全局（实例：system-approve/frontend/src/theme.css） */
.mod-{module} {
  --primary: H S L%;       /* HSL 色值，无逗号 */
  --primary-foreground: H S L%;
  --primary-bg: H S L% / A;  /* MUST 裸分量（消费式 = hsl(var(--primary-bg))）；写 hsl(...) 会双包装失效 */
  --color-primary: hsl(H S L%);  /* resoloved JSX inline 用法 */
}
```

> **`--primary-bg` 取值契约（2026-10-11 裁决，change `normalize-primary-bg-token-form`）**：消费式实测 6/6 为 `hsl(var(--primary-bg))` ⇒ 值 MUST 是**裸分量 + `/ α`**。写 `hsl(var(--primary)/8%)` 会解析成 `hsl(hsl(...))` 而失效（render 级实测 = `rgba(0,0,0,0)`；3 个 ns 仓 system-approve 与平台通用暗色规则曾犯此错，已修）。**跨模块通用覆写**（无法写死模块分量）只能写 `var(--primary) / α`——token 替换后仍是裸分量，同式有效（实例：`theme-base.css` 的 `.dark [class^='mod-']`）。

**必填 CSS 变量**（MUST）：

| 变量                                                  | 用途                       | 来源                     |
| ----------------------------------------------------- | -------------------------- | ------------------------ |
| `--primary` / `--primary-foreground` / `--primary-bg` | 主色 + 对比色 + 10% 透底色 | Gateway `--primary` 同步 |
| `--accent` / `--accent-foreground`                    | 强调色                     | shadcn 组件              |
| `--background` / `--foreground`                       | 背景/前景色                | 页面基础                 |
| `--border` / `--input`                                | 边框                       | 表单组件                 |
| `--muted` / `--muted-foreground`                      | 禁用态                     | 次要文字                 |
| `--color-*`（完整 resoloved set）                     | JSX inline style 用        | Tailwind 不可用时        |

#### 11.11 module.json 字段模式（参考：system-approve）

**权威参考**：`Pre-Proc/Alioth/Sources/Apps/Modules/system-approve/module.json`

```json
{
  "id": "{module}",
  "namespace": "Alioth",
  "name": "中文名",
  "description": "描述",
  "category": "infrastructure",
  "status": "active",
  "version": "0.1.3",
  "routePrefix": "/{module}",
  "icon": "Xxx",
  "hasBackend": false,
  "hasFrontend": true,
  "prototypeVersion": "m-v{N}",
  "blockAssembly": { "blocks": [{ "id": "{blockId}", "label": "中文名", "group": "分组名", "order": 0 }], "navigation": { "groups": [] } }
}
```

> **2026-10-11 校准**：键面 MUST 与**磁盘事实**一致（门禁 `scripts/check/check-module-manifest-consistency.ts`：`id` / `namespace` / `hasFrontend` / `hasBackend` / `prototypeVersion`）。实测 61 个 ns 模块：`category` 61、`prototypeVersion` 60、`blockAssembly` 59；`hasWebview` 33（**遗留可选键**）、`scenes[]` **0 命中**（已退役，勿新增）。

#### 11.12 修订历史

| 日期       | 来源                                         | 说明                                                    |
| ---------- | -------------------------------------------- | ------------------------------------------------------- |
| 2026-06-21 | system-dev `App.tsx` + MODULE_SPEC.md §11.11 | 首次框架锁定，固化双模式 + 布局 Token                   |
| 2026-06-21 | system-approve `App.tsx`                    | 标准 App.tsx 模式、i18n 键结构、主题色注入、module.json |
| 2026-10-11 | change `fix-stale-skill-refs` 复检校准                        | 样例模块 `system-settings`→`system-approve`（5 个 ns 仓全无此模块，`theme.css.txt` 无实例）；11.8 `MainNavItem` 归属改 `@alioth/components`；11.9 键改**扁平点号**（61/61 模块实测）并删 `scene` 家族（0 命中）；11.10 删退役的 `?raw`/`.css.txt` 注入通道、根类改 `.mod-{module}`（1272 vs 30）；11.11 module.json 键面写实（含 `prototypeVersion`/`blockAssembly`，删 `scenes[]`） |
