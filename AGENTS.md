# Repository Guidelines

## Project Overview

`dsh-alioth` is a [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) plugin that leverages the **Alioth V10 data model** to deliver **AppCreator** capabilities: a dialogue-driven enterprise app generator. The agent (running on the harness) turns a user request into AliothStudio-compatible artifacts — `app.json`, `module.json`/`block.json`, `extensions/*.yaml`, HTML prototypes, and backend `Sources/` skeletons — through the Alioth model pipeline (AppAgent state machine + `alioth-gen` + `ontology-mapping`), backed by a PostgreSQL `isahl_meta` schema.

This repo is currently an **empty scaffold**. It is authored against two sibling checkouts used as reference:

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

- `packages/alioth/tool-alioth/` — `alioth_app_inspect` (read/validate `app.json`), `alioth_app_write` (generate + persist contract-validated app tree: app.json, module.json, extensions skeletons, Sources dirs; refuses overwrite; `approvalMode` `required`|`bypass`, default bypass). `examples/alioth-agent/` composes the Alioth overlay for headless + web profiles.
- `packages/alioth/tool-alioth-meta/` — `alioth_schema_info`: read-only registry queries (entities/entity/search-fields) over `ctx.aliothEnv.sql()`; dev-seed test entities (`-testing`/`-test` suffixes) hidden by default, `filteredTesting` count reported, `includeTesting` opt-in. `alioth_schema_semantic_search`: multilingual semantic grounding (synonyms + cross-language, e.g. "inventory balance" hits 状态-库存) via transformers.js + Xenova/bge-small-zh-v1.5 (512 dims); model downloads on first use (endpoint `https://hf-mirror.com`, override `DSH_HF_ENDPOINT`), vectors cached under `<dataRoot>/semantic/` keyed by entries hash; real-model probe: `pnpm exec tsx packages/alioth/tool-alioth-meta/tests/probe-semantic.ts` (needs local dev DB). `alioth_entity_write`: register a new business entity on an existing isahl physical table (isahl forbids CREATE TABLE) — validates via skill-alioth `entity-validate` (naming/physical-table/conflicts/inheritance/references/real coordinate dictionary), then INSERTs meta_collections + meta_fields (config: depth/source dsh-alioth/inherits/category/coordinates; reference_config per the FK-index shape); `approvalMode` required|bypass.
- `packages/alioth/gen-alioth/` — artifact contracts (`src/contracts/*.schema.json`, validated by the dependency-free engine in `validate-engine.ts`) + pure generators (`generateApp`, `generateExtensions`, `sourceModuleDirs`); golden-alignment tests against the `ai-i-need-a` mirror.
- `packages/alioth/skill-alioth/` — adapter orchestration primitives: YAML parsing (`adapter.ts`), track/step state machine (`state.ts`), gate checks (`gates.ts`, output-glob + program hook), file-backed run state (`workspace.ts`), adapter-tool→harness-tool mapping (`mapping.ts`: read_file→read, write_file→write, search_files→glob/grep), external program runner + bun probe (`bun.ts` — the distribution's prototype gates run `bun scripts/prototype-tool.js`; not ported, bun is a declared deployment dependency). Entity write-path validation (`entity-validate.ts`): naming, physical-table existence (isahl forbids CREATE TABLE — entities map onto existing physical tables), conflicts, inheritance (exists/acyclic/depth), reference integrity (target/junction ∈ registry∪physical; local_key ∈ FK index ∪ root columns), coordinates — **no degradation**: scene/factor/function checked against real dictionary snapshots in `src/data/` (`coordinates.json` 651 codes, `fk-index.json` 2479 physical refs, `physical-tables.json` 1045 tables), regenerated from the AliothStudio dev DB via `scripts/generate-dicts.sh` (same pattern as AliothStudio's `fk_index.rs`).
- `packages/alioth/env-alioth/` — `ctx.aliothEnv` service: model snapshot pull (default `github:CosmicTools9/AppCreator@main`, cached per SHA under `dataRoot`; or local path), embedded PostgreSQL 18 auto-provision when no `databaseUrl`, `isahl_meta` bootstrap (schema-first; baseline is load-once, non-idempotent), provenance stamp in `dsh_alioth.model_state` (drift reported, never auto-migrated), `sql()` query surface, `resetRegistry()` (drops registry schemas, re-bootstraps on next `ready()`; handle is reused, cluster stays up), read-only `doctor()`. Config `databaseUrl`/`modelSource`/`dataRoot`, env `ALIOTH_DATABASE_URL`/`ALIOTH_MODEL_SOURCE`/`ALIOTH_DATA_ROOT`. `mise run alioth:doctor` (exit 0 = green; `--reset` flag re-bootstraps). Network tests gated by `DSH_ALIOTH_NETWORK_TESTS=1`. Gap: public tree ships no `Pre-Proc/Alioth/_schema/*.schema.json` — contracts are hand-written in gen-alioth.
- `packages/alioth/tool-alioth-workflow/` — model-facing AppAgent workflow bridge: `alioth_workflow_step` (current track/step instruction + tools + gates from the snapshot's skill-adapter), `alioth_workflow_complete` (runs gates — artifact globs on disk + program gates via the bun runner — advances the deterministic state machine, persists run state under `<dataRoot>/workflows/`). Config: `preProcRoot` (gate glob root), `adapter` (default alioth-app.yaml), `workflowRoot` (optional override).
- `packages/alioth/bundle-alioth/` — the Alioth plugin group: one bundle (`cordis.patch.yml` via `dsh.bundle.patch`) mounting the full capability set (env + 4 tool plugins + persona). Apply: `dsh --profile headless --patch packages/alioth/bundle-alioth/cordis.patch.yml "<task>"`. Structure roles: **plugins** (env-alioth service, tool-alioth, tool-alioth-meta, tool-alioth-workflow, tool-alioth-orchestrator) + **libraries** (gen-alioth contracts/generators, skill-alioth adapter/validator — consumed by plugins, never mounted).
- Commands: `pnpm install` / `test` / `typecheck` / `lint`; `mise run launch` (web GUI, port 3100, `DSH_WEB_PORT`/`DSH_OPEN`), `mise run dev` (headless one-shot; needs `DEEPSEEK_API_KEY`).
- Profile overlays: `--patch` top-level rows REPLACE existing rows (ids must match the base tree, e.g. `system-prompt` persona); new plugins go in an `insert:` block — rows with unmatched ids are silently ignored.
- Running the composed profiles needs the workspace packages in DSH's profile fallback: `bash scripts/link-dsh-profiles.sh` (symlinks `~/.dsh/profiles/node_modules/@dsh-alioth/*`; heal does not remove manual links). Package sources must be Node strip-only compatible — no TypeScript parameter properties (`constructor(private …)`) — the dsh loader runs `.ts` entries through Node's native TS strip.
- **Frozen-model positioning (2026-08-18, updated)**: the model distribution channel is the Alioth model repository — `github:CosmicTools9/Alioth` (or local `../Alioth`), versioned dirs (`v10.0.0/` + `latest.json`), MIT licensed (宇器科技 2025). The plugin vendors the consumption-side artifacts (`packages/alioth/env-alioth/vendor/`: isahl_meta DDL baseline, skill-adapters, prototype build scripts — Apache-2.0 from the AppCreator distribution) and defaults `modelSource: 'builtin'` — zero-network first install; `github:CosmicTools9/Alioth` / local paths remain overrides. The semantic-mapping library (`skill-alioth/src/data/`) is generated OFFLINE by `scripts/generate-semantic-dicts.ts` from the Alioth repo (coordinates/physical tables) + vendored isahl_meta seed (FK index) — no dev-DB access; rebuild with `ALIOTH_REPO=... node --import tsx scripts/generate-semantic-dicts.ts`. Model evolution = new plugin releases. Pure-consumer rules unchanged; shell reference assets (closed-source-only) replaced by in-repo equivalents (planned).
- Next: in-repo shell reference equivalents (gateway-shell/prototype-base), prototype build gate end-to-end, semantic-model release asset.
- Known LSP noise: `cordis.patch.yml` files report yaml-schema JSONPatch errors + unresolved `!!js` tags — dsh's patch format (id-match rows, insert blocks, JS-tag config) is valid and verified by `--dump-config`; harness has no suppression either. Ignore.
- Pipeline: the complete AppAgent flow is implemented in TS as a deterministic 7-stage machine (semantic-analysis → function-decomposition → ontology-analysis → module-creation → block-creation → ontology-transfer → service-api → publishing), data contracts unified with the Meta AppAgent (skill-alioth `agent-contract.ts`, serde-alias compatible with the frozen distribution's `state.rs`; `agent-machine.ts` = pure state machine, `tool-alioth-orchestrator/src/primitives.ts` = real tool bindings through `ctx.tools.execute`). `alioth_app_create` drives the full pipeline; semantic alignment stays a dialogue precondition (the only LLM seam).
- Docker delivery: `Dockerfile` ships the group as a runnable container (node:22.19-slim + bun + embedded PG 18 + builtin model; non-root `USER node`; en_US.UTF-8 locale required by embedded-postgres). Build `docker build -t dsh-alioth .`; run `docker run --rm -p 3100:3100 -e DEEPSEEK_API_KEY=... -v alioth-data:/data dsh-alioth`; keyless self-check `docker run --rm dsh-alioth --check` (composition smoke + doctor; semantic-index red until first rebuild is expected).
- Verification matrix: (1) per-plugin unit/integration — `pnpm run test` (117 tests, real embedded PG; network-gated behind `DSH_ALIOTH_NETWORK_TESTS=1`); (2) composition — `node --import tsx scripts/smoke-composition.ts` mounts the full group on a real Context: 8 tools registered, builtin env ready (zero network), schema_info round-trip, doctor core green; (3) tree assembly — `dsh --profile headless --patch packages/alioth/bundle-alioth/cordis.patch.yml --dump-config` (0 warnings); (4) real dialogue e2e — `mise run dev` with a real key (semantic → entity → app → workflow → inspect); (5) manual acceptance items — prototype build chain (bun), AliothStudio import, web-profile approval.
- Known LSP noise:  files report yaml-schema JSONPatch errors + unresolved  tags — dsh's patch format (id-match rows, insert blocks, JS-tag config) is valid and verified by ; harness has no suppression either. Ignore.

### Licensing

- Apache-2.0, following AppCreator's license (`LICENSE`, `NOTICE`; all manifests declare `"license": "Apache-2.0"`).
- dsh-alioth is a sibling consumer of the Alioth model (as is AppCreator); never consumes AppCreator's products. Pulls model artifacts only — `isahl_meta` baseline, skill-adapters, version anchor; the `*isahl_meta*` filename filter excludes the rest.
- Model distribution stays on `github:CosmicTools9/AppCreator`; do not re-route to the internal AliothStudio origin (no LICENSE).
- Snapshot caches keep upstream license files. Upstream gaps (public tree missing LICENSE, Cargo says MIT): authoritative Apache-2.0 prevails; raise upstream.

## Architecture principles

- **Deterministic main pipeline, zero LLM**: track/step state machine, gate checks, artifact generation (gen-alioth), entity validation and registration (entity-validate / alioth_entity_write), and semantic retrieval are deterministic code — no LLM calls inside them. The only LLM involvement is the harness-side dialogue driving tool calls, and the *semantic alignment* step where the model maps natural-language concepts to registry terms.
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
