# Landing Page 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** auth-alioth 服务器 `GET /` 改为深色科技感产品落地页（对话→应用演示流），登录表单平移至 `/login`。

**Architecture:** 单文件静态 `public/landing.html`（内联 CSS/JS，零依赖零外联），node:http 启动时 `readFileSync` 读入缓存伺服；路由表三处调整；测试一迁移两新增。

**Tech Stack:** TypeScript（Node strip-only）、node:http、vitest（真 Context + 嵌入式 PG）。

## Global Constraints

- Spec：`docs/superpowers/specs/2026-08-20-landing-page-design.md`（路由表/视觉规范以此为唯一真理）
- 九阶段 id 逐字取自 `packages/alioth/skill-alioth/src/agent-contract.ts` 的 `PIPELINE_ORDER`
- 新增 TS 代码 Node strip-only 兼容（无参数属性/枚举/运行时命名空间）
- 不新增依赖；不改 `Config` schema；不动四个 auth API
- 中文优先；`prefers-reduced-motion` 降级静态
- 工作区有用户在途改动：**本计划所有 commit 步骤跳过**，改动留工作区由用户随在途工作提交

---

### Task 1: 路由测试（red）

**Files:**
- Test: `packages/alioth/auth-alioth/tests/auth-alioth.spec.ts`（`describe('B/S HTTP surface (real server)')` 块内，约 151-158 行）

**Interfaces:**
- Consumes: 既有 `base()` helper（`http://127.0.0.1:${port}`）
- Produces: 三个测试名——`serves the landing page (GET /)`、`serves the login page (GET /login)`、`links register page back to /login`

- [ ] **Step 1: 替换既有 `serves the login page (GET /)` 测试为以下三个**

```ts
  it('serves the landing page (GET /)', async () => {
    const response = await fetch(`${base()}/`)
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('Alioth AppCreator')
    expect(html).toContain('app-creation')
    expect(html).toContain('e2e-verification')
  })

  it('serves the login page (GET /login)', async () => {
    const response = await fetch(`${base()}/login`)
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('<form')
    expect(html).toContain('/api/auth/login')
  })

  it('links register page back to /login', async () => {
    const response = await fetch(`${base()}/register`)
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('href="/login"')
  })
```

- [ ] **Step 2: 跑测试确认红**

Run: `pnpm exec vitest run packages/alioth/auth-alioth`
Expected: 3 FAIL——landing 测试无标记；`/login` 返回 404 JSON；register 页链接仍是 `href="/"`

- [ ] **Step 3: Commit —— 跳过（见 Global Constraints）**

### Task 2: landing.html + 路由实现（green）

**Files:**
- Create: `packages/alioth/auth-alioth/public/landing.html`
- Modify: `packages/alioth/auth-alioth/src/index.ts`（import 区 + `apply()` 内路由）

**Interfaces:**
- Consumes: Task 1 的三个测试
- Produces: 路由 `GET /`（landing HTML，响应头 `text/html; charset=utf-8` + `cache-control: no-cache`）、`GET /login`（原登录表单）

- [ ] **Step 1: 创建 `packages/alioth/auth-alioth/public/landing.html`，内容逐字如下**

