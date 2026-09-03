# Visual Verification Protocol — 结构化截图评分框架

> 本协议为 AliothStudio 所有 Track 2 前端校准提供统一的截图评分标准。
> 各技能（alioth-design / alioth-block）按上下文适配参考源、评分权重和截图清单。

## 1. 核心工作流

```
0. 生成截图计划: scripts/visual-verify.ts → 截图清单
1. 截取原型 baseline + 实现截图（按场景帧清单）
2. 像素叠加对比: scripts/visual-verify.ts --regions → 区域加权评分
3. CSS 变量对比: scripts/visual-verify.ts → 变量值一致性检查
4. 按 6 维度评分（0-100）+ 人工修正
5. 加权总分 → PASS(90+) / REVISE(60-89) / FAIL(<60)
6. REVISE/FAIL: 输出具体 mismatch + 区域/变量修复建议
7. 修复 → 重新截图评分 → 重复直到 PASS（最多 5 轮）
8. PASS: Track 2 完成
```

## 2. 评分维度与权重

| 维度               | 权重 | 评分区间 | 说明                                               |
| ------------------ | ---- | -------- | -------------------------------------------------- |
| Layout 布局        | 0.25 | 0-100    | 元素位置、间距、对齐精度                           |
| Typography 字体    | 0.15 | 0-100    | font-family/size/weight/line-height/letter-spacing |
| Color 颜色         | 0.15 | 0-100    | 背景/文字/边框/阴影色（Delta E 色差）              |
| Responsive 响应式  | 0.10 | 0-100    | 多断点渲染（375/768/1440）                         |
| Interactive 交互态 | 0.15 | 0-100    | hover/focus/active/disabled/loading                |
| Content 内容完整性 | 0.20 | 0-100    | 文字、图标、标签、数据齐全                         |

**加权总分公式**：

```
total = layout * 0.25 + typography * 0.15 + color * 0.15
      + responsive * 0.10 + interactive * 0.15 + content * 0.20
```

## 3. 评分细则

### Layout 布局（0-100）

| 分档   | 条件                                       |
| ------ | ------------------------------------------ |
| 90-100 | 所有元素位置精确、间距匹配原型 ≤2px        |
| 75-89  | 少数间距偏差 4-8px，整体对齐正确           |
| 50-74  | 明显错位 8-20px，列宽/行高不一致           |
| 0-49   | 布局严重错误（列顺序颠倒、元素缺失、溢出） |

### Typography 字体（0-100）

通过 `window.getComputedStyle()` 提取字体属性对比，非像素估算：

```javascript
const el = document.querySelector(target);
const s = window.getComputedStyle(el);
// 对比: fontFamily, fontSize, fontWeight, lineHeight, letterSpacing
```

| 分档   | 条件                                 |
| ------ | ------------------------------------ |
| 90-100 | 全部 5 项属性匹配                    |
| 70-89  | 1 项偏离（如 fontWeight 400 vs 700） |
| 50-69  | 2 项偏离                             |
| 0-49   | 错误字族（fontFamily 完全不同）      |

### Color 颜色（0-100）

色差用 Delta E 感知色差评估：

```javascript
// 提取 computed 颜色值
const s = window.getComputedStyle(el);
// 对比: color, backgroundColor, borderColor, boxShadow
```

| 分档   | 条件                         |
| ------ | ---------------------------- |
| 90-100 | Delta E < 2（人眼不可感知）  |
| 65-89  | Delta E 2-5（轻微可感）      |
| 40-64  | Delta E 5-10（明显差异）     |
| 0-39   | Delta E > 10 或暗/亮模式反转 |

### Responsive 响应式（0-100）

测试 3 个断点，每断点评分后取平均：

| 设备    | 宽度   | 目标            |
| ------- | ------ | --------------- |
| Mobile  | 375px  | iPhone SE       |
| Tablet  | 768px  | iPad portrait   |
| Desktop | 1440px | Standard laptop |

