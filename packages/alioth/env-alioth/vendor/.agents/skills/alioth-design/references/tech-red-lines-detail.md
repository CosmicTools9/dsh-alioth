# 技术红线详细说明（参考）

### 1. `const styles = {...}` 必须唯一命名

多个 `const styles = {...}` 在不同 `<script type="text/babel">` 标签中会覆盖。每个文件用唯一前缀：

```jsx
// terminal.jsx
const terminalStyles = { screen: {...}, line: {...} };

// sidebar.jsx
const sidebarStyles = { container: {...}, item: {...} };
```

或用 inline styles（小组件推荐）。

### 2. 跨 `<script>` 组件共享

每个 `<script type="text/babel">` 被 Babel 独立编译，scope 不通。在定义文件末尾显式 export：

```jsx
Object.assign(window, { MyComponent, helperFn });
```

### 3. 禁止 `scrollIntoView`

会破坏容器滚动。改用：

```jsx
container.scrollTop = target.offsetTop;
container.scrollTo({ top: target.offsetTop - 100, behavior: 'smooth' });
```

### 4. 全局别名禁止局部遮蔽

全局声明 `const h = React.createElement;` 后，禁止在函数内再声明 `const h = ...` 遮蔽。SVG/Canvas 尺寸优先用 `svgW`/`svgH`、`chartW`/`chartH`。

### 5. 页面根容器占满宽度 / 防止 flex 项被压扁

`.page` 必须 `width: 100%`；`.card-list`/`.card` 必须 `width: 100%` 且 `min-width: 0`；`.page-header` 用 `align-items: flex-start`，左侧标题区 `min-width: 0`，右侧 actions `flex-shrink: 0`。避免页面内容被挤压到左侧一小条。

### 6. Block 顶层必须 `.page` 包裹（组装上下文）

组装原型（`v{N}.html`）中，每个 scene 页面组件的顶层 return **禁止使用 `Fragment`**，必须包裹 `e('div', { className: 'page' }, ...)`。

```jsx
// ✅ 正确
function PayablePage() {
  return e('div', { className: 'page' },
    e('div', { className: 'page-header' }, ...),
    e('div', { className: 'kpi-row' }, ...),
    ...
  );
}

// ❌ 错误 — 无 padding，内容紧贴 Sidebar
function PayablePage() {
  return e(Fragment, null,
    e('div', { className: 'page-header' }, ...),
    ...
  );
}
```

`.page` 的 `padding: 20px 24px` 是 Sidebar + 内容区的标准间距，各 scene 一致使用。
在独立 Scene 原型（`Pre-Proc/{ns}/Prototypes/Blocks/*/v{N}.html`）中每个 Scene 已有自己的 `.page` 包裹，但组装时独立 Scene 不再直接渲染——scene 组件作为嵌入单元需要保持 `.page` 间距。

### 6. React 固定版本（优先本地 vendor）

原型使用本地 vendor 文件（`.agents/skills/alioth-design/references/vendor/` 共享目录），见项目约束 §7。引用方式：

```html
<!-- 同层模块引用（例如 Pre-Proc/{ns}/Prototypes/Modules/{name}/v{N}.html） -->
<script src="../../vendor/react.umd.js"></script>
<script src="../../vendor/react-dom.umd.js"></script>
<script src="../../vendor/react-dom-client.umd.js"></script>
<script src="../../vendor/react-bridge.js"></script>
<script src="../../vendor/babel.min.js"></script>
```

**CDN 回退**（本地 vendor 不可用时）：

```html
<!-- 默认：jsDelivr（中国大陆推荐，国内有 CDN 节点） -->
<script src="https://cdn.jsdelivr.net/npm/react-umd@19.2.7/dist/react.umd.js"></script>
<script src="https://cdn.jsdelivr.net/npm/react-umd@19.2.7/dist/react-dom.umd.js"></script>
<script src="https://cdn.jsdelivr.net/npm/react-umd@19.2.7/dist/react-dom-client.umd.js"></script>
<script src="../../vendor/react-bridge.js"></script>
<script src="https://cdn.jsdelivr.net/npm/@babel/standalone@8.0.3/babel.min.js"></script>

> React 19 移除了官方 UMD 构建，故使用 `react-umd` 包装包。CDN 回退也使用 `react-umd` 而非官方
`react/umd/` 路径。 > **CDN 选择依据**：团队主要在中国大陆开发时，默认使用 jsDelivr（国内 CDN
节点）。海外开发或 jsDelivr 不可达时回退到 unpkg。
```

