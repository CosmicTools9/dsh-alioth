# ModuleLayout 框架锁定（参考）

> **权威源**：`docs/specs/MODULE_SPEC.md` §11.11（ModuleLayout 统一布局）
> **参考实现**：`Pre-Proc/AVIC-CAASEC/Sources/Modules/system-dev/frontend/src/App.tsx`（稳定工作示例）

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

#### 11.8 App.tsx 标准模式（参考：system-settings）

**权威参考**：`Pre-Proc/Alioth/Sources/Modules/system-settings/frontend/src/App.tsx`

```tsx
import { Routes, Route, Navigate } from 'react-router';
import { ModuleI18nShell } from '@alioth/i18n';
import { componentsZhCN, componentsEn } from '@alioth/components';
import { createModuleLayout } from '@alioth/composables';
import type { MainNavItem } from '@alioth/composables';
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

#### 11.9 NavItem 与 i18n 键结构模式（参考：system-settings locale）

**权威参考**：`Pre-Proc/Alioth/Sources/Modules/system-settings/frontend/src/locales/{zh-CN,en}.json`

```json
{
  "{module}": {
    "module": {
      "title": "英文标题",
      "subtitle": "中文描述"
    },
    "nav": {
      "{blockId}": "导航标签",
      "group-{name}": "导航分组名"
    },
    "scene": {
      "{block-id}": "Scene 显示名"
    },
    "page": {
      "{block-id}": {
        "h2": "页面标题",
        "searchPlaceholder": "...",
        "col{Field}": "列名"
      }
    },
    "common": {
      "new": "新建",
      "edit": "编辑",
      "save": "保存",
      "cancel": "取消",
      "search": "搜索",
      "loading": "加载中…",
      "empty": "暂无数据",
      "confirmDeleteTitle": "确认删除 {name}？",
      "confirmDeleteMessage": "此操作不可撤销。"
    }
  }
}
```

**键结构规则**：

| 层级   | 模式                                                   | 说明                               |
| ------ | ------------------------------------------------------ | ---------------------------------- |
| module | `{module}.module.{title,subtitle}`                     | 模块元数据，用于 `getModuleConfig` |
| nav    | `{module}.nav.{blockId}` / `{module}.nav.group-{name}` | 导航项标签 + 分组名                |
| scene  | `{module}.scene.{block-id}`                            | Scene 显示名                       |
| page   | `{module}.page.{block-id}.{key}`                       | 页面级文案（标题、列名、占位符）   |
| common | `{module}.common.{key}`                                | 通用操作按钮、提示                 |

#### 11.10 主题色注入模式（参考：system-settings theme.css）

**权威参考**：`Pre-Proc/Alioth/Sources/Modules/system-settings/frontend/src/theme.css.txt`

模块的自有主题色通过**原始 CSS 文件**注入，不经过 Tailwind：  
（Vite 的 `?raw` 后缀绕过 Tailwind 处理，保留原生 CSS 变量定义）

```tsx
// App.tsx — 模块入口处注入
import themeCss from './theme.css.txt?raw';
import { useEffect } from 'react';

export default function App() {
  useEffect(() => {
    const el =
      document.getElementById('{module}-theme') ||
      (() => {
        const s = document.createElement('style');
        s.id = '{module}-theme';
        document.head.appendChild(s);
        return s;
      })();
    el.textContent = themeCss;
  }, []);
  // ...
}
```

**CSS scoping 规则**：

```css
/* 所有规则用 .{module}-page 前缀隔离，不泄漏到全局 */
.{module}-page {
  --primary: H S L%;       /* HSL 色值，无逗号 */
  --primary-foreground: H S L%;
  --primary-bg: H S L% / A;  /* 透明度 */
  --color-primary: hsl(H S L%);  /* resoloved JSX inline 用法 */
}
```

**必填 CSS 变量**（MUST）：

| 变量                                                  | 用途                       | 来源                     |
| ----------------------------------------------------- | -------------------------- | ------------------------ |
| `--primary` / `--primary-foreground` / `--primary-bg` | 主色 + 对比色 + 10% 透底色 | Gateway `--primary` 同步 |
| `--accent` / `--accent-foreground`                    | 强调色                     | shadcn 组件              |
| `--background` / `--foreground`                       | 背景/前景色                | 页面基础                 |
| `--border` / `--input`                                | 边框                       | 表单组件                 |
| `--muted` / `--muted-foreground`                      | 禁用态                     | 次要文字                 |
| `--color-*`（完整 resoloved set）                     | JSX inline style 用        | Tailwind 不可用时        |

#### 11.11 module.json 字段模式（参考：system-settings）

**权威参考**：`Pre-Proc/Alioth/Sources/Modules/system-settings/module.json`

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
  "hasWebview": true,
  "scenes": [{ "id": "scene-xxx", "group": "分组名" }]
}
```

#### 11.12 修订历史

| 日期       | 来源                                         | 说明                                                    |
| ---------- | -------------------------------------------- | ------------------------------------------------------- |
| 2026-06-21 | system-dev `App.tsx` + MODULE_SPEC.md §11.11 | 首次框架锁定，固化双模式 + 布局 Token                   |
| 2026-06-21 | system-settings `App.tsx`                    | 标准 App.tsx 模式、i18n 键结构、主题色注入、module.json |
