# Changelog

All notable changes to this project are documented here. Conventional Commits;
this file records user-visible changes per release.

## [Unreleased]

### Added
- **落地页定位象限标注同行竞对**：依据 `AliothMeta/reports/深度研究-AI时代企业核心系统与Alioth价值分析.md`
  的 §3.1 对标表、§3.3 厂商财务、§3.5 定价基准，标注 12 家：Palantir Foundry/AIP、金蝶 苍穹/星瀚、
  用友 BIP、OutSystems、Mendix、Appian、明道云、简道云、Lovable、v0、Replit、秒悟/bolt.new。
  统一按「轻量应用 ↔ 核心业务系统 × 平台云托管 ↔ 源码交付·私有化」定位，采用编号点 + 图例
  （窄屏图例单列），避免直接写品牌名互相压盖。
- **落地页本体面板补全**：左侧「Ontology coordinates」不再用「… 17 个业务域」省略——改为列出
  模型规范的 **10 个场景域**（制造 / 批发零售 / 物流运输 / 租赁 / 技术·IT / 金融 / 商务服务 /
  科研技术 / 公共管理 / 综合经营）、108 / 118 / 418 的编码规模，以及一条真实坐标示例
  （仓储保全 · 交易订单 · 费项纪录 = `GH · FJA · ↓_CA`）；首屏统计的「17 个业务本体域」同步
  改为可核对的「644 个本体坐标」，与右侧定位面板等高、消除空白。
- **商务域落地与单一权益缝（架构评审 C1–C6 全链实施）**：billing-alioth 从「过渡内存实现」升级为**持久化商务域**（`dsh_alioth_billing`：orders/subscriptions/bills/invoices/usage_daily/audit_log 六张表，PG + 内存双适配器——重启丢订阅/账单/发票的缺陷就此关闭）。新增：**订单生命周期**（pending→paid→fulfilled/cancelled/refunded，支付履约）、**期末取消**（`cancel` 置 `canceling` 保持权益至 `renewsAt`，续期执行方每 15 分钟扫到期订阅翻 `canceled` 并生成次期账单；`subscribe` 可撤销取消）、**退款/作废**（admin-only，账单订单联动）、**权益判定单一缝** `entitlement(actor, capability)`（source-download / llm-budget）、**账户计量账本**（`usage_daily` 账户×日×模型；缺价桶令当月成本 `null`，绝不以 0 冒充）、**append-only 审计**（认证/计费/管理/审批统一入缝）、**通知 webhook**（`ALIOTH_NOTIFY_WEBHOOK`，fire-and-forget）。
- **身份缝（C3）**：auth-alioth 新增 `authMode: 'oidc'` 适配器——零依赖自实现 OIDC RP（discovery / 授权 URL / 换码 / JWKS 缓存 + RS256/ES256 验签、一次性 state），对接 NS:Cosmic-Tools 的 embedded SSO 等真实 IdP，JIT 开户落到本地用户契约；本地口令适配器保持默认。MFA/密码重置/三方登录由 IdP 承载，本仓不再各自缺失。
- **AccountContext（C5）**：`accountForSession(sessionId)` 把会话一次解析成 `{userId, username, namespace, role, plan, monthlyCostCents}`（60s memo），guard/billing/计量共读，不再各自推导身份。
- **账户计量与月度预算（C2）**：guard-alioth 的 `AccountMeter` 在 turn 收口把台账增量按 日×模型 落账；`agent/pre-step` 执行**账户月度预算**（配额来自权益缝；成本口径不可得 ⇒ 留降级证据不判，规则码 `account-budget-exceeded`）；guard 内置 DeepSeek 默认价表（部署 `priceTable` 文件可整体覆盖）；`alioth_usage` 新增账户计量视图。
- **BYOK**：用户可存自己的模型 API key（AES-256-GCM under `ALIOTH_BYOK_SECRET`，未启用 fail-closed）；新插件 `llm-alioth` 替换基座 `llm-deepseek` 行（同 provider id + settings namespace），每请求 key 顺序 = 账户自持 key → 平台凭据。
- **支付通道缝与运营面（C4）**：`applyChannelPayment` + `POST /api/billing/channel/<channel>/callback`（HMAC-SHA256 + 5 分钟窗口，密钥未配置即 503 fail-closed）——对接形状 = 上游 `EXTERNAL_INTEGRATION_SPEC` 的 L-EXT/L-ADP/L-MAP，wechat-pay 等适配器住 NS:Cosmic-Tools 的 `Ext-adapter/` 面（契约文档 `docs/specs/billing-channel-ext-adapter.md`）；billing-web 新增 admin 页（对账/订单/审计/L2 开通/退款作废）与 `pnpm run admin:grant`（管理员唯一授予通道）；auth-web 补齐 CSRF Origin 校验、登录限流、Secure cookie（https 公网 origin 自动启用）。

