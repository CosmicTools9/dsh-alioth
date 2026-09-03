# Prototype Patterns: 迭代中沉淀的 HTML 原型决策

> 本文件收录 `alioth-design` 技能在真实迭代中产生的工程/设计决策，用于避免同类问题在新原型中重复出现。
> 与 `docs/specs/HTML_DESIGN_SPEC.md` 的关系：后者是「全局规约」，本文是「模式补遗」——当规约未覆盖具体场景时，按本文执行。

---

## 1. SVG 图标尺寸必须显式声明

### 决策

所有通过 `dangerouslySetInnerHTML` 注入的 SVG 图标，其外层 span 必须设置 `display: inline-flex` + 固定 `width/height`。

### 背景

HTML 原型使用 Lucide 风格的 SVG 字符串（`viewBox="0 0 24 24"`，不含 `width`/`height` 属性）。若父级 span 没有显式尺寸，SVG 会 collapse 到 `0×0`，按钮看起来是空的（如 structure-v14 的“重置缩放”按钮曾出现空图标）。

### 执行标准

| 场景                                 | 最小 CSS                                                                                         |
| ------------------------------------ | ------------------------------------------------------------------------------------------------ |
| `.btn > span`                        | `display: inline-flex; width: 14px; height: 14px; align-items: center; justify-content: center;` |
| `.btn-sm > span`                     | `width: 14px; height: 14px;`                                                                     |
| `.chart-scale-control .btn-icon svg` | `width: 14px; height: 14px; stroke-width: 2;`                                                    |
| `.gl-trigger > span:first-child`     | `display: inline-flex; width: 16px; height: 16px; align-items: center; justify-content: center;` |

### 禁止

- 仅依赖 `font-size: 14px` 或 `svg { stroke-width: 2 }` 控制图标大小。
- 在图标按钮中不写 `display: flex` / `align-items: center` / `justify-content: center`。

---

## 2. Flex 行内混排徽章/标签时必须 `align-items: center`

### 决策

当一行内同时混排文本、徽章（`.sb`）、图标按钮时，flex 容器必须显式声明 `align-items: center`。

### 背景

默认 `align-items: stretch` 会导致徽章/标签与文字基线不一致，视觉上明显偏下（如 structure-v14 的 chart info card 中“启用”状态徽章偏下）。

### 执行标准

```css
/* 信息卡元数据行 */
.chart-info-card .cic-meta {
  display: flex;
  align-items: center; /* 必须 */
  gap: 12px;
  flex-wrap: wrap;
}
```

### 推广场景

任何 `.meta` / `.subtitle` / `.toolbar` 内同时出现 `.sb`（状态徽章）、`.tag`、`.btn-icon` 时，父容器都必须 `align-items: center`。

---

## 3. 视图切换 Toolbar：状态不可见原则

### 决策

视图切换按钮遵循“状态不可见原则”：当前所在视图的入口按钮不应再显示，只保留切换目标视图的按钮。

### 背景

structure-v14 之前的版本中，树视图 toolbar 同时显示“树视图/架构图”分段按钮，造成当前状态按钮冗余。用户反馈后改为：

- 树视图：只显示“切换到架构图”按钮（带组织数量徽章）。
- 架构图：只显示“切换到树视图”图标按钮。

### 执行标准

| 当前视图 | 应显示的切换入口         | 不应显示的入口 |
| -------- | ------------------------ | -------------- |
| 树视图   | 架构图按钮（带数量徽章） | 树视图按钮     |
| 架构图   | 树视图图标按钮           | 架构图按钮     |

### 附加规则

- 数量徽章只挂在“切换到架构图”按钮上，不单独作为文本展示。
- 架构图视图空间更充裕，可用纯图标按钮返回树视图；树视图空间紧张，保留带文字+徽章的架构图按钮。

---

## 4. 架构图/树形图连线：用 SVG overlay 替代伪元素

### 决策

复杂层级结构的连接线应使用 SVG `<line>` 动态绘制，而非 CSS 伪元素（`::before`/`::after`）。

### 背景

