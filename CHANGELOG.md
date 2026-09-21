# Changelog

All notable changes to this project are documented here. Conventional Commits;
this file records user-visible changes per release.

## [Unreleased]

### Added
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