| 分档   | 条件                           |
| ------ | ------------------------------ |
| 90-100 | 3 个断点全部通过               |
| 65-89  | 1 个断点布局偏移               |
| 40-64  | 2 个断点出问题                 |
| 0-39   | 完全不响应或某个断点内容不可读 |

### Interactive 交互态（0-100）

| 分档   | 条件                                         |
| ------ | -------------------------------------------- |
| 90-100 | 全部实现 hover/focus/active/disabled/loading |
| 70-89  | 缺少 loading 或 disabled 态                  |
| 50-69  | 缺少 hover 或 focus                          |
| 0-49   | 无交互态                                     |

### Content 内容完整性（0-100）

| 分档   | 条件                               |
| ------ | ---------------------------------- |
| 90-100 | 全部文字、图标、标签、字段数据存在 |
| 75-89  | 非关键元素缺失（辅助信息）         |
| 50-74  | 关键元素缺失（CTA、标题、主数据）  |
| 0-49   | 多项缺失或完全不对应               |

## 4. 判定阈值

| 终裁       | 分数   | 含义     | 动作                                            |
| ---------- | ------ | -------- | ----------------------------------------------- |
| **PASS**   | 90-100 | 可交付   | 进入最终验证，无需修复                          |
| **REVISE** | 60-89  | 需要修复 | 输出 mismatch 表 + 修复建议，修复后重新评分     |
| **FAIL**   | <60    | 重大返工 | 输出全部 mismatch，必须修复到 REVISE 以上再迭代 |

## 5. 迭代限制

最多 5 轮评分迭代。5 轮后仍未 PASS：

```
ESCALATION: 视觉验证失败（5 轮迭代未通过）
组件: [name]
最佳评分: [score]
阻塞项: [反复失败的具体维度]
建议: [需要设计师介入 / 实现方案需要调整]
```

## 6. 截图清单（基线帧）

Track 2 开始前，先截取原型的关键状态作为 baseline：

| 帧  | 状态                          | 关注维度                     |
| --- | ----------------------------- | ---------------------------- |
| 1   | 列表页/首页（空数据加载完成） | Layout, Content              |
| 2   | 列表页/首页（有数据）         | Layout, Content, Color       |
| 3   | 搜索/筛选态                   | Layout, Responsive           |
| 4   | 详情/Drawer                   | Layout, Content, Typography  |
| 5   | 创建/编辑表单                 | Layout, Interactive, Content |
| 6   | 确认/删除弹窗                 | Layout, Color                |
| 7   | 空态/错误态/权限不足          | Content                      |

每帧保存为 `<SESSION_DIR>/baseline-{N}.png`，实现截图保存为 `<SESSION_DIR>/current-{N}.png`。

## 7. 常见视觉 bug 清单

比对时重点检查以下高频问题：

| 分类      | 具体问题                                     | 排查方向                         |
| --------- | -------------------------------------------- | -------------------------------- |
| 字体      | fontWeight 400 vs 700（bold 未生效）         | CSS font-weight 类               |
| 断点      | Tailwind 中 `md:` vs `lg:` 混淆              | 响应式断点配置                   |
| 交互      | hover 态未实现                               | CSS `:hover` / Tailwind `hover:` |
| 图标      | 尺寸不对（16px vs 20px vs 24px）             | SVG `width/height`               |
| 颜色      | CSS 变量定义了但未在 theme 中赋值            | `:root` 变量完整性               |
| 间距      | `px-3` vs `px-4`（12px vs 16px）             | Tailwind padding 类              |
| 层级      | z-index 遮挡交互元素                         | `z-*` stack context              |
| 圆角      | `rounded` vs `rounded-lg` vs `rounded-full`  | Tailwind radius 类               |
| 文本溢出  | `line-clamp` 未实现                          | CSS text-overflow                |
| 标量引用  | `_refs.qk_xxx` 显示 raw ID 而非解析值        | refs resolver                    |
| 空态      | 列表无数据时无占位符                         | empty state 组件                 |
| 图标+文字 | `dangerouslySetInnerHTML` 与 text child 冲突 | HTML_DESIGN_SPEC §1.3.5          |