### Changed
- **产品面配色统一到 dsh 设计令牌**：landing / 登录注册 / 工作区 / 用户中心 / 批注页与批注浮层
  由「深色底 + 终端绿/青」改为 DeepSeek Harness 的语义色板（品牌主色 = 中性 primary，
  强调 = DeepSeek 蓝，success/warn/error = dsh 状态色），并按 `prefers-color-scheme`
  提供明/暗两套（dsh 默认 `system`）；控制台内嵌的用户徽章与「应用状态 / 原型」侧栏
  直接读 `--dsw-alias-*` 变量，跟随控制台自身的明暗切换。`theme-color` 改为明暗双 meta；
  favicon 按 dsh 约定改为单色明暗双版本（`favicon.svg` 近黑 / `favicon-dark.svg` 近白，
  透明底、`media="(prefers-color-scheme: …)"` 二选一），ICO 与 apple-touch-icon 以不透明
  中性底重渲。状态色只用于边框/圆点/图标，小字状态文本回到 `--text`（浅色模式下
  green-500/amber-500 作文字不达 AA）；旧背景网格随旧配色移除。
- **复检注记按真实缺口闭环（2026-10-07 第二轮）**：①发票恢复**管理员开具队列**（申请落 pending，admin 页/`issueInvoice` 开具赋收据级票号；「无超管」不变）；②退款改为**按剩余服务期折算**——服务窗口记在订单上，退款额 = ⌊金额×剩余/总期⌋ 落 `bills.refund_amount_cents`，服务随退款终止（billing 域规则支持注入时钟，折算可确定性测试）；③OIDC 适配器补 **PKCE S256**（verifier 随一次性 state、challenge 进授权 URL、换码携带，假 IdP 断言强制）；④计量月度 memo 收紧为 30s 且**落账即失效**（预算判定不吃旧数）；⑤billing 状态变更端点加**动作限流**（`ActionRateLimiter`，30 次/5min，超限 429，GET 不计）。
### Changed
- **运行在 harness `0.2.1-alpha.2` 合并底（`b84f323f8e`）上**：harness 全量 manifest 随上游 release 前进到
  `0.2.1-alpha.2`，`apps/cli` importer 新增 8 个 experimental 成员依赖并升了 execa/`node-addon-require-builtin`/ws；
  既有声明范围 `^0.2.1-alpha.1` 按同元组预发布递增继续命中（74 条 importer 的 vendor/link 全部保住，无 registry 回退），
  本仓 lockfile 与 CI/Docker 引脚同 commit 前进。harness 侧验收：双面 typecheck、全量 vitest 41649 passed / 0 failed；
  本仓验收：7 项检查链 + knip + 829 tests + 一次性库 smoke 全绿，dev console 实机一轮工具调用回复点出本账号命名空间。
