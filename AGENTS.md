# Repository Guidelines

## 核心决策原则

**准确·稳定·一致性优先**——一切工具选型、方案设计、代码变更以此为标准：准确先于快速，稳定先于新奇，一致先于便利。

- 结构化格式（CSS/HTML/JS/JSON/YAML/TOML/SQL）必须用专用解析器，禁止正则模拟（见 `docs/specs/NO_REGEX_FOR_PARSING.md` 同源规范）；本仓库的离线生成/门禁脚本对固定行格式的种子/DDL 做行级提取属例外，但新解析一律先查现成实现。
- 工具链固化：同一类操作只留一个入口（pnpm scripts / mise tasks / lefthook），不重复造门禁。
- 现成方法优先：实现/修复/重构前必须四查现成实现（AGENTS.md 规约 / scripts 工具 / vendored 产物 / 上游 harness 包），有则不重写。
- 插件对齐 **AppAgent**（app-agent 状态机 + skill-adapters + 产物契约）——不迁移 AliothStudio 的 Meta/Gateway 开发约定（cargo/Rust 门禁、DB schema truth via Meta、SSO/NGAC、前端框架禁令等一概不适用）。

## 核心边界表

| 类型 | 规则 |
|---|---|
| ✅ Always | 程序化生成优先：产物一律走 gen-alioth 生成器 + 契约门工具（`alioth_app_write`/`alioth_app_configure`/`alioth_entity_write`），LLM 只供结构化参数与语义决策，禁止直写产物文件；模型面改动必须更新 keyless 快照（`tests/model-surface.spec.ts`）；namespace 参数一律先 `alioth_workspace_current` 解析，禁止猜测；提交前过 lefthook + CI 门禁；改动 AGENTS.md 数字（测试数/字典规模）同 PR 刷新 |
| ⚠️ Ask | 破坏性操作（删账号/删产物/改门禁/改 CI）；模型发行物同步（vendor 重生成 + `resetRegistry` 会清本地注册实体）；新增外部依赖；迁移 AppAgent 管线契约 |
| 🚫 Never | 用 `read` 工具读 vendor/模型快照文件（用 `alioth_workflow_info` 内省）；直接 SQL 写 `isahl_meta`/`dsh_alioth_auth`（产品工具是唯一写入通道，人工操作除外）；静默丢弃未提交改动（删除前先 stash 兜底）；agent 代执行 DB 域批量操作（重置/迁移/种子刷新——交付 SQL 或命令，执行归用户） |
| 🔍 Audit | 动 AppAgent 管线/契约/架构/安全前，对照本文件与 skill-alioth 契约（`agent-contract.ts` serde-alias 兼容）逐条核对；诊断与处方分离——只读审查任务给结论，不附代执行提议 |

## 规约刷新分层

| 层级 | 内容 | 刷新时机 |
|---|---|---|
| L-会话级 | 本 AGENTS.md 全文 | 会话启动、上下文压缩恢复后 |
| L-任务级 | 命中的技能 SKILL.md、契约文件 | 每步决策前（换任务重读） |
| L-冷层 | 未命中内容 | 永不重读 |

任务特征变化使冷层行命中时，该行升级为任务级——分层是刷新节奏约束，不是信息屏蔽。新增任何协议/文档前先分层，控制层自身不得成为过载源。

## 已知缺口（模型通道现状）

- **模型发布物（`github:CosmicTools9/Alioth`，如 v10.0.2）只含物理 DDL（`002_isahl_tables.sql`，0 条 REFERENCES 约束）+ 维度种子——不含 `isahl_meta` 注册表**。注册表语义（名称/类目/继承/`reference_config`）由 `env-alioth/vendor/backend/ddl/003|004_isahl_meta_seed_*.sql` 承载，来源是活的 AliothStudio 注册表（`sync_from_database`）。
- 语义库（skill-alioth/src/data/）为混合来源：`coordinates.json`/`physical-tables.json` 可从发布物离线提取（`check:dicts` 真门禁）；`fk-index.json` 只能从 vendored 种子提取，**其新鲜度跟着 vendor 同步节奏走**（`scripts/sync-vendor-registry.ts` → `check:vendor --update`），模型仓库演化不会直接触发它。
- 模型演化后的本地注册表需 `mise run alioth:doctor --reset` 重引导（清本地自定义实体）+ 语义索引按 entriesHash 自动重建。

## Project Overview

`dsh-alioth` is a [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) plugin that leverages the **Alioth V10 data model** to deliver **AppCreator** capabilities: a dialogue-driven enterprise app generator. The agent (running on the harness) turns a user request into AliothStudio-compatible artifacts — `app.json`, `module.json`/`block.json`, `extensions/*.yaml`, HTML prototypes, and backend `Sources/` skeletons — through the Alioth model pipeline (AppAgent state machine + `alioth-gen` + `ontology-mapping`), backed by a PostgreSQL `isahl_meta` schema.

The plugin group is authored against two sibling checkouts used as reference:

- `/Users/william.d.zk/WorkSpace/deepseek-harness` — the host harness (plugin-based agent framework on vendored Cordis; "everything is a plugin", upstream `deepseek-ai/deepseek-harness`).
- `/Users/william.d.zk/WorkSpace/AliothStudio` — the Alioth platform (model v10.0.0). `AppCreator/` inside it is the standalone consumer the plugin reproduces as harness-native capability.

Positioning (from `AppCreator/DESIGN_INTENT.md`): AppCreator is the open-source standalone consumer entry to the Alioth model — needs only PostgreSQL + an LLM API key, shares the AppAgent pipeline via vendored crates (`vendor/app-agent`, `vendor/alioth-gen`, `vendor/ontology-mapping`, `vendor/runtime-engine`), and its output imports cleanly into AliothStudio.

## Architecture & Data Flow

### Harness side (host)

