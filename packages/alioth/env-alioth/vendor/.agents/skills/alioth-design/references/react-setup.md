# React 原型（ESM 链路）规范

用 HTML+React 做原型时必须遵守的技术规范。不遵守会炸。

## ESM 链路总览（唯一构建路线）

> 双管线原型统一走 ESM 链路，**禁止** CDN babel（`@babel/standalone`、`text/babel`、`babel.min.js`）教学或调用示例。
> 源文件是 `llm-tsx/*.tsx`，产物由 `bun scripts/prototype-tool.js build` 构建。

```
llm-tsx/app.tsx ─┐
llm-tsx/module.tsx ─┤──> bun scripts/prototype-tool.js build <tsx 路径> ──> a-v{N}.html / m-v{N}.html / b-v{N}.html + .bundle.js
llm-tsx/block.tsx ─┘
```

- **源文件**：`Pre-Proc/{ns}/Prototypes/{Apps|Modules|Blocks}/{name}/llm-tsx/{app|module|block}.tsx`，`export default function` 组件。
- **构建**：`bun scripts/prototype-tool.js build <llm-tsx 文件路径>`（block 构建命令见 alioth-block SKILL；module/app 构建命令见 alioth-module/alioth-app SKILL）。
- **产物**：同目录 `v{N}.html` + `v{N}.bundle.js`（esbuild IIFE bundle）。build 自动算版本号、级联升版（block → module → app）。
- **Mock 数据**：放 `llm-tsx/mock.json`，组件内 `import MOCK from './mock.json'`。禁止内联 `const MOCK_xxx`（block-renderable-prototype 规约）。
- **共享壳**：TopBar / Sidebar / Footer / WorkspaceDock 由 `gateway-shell`（`alioth-design/references/gateway-shell`）提供，模板渲染时经单点常量注入 import 路径，不要手写壳 CSS。
- **运行时**：`createPrototypeLifecycle`（`_shared/lifecycle`）负责挂载；App 级原型同时挂 `window.AppLayout` 供集成探测。

HTML 产物是构建副产品，**不要手改**；迭代 = 改 `llm-tsx/*.tsx` → 重新 build → 过门禁（视觉验证 ≥90 分等）。

## 文件结构

```
llm-tsx/
├── app.tsx                # App 级入口组件（export default function AppLayout）
├── module.tsx             # Module 级入口组件（export default function ModuleLayout）
├── block.tsx              # Block 级入口组件（export default function Block）
└── mock.json              # Mock 数据（实体 10-20 条记录，业务概念英文驼峰命名）
```

跨文件共享用标准 ESM：`import` / `export`。esbuild 把整个 import 图打包进一个 IIFE bundle，**不存在** babel 时代的「多 script 各自编译、scope 不通」问题——但正因 bundle 共享同一作用域，命名冲突比 babel 时代更直接（见规矩 1）。

## 五条不可违反的规矩

### 规矩1：styles 对象必须用唯一命名

bundle 内所有模块共享同一作用域，`const styles` 同名会在 import 汇合时互相覆盖。

**错误**（多组件时必炸）：

```tsx
// components.tsx
const styles = { button: {...}, card: {...} };

// pages.tsx  ← 同名覆盖！
const styles = { container: {...}, header: {...} };
```

**正确**：每个组件文件的 styles 用唯一前缀，或用 inline styles（小组件推荐）：

```tsx
// terminal.tsx
const terminalStyles = { screen: {...}, line: {...} };
```

### 规矩2：跨文件共享用 import/export

```tsx
// components.tsx
export function Terminal(props) { ... }
export const colors = { green: '#...', red: '#...' };
```

```tsx
// pages.tsx
import { Terminal, colors } from './components';
```

### 规矩3：禁止 `scrollIntoView`

`scrollIntoView` 会把整个 HTML 容器往上推，搞坏 web harness 的布局。**永远不要用**。

### 规矩4：禁止同名局部遮蔽

组件内局部变量不要遮蔽外层约定别名/导入名（如 `h`、`cx`、`cn`）。同名遮蔽会让调用变成 `number(...)` 或返回 `NaN`。

```tsx
// ❌ 遮蔽
import { h } from './helpers';
function Chart() { const h = 180; return h('svg', ...); }

// ✅ 带前缀命名
function Chart() { const svgH = 180; return h('svg', ...); }
```

### 规矩5：页面根容器必须占满可用宽度且防止 flex 项被压扁