- **运行在 harness `0.2.1-alpha.1` 线上**：`@deepseek-ai/*` 声明范围（`^0.2.1-alpha.1`）与 vendor 层（`@deepseek-ai/cordis` `^4.0.5-alpha.1`、
  `schemastery` `^3.18.5-alpha.1`）随 sibling checkout 一起前进——`linkWorkspacePackages` 只在版本区间被 sibling 满足时才出 link，
  升版后旧的 `^0.2.0-rc.2`/`^4.0.4`/`^3.18.4` 一律静默回退 registry（members/examples 曾混装 registry dsh rc.2 + registry schemastery 3.18.4，
  与组合里的 vendor 副本形成双实例），本次全 manifest 清扫后 74 条 importer 全部落回 vendor/link，registry tarball 残留为 0。
  上游 invariant 插件重构删除了 `@deepseek-ai/dsh-invariants` 包，本仓零引用，死依赖随线切除；pnpm 单源到 12.8.1、node 26.10.0
  （`packageManager` + mise + CI 引脚同动，lockfile 随之重生成）。安全底维持同大版本内最小修复版
  （undici / ip-address / fast-uri / brace-expansion），`pnpm audit --audit-level=moderate` 保持绿。

### Fixed
- **共享在线空间禁止「完全控制」**：产品部署是多人 B/S 环境，权限选择不再提供
  `danger-full-access`。服务端 preset 目录（`permission` 行）只保留 `read-only` 与
  `workspace-write`，catalog / `/permission` / `set` / `resolve` 因此一致拒绝该值；
  `approval` 策略与 `sandbox-policy.mode` 同时钉死为 `ask` / `workspace-write`，
  避免继承的 `DSH_PERMISSION_MODE` 绕过 preset 放宽边界。执行面再加一道：
  `guard-alioth` 的 `tools/pre-execute` 对任何携带
  `sandbox_permissions: danger-full-access` 的调用直接拒绝（`[rule:full-access-escalation]`，
  class not-fixable）——bash/run_code 的单次升级审批卡不会渲染。`check:tree-assembly`
  新增部署组合断言：`permission` 行出现 `danger-full-access` 即红。
- **浅色模式下分割线隐形与层塌陷**：①落地页定位矩阵的 L 型轴线与未点亮阶段的圆点此前用
  10% 黑/白描边，浅色下完全不可见——新增 `--line-strong`（明 `rgba(0,0,0,.16)` / 暗
  `rgba(255,255,255,.20)`，用于矩阵轴线）与 `--idle`（明 `#d4d4d4` / 暗 `#545557`，
  用于未点亮圆点，取 dsh `state-idle-primary`）；②浅色 `--panel` 曾与 `--bg` 同为白底，
  卡片/面板「白底白」、bot 气泡与所在面板同色——浅色 `--panel` 改为 dsh `bg-module-platform`
  （bluish-60 `#f5f6f7`），`.bubble.bot` 改 `--bg`；登录/工作区/用户中心/批注页的浅色面板
  一并分层，控制台右侧栏卡片改用 `bg-module-platform` 令牌；③落地页定位矩阵的轴标签——
  x 轴标签此前无纵向定位而悬在顶边、y 轴标签负偏移压到卡片标题且底左角两个标签互相压字——
  改为 y 轴标签竖排贴左轴、x 轴标签横排贴底轴两端，并给矩阵加留白使标签留在面板内；
  同时把矩阵底图换成 **16×16 细网格**（每格 6.25%，取 dsh border-l3 的 `--grid` 色，
  比轴线轻一档），并以 **3px 轴线色重绘四象限分割线**（横竖中心线，明显粗于 1px 细网格）与**轴方向箭头**
  （x 轴向右指向核心业务系统、y 轴向上指向源码交付 · 私有化，取标签色）。
- **右侧栏两个 tab 的互跳按钮点了没反应**：「原型」与「应用状态」互为入口的按钮从一个只注入了
  `sidebarRightTabs` 的 cordis scope 读 `sidebarRight` —— 未注入的服务属性读取会**直接抛**
  `cannot get property "sidebarRight" without inject`（不是 undefined），于是点击只留一条控制台报错、界面毫无反应；
  typecheck / 树装配 / 桩测试全绿也复现（只有浏览器 E2E 看得见）。现注入列表同时声明两者，客户端工件的 inject 桩改为
  复刻该 guard（旧形态下 3 条测试红）。
