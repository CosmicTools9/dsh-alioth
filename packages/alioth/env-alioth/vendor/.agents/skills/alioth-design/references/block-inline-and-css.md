# Block 内联规则与 CSS 健壮性（参考）

> **2026-07 更新**：alioth-block 已统一使用 `Pre-Proc/Alioth/Prototypes/template.html` 作为 Scene 原型基线。模板自带完整 Gateway Shell，Scene 组件只实现内容，因此不再出现 `SCENE_STANDALONE`、`SceneApp`、`SceneShell` 等独立壳残留。本章的历史规则仍适用于从旧 Scene 原型迁移或理解为什么只提取内容。

#### 9.5.1 根因（2026-06-18 system-dev 重构案例）

将 Block 整段复制进 Module 原型时，**Block 自带的 Shell / Sidebar / TopBar / BlockApp / SceneContent 也会一起被复制**。若通过 `window.aliothBlockComponents[blockId].render(props)` 调用，每个 Block 的 `render` 会返回它自己的完整壳（Sidebar + TopBar + 内容），导致：

> 注：ESM 路径下不通过全局注册引用 Block；应通过 ESM `import` 引入 Block 内容组件，只渲染内容到 `GatewayShell.children`。

- ❌ Sidebar 嵌套（Module 的 Sidebar + Block 的 Sidebar）— 用户看到两套导航
- ❌ Topbar 嵌套（Module 的 Topbar 含子系统选择器 + Block 的 Topbar 含 search）— 用户的子系统选择器被遮蔽
- ❌ 多次 `root.render` 调用 — React 19 报"createRoot 重复"异常
- ❌ `SCENE_STANDALONE` 残留 — Scene 原型是 `file://` 独立运行的，集成后 `if (SCENE_STANDALONE) root.render(...)` 块会成为死代码，且第二次调用 `createRoot` 会抛错

#### 9.5.2 规则：Module 内联 Block 时**只提取内容**

对每个内联 Block，**保留**：

| 类型          | 示例                                                                                                        |
| ------------- | ----------------------------------------------------------------------------------------------------------- |
| Page 函数     | `DashboardPage`, `ProjectListPage`, `MaterialLibraryPage`, `GateStatusPage` 等                              |
| 辅助组件      | `Drawer`, `Pagination`, `BurndownChart`, `BomStructureTable`, `GateStepTimeline` 等                         |
| 工具函数      | `statusBadge`, `formatCost`, `parseDate`, `addDays`, `daysBetween`, `clamp` 等                              |
| Mock 数据     | 统一放在 `llm-tsx/mock.json`，Block 内通过 `import MOCK from './mock.json'` 引用；禁止在 block.tsx 内联定义 |
| ICONS 对象    | 每个 Block 自己的 ICONS（保留 key 不冲突即可）                                                              |
| 内部状态 Hook | 仅 page-level 的 useState（不含 scene shell 的）                                                            |

对每个内联 Block，**移除**：

| 必删                                                                         | 来源               | 后果（不删）                              |
| ---------------------------------------------------------------------------- | ------------------ | ----------------------------------------- |
| `SceneShell` / `Shell` 函数                                                  | Block 顶层布局     | 嵌套壳                                    |
| `Sidebar` / `SceneSidebar` 函数                                              | Block 导航         | 嵌套 sidebar                              |
| `TopBar` / `SceneTopBar` 函数                                                | Block 顶栏         | 嵌套 topbar                               |
| `SceneApp` 函数                                                              | Block 顶层 wrapper | 嵌套壳                                    |
| `SceneContent` 函数（原始版本）                                              | Block 路由         | 嵌套壳 / 内部 screen state 与 module 冲突 |
| `DarkModeToggle` / `ToastHost` / `ConfirmDialogHost` / `UserMenu` 等基础设施 | Module 已有        | 重复                                      |
| `SCENE_STANDALONE = true` 标志                                               | Scene 独立运行模式 | Module 已有 SCENE_STANDALONE              |
| `if (SCENE_STANDALONE) { root.render(...) }` 块                              | Scene 独立挂载     | 重复 createRoot → React 19 异常           |
| `ReactDOM.createRoot` 调用                                                   | Block 挂载         | Module 已有 createRoot                    |

#### 9.5.3 规则：每个 Block 加一个轻量 `*_SceneContent` wrapper

替换原始的 `SceneContent`，新增一个**只负责内部 screen state 路由**的轻量 wrapper：

```jsx
function developDash_SceneContent(props) {
  var _screen = useState('dashboard');
  var screen = _screen[0],
    setScreen = _screen[1];
  // ... 其他 scene 内部 state
  var page;
  if (screen === 'dashboard') page = h(DashboardPage, props);
  // ... 其他路由分支
  return h('div', { className: 'page' }, page);
}
```

规则：