## 8. 浏览器控制方式

使用 **ego-browser** 作为默认截图后端，控制真实浏览器。

### 8.1 健康检查

```bash
ego-browser --version
# 期望: running: true, extension_connected: true
```

### 8.2 截图命令

使用 `bun scripts/visual-verify.ts` CLI（推荐），不推荐手动 curl。ego-browser 命令示例：

```bash
bun scripts/visual-verify.ts capture <url> --out <output-dir>
bun scripts/visual-verify.ts close
```

详细操作见 `skill://ego-browser/SKILL.md`。

截图中必须使用 **ego-browser** 连接真实浏览器截图（复用登录态）。

> **工具统一**：所有视觉验证统一使用 ego-browser。

### 8.3 Gateway 认证处理（find_tab 优先）

Gateway 使用 SSO（port 9002）统一认证。推荐优先通过 `find_tab` 复用用户已登录的浏览器标签页，避免出现 SSO 登录页。

```bash
# 提取 domain（如 localhost:41717）
DEV_DOMAIN=$(echo "{dev-server-url}" | sed 's|https\?://||' | cut -d/ -f1)

# find_tab 按 domain 匹配已登录标签页
FIND_RESULT=$(ego-browser find-tab --url "$DEV_DOMAIN" --session visual-verify 2>/dev/null)

needNewTab=false
if echo "$FIND_RESULT" | grep -q '"success":false'; then
  echo "[EGO] No existing Gateway tab — opening new tab (SSO login may appear)"
  needNewTab=true
else
  echo "[EGO] Found existing Gateway tab — reusing session"
fi

ego-browser navigate --url "{dev-server-url}/{path}" --new-tab "$needNewTab" --session visual-verify
sleep 5

# 若出现 SSO 登录页，说明用户浏览器中无已登录的 Gateway 标签页。
# 关闭该标签页，先在浏览器中手动登录 Gateway，再重新执行验证。
```

### 8.4 Tab 生命周期（HARD RULE — 先查再建，用完即关）

ego-browser 每次操作必须遵循以下完整生命周期，禁止随意开标签页不关闭。

#### 8.4.1 规则

1. **先查已存在 tab**：对 http 链接优先调用 `find_tab`，不直接 `navigate { newTab: true }`。
2. **没有才新建**：`find_tab` 返回失败时才走 `navigate { newTab: true }`。
3. **用完后关 tab**：
   - 若本次是新建的 tab（`newTab=true`），操作完成后**必须** `close_tab`。
   - 若复用了已有 tab，`close_tab` 跳过（不关闭用户原有的页面）。
4. **整个 session 结束时**：在任何 yield 之前调用 `close_session` 关闭所有残留标签页。

#### 8.4.2 伪代码

```bash
# Step 1: 查已有 tab
find_result=$(ego-browser find-tab --url "$domain" --session "$session")
if echo "$find_result" | grep -q '"success":true'; then
  needNewTab=false
else
  needNewTab=true
fi

# Step 2: 导航
ego-browser navigate --url "$URL" --new-tab "$needNewTab" --session "$session"

# Step 3: 操作（截图/点击/读取...）
# ...

# Step 4: 用完关（仅本次新建的 tab）
if [ "$needNewTab" = true ]; then
  ego-browser close-tab --session "$session"
fi
```

#### 8.4.3 工具函数

项目 `bun scripts/visual-verify.ts capture <url> --out <dir>` 已实现此生命周期。Agent 直接使用该 CLI 即可，无需手动 curl 或 source shell 函数。

```bash
# 推荐：全自动流水线
bun scripts/visual-verify.ts verify <prototype.html>

# 或手工捕获
bun scripts/visual-verify.ts capture <url> --out <dir> --close
```

手动调用 ego-browser API 时，必须自行按照上述伪代码执行。

## 9. 验证门禁（HARD GATE）

