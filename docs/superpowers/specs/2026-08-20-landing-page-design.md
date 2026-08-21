# Landing Page 设计 — auth-alioth 登录前产品展示页

日期：2026-08-20 · 状态：待用户评审 · 宿主包：`@dsh-alioth/auth-alioth`

## 背景与目标

auth-alioth 的 node:http 服务器（默认 :3900）当前 `GET /` 直接渲染裸登录表单，
`GET /register` 渲染裸注册表单（`sendPage()` 内联 HTML）。B/S 部署下，未登录访客
没有任何产品能力与介绍的展示面。

目标：访客路径变为 **landing（能力展示）→ 登录/注册 → web GUI**，landing 页承担
产品叙事，风格与内容主线已确认：

- 宿主：auth 服务器同宿主（不引入新部署单元）
- 视觉：深色科技感（终端/开发者工具气质）
- 内容主线：对话 → 应用的演示流

## 技术形态（已选 A）

| 方案 | 结论 |
|---|---|
| **A. 单文件静态 HTML** | **采用**。`packages/alioth/auth-alioth/public/landing.html`，内联 CSS/JS 零依赖、零外联资源（B/S 离线可部署），node:http 启动时 `readFileSync` 读入内存缓存伺服 |
| B. 内联字符串进 index.ts | 否决。~15KB HTML 污染路由文件 |
| C. 框架化前端 | 否决。为一张营销页引入构建链，过度工程 |

## 路由变更

| 路由 | 现状 | 变更后 |
|---|---|---|
| `GET /` | 登录表单 | **landing.html**（200, text/html） |
| `GET /login` | — | 登录表单（原 `/` 内容平移，含「注册」链接指向 `/register`） |
| `GET /register` | 注册表单 | 不变（「登录」链接改指 `/login`） |
| `POST /api/auth/*` | 四个 auth API | 不变 |
| 其他 | 404 JSON | 不变 |

向后兼容：无持久化契约依赖 `GET /` 返回表单；landing 页首屏即有登录入口。
既有测试 `serves the login page (GET /)`（`auth-alioth.spec.ts` B/S HTTP surface 块）
断言 `/` 含登录表单——随路由平移改为 `GET /login`，属本工作的必做迁移项。

## 页面结构（中文优先，文案沿 Alioth 惯例）

1. **Hero** — 产品名「Alioth AppCreator」+ 主张「对话即企业应用」+ 双 CTA
   （`登录` → `/login`，`注册` → `/register`）
2. **演示流主块** — 左：模拟对话气泡（用户：「帮我做一个库存管理应用」）；
   右/下：九阶段 pipeline 终端风步进动画，阶段名使用
   `skill-alioth/src/agent-contract.ts` 的 `PIPELINE_ORDER` 真实 id：
   `app-creation → semantic-analysis → function-decomposition → ontology-analysis
   → module-creation → block-creation → ontology-transfer → service-api
   → e2e-verification`，终态 `published`；完成后浮现产物 chips：
   `app.json` / `module.json` / `extensions/` / `prototype.html` / `Sources/`
3. **能力栅格（4 卡）** — 语义对齐（bge-small-zh 跨语言 grounding）·
   契约校验生成（gen-alioth 程序化产物，非 LLM 自由文本）·
   namespace 隔离（`U-<username>`）· 确定性工作流门禁（state machine + gates）
4. **产物预览** — 真实 `app.json` 结构片段的等宽代码块（手写精简示例，键名
   与 `gen-alioth` 契约一致：`namespace` / `config.modules` / `permissions` /
   `routing` / `min_alioth_version`）
5. **Footer** — `Alioth 模型 v10.0.0` · `Apache-2.0` · 登录/注册链接

## 视觉规范

- 底色 `#0a0e14` 系深色；强调色终端绿/青（单一强调色，不堆砌渐变）
- 正文字体 system-ui 栈；代码/阶段名用等宽栈（`ui-monospace, SFMono-Regular, Menlo, Consolas, monospace`）——不加载外部字体
- 背景网格微纹理（CSS gradient 实现，无图片资源）
- pipeline 动画：节点逐个点亮 + 当前节点脉冲，循环播放；纯 CSS/JS 少量
  `setInterval` 步进，无框架
- `prefers-reduced-motion: reduce` 时禁用动画，所有阶段静态全亮展示
- 响应式：桌面双栏（对话 | pipeline），移动单栏纵向堆叠（`max-width` 媒体查询一档即可）

## 实现要点

- 新文件：`packages/alioth/auth-alioth/public/landing.html`（唯一新增内容文件）
- `index.ts` 改动：
  - `apply()` 内启动时 `readFileSync(new URL('../public/landing.html', import.meta.url))`
    读入缓存（失败即 throw —— 部署缺文件属于 loud failure，不做静默降级）
  - 路由表按上表调整；`sendPage()`/`loginForm()` 保留服务于 `/login`、`/register`
  - landing 响应头与现有页面一致（`text/html; charset=utf-8`），加
    `cache-control: no-cache`（与 sendPage 行为对齐，不引入缓存策略分歧）
- strip-only 约束：新增 TS 代码保持 Node strip-only 兼容（无参数属性/枚举/命名空间）
- 不新增依赖；不改变 `Config` schema

## 测试（auth-alioth 现有测试模式：真 Context + 真嵌入式 PG）

- 迁移：`serves the login page (GET /)` → `serves the login page (GET /login)`，
  fetch 目标改 `${base()}/login`，断言不变（`<form` + `/api/auth/login`）
- 新增：`GET /` → 200，body 含关键标记：`app-creation`、`e2e-verification`、`Alioth AppCreator`
- 新增：`GET /register` → 200 且「登录」链接指向 `/login`
- 既有 auth API 测试不变（路由未触及）

## 非目标（YAGNI）

- 不做 i18n（中文优先，与 Alioth 惯例一致；英文版后续需要再加）
- 不做 SEO meta/OG 标签、不做 analytics
- 不改 web GUI（:3100）任何行为
- 不动登录/注册表单的视觉样式（本次只平移路由；样式统一是独立工作项）