原型使用 flex 布局时，若内容区（`.page`）直接放在 `flex:1` 的容器内，必须显式声明 `width: 100%`（或 `min-width: 0`），否则子 flex 项会被父容器压缩到极小宽度，出现「页面内容挤到左侧一小条」的布局错位。

**错误**：

```css
.page {
  padding: 20px 24px;
}
.card-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.card {
  display: flex;
  flex-direction: column;
}
```

**正确**：

```css
.page {
  padding: 20px 24px;
  width: 100%;
}
.card-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
  width: 100%;
}
.card {
  display: flex;
  flex-direction: column;
  width: 100%;
  min-width: 0;
}
```

页面 header 若采用 `display: flex; align-items: center;`，左侧标题会挤压右侧 actions，导致按钮折行或错位。应使用：

```css
.page-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  flex-wrap: wrap;
}
.page-header > div:first-child {
  min-width: 0;
}
.page-header-actions {
  flex-shrink: 0;
}
```

**硬性要求**：

- 每个页面根容器（通常是 `.page`）必须声明 `width: 100%`。
- 卡片/表格/列表容器必须声明 `width: 100%`。
- 在 flex 行中的内容项必须声明 `min-width: 0`，允许内部文本正常折行/省略。
- `page-header` 左侧标题区必须 `min-width: 0`，右侧 actions 必须 `flex-shrink: 0`。
- 验证脚本必须检查 `.page { width: ... }` 和 `.card-list/.card { width: ... }` 是否存在。

## 典型 TSX 起手骨架

```tsx
import { useState } from 'react';
import {
  GatewayShell,
  type ModuleTab,
} from '../../../../../../.agents/skills/alioth-design/references/gateway-shell';
import { createPrototypeLifecycle } from '../../../_shared/lifecycle';

function App() {
  const [count, setCount] = useState(0);
  return (
    <div style={{ padding: 40, width: '100%' }}>
      <h1>Hello</h1>
      <button onClick={() => setCount((c) => c + 1)}>count: {count}</button>
    </div>
  );
}

window.AppLayout = App;
export default AppLayout;
export const { bootstrap, mount, unmount } = createPrototypeLifecycle({
  name: 'my-app',
  App: AppLayout,
});
```

> import 的 `../` 深度以产物相对项目根的层级为准（App 级 6 级、Sources 侧 5 级），由模板渲染器经单点常量注入，手工新建时参考同目录既有 llm-tsx 文件。

## 常见报错及解决

**esbuild build 失败：`Could not resolve '.../gateway-shell'`**
→ import 相对路径深度不对（少/多 `../`）。参考同目录既有 llm-tsx 的 import 写法。

**esbuild build 失败：`Symbol already declared`**
→ 组件名/变量名与 bundle 内其他模块冲突。改名（避免 `ModuleLayout`、`AppLayout` 等通用名重复声明）。

**esbuild build 失败：`Expected "(" but found "-"`**
→ 模块/组件名含连字符拼进了标识符。用驼峰/下划线归一化后再拼名字。

**整个页面白屏，控制台没错误**
→ 多半是 JSX 语法错误被 esbuild 吞掉或组件未默认导出。先看 build 输出，再确认 `export default function` 存在（gate 检查项）。

**`Objects are not valid as a React child`**
→ 你渲染了一个对象而不是 JSX/字符串。通常是 `{someObj}` 写成了 `{someObj.name}`。

## 大项目怎么拆文件

**>1000行的单文件**难维护。分拆思路（全部走 import 组合，不靠 script 顺序）：

```
llm-tsx/
├── primitives.tsx      # 基础元素：Button、Card、Badge...
├── components.tsx      # 业务组件：UserCard、PostList...
├── pages/
│   ├── home.tsx        # 首页
│   ├── detail.tsx      # 详情页
│   └── settings.tsx    # 设置页
├── router.tsx          # 简单路由（React state 切换）
├── app.tsx             # 入口组件（import 以上所有）
└── mock.json           # mock 数据
```

## LLM demo 数据怎么来

- **默认：mock.json**。`llm-tsx/mock.json` 每实体 10-20 条记录，组件 `import MOCK from './mock.json'`。demo 场景推荐，零网络、可离线。
- **真调 LLM**：浏览器直调外部 LLM API 会遇 CORS，且 API key 不应硬编码进 HTML。需要真调时，在 agent 会话侧用 LLM 能力生成 mock 响应数据后写入 `mock.json`；确需运行时真调则必须走项目自己的 proxy 后端。
