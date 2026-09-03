# 原型验证门禁（参考）

## 原型验证门禁（MUST — 原型创建后立即执行）

每次创建或迭代原型后，**必须在 yield 前**通过以下全部检查。

### 1. CSS 框架选择

| 目标                                       | CSS 文件                            | 布局类          | Vendor 路径深度        | `<link>` 路径                                                                                  |
| ------------------------------------------ | ----------------------------------- | --------------- | ---------------------- | ---------------------------------------------------------------------------------------------- |
| `Pre-Proc/{ns}/Prototypes/Modules/{name}/` | `prototype-base.css + 内联组件 CSS` | Tailwind 工具类 | 5 级 `../../../../../` | `../../../../../.agents/skills/alioth-design/references/prototype-base.css`                    |
| `Meta/`                                    | `meta-layout.css`                   | `al-*`          | 3 级 `../../../`       | `../../../Framework/frontend/components/src/meta-layout.css`                                   |
| `Gateway/`                                 | `prototype-base.css + 内联组件 CSS` | Tailwind 工具类 | —                      | 生产用 Tailwind（不直接引用此文件）；原型统一走 references/（已弃用 `tailwind-utilities.css`） |

**路径验证公式**：

```
从 Pre-Proc/{ns}/Prototypes/{type}/{path}/ 到项目根 = 所在目录深度
  Pre-Proc/{ns}/Prototypes/Modules/{name}/ = 5 级 (modules → design → docs → /)
  Meta/                     = 3 级 (meta → design → docs → /)
  gateway/                  = 3 级 (gateway → design → docs → /)
```

验证方法：运行后手动计算 `<link>` 路径中的 `../` 数量是否等于该深度。

### 2. 主题选择

| 目标                                       | 主题策略 | HTML class            | 暗色脚本                                 |
| ------------------------------------------ | -------- | --------------------- | ---------------------------------------- |
| `Pre-Proc/{ns}/Prototypes/Modules/{name}/` | 自动检测 | `<html lang="zh">`    | ✅ 保留（`matchMedia("dark")` 自动切换） |
| `Meta/`                                    | 暗色优先 | `<html class="dark">` | ❌ 不需要（Meta 是暗色强制）             |
| `Gateway/`                                 | 浅色锁定 | `<html lang="zh">`    | ❌ 移除（Gateway 生产是浅色）            |

### 3. CSS 变量完整性

原型必须含 `:root` 块（覆盖 `--primary` 等模块品牌色）。基础变量由 `<link>` 加载的框架 CSS 提供。

```css
:root {
  --primary: {模块主色 HSL};
  --primary-foreground: 0 0% 100%;
  --primary-hover: {主色 浅 8%};
  --primary-bg: hsl(var(--primary)/0.08);
}
```

### 4. 浏览器渲染验证（MUST — ego-browser 优先）

**禁止**在未打开浏览器实际查看的情况下声明原型"可用"。

> **Tab 生命周期**：每次验证操作必须先通过 `find_tab` 查找已有标签页，没有才新建；操作完成后新建的 tab **必须立即关闭**。
> 完整规则见 `visual-verification-protocol.md §8.4`。

```javascript
// 自动化验证脚本（在 browser tab 中运行）
// 注意：Tailwind-only 输出不包含 gl-* 类名，选用结构性/语义化查询
const checks = await page.evaluate(() => ({
  // 布局骨架 — 检查 DOM 结构而非特定 class 名
  root: document.getElementById('root').children.length > 0,
  topbar: !!document.querySelector('#root header, #root div[class*="h-14"]'),
  sidebar: !!document.querySelector('#root nav, #root div[class*="bg-secondary"]'),
  main: !!document.querySelector('#root main, #root div[class*="overflow-y-auto"]'),
  contentArea: document.getElementById('root').children.length >= 2,
  // 导航 — 检查是否存在可交互的导航按钮
  navItems:
    document.querySelectorAll('#root nav button').length ||
    document.querySelectorAll('#root button[class*="rounded-lg"]').length,
  // 主题
  isDark: document.documentElement.classList.contains('dark'),
  bodyBg: window.getComputedStyle(document.body).backgroundColor.slice(0, 30),
  bodyColor: window.getComputedStyle(document.body).color,
  // 功能 — 内容区域有可交互元素
  interactive: document.querySelectorAll('#root button, #root a').length > 0,
}));
```