- **应用页把 `app.json` 的声明 code 当成工作区名**：改名只动目录、不更新 `<app>/app.json` 的 `code`，于是列表按声明显示旧名（实测 `Apps/wms` 的 app.json 声明 `warehouse-management`）——改名到目录名会被「already exists」拦下，而它声称要改的源目录根本不存在。现在：目录名即工作区身份；`renameApp` 先校验源目录（错误指向真因）、改名时同步 `app.json.code`（同名改名=就地修订）；列表读到漂移**自动修订**，仅当产物不可写时才渲染不一致徽标。
- **SQL 漏斗串行化**：`PgHandle` 只持一个 `pg.Client`，此前并发调用者（多个插件在 boot 期同时打 `ctx.aliothEnv.sql()`）会让语句在同一条连接上重叠——pg@8 只打弃用告警（启动日志可见），pg@9 变成硬错；更要紧的是漏斗自己的重放判定（「未发出 ⇒ 可重放」/「可能已执行 ⇒ 绝不重放」）只在无并发在飞时成立。现所有语句走单车道 FIFO 队列（`createSerialLane`），关闭时先排空车道再断连；新增 4 条测试（3 条队列不变量 + 1 条真库并发串行断言）。
- **控制台构建产物（web 面）不再被当成「库建好就算完」**：容器镜像与部署机此前只建 harness 的库
  （`build:lib:*` / `build:native`），而控制台发的是 `apps/web/dist` + `.dsh-build/client-build-environment.json`
  ——只建库 ⇒ 新客户端包加载失败（`Failed to load plugins`、无 composer），而 `/landing` 仍 200、node 侧门禁全绿
  （2026-09-26 dev 实测）。新增单一入口 `scripts/build-harness-web.ts`（复刻 harness `scripts/build.ts` 的 web 段），
  Dockerfile 在库构建后调用它、运行阶段随产物一起拷 `.dsh-build/`，`scripts/docker-check.sh` 断言两者在位；
  运维发布脚本把它接为安装相位 `[6/6]`，并新增只读前置 `0e`（记录里的 commit 必须等于目标机/本地 harness HEAD，
  否则拒绝发布）。

### Added
- **运行期（容器）验证的证据面**：`verify-alioth` 新增 `runtime-verify.ts`（canonical `runtime-verify.json` 与降级 `runtime-verify.degraded.json` **互斥**——真实执行 `verdict ∈ ready|failed` 才写 canonical，降级写 degraded 并清 canonical；证据按键名脱敏）；**全自动链路**：失败给 `runtimeFailure` 机器可执行契约（`artifact|environment|auth|capability` × `RepairClass` 三态）交自动重试与 agent 修复，**不登记人工门**；通过与否按 `/verdict == "ready"` 幂等判定（非文件存在）与 `runtime-control.ts`（容器托管平面控制面消费端：`/containers` 动词、§11 状态码归一、token 载体与 Meta 同序且 `0600`/≥32 字符 fail-closed、`Host` 覆写、TLS 校验可显式关闭）。对齐 `NS_APP_RUNTIME_HOSTING_SPEC.md` §9/§11 与 上游 AppAgent `verify_runtime.rs`；**尚不含**模型面工具与管线门接线。
- **会话绑定改由服务端写入**：客户端脚本嗅探的是 harness 旧 REST 路径（`/api/session/create`），harness 换传输后绑定
  静默失效——m2 实测同一会话落到另一账号的命名空间（工具守卫与工作区选择器双双退化为路径推断）。现在
  `session/created` 时用 harness 的连接账户上下文写入 `dsh_alioth_auth.session_bindings`，`userForSessionId` 优先读它，
  守卫在拒绝前再补绑一次；客户端脚本降为冗余路径。