```
视觉验证未通过（PASS < 90 或迭代 > 5 轮）= Track 2 未完成，禁止交付。
```

- 所有 mismatch 必须录入修复清单
- P0（内容缺失/功能不可达）必须先修，不得跳过
- 修复后重新截图评分，直到 PASS 或用户明确确认接受 REVISE 级

---

## 10. 自动化工具

### 10.1 场景驱动截图计划（scripts/visual-verify.ts）

取代固定 7 帧，改为按 Module/Block 分别定义截图帧，确保不同执行者截取相同状态：

```bash
# 生成模块级截图计划
bun scripts/visual-verify.ts \
  Pre-Proc/{namespace}/Sources/Modules/{name}/module.json \
  --output scenarios.json

# 生成单 Scene 截图计划
bun scripts/visual-verify.ts \
  --scene Pre-Proc/{namespace}/Sources/Blocks/{id}/block.json
```

每帧标注 `P0/P1/P2` 优先级和对应的评估维度。P0 帧为必截图，P1/P2 根据场景可用性选取。

### 10.2 区域加权像素叠加对比（scripts/visual-verify.ts --regions）

按 UI 区域（Sidebar/Content/Footer）分别评估像素偏差，避免整体高分掩盖局部问题：

```bash
# 使用默认区域权重
bun scripts/visual-verify.ts baseline.png current.png --regions

# 自定义区域
bun scripts/visual-verify.ts baseline.png current.png \
  --regions "sidebar:0,0,0.17,1:0.1 content:0.17,0.065,0.83,0.87:0.7"
```

默认区域权重：

| 区域    | 占比             | 权重     | 说明                   |
| ------- | ---------------- | -------- | ---------------------- |
| Sidebar | 17% 宽 × 全高    | 0.10     | 导航菜单区域           |
| TopBar  | 83% 宽 × 6.5% 高 | 0.05     | 顶栏                   |
| Content | 83% 宽 × 87% 高  | **0.70** | 主要内容区（最高权重） |
| Footer  | 83% 宽 × 6.5% 高 | 0.05     | 底栏                   |
| Chrome  | 全屏             | 0.10     | 其他边框/留白          |

退出码基于`区域加权总分`（≥95 通过）。

### 10.3 CSS 变量一致性检查（scripts/visual-verify.ts）

从原型提取 CSS 变量声明值，通过 ego-browser 获取实现页面 computed 值，逐项对比：

```bash
# 提取原型变量（离线检查用）
bun scripts/visual-verify.ts \
  Pre-Proc/{namespace}/Prototypes/Modules/{name}/v{N}.html \
  --extract-only

# 在线对比
bun scripts/visual-verify.ts \
  Pre-Proc/{namespace}/Prototypes/Modules/{name}/v{N}.html \
  --impl-url http://localhost:41717/{module}
```

检查范围：`--primary`、`--background`、`--foreground`、`--border`、`--radius` 等 30+ 设计 Token。
HSL 值按分量加权对比（H 权重 0.1，S/L 权重 0.45），Delta E < 2 视为匹配。
匹配率 ≥ 95% 通过。

### 10.4 元素级布局比对（scripts/visual-verify.ts）

传统像素叠加对比只能告诉你"有差异"，无法告诉你"哪个元素偏了"。元素级布局比对通过 ego-browser 直接从浏览器提取每个 DOM 元素的 `getBoundingClientRect()`，逐元素对比位置和尺寸：

```bash
# Step 1: 从原型提取元素布局
bun scripts/visual-verify.ts extract \
  --url file:///path/to/v{N}.html \
  --output proto-layout.json

# Step 2: 从实现提取元素布局
bun scripts/visual-verify.ts extract \
  --url http://localhost:41717/{module} \
  --output impl-layout.json

# Step 3: 逐元素对比
bun scripts/visual-verify.ts compare \
  --expected proto-layout.json \
  --actual impl-layout.json
```

