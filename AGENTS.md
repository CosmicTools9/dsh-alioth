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

Initial plugin skeleton (verified green): `packages/alioth/tool-alioth/` — `@dsh-alioth/tool-alioth`, the `alioth_app_inspect` tool (reads `app.json` from a configured Pre-Proc root, validates required fields, returns a structured summary). `examples/alioth-agent/cordis.yml` composes it into a headless agent profile. Local commands: `pnpm install` / `pnpm run test` / `pnpm run typecheck` / `pnpm run lint`. Launch via mise: `mise run launch` boots the **web** profile (browser GUI) with `examples/alioth-agent/web.patch.yml` and auto-opens the browser (port 3100 default; `DSH_WEB_PORT`/`DSH_WEB_HOST` override, `DSH_OPEN=0` disables the auto-open); `mise run dev` boots a headless one-shot create-app session. Both need `DEEPSEEK_API_KEY` (composition packages are root devDependencies so the CLI loader resolves them from cwd). Next steps: more Alioth tools (schema-info bridge, AppAgent pipeline invocation), then wiring into a real `dsh` profile run.

Self-bootstrapping environment (verified green against the real model): `packages/alioth/env-alioth/` — `@dsh-alioth/env-alioth`, the `ctx.aliothEnv` service. It pulls the latest Alioth model snapshot (`github:CosmicTools9/AppCreator[@ref]` by default, cached per SHA under the data root, or a local checkout path), auto-provisions an embedded PostgreSQL 18 when no `databaseUrl` is configured (persisted under `<dataRoot>/postgres`, restarted on a fresh port), bootstraps `isahl_meta` per the snapshot's `backend/ddl/*isahl_meta*.sql` baseline (schema-first — the baseline assumes a loader created the schema; non-idempotent, load-once semantics), stamps provenance into private `dsh_alioth.model_state`, and exposes a read-only `doctor()`. Stamp mismatch reports drift, never auto-migrates. Config: `databaseUrl` / `modelSource` / `dataRoot` (default XDG data home + `/dsh-alioth`); env overrides `ALIOTH_DATABASE_URL` / `ALIOTH_MODEL_SOURCE` / `ALIOTH_DATA_ROOT`. `mise run alioth:doctor` (or `tsx scripts/alioth-doctor.ts`) prints the health report, exit 0 = green. Known gap for the next increment: the public AppCreator tree ships no `Pre-Proc/Alioth/_schema/*.schema.json` (0 artifact schemas) — the L2 artifact-contract source needs resolving (main-repo `Pre-Proc` or regenerate). Network-dependent tests are gated behind `DSH_ALIOTH_NETWORK_TESTS=1`.

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