### Added
- **产物声明模型依赖，并在工具面与控制台显示**：生成期把**本部署的模型版本**写进产物——`app.json.min_alioth_version`、
  `block.json`/`service.json.aliothVersion`（此前是硬编码 `10.0.0` 下限，只有 orchestrator 的 block 骨架传真值）；
  非发行形态的模型源（夹具 / git ref）退契约下限，绝不把 `0.0.0-fixture` 写成依赖。读回侧 `alioth_app_inspect` 新增
  `modelVersion` / `modelVersionSatisfied`（声明的下限 vs 部署提供的模型；任一侧不可判定即 false），控制台「应用状态」
  面板新增「模型版本 / 模型依赖」两行。显示面走新增的 `ctx.aliothEnv.modelInfo()`——只解析模型源、**不连数据库、不引导注册表**。
  比较口径单点在 `gen-alioth/src/version.ts`；`module.json` 不动（上游 MODULE_SPEC 无模型版本键，module 的依赖由所属 app 承载）。
- **裁决召回进决策点**：`alioth_schema_semantic_search` 新增可选 `namespace` + `domain`，命中本命名空间的人工映射裁决时返回 `precedent`（含 `verdict` 与 `ignored` 可判原因）；判定为纯函数（域精确匹配 + 置信门槛 + 目录表存在性，目录为空豁免），账本/目录不可读均降级不阻断。
- **计划 → 扩展的确定性组装**：`alioth_app_write` 新增 `plan` 参数（flow-plan wire 形态，非法即拒），由 `gen-alioth/src/extension-plan.ts` 按上游 `compose_from_flow_plan` 逐文件派生 `extensions/{constraints,rules,statemachines,workflows}.yaml` + 有模块时的 `profiles.yaml`；空来源保持如实骨架（绝不伪造条目），本体 JSON 不可解析同样退骨架。
- **AppAgent 机制吸收（③②①④）**：步骤输入缺失 fail-fast（`step-input-missing`，未启动即拒）、
  人工映射裁决的沉降与召回（`alioth_mapping_verdict`：`keep_gap`/目录外表/目录不可判定一律拒绝）、
  7 阶段**诚实**进度投影 + `pipeline_manifest.json` 交接产物（全部前置通过才写；缺失=pending 不谎报）、
  `flow-plan.json` 产物面（上游同路径 + snake/camel 兼容编解码 + `CreateArgs` 扩展规划面）。
  详见 `docs/appagent-mechanism-uptake.md`。

- **Iron-rule gate** (`tests/iron-rule-no-isahl-sql.spec.ts`): no shipped source may name a
  relation in the model's own `isahl` schema. The "never touch `isahl`" rule was discipline only —
  it is now mechanical (a planted `FROM isahl.<table>` fails the gate; `isahl_meta` and the other
  prefixed schemas are unaffected).
- **DDL-only degradation** for registry-backed tools: when a deployment has no registry rows
  (an open-source model copy ships none), `alioth_schema_info` now serves the model's own DDL
  inventory — tables and inheritance, from the shipped snapshot — instead of an empty result, and
  says so via `degraded` + `note` (rendered as `[DDL-only]`); `alioth_schema_semantic_search`
  indexes the DDL tables so grounding stays on real tables. Field-level metadata, categories and
  cross-entity references remain registry-only and are reported as unavailable.
- **AppAgent contract layer**: gate content predicates (`require_json_pointer` /
  `require_json_equals`, fail-closed over the newest matching artifact), step
  phases (`plan` steps write only their declared plan artifact), a structured
  repair contract (`RepairClass` + `[rule:<id>]` rules with a suggested next
  action) and a retry budget (ping-pong and repair-wall decisions).