- **必须**用 `*_SceneContent` 前缀命名（避免与同名 `SceneContent` 冲突）
- 内部用 `useState` 管理 scene 自己的 screen state
- 每个 page 必须用 `h('div', { className: 'page' }, ...)` 包裹（保持 ModuleLayout 的 20px 24px padding）
- 接收 `{compact, projectContext, activeFlowId}` props

#### 9.5.4 规则：Block ESM 导入与渲染

在 Module `llm-tsx/module.tsx` 中显式 import 每个 Block 内容组件，并渲染到 `GatewayShell` 的 `children`：

```tsx
import Block__developDashboard from '../Blocks/develop-dashboard/flows/main/index';
import Block__qualityDashboard from '../Blocks/quality-dashboard/flows/main/index';

function ModuleContent({ activeId }) {
  switch (activeId) {
    case 'develop-dashboard':
      return <Block__developDashboard compact={false} />;
    case 'quality-dashboard':
      return <Block__qualityDashboard compact={false} />;
    default:
      return <div className="page">Select a block</div>;
  }
}
```

每个 Block 内容组件必须：

- 不渲染自己的 Shell/Sidebar/TopBar
- 接收 `compact` 等 Module 传入的 props
- 通过 `block.json` 声明 `factors` / `flows` / `serviceIds`

#### 9.5.5 验证规则（自动化）

| 模式                                                             | 期望次数                                    | 说明                                                        |
| ---------------------------------------------------------------- | ------------------------------------------- | ----------------------------------------------------------- |
| `SCENE_STANDALONE`                                               | **0**                                       | Module 不需要此标志                                         |
| `ReactDOM.createRoot`                                            | **1**                                       | 仅 ModuleLayout 的 root.render 用到                         |
| `root.render`                                                    | **1**                                       | 仅 ModuleLayout                                             |
| `function Shell`                                                 | **0**                                       | Module 用 ModuleLayout，不用 Shell                          |
| `function Sidebar`                                               | **0**                                       | Module 用 ModuleLayout 自己的 Sidebar                       |
| `function TopBar`                                                | **0**                                       | Module 用 ModuleLayout 自己的 Topbar                        |
| `function SceneApp`                                              | **0**                                       | 原始 SceneApp 必删                                          |
| `function SceneContent`                                          | **0**                                       | 原始 SceneContent 必删（由 `*_SceneContent` 替代）          |
| `import Block__* from '...'` / `export default function Block()` | **≥N**（N 个 Block ESM 导出被 Module 导入） | 每个 Block 必须被显式 import 并渲染到 GatewayShell children |

- **审计脚本**：`bun .agents/skills/alioth-block/scripts/audit-block-integration.ts Pre-Proc/{ns}/Prototypes/Modules/{name}/m-v{N}.html`

#### 9.5.6 反模式示例（禁止）

```jsx
// ❌ 错误：整段嵌入 Scene
function SceneApp() {
 return h(Shell, null, h(Sidebar, null, ...), h(Main, null, h(DashboardPage)));
}
function SceneContent({page}) { ... }
var SCENE_STANDALONE = true;
if (SCENE_STANDALONE) {
 ReactDOM.createRoot(document.getElementById('root')).render(h(SceneApp));
}
// 全部必删
function DashboardPage(props) { return h('div', {className: 'page'}, ...); }
function developDash_SceneContent(props) {
 return h('div', {className: 'page'}, h(DashboardPage, props));
}
// 在 Module tsx 中通过 ESM import 引用 Block 内容组件
import Block__developDashboard from './Blocks/develop-dashboard/flows/main/index';

function ModuleContent({ activeBlock }) {
  switch (activeBlock) {
    case 'develop-dashboard':
      return <Block__developDashboard compact={false} />;
    default:
      return <div className="page">请选择 Block</div>;
  }
}
```

#### 9.5.7 已知限制

- 适用 Track 1N（组装原型设计）。Single-scene 模式下没有 ModuleLayout，但仍不应有 `SCENE_STANDALONE`/`root.render` 残留（Module 自身的 root.render 已处理）。
- 不读 Block 的 `frontend/src/`——保持 Track 1 边界。

### 9.6 CSS 语法健壮性与静默错误审计（2026-06-18 立）

**适用范围**：所有 `<style>` 块内容 > 100 行的 Module/Block 原型。

#### 9.6.1 根因（2026-06-18 system-dev CSS syntax error 案例）

system-dev v1 的 `<style>` 块（20K+ 行）中，两处 CSS 语法错误导致浏览器 CSS 解析器静默崩溃：

| 位置     | 错误                                     | 后果                                                                                                                                | 发现方式                                               |
| -------- | ---------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| 原 L1767 | `.04);` — 孤立的 `@keyframes` 阴影值片段 | 浏览器遇到无法解析的选择器 → 跳过 `* {` 之后所有规则（`.alert`、`.metric-row`、`.card-grid`、`.page`、`.page-header` 等完全不可见） | 运行 `audit-css-robustness.ts` 后 `{`/`}` balance 异常 |
| 原 L1826 | `.5;` — 孤立的动画 `opacity` 值片段      | 同上（与上一条相距 60 行，两个独立错误）                                                                                            | 同上                                                   |