structure-v13 及之前使用 CSS 伪元素绘制竖线和横线，在节点宽度差异大、层级深时出现：

- 横线被节点宽度撑断；
- 多子节点时竖线与横线重叠；
- 缩放/滚动时伪元素定位失效。

structure-v14 改为 SVG overlay，通过 `getBoundingClientRect()` 计算每个父节点与子节点集合的中心点，生成：

1. 父节点底部中心 → 横线中点的竖线；
2. 首子节点中心 → 末子节点中心的横线；
3. 每个子节点顶部 → 横线的竖线；
4. 单个子节点时仅画一条竖线。

### 执行标准

```css
.orgchart-canvas-wrapper {
  position: relative;
}
.orgchart-lines-svg {
  position: absolute;
  top: 0;
  left: 0;
  pointer-events: none;
  z-index: 0;
}
.orgchart-canvas {
  position: relative;
  z-index: 1;
}
```

### 关键实现点

- 使用 `useLayoutEffect` 或 `useEffect` 在 DOM 布局完成后计算坐标。
- 监听 `chartScale` 变化，重新计算 SVG 尺寸。
- SVG `pointer-events: none` 避免遮挡节点点击。
- 单个子节点时避免绘制无意义的横线。

---

## 5. 图标映射对象命名与兜底

### 决策

图标字符串映射统一命名为 `ICONS`，渲染组件统一命名为 `I`，并在 `I` 组件中对缺失图标返回空 span 而非抛出错误。

### 背景

structure-v14 使用：

```jsx
const ICONS = { RotateCcw: '...', FolderTree: '...' };
function I({ name, className }) {
  const svg = ICONS[name] || '';
  return <span className={className || ''} dangerouslySetInnerHTML={{ __html: svg }} />;
}
```

### 执行标准

- 图标名使用 PascalCase（与 Lucide 命名一致）。
- `I` 组件必须支持 `className` prop，以便调用方控制尺寸。
- 缺失图标时不抛错，避免单个图标错误导致整个按钮/页面白屏。

---

## 6. 按钮组/工具栏中“纯图标按钮”必须有 `title`

### 决策

所有纯图标按钮（无文字）必须设置 `title` 属性，提供无障碍提示与悬停说明。

### 背景

structure-v14 的缩放控制按钮（`−`、`+`、`↺`）均依赖 `title` 说明功能。`title` 同时是审计脚本判定按钮用途的依据之一。

### 执行标准

```jsx
<button className="btn-icon" title="重置缩放"><I name="RotateCcw" /></button>
<button className="btn-icon" title="放大">+</button>
```

---

## 7. 版本化原型文件命名与 `module.json` 同步

### 决策

每次原型迭代必须创建新版本文件 `{module}-v{N}.html`，并使用 `bun .agents/skills/alioth-module/scripts/bump-module-version.ts` 原子递增 `Pre-Proc/{namespace}/Sources/Modules/{module}/module.json` 的 `reversion` 段。

### 背景

structure-v14 由 v13 复制而来，版本号同步递增。禁止覆盖已有原型文件，以便保留设计决策历史。

### 执行标准

```bash
bun .agents/skills/alioth-module/scripts/bump-module-version.ts structure
# 生成 structure-v14.html，module.json version 0.1.14
```

---

## 8. 审计脚本新增规则的接入原则

### 决策

当新原型出现可被规约化的缺陷时，必须同时做两件事：

1. 将决策写入 `references/prototype-patterns.md`（本文档）或 `docs/specs/HTML_DESIGN_SPEC.md`。
2. 在 `audit-html-spec.ts` 中增加对应自动检测，使未来同类问题在审计阶段即可被发现。

### 背景

structure-v14 迭代中暴露的“空图标”、“徽章偏下”问题，已通过新增审计规则固化（见 `audit-html-spec.py` 对应检查）。

---

## 9. 甘特图/时间线布局：内容撑开，坐标系统一

### 决策

甘特图/时间线类复杂数据可视化区域必须同时满足两条硬规则：

