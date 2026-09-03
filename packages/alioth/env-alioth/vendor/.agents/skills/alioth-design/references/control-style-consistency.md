# 控件样式一致性（参考）

### 10. 控件样式一致性（MUST）

为保证跨浏览器一致性和窄屏可读性，表单控件和表格内按钮必须遵循以下样式规约。

#### 10.1 `<select>` 控件（dropdown）

原生 `<select>` 在 Safari/Chrome/Firefox 渲染差异显著，**必须**重置：

```css
.select-group select {
  padding: 4px 24px 4px 8px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--card)
    url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10' viewBox='0 0 24 24' fill='none' stroke='%236e6e73' stroke-width='2'><polyline points='6 9 12 15 18 9'/></svg>")
    no-repeat right 8px center;
  cursor: pointer;
  appearance: none;
  -webkit-appearance: none;
  outline: none;
  transition:
    border-color 0.15s,
    box-shadow 0.15s;
}
.select-group select:hover {
  border-color: var(--accent);
}
.select-group select:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px rgba(0, 113, 227, 0.15);
}
```

URL 编码：`%236e6e73` 表示 `#6e6e73`（`--muted-foreground` 颜色）。

变体：

- 表格内嵌 `<select>` 必须显式加 `select-cell` 类，使用更紧凑 padding（`2px 22px 2px 6px`）
- Drawer 表单内 `<select>` 使用 `.drawer-body .form-group select`，宽度 100%、padding `8px 32px 8px 12px`

#### 10.2 表格操作列按钮

**禁止**直接在 `<td>` 中放按钮。HTML 空白字符会导致按钮间距不可控，窄屏下变成异常堆叠。

**必须**包入 `<div className="cell-actions">`：

```jsx
h(
  'td',
  null,
  h(
    'div',
    { className: 'cell-actions' },
    h(
      'button',
      { className: 'btn btn-icon', title: '编辑' },
      h('span', { className: 'icon-inline', dangerouslySetInnerHTML: { __html: ICONS.Edit } }),
    ),
    h(
      'button',
      { className: 'btn btn-icon', title: '删除' },
      h('span', { className: 'icon-inline', dangerouslySetInnerHTML: { __html: ICONS.Trash } }),
    ),
  ),
);
```

> ⚠️ **`display: flex` 禁止直接写在 `<td>` / `<th>` 上**，必须通过内层 `<div>`/`<span>` 间接使用（如 `.cell-actions`）。否则单元格脱离表格布局算法，导致列宽错位（见 §9 禁止项 #13）。

操作列 `<th>` **必须**显式声明宽度（按钮不会自动撑开列宽）：

| 按钮数 | 最小宽度 | 推荐宽度 |
| ------ | -------- | -------- |
| 1 个   | 50px     | 60px     |
| 2 个   | 80px     | 88px     |
| 3 个   | 110px    | 120px    |

```jsx
// ❌ 错误：宽度不够 → 按钮溢出/换行
h('th', { style: { width: 60 } }, '操作');

// ✅ 正确：2 个按钮需要 88px
h('th', { style: { width: 88, textAlign: 'right' } }, '操作');
```

配套 CSS（**必须**全部定义，缺一不可）：

```css
.cell-actions {
  display: inline-flex;
  gap: 4px;
  align-items: center;
  flex-wrap: nowrap; /* 表格 cell 内禁止换行 */
  vertical-align: middle; /* 表格 cell 默认 baseline 对齐，flex 子元素错位 */
  white-space: nowrap; /* 防止 HTML 空白撑开间距 */
}
.btn-icon {
  width: 32px;
  height: 32px;
  padding: 0;
  flex-shrink: 0;
  display: inline-flex; /* 显式声明，不依赖 .btn 继承 */
  align-items: center; /* 显式声明，SVG 居中 */
}
.btn-icon svg,
.btn-icon .icon-inline svg {
  width: 14px;
  height: 14px;
  stroke-width: 2; /* 防止 SVG 被表格 cell 压缩到不可见 */
}
```

#### 10.3 Drawer/Modal 表单输入

`input`/`select`/`textarea` 共享同一 focus 样式：