```html
<!doctype html>
<html lang="zh">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Alioth AppCreator — 对话即企业应用</title>
<style>
:root {
  --bg: #0a0e14;
  --panel: #101724;
  --line: #1e2a3a;
  --text: #d7e0ea;
  --dim: #7d8ca0;
  --accent: #3ee6a8;
  --accent-2: #4fc3f7;
  --mono: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
* { box-sizing: border-box; margin: 0; padding: 0 }
body {
  background: var(--bg);
  color: var(--text);
  font-family: system-ui, -apple-system, "PingFang SC", "Microsoft YaHei", sans-serif;
  line-height: 1.6;
  background-image:
    linear-gradient(rgba(62,230,168,.05) 1px, transparent 1px),
    linear-gradient(90deg, rgba(62,230,168,.05) 1px, transparent 1px);
  background-size: 44px 44px;
}
a { color: var(--accent-2); text-decoration: none }
.wrap { max-width: 1080px; margin: 0 auto; padding: 0 1.5rem }
nav {
  display: flex; justify-content: space-between; align-items: center;
  padding: 1.25rem 0;
}
.wordmark { font-family: var(--mono); font-weight: 700; letter-spacing: .04em }
.wordmark span { color: var(--accent) }
.btn {
  display: inline-block; padding: .5rem 1.1rem; border-radius: 6px;
  font-size: .92rem; border: 1px solid var(--line); color: var(--text);
}
.btn.primary { background: var(--accent); border-color: var(--accent); color: #06251a; font-weight: 600 }
nav .btn + .btn { margin-left: .6rem }
.hero { padding: 4.5rem 0 3rem; text-align: center }
.hero h1 { font-size: 2.6rem; letter-spacing: .02em }
.hero h1 em { color: var(--accent); font-style: normal }
.hero p { color: var(--dim); max-width: 40rem; margin: 1rem auto 1.8rem }
.demo {
  display: grid; grid-template-columns: 1fr 1.2fr; gap: 1.25rem;
  margin: 2rem 0 3.5rem;
}
@media (max-width: 880px) { .demo { grid-template-columns: 1fr } }
.panel {
  background: var(--panel); border: 1px solid var(--line);
  border-radius: 10px; padding: 1.25rem;
}
.panel h2 {
  font-family: var(--mono); font-size: .78rem; color: var(--dim);
  text-transform: uppercase; letter-spacing: .12em; margin-bottom: 1rem;
}
.bubble {
  max-width: 85%; padding: .6rem .9rem; border-radius: 10px; margin-bottom: .8rem;
  font-size: .95rem;
}
.bubble.user { background: #1b2b3f; margin-left: auto; border-bottom-right-radius: 2px }
.bubble.bot { background: #122019; border: 1px solid var(--line); border-bottom-left-radius: 2px }
.bubble.bot code { font-family: var(--mono); color: var(--accent); font-size: .85em }
.pipeline { font-family: var(--mono); font-size: .86rem; list-style: none }
.pipeline li {
  display: flex; align-items: center; gap: .6rem;
  padding: .3rem 0; color: var(--dim); transition: color .3s;
}
.pipeline li .dot {
  width: 8px; height: 8px; border-radius: 50%;
  background: var(--line); flex: none; transition: background .3s;
}
.pipeline li.done { color: var(--text) }
.pipeline li.done .dot { background: var(--accent) }
.pipeline li.done .mark { color: var(--accent) }
.pipeline li.active { color: var(--accent) }
.pipeline li.active .dot { background: var(--accent); animation: pulse 1s infinite }
.pipeline li .zh { margin-left: auto; font-family: system-ui, sans-serif; font-size: .78rem; color: var(--dim) }
.pipeline li.final.done { color: var(--accent); font-weight: 700 }
@keyframes pulse { 50% { opacity: .3 } }
.chips { display: none; flex-wrap: wrap; gap: .5rem; margin-top: 1rem }
.chips.show { display: flex }
.chips span {
  font-family: var(--mono); font-size: .78rem; padding: .25rem .6rem;
  border: 1px solid var(--accent); color: var(--accent); border-radius: 999px;
}
.cards { display: grid; grid-template-columns: repeat(4, 1fr); gap: 1rem; margin-bottom: 3.5rem }
@media (max-width: 880px) { .cards { grid-template-columns: 1fr 1fr } }
@media (max-width: 560px) { .cards { grid-template-columns: 1fr } }
.card { background: var(--panel); border: 1px solid var(--line); border-radius: 10px; padding: 1.1rem }
.card h3 { font-size: 1rem; margin-bottom: .5rem; color: var(--accent-2) }
.card p { font-size: .88rem; color: var(--dim) }
pre {
  background: #070b11; border: 1px solid var(--line); border-radius: 10px;
  padding: 1.25rem; overflow-x: auto; font-family: var(--mono);
  font-size: .85rem; line-height: 1.55; margin-bottom: 3.5rem;
}
pre .k { color: var(--accent-2) }
pre .s { color: var(--accent) }
footer {
  border-top: 1px solid var(--line); padding: 1.5rem 0 2.5rem;
  color: var(--dim); font-size: .85rem; display: flex; justify-content: space-between;
  flex-wrap: wrap; gap: .5rem;
}
@media (prefers-reduced-motion: reduce) {
  .pipeline li.active .dot { animation: none }
}
</style>
</head>
<body>
<div class="wrap">
  <nav>
    <div class="wordmark">Alioth<span>·</span>AppCreator</div>
    <div>
      <a class="btn" href="/login">登录</a>
      <a class="btn primary" href="/register">注册</a>
    </div>
  </nav>

  <header class="hero">
    <h1>对话，<em>即企业应用</em>。</h1>
    <p>Alioth AppCreator 把一句自然语言变成可运行的企业应用——确定性九阶段流水线，
       产物通过契约校验，直接导入 AliothStudio。</p>
    <a class="btn primary" href="/register">立即开始</a>
    <a class="btn" href="/login">已有账号登录</a>
  </header>

  <section class="demo">
    <div class="panel">
      <h2>Dialogue</h2>
      <div class="bubble user">帮我做一个库存管理应用</div>
      <div class="bubble bot">已解析需求：注册实体 <code>6</code> 个，生成应用
        <code>inventory-management</code>，九阶段全部通过。</div>
      <div class="chips" id="chips">
        <span>app.json</span><span>module.json</span><span>extensions/</span>
        <span>prototype.html</span><span>Sources/</span>
      </div>
    </div>
    <div class="panel">
      <h2>Pipeline — deterministic, zero-LLM</h2>
      <ul class="pipeline" id="pipeline">
        <li class="done"><span class="dot"></span>app-creation<span class="mark">✓</span><span class="zh">应用创建</span></li>
        <li class="done"><span class="dot"></span>semantic-analysis<span class="mark">✓</span><span class="zh">语义解析</span></li>
        <li class="done"><span class="dot"></span>function-decomposition<span class="mark">✓</span><span class="zh">功能分解</span></li>
        <li class="done"><span class="dot"></span>ontology-analysis<span class="mark">✓</span><span class="zh">本体分析</span></li>
        <li class="done"><span class="dot"></span>module-creation<span class="mark">✓</span><span class="zh">模块创建</span></li>
        <li class="done"><span class="dot"></span>block-creation<span class="mark">✓</span><span class="zh">区块创建</span></li>
        <li class="done"><span class="dot"></span>ontology-transfer<span class="mark">✓</span><span class="zh">本体迁移</span></li>
        <li class="done"><span class="dot"></span>service-api<span class="mark">✓</span><span class="zh">服务接口</span></li>
        <li class="done"><span class="dot"></span>e2e-verification<span class="mark">✓</span><span class="zh">端到端验证</span></li>
        <li class="final done"><span class="dot"></span>published<span class="mark">✓</span><span class="zh">发布</span></li>
      </ul>
    </div>
  </section>

  <section class="cards">
    <div class="card"><h3>语义对齐</h3>
      <p>bge-small-zh 向量检索跨语言 grounding，自然语言概念确定性映射到注册表术语。</p></div>
    <div class="card"><h3>契约校验生成</h3>
      <p>产物由 gen-alioth 程序化生成并通过 JSON Schema 契约校验，非 LLM 自由文本。</p></div>
    <div class="card"><h3>Namespace 隔离</h3>
      <p>每个用户独占 <code>U-&lt;username&gt;</code> 命名空间，工具调用前置鉴权。</p></div>
    <div class="card"><h3>确定性工作流门禁</h3>
      <p>Track/Step 状态机 + 产物 glob 与程序门禁，主管线零 LLM 调用。</p></div>
  </section>

  <section>
    <div class="panel" style="padding:0;background:none;border:none">
      <h2 style="margin-bottom:.6rem">Artifact — app.json</h2>
      <pre>{
  <span class="k">"namespace"</span>: <span class="s">"U-demo"</span>,
  <span class="k">"min_alioth_version"</span>: <span class="s">"10.0.0"</span>,
  <span class="k">"config"</span>: {
    <span class="k">"modules"</span>: [<span class="s">"inventory"</span>],
    <span class="k">"blocks"</span>: [<span class="s">"stock-list"</span>, <span class="s">"stock-form"</span>]
  },
  <span class="k">"permissions"</span>: {
    <span class="k">"inventory:read"</span>: <span class="s">"查看库存"</span>,
    <span class="k">"inventory:write"</span>: <span class="s">"维护库存"</span>
  },
  <span class="k">"routing"</span>: { <span class="k">"/inventory"</span>: <span class="s">"stock-list"</span> }
}</pre>
    </div>
  </section>

  <footer>
    <span>Alioth 模型 v10.0.0 · Apache-2.0 · 由 DeepSeek Harness 驱动</span>
    <span><a href="/login">登录</a> · <a href="/register">注册</a></span>
  </footer>
</div>
<script>
(function () {
  if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
    document.getElementById('chips').classList.add('show')
    return
  }
  var items = Array.prototype.slice.call(document.querySelectorAll('#pipeline li'))
  var chips = document.getElementById('chips')
  var i = 0
  function reset() {
    items.forEach(function (li) { li.classList.remove('done', 'active') })
    chips.classList.remove('show')
    i = 0
  }
  function tick() {
    if (i > 0) {
      items[i - 1].classList.remove('active')
      items[i - 1].classList.add('done')
    }
    if (i === items.length) {
      chips.classList.add('show')
      window.setTimeout(function () { reset(); tick() }, 2600)
      return
    }
    items[i].classList.add('active')
    i++
    window.setTimeout(tick, i === 1 ? 600 : 420)
  }
  reset()
  tick()
})()
</script>
</body>
</html>
```