1. **高度由内容决定**，不得被父 flex 容器在交叉轴上拉伸撑满窗口。
2. **所有 SVG overlay 必须与被覆盖内容区域共享同一坐标系**，viewBox 只能作为内部图形比例尺，不能替代容器尺寸声明。

### 背景

develop-v15 修复前出现两类典型缺陷：

- `.gantt-wrap` 在 `.timeline-layout` 中被父 flex 默认 `align-items: stretch` 拉成窗口剩余高度，导致右侧大片空白与图例堆叠；甘特图整体高度应只由 header + rows + legend 自然高度决定。
- `.gantt-dep-svg` 仅有 `viewBox="0 0 100 100"` 却无明确 `width/height`，浏览器按 1:1 内在比把它撑成 900px 高，而实际 rows 区域只有 448px，依赖线端点全部错位。

### 执行标准

```css
/* 甘特图容器不得被父 flex 拉伸 */
.gantt-wrap {
  flex: 1;
  overflow-x: auto;
  align-self: flex-start; /* 必须 */
}

/* 甘特图主体高度 = header + rows + legend 自然高度 */
.gantt {
  min-width: 900px;
  position: relative;
  padding-bottom: 44px; /* 为固定高图例占位 */
}

/* 图例固定单行高度，禁止块级堆叠撑开容器 */
.gantt-legend {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  height: 44px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 16px;
  white-space: nowrap;
}

/* SVG overlay 必须显式填满内容区域，且被放置在内容坐标系内 */
.gantt-body {
  position: relative;
}
.gantt-dep-svg {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%; /* 必须与 .gantt-body 等高 */
  pointer-events: none;
  overflow: visible;
}
```

### 依赖连线端点规则

- 禁止用 item 整体起止（`item.start` / `item.end`）计算端点；必须取具体 bar 的端点：
  - FS（Finish-to-Start）：源 bar 右端 → 目标 bar 左端。
  - SS（Start-to-Start）：源 bar 左端 → 目标 bar 左端。
- 临时拖动连线（`renderTempDep`）的坐标计算基准必须与 SVG 所在容器一致；若 SVG 在 `.gantt-body` 内，鼠标百分比应基于 `.gantt-body` 的 `getBoundingClientRect()`。

### 禁止

- ❌ `.gantt-wrap` 缺少 `align-self: flex-start;` 导致被 flex 父容器拉伸。
- ❌ `.gantt-dep-svg` 只有 `viewBox` 没有 `width/height: 100%`。
- ❌ SVG overlay 放在 `.gantt` 根节点而坐标却按 rows 区域计算。
- ❌ 依赖线端点用 `item.start` / `item.end` 而非实际 bar 端点。
- ❌ 图例项默认 `display: block` 堆叠，把 legend 撑到 100px+ 高。

---

## 10. SVG overlay 坐标系：容器决定一切

### 决策

任何使用 SVG overlay 绘制连线的组件，必须保证 SVG 的 `width/height` 与 CSS 实际渲染尺寸一致，并且连线坐标计算使用 SVG 所在容器的实际像素，再映射到 viewBox 比例。

### 背景

develop-v15 中 `.gantt-dep-svg` 的 `viewBox="0 0 100 100"` 配合未声明尺寸，导致 SVG 实际渲染高度 900px，而连线百分比按 448px 的 rows 区域计算，所有依赖线终点都发生系统性偏移。

### 执行标准

- 若 SVG 使用百分比 viewBox，必须同时声明 `width: 100%; height: 100%;` 并置于与内容同尺寸的容器内。
- 动态计算连线时，先取容器 `getBoundingClientRect()`，再用 `(elPos - containerPos) / containerSize * viewBoxSize` 映射。
- 临时拖动线的 `onMove` 事件必须使用 SVG 所在容器作为参考，不能用更高层级的 `.gantt` 或 `.page`。

### 推广场景

- 甘特图依赖线、组织架构图连线、流程图/拓扑图连线、树形结构高亮路径等所有 SVG overlay 场景。

---

**版本**：v1.1  
**更新日期**：2026-06-14  
**来源**：develop 模块 v15 甘特图依赖连线与高度撑开修复迭代