```css
.drawer-body .form-group input,
.drawer-body .form-group textarea {
  width: 100%;
  padding: 8px 12px;
  border: 1px solid var(--border);
  border-radius: 8px;
  outline: none;
  transition:
    border-color 0.15s,
    box-shadow 0.15s;
}
.drawer-body .form-group input:focus,
.drawer-body .form-group select:focus,
.drawer-body .form-group textarea:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px rgba(0, 113, 227, 0.15);
}
```

#### 10.4 edit 工具坑（CSS + JSX 跨边界修改）

跨 CSS+JSX 边界的**多 hunk `edit` 操作**极易破坏文件结构（Tag 上下文漂移导致替换行错位）。
**遇到以下情况必须用 `write` 重写整个文件，而不是 `edit`：**

- 同一文件需要修改 CSS 和 JSX 两段代码
- 修改涉及 ≥3 处文件位置
- 替换文本跨越多个语法边界（如同时改 `.btn` 类的 CSS + 改 JSX 中的 `.btn-icon` 使用）
- `edit` 出现 "Auto-repaired a delimiter-balance mismatch" 警告（说明 payload 已应用到错误位置）

回退策略：`edit` 报错 → 用 `write` 工具整块重写（优先）或 Edit 工具精确替换 → 验证。

#### 10.5 Babel parse 错误排查

常见症状与根因：

| 错误现象                                           | 根因                                                             | 修复                                     |
| -------------------------------------------------- | ---------------------------------------------------------------- | ---------------------------------------- |
| `Unexpected token, expected ","` 在 `);` 处        | JSX 中 `<td>`/`h(...)` 嵌套未关闭，前一个 `);` 提前终止 `return` | 找出未关闭的 `h('td',...)` 或多余的 `);` |
| `Unexpected token` 在 `}` 后                       | CSS 规则缺失闭合 `}` 或 JSX 嵌套错位                             | 检查花括号配对，js/JSX 缩进              |
| `extract_script_content` 报告 `style 块多余的 '}'` | `edit` 把 CSS + JSX 混在一起替换，CSS 规则被吃掉                 | 用 `write` 工具重建整段被破坏代码        |

#### 10.6 按钮换行防御（三层防护）

按钮在窄视口 / 多按钮容器中容易出现 4 种换行问题：

1. 按钮文字内换行（如"激\n活许可证"）
2. 按钮组内换行（同行 2 个按钮被挤到 2 行）
3. Drawer footer 按钮换行（3 个按钮挤成多行）
4. 按钮被 flex 容器挤压变形（文字溢出）

**必须**三层同时设置，互为补充：

```css
/* 第 1 层：按钮自身 —— 防内换行 + 防压缩 */
.btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  white-space: nowrap; /* 按钮文字不内换行 */
  flex-shrink: 0; /* 不被父容器挤压 */
}

/* 第 2 层：page-header 按钮容器 —— 防按钮组内换行 */
.page-header .actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
  flex-wrap: nowrap; /* 按钮组内不换行（页面窄时整体跳到下一行） */
}

/* 第 3 层：Drawer footer —— 防抽屉底部按钮换行 */
.drawer-footer {
  display: flex;
  gap: 8px;
  justify-content: flex-end;
  flex-wrap: nowrap; /* 抽屉内不换行 */
}
```

**关键模式**：

| 场景                  | 容器                      | 按钮                                                       |
| --------------------- | ------------------------- | ---------------------------------------------------------- |
| 页面顶部（标题+按钮） | `.page-header > .actions` | 1-3 个按钮                                                 |
| Drawer 表单底部       | `.drawer-footer`          | 1-3 个按钮（取消+主操作；环境页例外 3 个：取消+测试+创建） |
| 表格操作列            | `<td> > .cell-actions`    | 1-3 个图标按钮（详见 §10.2）                               |
| 表格工具栏            | `.toolbar-row`            | 搜索 + 过滤器 + 操作按钮                                   |
| KPI 行                | `.kpi-row`                | 仅展示，无按钮                                             |

**Drawer footer 按钮数 ≤ 3 原则**：超过 3 个应改用「主操作 + 下拉菜单」或「步骤条」（多步骤表单）。第 4 个动作（如"保存草稿"）应放 Drawer body 内表单末尾，不在 footer。

#### 10.7 Drawer 内多选项选择（chip-grid）

Drawer 表单中 radio / checkbox 多选项需等宽对齐的场景，**必须**用 `.chip-grid` CSS 类替代 `display: flex` 父容器：