### 7. React 版本

用 `production.min.js`（原型不调试，追求渲染速度）。`development.js` 仅用于早期调试。

### 8. `file://` 安全起源限制与验证方法

`ontology-mapping prototype-check` 已拦住所有违规。通过后 `file://` 协议完全可用。需要 HTTP server 说明有未拦截请求——退回修复，见「禁止项」完整清单。

**验证方法**：在原型 HTML 中搜索以下关键词确认无出现：

```bash
grep -n 'iframe\|fetch(\|XMLHttpRequest\|srcdoc\|document.write\|file://\|window\.open' Pre-Proc/{ns}/Prototypes/Modules/{name}/v{version}.html
```

> 若仅出现于 Babel standalone 内部实现，可容忍。若出现在 `<script type="text/babel">` 内，必须删除。

Chrome 将 `file://` 视为唯一安全起源。以下在 `file://` 下触发安全错误但仍通过 grep（跨域载入被执行前被 CSP 拦截）：

- ❌ `<iframe src="...">` 加载本地 `file://` 资源
- ❌ `fetch("file:///...")` 或 `XMLHttpRequest` 请求本地文件
- ❌ `window.open("file:///...")`

---

### 7. 已知 Console 消息（无害 vs 需修复）

原型在浏览器 Console 面板中可能出现以下消息。**不是所有 console 输出都代表问题。**

| 消息                                                                       | 来源                     | 判定                         | 处理                                                                                                                                                                                                                                                                    |
| -------------------------------------------------------------------------- | ------------------------ | ---------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `You are using the in-browser Babel transformer. Be sure to precompile...` | Babel standalone         | ✅ 无害                      | Babel 的预期行为，仅出现在开发原型中                                                                                                                                                                                                                                    |
| `Unsafe attempt to load URL file://... from frame with URL file://...`     | Chrome 安全策略          | ✅ 无害                      | 仅出现在 `file://` 直接打开时；若页面渲染正常（React 组件可见），忽略                                                                                                                                                                                                   |
| `Failed to load resource: net::ERR_CONNECTION_TIMED_OUT` (css2:1)          | Google Fonts CDN 超时    | 🔴 视为违规                  | 自 2026-06-14 起，原型禁止引用 Google Fonts（详见 `MODULE_SPEC §10.0.1`）。所有原型必须使用 `.agents/skills/alioth-design/references/vendor/fonts/inter.css` + `.agents/skills/alioth-design/references/vendor/fonts/jetbrains-mono.css` 本地引用。审计脚本会报 error。 |
| `Failed to load resource: net::ERR_FAILED` (jsDelivr/unpkg CDN)            | React/Babel CDN 回退超时 | 🔴 仅在 CDN 回退路径时需修复 | 本地 vendor 文件不受影响。若用了 CDN 回退且失败，检查网络或切换 CDN 源                                                                                                                                                                                                  |
| `Uncaught SyntaxError: Unexpected token '<'`                               | CDN 回退脚本返回了 HTML  | 🔴 仅在 CDN 回退路径时需修复 | 网络代理返回了登录页面。改用本地 vendor 即可绕过                                                                                                                                                                                                                        |

**预检方法** — 打开原型前先验证本地 vendor 文件存在：

```bash
# 检查本地 vendor 文件完整性（所有原型必须引用这些文件）
ls -l .agents/skills/alioth-design/references/vendor/{react.umd.js,react-dom.umd.js,react-dom-client.umd.js,react-bridge.js}
```

如在原型中使用 CDN 回退路径，则需检查 CDN 可达性：

```bash
# 仅 CDN 回退时检查
curl -sI --connect-timeout 5 https://cdn.jsdelivr.net/npm/react-umd@19.2.7/dist/react.umd.js 2>&1 | head -1
curl -sI --connect-timeout 5 "https://cdn.jsdelivr.net/npm/@fontsource/plus-jakarta-sans@5.2.5/index.css" 2>&1 | head -1
```

- 本地 vendor 缺失 → 执行 `curl` 下载或临时切 CDN 回退
- Google Fonts 不可达 → 已知限制，忽略（系统字体 fallback）