注：`<li>` 的初始静态状态为全部 `done`（无 JS / reduced-motion 用户看到完整静态结果）；
脚本仅在允许动效时 reset 并逐步点亮，循环播放。

- [ ] **Step 2: 修改 `packages/alioth/auth-alioth/src/index.ts`**

import 区加：

```ts
import { readFileSync } from 'node:fs'
```

`apply()` 内、`createServer` 之前加（缺文件即 throw，loud failure）：

```ts
  // Landing page (product showcase) served at GET /; login form moved to /login.
  const landingHtml = readFileSync(new URL('../public/landing.html', import.meta.url), 'utf8')
```

路由替换——把现有 `GET /` 分支改为两个分支：

```ts
      if (request.method === 'GET' && url.pathname === '/') {
        response.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-cache' })
        response.end(landingHtml)
        return
      }
      if (request.method === 'GET' && url.pathname === '/login') {
        sendPage(response, '登录', loginForm(''))
        return
      }
```

`/register` 分支的登录链接改指向：

```ts
<button>注册</button></form><p><a href="/login">登录</a></p>`)
```

- [ ] **Step 3: 跑测试确认绿**

Run: `pnpm exec vitest run packages/alioth/auth-alioth`
Expected: 全部 PASS（含三个新/迁移测试）

- [ ] **Step 4: Commit —— 跳过（见 Global Constraints）**

### Task 3: 全量门禁验证

**Files:** 无新增改动。

- [ ] **Step 1: 全量门禁**

Run: `mise run gates`
Expected: typecheck / lint（--deny-warnings）/ 全部测试 / strip-only / vendor / versions / dicts / tree-assembly 全绿

- [ ] **Step 2: 冒烟（真实伺服）**

Run: `pnpm exec vitest run packages/alioth/auth-alioth` 已通过真实 node:http 服务器覆盖
`GET /`、`/login`、`/register`；另可用 `mise run launch` 人工过一眼视觉（可选）。

---

## Self-Review 记录

- Spec 覆盖：路由表三行 → Task 2 Step 2；页面五段结构/视觉规范 → Task 2 Step 1；
  测试一迁移两新增 → Task 1；strip-only/零依赖 → Global Constraints + Task 3 门禁兜底。无缺口。
- 占位符：无；landing.html 全文在计划内。
- 一致性：测试断言标记 `app-creation`/`e2e-verification`/`Alioth AppCreator` 均在
  landing.html 中出现；`/login` 路由与 register 页 `href="/login"` 互洽。