```jsx
// ❌ 错误：flex-wrap 导致最后一行空格 / 文本换行
h('div', {style: {display: 'flex', flexWrap: 'wrap', gap: 6}},
  options.map(function (o) { ... })
)

// ✅ 正确：CSS Grid 等宽自动换行
h('div', {className: 'chip-grid'},
  options.map(function (o) { ... })
)
```

```css
/* chip-grid CSS — 注意特异性必须 ≥ (0,3,1) */
.drawer-body .form-group .chip-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(80px, 1fr));
  gap: 6px;
  margin-top: 4px;
}
.drawer-body .form-group .chip-grid label {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  padding: 6px 10px;
  border: 1px solid var(--border);
  border-radius: 4px;
  font-size: 12px;
  font-weight: 400;
  color: var(--foreground);
  margin-bottom: 0;
  cursor: pointer;
  white-space: nowrap;
  transition:
    border-color 0.15s,
    background 0.15s;
  background: var(--card);
  flex-shrink: 0; /* 防止 grid 列宽 < 内容宽度（强制 checkbox + text 同行） */
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}
/* chip-grid 内 input 必须重置默认 margin（Chrome 默认 input { margin: 2px 0 }） */
.drawer-body .form-group .chip-grid label input[type='checkbox'],
.drawer-body .form-group .chip-grid label input[type='radio'] {
  margin: 0;
  flex-shrink: 0;
}
.chip-grid label:hover {
  border-color: var(--accent);
  background: var(--primary-bg);
}
```

**5 项必填属性（缺一不可）**：

| 属性                                           | 缺失后果                                                 |
| ---------------------------------------------- | -------------------------------------------------------- |
| `flex-shrink: 0`                               | grid 列宽 < 内容 → checkbox 与 text 垂直堆叠（红框 bug） |
| `white-space: nowrap`                          | 长 label 文字换行（如 "物质量" → "物\n质\n量"）          |
| `min-width: 0`                                 | grid 列内 content overflow 计算错误                      |
| `overflow: hidden` + `text-overflow: ellipsis` | 超长 label 撑破 cell 宽度                                |
| `input { margin: 0 }`                          | checkbox 上下 2px margin → 垂直居中错位（红框 bug）      |

**使用判定**：

| 选项数   | 布局模式                 | 原因                                   |
| -------- | ------------------------ | -------------------------------------- |
| ≤ 3      | flex row (eg 通知渠道)   | 数量少，不会不均匀                     |
| 4-8      | `.chip-grid` (auto-fill) | 4 行以内，最后一行 0-4 个，可接受      |
| 9-20     | `.chip-grid` (auto-fill) | 多列等宽，最后一行可能落单（不可避免） |
| 垂直排列 | `flex-direction: column` | 选项含长文本描述时                     |

#### 10.8 CSS 特异性在 Drawer 表单内的陷阱

`.drawer-body .form-group label` 全局规则的特异性为 `(0,3,1)`。
在此容器内创建新 CSS 类时，**选择器特异性必须 ≥ (0,3,1)**，否则被覆盖：

| 自定义类位置                    | 所需特异性  | 推荐写法                                     |
| ------------------------------- | ----------- | -------------------------------------------- |
| 1 层嵌套                        | (0,1,1)     | `.class-name label`                          |
| 2 层嵌套                        | (0,2,1)     | `.form-group .class-name`                    |
| **在 `.form-group` 内**         | **(0,3,1)** | **`.drawer-body .form-group .class-name`**   |
| 在 `.form-group` 内 + `<label>` | (0,3,2)     | `.drawer-body .form-group .class-name label` |

**诊断方法**（发现自定义类不生效时）：

1. 在浏览器 DevTools Elements 面板中检查该元素
2. 查看 **Computed** → 如果某个属性被划掉，说明被更高特异性的规则覆盖
3. 用 `Editable CSS` 临时添加 `!import` 确认问题后，提升选择器特异性

**绝对禁止**用 `!important` 修复特异性问题——加一层选择器嵌套即可。

**未来可能会遇到的问题**：`<input>` / `<select>` / `<textarea>` 在 `drawer-body .form-group` 内也有类似全局规则（特异性 0,3,2），需要自定义时同样要注意。