- **Execution-surface guard** (`guard-alioth`): denies calls outside the step's
  declared tool surface, writes outside `Pre-Proc/{ns}/…`, and plan-step writes
  outside the step's output glob; enforces the repair wall, the turn budget
  (wall-clock, steps, optional per-turn cost) and a one-shot closure-evidence
  nudge. Unresolvable scope degrades visibly instead of guessing.
- **Verification/closure tooling** (`verify-alioth` + `tool-alioth-verify`):
  `alioth_verify` (eval report, extension runtime verification, stage gates),
  `alioth_closure`, `alioth_version` (snapshot/rollback), `alioth_patch_assets`
  (two-stage confirm), `alioth_capabilities`, `alioth_deferred`, `alioth_usage`.
  Extension verification reports `degraded` (never `passed`) for declarations
  the loader cannot wire, binds its report to the artifact fingerprint and
  registers a deferred human gate.
- **Publish preconditions**: publishing now fails closed unless extension
  verification passed for the current artifacts, the quality report passed, an
  artifact snapshot exists, an approved closure verdict matches the current
  fingerprint, and no degraded gate is open. A shadow evaluator records the
  auto-approval predicates without changing behaviour.
- **Build-regression runner**: `pnpm run eval:appagent validate|run --cases …`
  scores mechanical evidence per case (`extension_verify`, `e2e`,
  `closure_audit`, `eval_report_rules`, `artifacts`), marks a round degraded
  when evidence is missing, and gates on regression only with `--gate`.
- Per-turn cost cap for AppAgent sessions (price table is a deployment choice;
  without one the cost figure stays explicitly unavailable).

### Changed
- Test timeouts are explicit upper bounds (60s test / 60s hook) instead of vitest's 5s/10s
  defaults: every DB-backed suite creates a throwaway database and bootstraps the registry
  (~27k seed rows) inside its hooks, so a busy dev machine (observed at load ~99 from unrelated
  builds) turned the gate red on wall-clock alone. CI is unaffected — a passing test never waits
  that long.