**关键教训**：CSS 语法错误导致**静默失败**——没有 JS 异常、没有 console 警告、没有 React error boundary 捕获。页面似乎"正常运行"但布局、颜色、尺寸全部错误。开发者容易误判为"布局 bug"而非"CSS 未加载"。

#### 9.6.2 规则（4 条）

| 规则                                        | 一句话                                                                                                                                                                                                                                                 | 检测方法                                                                                                                                                     |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **1. CSS brace balance**                    | `<style>` 块中 `{` 和 `}` 的计数必须平衡（差 0）。若平衡漂移，浏览器会在第一次漂移处停止解析——不是因为 brace 不匹配，而是两个规则之间出现了不属于任何规则的浮动值（如 `.5;` 或 `.04);`）                                                               | `audit-css-robustness.ts` 逐块扫描，记录每 50 行的 balance 快照                                                                                              |
| **2. 无孤立 CSS 值片段**                    | 所有 CSS 文本必须符合 `selector { ... }` 结构。孤立的值片段（完全独立于任何 `{...}` 块之外的 `;` 结尾裸值）一律报错                                                                                                                                    | `audit-css-robustness.ts` 检查 `<style>` 中每个非空行是否可映射到某个规则块                                                                                  |
| **3. 含 `/` 的 Tailwind 类必须转义**        | CSS 选择器中 `/` 必须写为 `\/`。`.top-1/2`、`.-translate-y-1/2`、`.text-muted-foreground/60` 等若未转义，浏览器会整段忽略该规则                                                                                                                        | `audit-css-robustness.ts` 扫描 `.class/...` 未转义的选择器                                                                                                   |
| **4. SVG 图标 `ICONS.xxx` 必须有显式 size** | Module 原型中使用 `dangerouslySetInnerHTML: { __html: ICONS.xxx }` 的容器必须有 `width`/`height` 属性（inline style）或 CSS 类定义显式尺寸。原因：CSS 语法错误可能导致整个 CSS 块静默不加载，此时依赖 CSS 类的尺寸化彻底失效，只剩 inline style 能兜底 | `audit-css-robustness.ts` 检测所有 `ICONS.xxx`→`dangerouslySetInnerHTML` 引用点的容器是否有 inline `style: {width, height}` 或 className 在 CSSOM 中验证存在 |

> **特别注意 — 规则 3 示例**：
>
> ```css
> /* ✅ 正确 */
> .top-1\/2 {
>   top: 50%;
> }
> .-translate-y-1\/2 {
>   transform: translateY(-50%);
> }
> .text-muted-foreground\/60 {
>   color: hsl(var(--muted-foreground) / 0.6);
> }
> .focus\:border-primary\/30:focus {
>   border-color: hsl(var(--primary) / 0.3);
> }
>
> /* ❌ 错误：浏览器会忽略这些选择器 */
> .top-1/2 {
>   top: 50%;
> }
> .-translate-y-1/2 {
>   transform: translateY(-50%);
> }
> ```

#### 9.6.3 实施建议

- CSS 变量声明（`:root { --x: ... }`）建议使用 `width: calc(18 * 1px)` 而非 `width: 18` 配合 JS 数字——但 inline style 用数字更简洁
- 在 `<script>` 末尾添加 CSSOM 健康检查（console 输出 last 5 CSS rules）用于调试
- 构建工具（如 sync-prototype.sh）**必须在交付前**运行 `audit-css-robustness.ts`

#### 9.6.4 反模式示例（禁止）

```css
/* ❌ 错误：孤立的值片段（浏览器无法解析，会跳过后续所有规则） */
.04);
.5;

/* ❌ 错误：`@keyframes` 块内出现非 keyframe 选择器 */
@keyframes pulse {
  0% { opacity: 0.5; }
  .skeleton { ... }  /* 非 keyframe 选择器，IE/Chrome 会直接丢弃整个 @keyframes */
}
```

```jsx
// ❌ 错误：SVG 图标无尺寸兜底（CSS 加载失败时直接撑爆）
h('span', { dangerouslySetInnerHTML: { __html: ICONS.AlertTriangle } });

// ✅ 正确：显式 inline style + className（CSS 兜底 + 调试标识）
h('span', {
  className: 'alert-icon',
  style: {
    width: 18,
    height: 18,
    display: 'inline-flex',
    alignItems: 'center',
    justifyContent: 'center',
    flexShrink: 0,
  },
  dangerouslySetInnerHTML: { __html: ICONS.AlertTriangle },
});
```

#### 9.6.5 跨规约联动

- §9.6 规则 1（CSS 语法）**前置条件**：必须通过 `audit-css-robustness.ts` 后才运行其他可视化审计
- §9 禁止项 #13（CSS 语法错误）提供**完整根因描述**
