# Changelog

All notable changes to this project are documented here. Conventional Commits;
this file records user-visible changes per release.

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