- **Registry rows come from the model snapshot only** (2026-09-24): `registrySource` reports
  `snapshot` (the official distribution ships the dedicated copy a release is generated with, or
  the frozen copy inside this package for `builtin`) or `missing` (an open-source/older snapshot
  ships none: boot **warns and continues** with an empty registry, and the agent works from the
  metadata the model's own DDL carries). Rows a snapshot never shipped are never borrowed from
  another release. Model releases are read in both published layouts — flat fixed paths (version
  from `latest.json`) and the older `<root>/<version>/`.
- **Registry database contract (breaking, 2026-09-24)**: this plugin's registry is
  private deployment state and now lives in its own database (`dsh_alioth`) — never
  inside a database that holds the Alioth model. `bootstrapDatabase()` refuses such a
  target on every call (creation, adoption and drift reporting alike): any regular table
  under `isahl` marks the database as the model's, and `isahl_meta` is a namespace the
  model owns too. The container entry creates `dsh_alioth` and renames a legacy `alioth`
  volume in place. Databases that still hold the registry (`isahl_meta` + `dsh_alioth*`)
  need the one-time move and detach in
  `docs/migrations/2026-09-24-registry-out-of-model-database.md`.
- The console's **operator surfaces** are now **loopback-only**: the
  **Settings** panel (模型 / 内置插件 / Agent 预设 — it reads and writes the
  serving Host's own configuration and its rich actions are themselves
  loopback-gated) and the **Plugins** panel (`ui-plugin-manager` — it installs,
  enables, disables and removes the Host's plugin rows). Off a loopback
  authority — `localhost`, IPv6 loopback, `127/8`, the same rule the harness's
  `/api` Host fence uses — a priority `-1` occupant shadows the settings seat
  and the panel page, and the sidebar's Plugins row hides itself, so a
  multi-tenant console reached through a domain, reverse proxy or tunnel no
  longer offers a half-broken machine-owner surface; operators at
  `localhost:3100` keep both unchanged.
- Gate failures now carry the repair contract only — the retired
  `GateErrorKind` vocabulary is internal to the gate module and no longer
  reaches the model.
- The adapter tool mapping covers the whole tool vocabulary the shipped
  adapters declare, with manual paths documented where no harness tool exists.
- Vendored framework re-synced to the current AliothMeta line (adapters moved
  under `Meta/backend/app-agent/skill-adapters`; new gate programs and the eval
  toolchain synced). Adapter gate scripts *and* their non-script references are
  now checked for reachability at sync time and in CI.
- Semantic dictionaries refreshed against the model release (coordinates 644,
  physical tables 986, FK index 2661); the generator accepts both the versioned
  and the flat model publication layout.
- Semantic dictionaries refreshed against model **v10.0.36**: coordinates 644
  (scene 108 / factor 118 / function 418) and physical tables 990 unchanged,
  FK index 3528 → **3529** refs; `check:dicts` anchors the snapshots to that
  release. The release's registry sidecar carries 990 collections / 32298
  fields (v10.0.35: 990 / 32295).
- Generated apps carry the lifecycle `status` the evaluation requires, so no
  later stage patches the artifact.

### Removed
- "Open in application" from the browser console: the Host launcher
  (`open-in-app` — it spawned the probed desktop application on the *serving*
  host as the service user, on any absolute directory the caller named) and its
  browser surface (`ui-open-in-app`, the session-header split button plus the
  document path actions). A multi-tenant deployment has no "the browser user's
  own machine" for that feature to mean.
- Installability as a local application: the origin publishes no web app
  manifest. `/manifest.webmanifest` (the harness console shell's own dist
  manifest, `DeepSeek Harness` / `display: fullscreen`) and `/site.webmanifest`
  now answer 404, the site manifest asset is gone, and the landing, auth and
  user-center documents no longer declare `<link rel="manifest">` — the brand
  icons stay, the browser's "install this app" offer does not.

## [0.1.0] — 2026-08-20

### Added
- Full AppCreator capability as a dsh plugin group (6 tool packages, 9 model-facing
  tools), self-bootstrapping environment: vendored frozen Alioth v10 model,
  embedded PostgreSQL 18, `isahl_meta` bootstrap, provenance stamping, doctor.
- Semantic entity grounding: `alioth_schema_semantic_search` (transformers.js +
  bge-small-zh-v1.5, multilingual synonyms, offline library, cached index).
- Entity registration with hard validation: naming, physical-table, inheritance,
  references, real coordinate dictionaries (`entity-validate`).
- PTC orchestrator `alioth_app_create`: deterministic validate → entity → app →
  verify pipeline, atomic failure, every step through `ctx.tools.execute`.
- 9-stage AppAgent state machine aligned with the ACTIVE AliothMeta Meta line.
- B/S delivery: `auth-alioth` (registration/login, scrypt, token sessions,
  `U-<username>` namespace isolation, admin/user roles, `tools/pre-execute` guard).
- Docker delivery: runnable container (node:24.19-slim + PGDG PostgreSQL 18 +
  bun), keyless `--check` self-check.
- Gate suite: CI matrix (typecheck/lint/tests+coverage/knip/strip-only/vendor
  provenance/version sync/dict freshness/tree assembly/composition smoke/
  commitlint/audit/shellcheck/gitleaks) + docker build gate; lefthook
  pre-commit + commitlint; model-surface keyless snapshot; semantic-dict anchor;
  vendor LICENSE/NOTICE + PROVENANCE.json; registry pinned to npmjs;
  security floors for adm-zip/sharp/yaml.

### Fixed
- `link-dsh-profiles.sh` now links `auth-alioth` (bundle dep was unresolvable).
- Strict-mode indexing errors in `scripts/generate-semantic-dicts.ts` exposed by
  extending typecheck to `scripts/`.
- AGENTS.md doc drift: duplicated "Known LSP noise" paragraph, stale test count,
  stale dictionary counts (real: 651 codes / 902 tables / 899 refs), Docker
  tool-count wording.