- **Plugin model (Cordis)**: a plugin exports `name`, `inject`, `apply(ctx, config)`, and a schemastery `Config`; it contributes services, typed events, and reversible effects to a shared `Context`. All registrations go through `ctx.effect()` / `ctx.on()` / `ctx.waterfall()`; a registry's `register()` returns a disposer. Every part of the product — model adapter, tool registry, session log, agent loop — is a replaceable plugin.
- **Agent loop**: one *step* = one model request plus its tool calls; a *turn* = zero or more steps. Durable session events (`turn/*`, `step/*`, `user/message`, `assistant/*`, `tool/*`) persist to an append-only `SessionEvent` log; live extension events (`agent/*`, `tools/*`, capability events) observe/intercept work in flight. `agent/pre-step`, `agent/request`, `llm/stream`, and the three `tools/*` events are waterfalls — listeners MUST call `next()`.
- **Capability seam** = Service Definition + Service Provider + Consumer (commonly a model-facing tool). Adding a capability means designing all three roles. A data-backed tool is registered with `defineTool(...)` on `ctx.tools`; its JSON schema joins prompt assembly automatically.
- **Profiles & bundles**: a running `dsh` is a plugin tree composed from ordered layers — bundles (patch layers, `dsh.bundle` in package.json) stacked by profiles (`dsh.profile`, templates `web`/`headless`). Inspect your boot tree with `dsh --profile web --dump-config`.
- **Model-visible ⟺ logged**: anything that reaches a model request must be reconstructable from the session log; new model-visible input requires a new `SessionEventMap` member.

### Alioth side (data model the plugin operates on)

- **Alioth 模型 v10.0.0** — single source of truth: `AliothStudio/Meta/backend/alioth-gen/src/lib.rs` (`ALIOTH_MODEL_VERSION`, env `MODEL_VERSION`, default `10.0.0`).
- **Ontology → DB mapping**: `docs/specs/ALIOTH_ONTOLOGY_SPEC.md` is the unique mapping truth. Business entities map onto lifecycle leaf tables in the `isahl` schema (`zc_id_lifecycle` inheritance family); `isahl` forbids `CREATE TABLE`/`ALTER TABLE` — new entity types extend via `meta_collections` / `meta_fields`. **Schema truth is the DB**: query live with `mise run schema-info`, never read DDL files.
- **Artifact model**: `app.json` (requires `namespace`, plus `config.modules/blocks`, `permissions`, `routing`, `navigation`, `min_alioth_version`), `module.json`, `block.json`; authoritative paths under `Pre-Proc/{namespace}/`. Concrete example: `AliothStudio/Pre-Proc/Alioth/Apps/ai-i-need-a/app.json`.
- **AppAgent pipeline**: `Meta/backend/app-agent/` — a state machine consuming `skill-adapters/*.yaml` (typed Track/Step/gate definitions: `alioth-app.yaml`, `alioth-module.yaml`, `alioth-block.yaml`, `alioth-compose.yaml`, `alioth-ontology.yaml`, ...). This is the pipeline the plugin drives to create apps.
- **Data flow**: dialogue request → AppAgent state machine (tracks/steps/gates) → `alioth-gen`/`ontology-mapping` engines → Pre-Proc artifacts (`Pre-Proc/{namespace}/Apps/{app}/app.json` + `extensions/` + `prototype.html` + `gateway_design.md` + `Sources/` skeleton) → importable by AliothStudio/Gateway. Plugin bridge shape: harness tool → Rust binary/API (app-agent / alioth-gen CLI) → PostgreSQL `isahl_meta` → artifacts.

## Key Directories

### Harness (`/Users/william.d.zk/WorkSpace/deepseek-harness`)

- `packages/<group>/<pkg>/` — `@deepseek-ai/dsh-*` workspaces; `core/` (session, system-prompt, tools, agent, agent-loop), `llm/`, `session/`, `skill/`, `shell/`, `fs/`, `web/`, `subagent/`, `todo/`, `bundle/`, `examples/`, `util/`. Full map: `packages/README.md`.
- `apps/cli`, `apps/web` — bins. `docs/` — architecture, testing, cookbook. `examples/` — runnable `cordis.yml` leaves + e2e/snapshot scenarios. `scripts/` — repo gates (`run-gates.ts`) and generators. `.agents/` — workflows, skills, Agent Notes.

### Alioth (`/Users/william.d.zk/WorkSpace/AliothStudio`)

- `Meta/` — metadata platform (backend Rust :4949, frontend React :49494); `app-agent/` = AppAgent pipeline.
- `AppCreator/` — independent deliverable: own Cargo/pnpm workspace, `backend/` (Rust, `vendor/` crates), `Pre-Proc/Alioth`, `skill-adapters/` (vendored copy, synced by `scripts/sync-framework.sh` — never hand-edit), `scripts/` (`dev.sh`, `prototype-tool.js`, `db/`, `check/`).
- `Gateway/` (business runtime, SSO auth), `SSO/`, `Framework/` (shared infra), `Pre-Proc/{namespace}/` (generated `Apps/`, `Sources/`, `Prototypes/`, `_schema`, `seed`, `openapi/`), `skill-adapters/` (AppAgent YAML), `docs/specs/` (executable specs), `scripts/`, `Samples/`.

### This repo (`dsh-alioth`)