通过标准：

| 检查项                                   | 必须值                                |
| ---------------------------------------- | ------------------------------------- |
| root, topbar, sidebar, main, contentArea | 全部存在（结构检测，非特定 class 名） |
| navItems                                 | ≥ 1                                   |
| interactive                              | ≥ 1（#root 内按钮/链接可交互）        |
| bodyBg                                   | 浅色模式下 `rgb(255, 255, 255)`       |
| bodyColor                                | 浅色模式下 `rgb(2, 8, 23)`            |

### 5. 审计通过

```bash
bun .agents/skills/alioth-app/scripts/audit-html-spec.ts \
  Pre-Proc/{ns}/Prototypes/Modules/{name}/m-v{N}.html
# 结果: 0 错误
```

### 6. CSS 框架合规审计

```bash
bun scripts/check/audit-css-framework.mjs \
  Pre-Proc/{ns}/Prototypes/Modules/{name}/m-v{N}.html
# 结果: 0 错误
```

### 7. Gateway 布局对齐（仅 Gateway 原型）

Gateway 原型必须精确复刻 Gateway 生产代码的 TopBar 和 WorkspaceDock 布局：

**TopBar 结构** (来自 `Gateway/frontend/src/components/layout/TopBar.tsx`):

```
header.h-14
  ├── 左侧: GatewayLogo + ModuleTabs(App 模式) + Breadcrumbs
  └── 右侧: SearchSlot + InboxTrigger + ScheduleTrigger + ApprovalTrigger + UserMenu
```

**WorkspaceDock** (来自 `Gateway/frontend/src/components/workspace/WorkspaceDock.tsx`):

```jsx
<WorkspaceDock slots={[inbox, schedule, approval]} />
```

**验证**:

- [ ] TopBar 包含 6 个元素（Logo/Tabs/Search/Inbox/Schedule/Approval/UserMenu）——App 模式
- [ ] WorkspaceDock 存在且面板可独立打开/关闭
- [ ] ModuleTabs 可切换（App 模式）
- [ ] 搜索框可输入，值正确更新

### 8. 交互完整性验证

| 交互           | 检查                                                  | 预期          |
| -------------- | ----------------------------------------------------- | ------------- |
| 导航切换       | 点击 Sidebar 导航按钮（`nav button`）                 | 内容区变化    |
| 侧边栏折叠     | 点击 SidebarFoot 折叠按钮（`panelLeft`/`panelRight`） | 宽度 240↔64   |
| 搜索输入       | 输入文字                                              | value 更新    |
| ModuleTab 切换 | 点击 TopBar 区域 Tab 按钮                             | Tab 切换      |
| Workspace 面板 | 点击 TopBar 右侧 trigger 按钮                         | 面板打开/关闭 |

**如果任何交互不工作** → 原型不能 claim done。

### 禁止声明完成的情形

| 如果你做了…               | 但没有…                    | 则不能 claim done |
| ------------------------- | -------------------------- | ----------------- |
| 创建了原型 HTML           | 用 browser 打开并截图验证  | ❌                |
| 写了 `<link>` 引用        | 验证路径深度与实际目录匹配 | ❌                |
| 设置了 `class="dark"`     | 确认目标应用确实需要暗色   | ❌                |
| 让脚本自动添加了 `@layer` | 移除（原型直连不需要）     | ❌                |
| 运行了 audit              | 检查结果是否 0 errors      | ❌                |
