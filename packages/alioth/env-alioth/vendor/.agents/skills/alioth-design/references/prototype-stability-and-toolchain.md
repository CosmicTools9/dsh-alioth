# 原型性能、稳定性与工具链（参考）

## 原型性能与稳定性硬规约

### Always（必做）

1. 原型必须含静态 HTML 加载骨架（`#boot-skeleton`，TopBar/Sidebar 占位 + spinner），React mount 后由 JS 淡出移除，掩盖 Babel 浏览器端编译 1-3s 延迟。
2. 所有 React+JSX 原型在 `<script type="text/babel">` 后第一行必须含 `const { useState, useEffect, useRef, useCallback, Fragment, createElement: h } = React;` 解构声明。
3. `root.render(h(App))` 必须包 try/catch，错误时设置 `document.title` 以便诊断。
4. **CSS `:root` 选择器禁止转义**：`<style>` 块内必须使用 `:root { ... }`，禁止 `\:root`。
5. **CSS 变量定义完整性**：`:root` 块必须定义所有 `var(--X)` 引用的变量。两套命名风格必须同时在 `:root` 中赋值。
6. **React `h()` 参数顺序**：`h(type, props, ...children)` 中第二个参数是 props 对象，禁止将 `{style}` 等对象放在 children 位置。
7. **`dangerouslySetInnerHTML` 隔离**：图标 + 标签组合必须将图标移入子 `<span>` 元素，不得与 text children 共存于同一元素。
8. **布局使用共享 CSS**：新建模块原型使用 `prototype-base.css`（已弃用 `tailwind-utilities.css`），组件样式使用 Tailwind 工具类，与生产代码共用 className。

### Ask（先问）

1. 新增外部字体/库 CDN 引用。
2. 将原型拆分为多个 `<script type="text/babel">` 块。

### Never（禁止）

1. 任何境外 CDN 依赖（Google Fonts / fonts.googleapis / fonts.gstatic / Typekit / Mermaid CDN 等），详见 `HTML_DESIGN_SPEC §1.2`。
2. JSX 大文件原型（>2000 行）不得无加载骨架，首屏白屏 >1s 即视为违规。
3. 用 `file://` 直接打开原型。
4. 丢失 React 解构声明后继续渲染（必失败）。
5. CSS 中使用 `\:root`（反斜杠转义冒号），导致全部 CSS 变量不生效。
6. 在 `h()` 调用中将 props 对象放在 children 位置（如 `h('div', null, {style: …})`）。
7. 同一元素同时使用 `dangerouslySetInnerHTML` 和 text children。
8. LLM eval 中 **Python f-string/`+` 拼接构造 HTML/JS/CSS 片段**。必须用 JS eval (`language: 'js'`) 的 template literal。项目工具禁用 Python（`NO_PYTHON_FOR_PROJECT_TOOLS`），数据处理/验证一律用 Bun + 标准解析器。

### Audit（验证）

1. 每次完成 JSX 编辑后，必须运行离线 Babel 解析测试（`node -e "require('@babel/standalone').transform(...)"`）作为前置验证。
2. `target/debug/ontology-mapping prototype-check` 强制扫描所有原型。
3. 新增原型或大幅修改原型文件后，需手动验证 `target/debug/ontology-mapping prototype-check` 输出 0 错误 0 警告。

## 原型迭代工具链

> `Pre-Proc/{ns}/Prototypes/Modules/{module}-v{N}.html` 在 424 个文件、878K 行规模下迭代。

### 校验+同步

- **`scripts/sync-prototype.sh`**：原型验证通过后同步到 `prototype.html` + 清理临时文件。
  - 用法：`bash scripts/sync-prototype.sh Pre-Proc/{ns}/Prototypes/Modules/{module}/v{N}.html`

### 视觉对比

- **`design-compare.html` + `design-compare-server.ts`**：双 iframe 同模块左右版本对比。
  - 启动：`bun .agents/skills/alioth-design/scripts/design-compare-server.ts`

### 结构化 diff

- **`design-diff.ts`**：输出 JSX 体积、page 函数、路由表、CSS 变量等 7 维度增量。
  - 用法：`bun .agents/skills/alioth-design/scripts/design-diff.ts finance v37 v38`

### 工具组合最佳实践

| 场景                  | 工具链                                                                                              |
| --------------------- | --------------------------------------------------------------------------------------------------- |
| 改完一个原型保存      | OMP hook 自动跑 jsx-balance + standalone                                                            |
| 历史 baseline 校验    | `bump-module-version.ts` + `jsx-balance.ts` + `ontology-mapping prototype-check` + `design-diff.ts` |
| v(N)→v(N+1) 结构 diff | `bun .agents/skills/alioth-design/scripts/design-diff.ts`                                           |

## 核心提醒

- **Track 1 vs Track 2**：Track 1 创建/迭代 HTML 原型，不碰代码。Track 2 以前端代码为产出，以原型为基准。
- **shop 去 alioth-shop**：检测到 `shop` 模块自动跳过，提示使用 `alioth-shop` 技能。
- **技术红线先读**：进入原型构建前必须过一遍「技术红线」——const styles 冲突、跨 script 共享、scrollIntoView 禁止。
- **业务语义第一（Track 1）**：字段名用业务概念（`customerName`），数据从 UI 表达需求出发。
- **尽早 show**：方向错了晚改比早改贵 100 倍。仅新建原型时执行。
- **诚实 placeholder > 烂实现**：没图不画 SVG，没数据不编造。
- **不读实现代码（Track 1）**：禁止加载 `frontend/src/` 和 `backend/`。
- **原子写入 version**：用 tempfile + rename 写 `module.json`，严禁直接写。