- `packages/alioth/tool-alioth/` — `alioth_app_list` (discovery: enumerate namespaces/apps with contract validity), `alioth_app_inspect` (read/validate `app.json`), `alioth_app_write` (generate + persist contract-validated app tree: app.json, module.json, extensions skeletons, Sources dirs; refuses overwrite; `approvalMode` `required`|`bypass`, default bypass), `alioth_app_configure` (enrichment + growth: merge brand/navigation/routing/permissions/status/goal/non_scope AND add modules — each new module gets a contract-valid module.json + Sources dir and joins config.modules/navigation — or replace blocks), `alioth_app_delete` (destructive, irreversible: requires `confirm: true`, approval seam shared with write; archive via configure `status`). `examples/alioth-agent/` composes the Alioth overlay for headless + web profiles.
- `packages/alioth/tool-alioth-meta/` — `alioth_schema_info`: read-only registry queries (entities/entity/search-fields) over `ctx.aliothEnv.sql()`; dev-seed test entities (`-testing`/`-test` suffixes) hidden by default, `filteredTesting` count reported, `includeTesting` opt-in. `alioth_schema_semantic_search`: multilingual semantic grounding (synonyms + cross-language, e.g. "inventory balance" hits 状态-库存) via transformers.js + Xenova/bge-small-zh-v1.5 (512 dims); model downloads on first use (endpoint `https://hf-mirror.com`, override `DSH_HF_ENDPOINT`), vectors cached under `<dataRoot>/semantic/` keyed by entries hash; real-model probe: `pnpm exec tsx packages/alioth/tool-alioth-meta/tests/probe-semantic.ts` (needs local dev DB). `alioth_entity_write`: register a new business entity on an existing isahl physical table (isahl forbids CREATE TABLE) — validates via skill-alioth `entity-validate` (naming/physical-table/conflicts/inheritance/references/real coordinate dictionary), then INSERTs meta_collections + meta_fields (config: depth/source dsh-alioth/inherits/category/coordinates; reference_config per the FK-index shape); `approvalMode` required|bypass.
- `packages/alioth/gen-alioth/` — artifact contracts (`src/contracts/*.schema.json`, validated by the dependency-free engine in `validate-engine.ts`) + pure generators (`generateApp`, `generateModule`, `generateExtensions`, `generateService`, `sourceModuleDirs`); contract-alignment tests against hand-written fixtures.
- `packages/alioth/skill-alioth/` — adapter orchestration primitives: YAML parsing (`adapter.ts`), track/step state machine (`state.ts`), gate checks (`gates.ts`, output-glob + program hook), file-backed run state (`workspace.ts`), adapter-tool→harness-tool mapping (`mapping.ts`: read_file→read, write_file→write, search_files→glob/grep), external program runner + bun probe (`bun.ts` — the distribution's prototype gates run `bun scripts/prototype-tool.js`; not ported, bun is a declared deployment dependency). Entity write-path validation (`entity-validate.ts`): naming, physical-table existence (isahl forbids CREATE TABLE — entities map onto existing physical tables), conflicts, inheritance (exists/acyclic/depth), reference integrity (target/junction ∈ registry∪physical; local_key ∈ FK index ∪ root columns), coordinates — **no degradation**: scene/factor/function checked against real dictionary snapshots in `src/data/` (`coordinates.json` 651 codes — scene 109 / factor 118 / function 424, `fk-index.json` 2661 physical refs, `physical-tables.json` 938 tables + 9 root columns), generated offline from the Alioth repo + vendored seed by `scripts/generate-semantic-dicts.ts` (writes the tamper-evidence `anchor.json`; freshness gate `pnpm run check:dicts` regenerates and diffs — same pattern as AliothStudio's `fk_index.rs`).
- `packages/alioth/env-alioth/` — `ctx.aliothEnv` service: model snapshot pull (default `github:CosmicTools9/AppCreator@main`, cached per SHA under `dataRoot`; or local path), embedded PostgreSQL 18 auto-provision when no `databaseUrl`, `isahl_meta` bootstrap (schema-first; baseline is load-once, non-idempotent), provenance stamp in `dsh_alioth.model_state` (drift reported, never auto-migrated), `sql()` query surface, `resetRegistry()` (drops registry schemas, re-bootstraps on next `ready()`; handle is reused, cluster stays up), read-only `doctor()`. Config `databaseUrl`/`modelSource`/`dataRoot`, env `ALIOTH_DATABASE_URL`/`ALIOTH_MODEL_SOURCE`/`ALIOTH_DATA_ROOT`. `mise run alioth:doctor` (exit 0 = green; `--reset` flag re-bootstraps). Network tests gated by `DSH_ALIOTH_NETWORK_TESTS=1`. Gap: public tree ships no `Pre-Proc/Alioth/_schema/*.schema.json` — contracts are hand-written in gen-alioth.
- `packages/alioth/tool-alioth-workflow/` — model-facing AppAgent workflow bridge: `alioth_workflow_step` (current track/step instruction + tools + gates from the snapshot's skill-adapter), `alioth_workflow_complete` (runs gates — artifact globs on disk + program gates via the bun runner — advances the deterministic state machine, persists run state under `<dataRoot>/workflows/`). Config: `preProcRoot` (gate glob root), `adapter` (default alioth-app.yaml), `workflowRoot` (optional override).
- `packages/alioth/bundle-alioth/` — the Alioth plugin group: one bundle (`cordis.patch.yml` via `dsh.bundle.patch`) mounting the full capability set (env + 4 tool plugins + auth trio + persona). Apply: `dsh --profile headless --patch packages/alioth/bundle-alioth/cordis.patch.yml "<task>"`. Structure roles: **plugins** (env-alioth service, tool-alioth, tool-alioth-meta, tool-alioth-workflow, tool-alioth-orchestrator, landing-alioth, auth-alioth capability, auth-web-alioth carrier, billing-alioth capability, billing-web-alioth user-center carrier, feedback-alioth capability, feedback-web-alioth carrier, tool-feedback-alioth tools) + **libraries** (gen-alioth contracts/generators, skill-alioth adapter/validator — consumed by plugins, never mounted). Auth 按能力/载体拆分（harness 规范）: `landing-alioth` = 落地页（`ctx.aliothLanding` service + `/landing` 路由）; `auth-alioth` = 认证能力（`ctx.aliothAuth` + tools/pre-execute / agent/pre-step 守卫，无 HTTP）; `auth-web-alioth` = B/S 载体（:3900 独立服务器 + webServer 同源路由/门禁/cookie + client 面用户徽章）。
- Commands: `pnpm install` / `test` / `test:coverage` / `typecheck` / `lint` / `lint:fix`（oxlint `--fix --fix-suggestions`；`lint` 与 pre-commit hook 均带 `--deny-warnings`，警告即红）+ gate checks `check:strip-only` / `check:vendor` / `check:versions` / `check:dicts` / `check:tree-assembly` (one-shot: `mise run gates`); `mise run launch` (web GUI, port 3100, `DSH_WEB_PORT`/`DSH_OPEN`), `mise run dev` (headless one-shot; needs `DEEPSEEK_API_KEY`), `mise run alioth:network-tests` (runs the suite with `DSH_ALIOTH_NETWORK_TESTS=1` — needs the local dev DB).
- Profile overlays: `--patch` top-level rows REPLACE existing rows (ids must match the base tree, e.g. `system-prompt` persona); new plugins go in an `insert:` block — rows with unmatched ids are silently ignored. Tree assembly over ALL shipped compositions (bundle headless, example headless, example web) is gated: `pnpm run check:tree-assembly` asserts plugin rows + persona + zero warnings.
- Running the composed profiles needs the workspace packages in DSH's profile fallback: `bash scripts/link-dsh-profiles.sh` (symlinks `~/.dsh/profiles/node_modules/@dsh-alioth/*`, all 16 packages incl. the auth trio + billing duo + feedback trio; heal does not remove manual links). Package sources must be Node strip-only compatible — no TypeScript parameter properties (`constructor(private …)`), no enums, no runtime namespaces — the dsh loader runs `.ts` entries through Node's native TS strip; ENFORCED by `pnpm run check:strip-only` (TS-AST scan) in CI + pre-commit.
- **Frozen-model positioning (2026-08-18, updated)**: the model distribution channel is the Alioth model repository — `github:CosmicTools9/Alioth` (or local `../Alioth`), versioned dirs (`v10.0.0/` + `latest.json`), MIT licensed (宇器科技 2025). The plugin vendors the consumption-side artifacts (`packages/alioth/env-alioth/vendor/`: isahl_meta DDL baseline, skill-adapters, prototype build scripts — Apache-2.0 from the AppCreator distribution) and defaults `modelSource: 'builtin'` — zero-network first install; `github:CosmicTools9/Alioth` / local paths remain overrides. The semantic-mapping library (`skill-alioth/src/data/`) is generated OFFLINE by `scripts/generate-semantic-dicts.ts` from the Alioth repo (coordinates/physical tables) + vendored isahl_meta seed (FK index) — no dev-DB access; rebuild with `ALIOTH_REPO=... node --import tsx scripts/generate-semantic-dicts.ts`. Model evolution = new plugin releases. Pure-consumer rules unchanged; shell reference assets (closed-source-only) replaced by in-repo equivalents (planned).
- Next: in-repo shell reference equivalents (gateway-shell/prototype-base), prototype build gate end-to-end, semantic-model release asset.
- Known LSP noise: `cordis.patch.yml` files report yaml-schema JSONPatch errors + unresolved `!!js` tags — dsh's patch format (id-match rows, insert blocks, JS-tag config) is valid and verified by `--dump-config`; harness has no suppression either. Ignore.
- Pipeline: the complete AppAgent flow is implemented in TS as a deterministic 9-stage machine aligned with the ACTIVE AliothStudio Meta app-agent (NOT the frozen vendor copy — the active line added AppCreation stage 0, E2EVerification retry ≤3, PipelineAdvance gate sweep over the 7 StageIds, PipelineGateAwaiting). 2026-08-25 对齐（remove-appagent-hollow-analysis-stages）: Meta 废弃三分析状态（SemanticAnalysis/FunctionDecomposition/OntologyAnalysis——关键词空壳+透传，legacy passthrough 直跳 Planning，真实分析 = Planning LLM 本体输出）; dsh-alioth 保留三阶段（有真实确定性工作：semantic audit 确认 / registry grounding / 实体注册），管线序 serde wire 兼容。产物契约对齐: module.json status 枚举 = Meta ModuleStatus 全集（active/inactive/beta/planned/developing，draft 由 Meta 归一 planned），生成器默认 category=business/status=planned; extensions 骨架顶层数组（Gateway ExtensionLoader `Vec<T>` 契约，`{}` map 包装会崩启动）。 Contracts: skill-alioth `agent-contract.ts` (serde-alias wire compat), `agent-machine.ts` (pure state machine), `tool-alioth-orchestrator/src/primitives.ts` (real tool bindings through `ctx.tools.execute`; app-creation preflights entity validation to keep PTC atomicity — nothing written before entity checks pass). `alioth_app_create` drives the full pipeline; semantic alignment stays a dialogue precondition (the only LLM seam).
- Pipeline（2026-09-03 对齐，dialog-loop refactor）: Meta 拆除旧会话状态机（orchestrator/state 整拆，会话收敛为对话循环，turn state 由消息推导——业务状态枚举不再存在）。dsh-alioth 保留本地确定性 9 阶段机（PTC 定位不变），合同重锚定到幸存 wire 面：`FlowPlan` 扩展规划字段（semantic_concepts/computations/constraints/business_rules/app_meta/core_constraints）、7 个 StageId 不变、skill Track/Step/Gate schema 对齐（StepGate 增 `expected_exit_code`/`timeout_sec`，Step 增 `reference_paths`/`inputs`，废弃 `outputs` 迁移为 output-glob 门）、GateResult 三态（pass/not-attempted/fail）+ GateErrorKind 四分类（决定 LLM 可修复性）、E2E 阶段产出上游 `e2e-report.json` 证据（orchestrator `preProcRoot` 可选 Config，默认 ALIOTH_PRE_PROC_ROOT 约定）、workflow 桥 step 载荷带 referencePaths/inputs（引擎注入，消除模型探索）、vendored skill-adapters 同步为 dialog-loop 时代 v2 轨道（alioth-ontology/spec-audit adapter 上游已删）。
- **Framework 同步（真相源 = AliothStudio）**: dsh-alioth vendor 的框架代码（design references、prototype 工具链、build/check 脚本、skill-adapters、Framework/backend crates——生成的 Sources path-dep 依赖它们，vendor 后任意部署可 cargo 解析）一律以 AliothStudio 工作仓为真相源，定期 `ALIOTH_STUDIO_ROOT=../AliothStudio pnpm run sync:framework` 同步 + `check:vendor --update` 刷新哈希；`sync:framework --check` 为本地新鲜度门（逐文件 sha256、补丁豁免——本地对 prototype-tool.js 的 PROTOTYPE_TOOL_ROOT 覆写自动重应用且不计漂移）。CI 不跑此门（真相源在工作仓，不在公共克隆）。
- **Stack alignment with AliothStudio (verified live)**: PostgreSQL 18.6 — container runs PGDG postgresql-18 (embedded-postgres npm tops out at 18.4, kept for host dev), Node 24.20 (`node:24.20-slim`), bun 1.3.14 (pinned), Rust 1.96/edition 2021 (declared; dsh-alioth has no Rust runtime components — prototype gates are the bun scripts).
- **Host toolchain single-sourcing (2026-08-20, updated 2026-08-29; environment-consistency policy)**: repo `.mise.toml` pins `node = "24.20.0"` — overriding the global mise default so host dev matches the container image exactly; `mise install` in the repo is the one-command rebuild. pnpm is enforced to `packageManager: pnpm@11.24.0` (corepack in Docker/CI) — deliberately matching AliothStudio's own pin; on the host the mise global shim serves the same 11.24.0 (`mise use -g npm:pnpm@11.24.0`, restored 2026-08-29 after the brew fallback copy disappeared and left every shell with an unset mise shim), so every pnpm resolution on this machine except the harness repo's internal 11.7 yields 11.24.0 (do NOT add pnpm via this repo's mise config — a second repo-level copy only creates drift). bun stays OUT of mise deliberately: it is a per-environment deployment dependency (brew on mac host, `npm i -g bun@1.3.14` in Docker, version truth = Dockerfile) — as a mise tool its GitHub-release download (>5min on this network) would block every `mise install`/`mise exec`/`mise run`. Same policy applies to future tooling: one channel per tool per environment, pinned at the narrowest config that still covers every consumer.
- **Host harness source dependency (2026-09-04; no npm harness releases)**: `@deepseek-ai/*` devDependencies pin `0.1.3-alpha.1` and resolve to the sibling `../deepseek-harness` checkout through `pnpm-workspace.yaml` (same pattern as the harness workspace itself including `../dsh-chess`). The checkout must be built first (`pnpm run build:lib:host && pnpm run build:lib:client` in deepseek-harness; watch for stray no-`package.json` dirs under `packages/*/*` that make tsdown read the root manifest and fail entry resolution, and for the unapproved `@deepseek-ai/dsh-subprocess-local` build script — approve it in the harness `pnpm-workspace.yaml`). CI/Docker clone the harness at tag `dsh-v0.1.3-alpha.1` and build it before installing this workspace. Without the sibling checkout, `pnpm install` fails (the pinned version is not on npm) — that is intentional: adapt against source, never against an older registry line.
- Docker delivery: `Dockerfile` ships the group as a runnable container (node:24.20-slim + bun + embedded PG 18 + builtin model; non-root `USER node`; en_US.UTF-8 locale required by embedded-postgres). Build `docker build -t dsh-alioth .`; run `docker run --rm -p 3100:3100 -e DEEPSEEK_API_KEY=... -v alioth-data:/data dsh-alioth`; keyless self-check `docker run --rm --entrypoint /app/scripts/docker-check.sh dsh-alioth` (composition smoke + doctor; semantic-index red until first rebuild is expected). Docker build + self-check run as a CI job (`.github/workflows/docker.yml`).
- Verification matrix: (0) CI gate matrix (`.github/workflows/ci.yml`) — typecheck (packages+scripts+tests) · lint · tests + coverage thresholds (80/70/80/80 ratchet) · knip · strip-only · vendor provenance · version sync · dict freshness (clones the Alioth repo) · tree assembly · composition smoke · commitlint · audit (moderate floor; overrides in `pnpm-workspace.yaml`) · shellcheck · gitleaks; (0b) L2 agent auto-repair (`.github/workflows/agent-fix.yml`) — on `ci` failure for same-repo PRs: L0 `lint:fix` → dsh headless agent 修复残余语义错误（prompt 带失败日志，禁止改门禁/CI 配置）→ 本地 quick gates 验证 → 推送修复 commit 回 PR 分支（`[agent-fix]` 标记，每分支上限 3 次；需 repo secret `DEEPSEEK_API_KEY`，缺失则 no-op）； (1) per-plugin unit/integration — `pnpm run test` (230 tests, real embedded PG; network-gated behind `DSH_ALIOTH_NETWORK_TESTS=1`, run via `mise run alioth:network-tests`); (1b) model-visible surface keyless snapshot — `tests/model-surface.spec.ts` pins all 18 tool JSON schemas to `tests/__snapshots__/model-surface.json` (refresh: `UPDATE_SNAPSHOTS=1 pnpm exec vitest run tests/model-surface.spec.ts`); (2) composition — `node --import tsx scripts/smoke-composition.ts` mounts the full group on a real Context: 18 tools registered, builtin env ready (zero network), schema_info round-trip, doctor core green; (3) tree assembly — `pnpm run check:tree-assembly` runs `--dump-config` over ALL shipped compositions (0 warnings, rows + persona asserted); (4) real dialogue e2e — `mise run dev` with a real key (semantic → entity → app → workflow → inspect); (5) manual acceptance items — prototype build chain (bun), AliothStudio import, web-profile approval.

### Licensing

- Apache-2.0, following AppCreator's license (`LICENSE`, `NOTICE`; every manifest declares `"license": "Apache-2.0"` — gated by `pnpm run check:versions` together with single-version sync across all 11 packages).
- dsh-alioth is a sibling consumer of the Alioth model (as is AppCreator); never consumes AppCreator's products. Pulls model artifacts only — `isahl_meta` baseline, skill-adapters, version anchor; the `*isahl_meta*` filename filter excludes the rest.
- Model distribution: `github:CosmicTools9/Alioth` (public, verified reachable; cloned by the CI dicts-freshness job); do not re-route to the internal AliothStudio origin (no LICENSE).
- Vendored artifacts (`env-alioth/vendor/`) ship with in-tree `LICENSE` + `NOTICE` (The Alioth Authors, Apache-2.0) and `PROVENANCE.json` sha256 manifest — gated by `pnpm run check:vendor` (refresh: `check:vendor --update` after a vendor change). Upstream gaps (public tree missing LICENSE): authoritative Apache-2.0 prevails; raise upstream.

### Repo conventions (enforced)

- **Pre-commit (lefthook)**: staged-file oxlint（`--fix --fix-suggestions` 可修复项自动落地并由 `stage_fixed` 重暂存；`--deny-warnings` 使修复后残留的警告/错误 fail commit）+ strip-only + versions + vendor checks; `pnpm install` auto-installs hooks (`prepare` script — skips cleanly in git-less build stages like the Docker build stage). 手动全量修复：`pnpm run lint:fix`。Typecheck/tests stay in CI + `mise run gates`.
- **Registry**: `.npmrc` pins `registry=https://registry.npmjs.org/` — npmmirror lags on `@embedded-postgres` betas (404s); CI runners default to npmjs anyway. `engine-strict=false` explicit.
- **Typecheck scope**: `tsconfig.json` covers `packages/**/src`, `packages/**/tests`, `scripts`, root `tests` — gate scripts are compiled code too.
- **Coverage floor** (vitest v8): statements/functions/lines 80, branches 70 — ratchet up only.
- **Dependency hygiene**: knip (config `knip.json`) + frozen-lockfile CI installs + `pnpm audit --audit-level=moderate` + security floors as `overrides` in `pnpm-workspace.yaml` (adm-zip ≥0.6.0, sharp ≥0.35.0, yaml ≥2.8.3).
- **Docs**: README is the entry point; CHANGELOG.md records releases; PR/issue templates + CODEOWNERS under `.github/`. AGENTS.md numbers (test counts, dict sizes) must be refreshed in the same PR that changes them.

## Architecture principles

- **B/S 交付与用户隔离（auth 三插件：landing-alioth / auth-alioth / auth-web-alioth）**: the plugin group ships as a browser-accessible B/S deployment, decomposed per the harness capability/carrier norm — landing-alioth (落地页能力: `ctx.aliothLanding` + `/landing`), auth-alioth (认证能力: `ctx.aliothAuth`, scrypt passwords, bearer sessions in `dsh_alioth_auth` — SEPARATE from the registry so `resetRegistry()` never wipes users; `tools/pre-execute` namespace guard + `agent/pre-step` enforce guard; **工作区能力**: 模式判定 `ALIOTH_WORKSPACE_MODE` > Config.workspaceMode > 默认 standard——只有 `unlimited` 放开「自定义工作区」（workspaces() 对每个用户列出全部 namespace + 路径），standard 固定用户自己的 namespace；注册时无条件自动为用户创建同名 namespace 工作区（AliothStudio 路径结构 `Pre-Proc/{namespace}/` + `Deploy/{namespace}/`，roots 默认 `ALIOTH_PRE_PROC_ROOT`/`ALIOTH_DEPLOY_ROOT` ?? `~/.dsh-alioth/{Pre-Proc,Deploy}`——部署自有目录，**绝不默认牵扯 AliothStudio checkout**（要往 AliothStudio 写产物必须显式设 `ALIOTH_PRE_PROC_ROOT`）；`ctx.aliothAuth.ensureWorkspace/createWorkspace/workspaces` 幂等 + createWorkspace 仅 unlimited 开放（U- 前缀保留给用户工作区），自动建 `Pre-Proc/{ns}/`+`Deploy/{ns}/` + 命名空间模式防穿越，standard 下 user 只见自己 namespace、admin 跨全部）), auth-web-alioth (载体: `POST /api/auth/register|login|logout|bind`, `GET /api/auth/me`（含 `workspaceMode` 供客户端徽章切换入口）, `GET /api/workspace` + `/workspace` 页面（unlimited 呈现「工作区」——列出全部 namespace（AppAgent 档）；standard 呈现「应用」——锁当前用户 namespace 的应用列表（AppCreator 档，人人平等、无超管）; standalone :3900 server + webServer 同源路由; login sets HttpOnly `alioth_session` + marker `alioth_user` cookies; 徽章 chip 带「工作区/应用」入口). Each user owns an Alioth namespace `U-<username>` (no super-admin; every registered user is equal — no super-admin exists (workspaces/guard lock every account to its own `U-<username>` namespace)). `mode: 'enforce'` requires an authenticated session for every namespace-scoped call (deployment override `ALIOTH_AUTH_MODE=enforce` for B/S production); `open` keeps headless deployments working. Browser form posts (urlencoded) get styled HTML; JSON clients get JSON. Single shared workspace: isolation is namespace-level, not filesystem-level. **Web gate（:3100 登录拦截）**: auth-web-alioth 经 `ctx.inject(['webServer'])` 延迟挂载同源认证面（`/login`、`/register`、prefix `/api/auth/*`，longest-prefix-wins 胜过 client-connection 的 `/api`）+ tapIndex 门禁脚本（无 `alioth_user` cookie → `location.replace(aliothLanding.path)`；包裹 fetch 嗅探 `/api/sessions.create` 响应 → `POST /api/auth/bind` 完成 session 绑定——harness API 是 in-process，HTTP 身份到不了 tool 执行，只有 session binding 携带身份）。`agent/pre-step`（enforce）reject 未绑定 session 的步。Config `webGate: false` 关闭挂载（默认 true；headless 无 webServer 自动不生效）。**Client 面**: `auth-web-alioth/lib/client.js` 手写 closure-factory 工件（harness clientBundle 预设未发布，本模块零打包），经 `dsh.client` 声明组进 `__DSH_BOOT__`；用户徽章注册进 `shell.overlay` list 槽（必须 `ctx.slots.inject` 延迟注册，直接 register 与 ui-layout 声明赛跑）。`.gitignore` 对该 lib/ 有例外。
- **用户中心（billing 双插件：billing-alioth / billing-web-alioth）**: 订阅（L0/L1）、账单（按月生成）、发票（自助：申请即开具——无管理员审核，2026-09-05 去超管后）按能力/载体拆分 — billing-alioth 定义 `ctx.aliothBilling` 契约 + **过渡内存实现**（用户决策 2026-08-21：不做 DB 建模，正式计费后端在同名接口后替换 provider；支付无外部渠道，账单 unpaid→paid 走明示的线下确认）；billing-web-alioth 挂 `/usercenter{,/subscription,/bills,/invoices}` 页面 + `/api/billing/overview|subscribe|cancel|pay|invoice|issue`（cookie 鉴权，未登录弹 /login；表单流 302 回对应 tab 带 notice/error）。入口：徽章「用户中心」链接。定价常量 L1=139900 分（¥1,399/月）。
- **页面批注闭环（feedback 三插件，移植 AliothStudio scripts/feedback）**: feedback-alioth（能力：`ctx.aliothFeedback` 批注存储 + pending⇄acknowledged→resolved/dismissed 状态机 + long-poll watch；node:sqlite 持久化 `~/.dsh-alioth/feedback.db`，Config.dbPath 覆盖）；feedback-web-alioth（载体：独立 HTTP :14747 回环——CORS allowlist 浏览器写入、loopback 消费者端点、`/feedback` 书签页 + `overlay.js` 注入脚本，Alt+点击元素留言）；tool-feedback-alioth（模型面工具 `alioth_feedback_pending|ack|resolve|dismiss` 消费闭环）。信任边界同原作：写入需 allowlist Origin，`Origin:null` 需 `allowNullOrigin` opt-in。同机与 AliothStudio 自家 dev server 共存：bundle 钉 14748。
- **程序化生成优先（alioth-* 技能硬性规范；2026-09-03 修订——全栈面）**: 契约产物（app.json/module.json/extensions/entity 行）一律程序化生成（gen-alioth 生成器 + 契约门工具 `alioth_app_write` / `alioth_app_configure` / `alioth_entity_write`），NEVER free-text LLM 生成——LLM 对契约只供结构化参数与语义决策。**`Sources/` 与 `Prototypes/` 代码文件是明示例外**：模型在 workflow 步骤白名单内用 harness `write` 编写代码（`write_file` 映射到 write 面），由程序化门禁验收（bun prototype build / nav 校验 / cargo check）——门禁失败即拒绝，不经映射层拦截。LLM seams 共两个：semantic alignment（semantic_search hits → model decision）+ gated code authoring（步骤白名单内的代码编写）。Text prompt → LLM → **契约**产物 content 仍是 spec violation。
- **Deterministic main pipeline, zero LLM inside**: track/step state machine, gate checks, artifact generation (gen-alioth), entity validation and registration (entity-validate / alioth_entity_write), and semantic retrieval are deterministic code — no LLM calls inside them. LLM involvement lives harness-side: dialogue driving tool calls, semantic alignment, and gated code authoring in workflow steps (see the amended 程序化生成优先 rule).
- **Semantic alignment served by embedding first**: `alioth_schema_semantic_search` covers synonyms + cross-language deterministically (transformers.js + Xenova/bge-small-zh-v1.5, 512 dims); the model decides on top of the hits. Complex multi-concept alignment may use LLM reasoning — that is the sanctioned LLM seam.
- **Semantic space is a maintainable artifact**: index cached under `<dataRoot>/semantic/`, auto-invalidated by entries hash; explicit rebuild via `mise run alioth:rebuild-semantic` (force re-embed; ~11s for the seeded registry). Model upgrades (resetRegistry → re-bootstrap) change entries → next search auto-rebuilds. Inference backend stays onnxruntime-node CPU (XNNPACK) — decided, no ANE/CoreML work (bge-small is fast enough at 3ms/entry batch; ANE would need a Python service + static-fp16 CoreML conversion; if the model grows to large/m3 or search becomes hot-path, revisit with the omp-ane-embedding route). The semantic library = dict snapshots (in-repo) + model files (release asset, offline via `DSH_EMBEDDING_MODEL` local path; default hf-mirror download; no-model degrades to literal search).
- **PTC (Programmatic Tool Calling) landing**: `alioth_app_create` (@dsh-alioth/tool-alioth-orchestrator) is the programmatic pipeline — fixed sequence (validate → entity_write → app_write → inspect verify), every step through `ctx.tools.execute` so approvals/gates/session log apply identically to model calls. Semantic alignment is a PRE-condition done in dialogue (semantic_search + model decision) and passed in as parameters; the orchestrator makes zero LLM calls. Failure is atomic: a failed entity write aborts before any artifact is written. Optional `Config.adapter` stitches the AppAgent workflow gate into the pipeline (create → first-step gate; gate failure keeps artifacts, re-run workflow_complete after fixing).
- **Operational boundaries (known, documented)**: (1) approval — `approvalMode: required` needs a composed ApprovalService with a UI answerer (web profile); headless deployments must use `bypass`. (2) program gates — the bun prototype-tool build chain is a manual acceptance item (needs the real model tree + concrete artifacts); automated tests cover the runner mechanics only. (3) external DB access — `probe-semantic.ts` / `generate-dicts.sh` need a local dev DB (`DATABASE_URL` / first arg override, default `postgres://isahl@localhost/aliothstudio_dev`).

## Development Commands

### Harness (run in `deepseek-harness`)

```sh
pnpm install                  # pnpm workspaces; Node ^22.19 || >=24
pnpm run test                 # vitest unit tests
pnpm run test:coverage        # CI coverage gate: per-file 100% on packages/*/*/src
pnpm run test:e2e             # real-API tests; self-skip without DEEPSEEK_API_KEY
pnpm run test:snapshot        # keyless ACP/headless replay vs expected outputs
pnpm run typecheck            # tsc -b host + client faces
pnpm run lint                 # oxlint
pnpm run build                # tsc emits lib/, tsdown bundles runtime
pnpm run hygiene              # knip + publint + constraints + invariants
pnpm run check:all            # all repo gates (scripts/run-gates.ts)
pnpm dsh --profile headless "task"   # run one task from source (needs key)
pnpm run mock:llm             # local LLM mock server (keyless dev)
```

### Alioth (run in `AliothStudio`)

```sh
bash scripts/db/reset-db.sh --dev
cd Meta/backend && mise run dev        # Meta backend; frontend: cd Meta/frontend && mise run dev
cd Gateway/backend && mise run dev     # Gateway backend; frontend likewise
bash scripts/test-all.sh               # or: mise test — full test suite
cd Meta/backend && mise run schema-info -- list-tables
cd Meta/backend && mise run schema-info -- describe-table <t>
cd Meta/backend && mise run schema-info -- relations <t>      # also: leafs-of, parents-of, resolve-coordinate --dim <scene|factor|function>
```

## Code Conventions & Common Patterns

### Harness

- Every package is `@deepseek-ai/dsh-<name>`; `@deepseek-ai/cordis` is a peerDependency of every package; **ESM everywhere** (`"type": "module"`), `.ts` in local relative imports.
- Plugin anatomy — copy `packages/todo/tool-todo/`:

```ts
export const name = 'tool-<x>'
export const inject = ['tools']
export interface Config { ... }                    // validated deployment choices
export const Config = z.object({ ... })            // schemastery
export function apply(ctx: Context, config: Config): void {
  ctx.tools.register(defineTool({ name, description, parameters, output, execute, presentCall }))
}
```

- Registrations are effects (`ctx.effect()`/`ctx.on()`); waterfall listeners MUST call `next()`. No hardcoded tunables — deployment choices are validated `Config` fields from cordis.yml; misconfiguration fails loud.
- `strict: true` + `noImplicitAny`; opaque cross-boundary ids are branded (`Branded<B>` from `dsh-brand`), never bare `string`; closed unions end in `assertNever`; trust TypeScript at typed same-process boundaries (validate only at parser/config, wire, durable, process boundaries).
- Tool design: `presentCall` UI render intent (`generic`/`terminal`/`diff`) decided up front; description instructs the model (see the `todo_write` example).
- Non-trivial changes include an Agent Note in the same PR; tests describe behavior, not correctness; docs/JSDoc accompany code changes.

### Alioth

- **Language**: Chinese default for dialogue, commits, comments, docs; code identifiers English.
- **Naming**: L1 DB physical columns `fk_*`/`qk_*`/`sk_*`/`notice`/`code`; L2 DTO fields carry business semantics (`notice`→`name`); L3 frontend model = DTO 1:1.
- `qk_*` scalar references are `bigint` (Rust `Option<i64>`) — never `DateTime`/`Decimal`/`String`; resolve actual values via `list_refs`/`get_refs`.
- Structured formats (CSS/HTML/JS/JSON/YAML/TOML/SQL) MUST use dedicated parsers (`scripts/parser-utils.mjs` / `scripts/lib/parsers.ts`) — regex-as-parser is forbidden (`docs/specs/NO_REGEX_FOR_PARSING.md`); reuse existing implementations before writing (四查: spec index / scripts / DB facilities / public crates).
- Backend skeleton: `handlers/` + `models/` + `repositories/` + `services/` (see `BACKEND_FRAMEWORK.md`); `decimal` over `double`; frontend state via Jotai v2 (no Zustand/Redux/Recoil); module-local auth middleware forbidden (Gateway owns auth).
- DB is schema truth: `mise run schema-info` for everything; never read DDL files; `isahl` gets views/functions only.

## Important Files

### Harness

- `AGENTS.md` (root standing orders), `docs/AGENTS.md` (doc standards), `docs/architecture.md` (read before touching `packages/`), `packages/README.md` (package map), `docs/testing.md`, `docs/cordis-primer.md`.
- Cookbook: `docs/cookbook/adding-a-tool.md`, `docs/cookbook/adding-a-package.md`, `docs/cookbook/extension-cookbook.md`.
- Reference tool + tests: `packages/todo/tool-todo/src/index.ts`, `packages/todo/tool-todo/tests/tool-todo.spec.ts`.
- Configs: `package.json` (scripts), `pnpm-workspace.yaml`, `tsconfig.host.json`/`tsconfig.client.json` (compiler faces), `tsdown.config.ts`, `vitest*.config.ts`, `.oxlintrc.json`, `lefthook.yml`, `knip.json`.

### Alioth

- `AGENTS.md`, `.agents/critical-rules.md` (hard gates), `.agents/instructions.md` (task → spec/skill loading table).
- Version truth: `Meta/backend/alioth-gen/src/lib.rs`. Ontology: `docs/specs/ALIOTH_ONTOLOGY_SPEC.md`, `ONTOLOGY_REFERENCE.md`; DTO: `DTO_DESIGN_SPEC.md`; backend: `BACKEND_FRAMEWORK.md`; app format: `META_AI_SPEC.md` (+ `BLOCK_SCHEMA.md`, `SERVICE_SPEC.md`, `APP_EXTENSION.md`).
- Concrete artifacts: `Pre-Proc/Alioth/Apps/ai-i-need-a/app.json`, `AppCreator/DESIGN_INTENT.md`, `AppCreator/Pre-Proc/Alioth/` (generated output), `skill-adapters/alioth-app.yaml` (Track/Step/gate shape), `Meta/backend/app-agent/` (pipeline).

## Runtime/Tooling Preferences

- **Harness**: Node `^22.19.0 || >=24.0.0`; pnpm `11.7.0` (workspaces); ESM-only; tsx for source launch (`node --import tsx/esm`); build = `tsc -b` + `tsdown`; lint = oxlint; git hooks = lefthook; `DEEPSEEK_API_KEY` required only for e2e/demos/snapshot re-record (keyless replay works without it).
- **Alioth**: mise (Rust backend dev, `mise run dev`), bun (audit/check scripts — new `scripts/check/`/`scripts/ts/` scripts default to bun, Python is legacy), pnpm (frontend workspaces), PostgreSQL with four-layer isolation (`aliothstudio` prod ← `aliothstudio_pre` → `aliothstudio_dev` → `aliothstudio_test`; `DATABASE_URL` must match the intended layer).
- **dsh-alioth plugin**: runs on the harness (Node/pnpm); talking to Alioth means invoking the Rust pipeline (`app-agent`/`alioth-gen`) and/or connecting to the correct PostgreSQL layer.

## Testing & QA

### Harness

- vitest, files named `*.spec.ts` under `tests/` (or colocated), run via `pnpm run test`; **coverage gate is `test:coverage`** — per-file 100% on `packages/*/*/src` (CI enforces).
- Unit pattern (from `tool-todo.spec.ts`): mount the REAL plugin on a real `Context` (+ `SystemPrompt`, `ToolRuntime`, optionally `Loader`), drive through `ctx.tools.execute` with a fake parent `Agent` backed by a real `Session` — only the agent wrapper is a stand-in.
- e2e (`test:e2e`) hits real APIs and self-skips without `DEEPSEEK_API_KEY`; keyless snapshots (`test:snapshot`) replay expected outputs; re-record with `pnpm run test:snapshot:record` (needs key); `pnpm run mock:llm` for local keyless dev.
- Every non-trivial model-visible behavior change adds/updates a keyless snapshot through a real runnable example in the same PR.

### Alioth

- `bash scripts/test-all.sh` (or `mise test`); Rust: `#[tokio::test]` + `connect_test_db()` — `#[sqlx::test]` is forbidden; framework/CRUD crates need unit tests, Service backends need integration tests.
- Frontend is verified against the real backend API (mock data forbidden for integration verification); visual verification via `bun scripts/check/check-visual-verify.ts <prototype>`; full policy in `docs/specs/TEST_INFRASTRUCTURE.md`, `E2E_SPEC.md`.
- Before commit: `npx tsc --noEmit` (TS) + `cargo check` + `clippy` (Rust); conventional commits.