默认检查 14 个关键布局元素（适配 Tailwind-only 输出，选择器按实际 DOM 结构调整）：

| 元素         | 选择器（示例）                                          | 说明       |
| ------------ | ------------------------------------------------------- | ---------- |
| sidebar      | `#root div[class*="bg-secondary"]`, `#root nav`         | 侧栏容器   |
| topbar       | `#root header`, `#root div[class*="h-14"]`              | 顶栏       |
| content-area | `#root main`, `#root div[class*="overflow-y-auto"]`     | 内容区域   |
| main-content | `#root div[class*="min-h-0"][class*="overflow-y-auto"]` | 滚动内容区 |
| nav-items    | `#root nav button`, `#root button[class*="rounded-lg"]` | 导航项     |
| table        | `table`                                                 | 数据表格   |
| form-input   | `input, select`                                         | 表单输入   |
| modal        | `.drawer-overlay`                                       | 弹窗遮罩   |
| empty-state  | `.empty-state`                                          | 空态占位   |

> 注意：Tailwind-only 输出不包含 `gl-*` 类名。上表选择器为指导性示例，实际使用时应以 `--selectors` 自定义配置覆盖。

支持自定义选择器配置（`--selectors my-config.json`），格式：

```json
{ "sidebar": ".gl-sidebar", "custom-table": ".data-table" }
```

每元素提取以下属性：`x, y, width, height, top, left, right, bottom` + 计算样式（margin, padding, fontSize, borderRadius）。
默认偏差阈值 ±4px，超过 ±8px 标记 HIGH 严重。

### 10.5 浮层/表单校准（scripts/visual-verify.ts）

弹窗（Dialog）、抽屉（Drawer）、表单（Form）具有动态交互特性，截静态图无法覆盖。本工具通过 ego-browser 自动触发交互（点击按钮打开弹窗），然后检查：

| 检查项        | 验证逻辑                                                          |
| ------------- | ----------------------------------------------------------------- |
| **遮罩层**    | `.drawer-overlay` 必须 `position: fixed`、覆盖视口、有背景色/模糊 |
| **抽屉定位**  | 右侧滑入（`right` 接近 0）、有阴影、可滚动                        |
| **弹窗居中**  | 水平居中（偏差 < 50px）、有阴影、z-index 高于遮罩                 |
| **表单字段**  | 字段数一致、有 label、输入框高度/内边距一致                       |
| **body 滚动** | 弹窗打开时 `overflow: hidden`（滚动锁定）                         |

```bash
# 使用预置动作配置文件捕获
bun scripts/visual-verify.ts capture \
  --url file:///path/to/v{N}.html \
  --actions .agents/skills/alioth-design/samples/overlay-actions.json \
  --output proto-overlays.json

# 同样方式捕获实现
bun scripts/visual-verify.ts capture \
  --url http://localhost:41717/{module} \
  --actions .agents/skills/alioth-design/samples/overlay-actions.json \
  --output impl-overlays.json

# 对比
bun scripts/visual-verify.ts compare \
  --expected proto-overlays.json \
  --actual impl-overlays.json
```

支持自定义动作配置，覆盖模块特定的交互路径。

### 10.6 工具组合评分公式

最终视觉验证分数由三项自动化指标 + 人工修正分加权组成：

```
auto_score = regional_pixel_score * 0.40 + css_var_match_pct * 0.20 + 6_dimension_score * 0.40
```

| 分量           | 权重 | 工具                                 | 阈值  |
| -------------- | ---- | ------------------------------------ | ----- |
| 像素区域加权分 | 0.40 | `scripts/visual-verify.ts --regions` | ≥ 95  |
| CSS 变量匹配率 | 0.20 | `scripts/visual-verify.ts`           | ≥ 95% |
| 6 维度人工评分 | 0.40 | 评审者按评分细则打分                 | ≥ 90  |

`auto_score` ≥ 90 判定 PASS。任一项低于阈值（像素 < 90 / CSS < 80 / 人工 < 80）直接 FAIL。
