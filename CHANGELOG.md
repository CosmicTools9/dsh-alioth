# Changelog

All notable changes to this project are documented here. Conventional Commits;
this file records user-visible changes per release.

## [Unreleased]

### Added
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
- Vendored framework re-synced to the current AliothStudio line (adapters moved
  under `Meta/backend/app-agent/skill-adapters`; new gate programs and the eval
  toolchain synced). Adapter gate scripts *and* their non-script references are
  now checked for reachability at sync time and in CI.
- Semantic dictionaries refreshed against the model release (coordinates 644,
  physical tables 986, FK index 2661); the generator accepts both the versioned
  and the flat model publication layout.
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
- 9-stage AppAgent state machine aligned with the ACTIVE AliothStudio Meta line.
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
