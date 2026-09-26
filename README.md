# dsh-alioth

**Dialogue-driven enterprise app generator for the [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) (dsh), built on the Alioth v10 data model.**

A user describes an app in dialogue; the plugin group registers business entities
in the Alioth entity registry and emits AliothMeta-importable artifacts
(`app.json`, `module.json`, `extensions/`, prototype, `Sources/` skeleton)
through a **deterministic, programmatic pipeline** — the LLM supplies structured
parameters and semantic decisions only, never artifact text.

- Pure consumer of the published Alioth model ([CosmicTools9/Alioth](https://github.com/CosmicTools9/Alioth), Apache-2.0) — never advances it.
- Keyless first boot: the model source is a directory assembled once (`mise run alioth:model-source`, release content + the vendored adapters); PostgreSQL 18 comes from the environment (host server, the container's PGDG build, or the CI service) through `ALIOTH_DATABASE_URL`.
- B/S deliverable: register/login, per-user `U-<username>` namespace isolation over a shared workspace.

## Packages

| Package | Role |
|---|---|
| `env-alioth` | Model snapshot sync, PostgreSQL connection lifecycle, `isahl_meta` bootstrap, doctor |
| `tool-alioth` | `alioth_app_list` / `alioth_app_inspect` / `alioth_app_write` / `alioth_app_configure` / `alioth_app_delete` — contract-validated artifact tools (discover, create, grow, enrich, retire/delete) |
| `tool-alioth-meta` | `alioth_schema_info` / `alioth_schema_semantic_search` / `alioth_entity_write` — registry + embedding search (bge-small-zh, deterministic) |
| `tool-alioth-workflow` | `alioth_workflow_step` / `alioth_workflow_complete` — AppAgent track/step/gate bridge |
| `tool-alioth-orchestrator` | `alioth_app_create` — PTC pipeline: validate → entity → app → verify (atomic, zero LLM); 7 stage gates, publish preconditions, controlled parallel dispatch |
| `tool-alioth-verify` | `alioth_verify` / `alioth_closure` / `alioth_version` / `alioth_patch_assets` / `alioth_capabilities` / `alioth_deferred` / `alioth_usage` — eval report, extension runtime verification, closure verdicts, artifact versioning + rollback, two-stage asset patches, capability advertisement, deferred blockers, usage |
| `guard-alioth` | Execution-surface guard: declared tool surface, write sandbox (`Pre-Proc/{ns}/…` + plan-step write scope), repair wall, turn budget (wall-clock / steps / optional cost), closure-evidence nudge |
| `verify-alioth` | Verification/closure library behind the tools above (extension loader contract, artifact fingerprints, all-or-nothing restore, build-eval scoring, honest `unknown` advertisement) |
| `auth-alioth` | B/S auth + workspace: scrypt passwords, token sessions, namespace guard, workspace mode (unlimited opens 自定义工作区), `Pre-Proc/{ns}`+`Deploy/{ns}` bootstrap |
| `skill-alioth` | Adapter state machine, entity-validate (real dictionary data), 9-stage AppAgent machine |
| `gen-alioth` | Artifact JSON-Schema contracts + pure generators |
| `bundle-alioth` | One `cordis.patch.yml` mounting the whole group |

## Quickstart

```sh
pnpm install                 # registry pinned to npmjs (see .npmrc)
pnpm run test                # full suite (needs PostgreSQL 18 — per-suite throwaway databases); counts live in AGENTS.md's verification matrix
mise run dev                 # headless dialogue (needs DEEPSEEK_API_KEY)
mise run launch              # web GUI on :3100
mise run alioth:doctor       # environment self-check (exit 0 = green)

dsh --profile headless --patch packages/alioth/bundle-alioth/cordis.patch.yml "<task>"
```

Docker: `docker build -t dsh-alioth . && docker run --rm -p 3100:3100 \
  -e DEEPSEEK_API_KEY=... -e ALIOTH_MODEL_SOURCE=/app/model-source -v alioth-data:/data dsh-alioth`
(mount an assembled model source, or pass its path inside the container; the keyless `--check` assembles a fixture itself).
(keyless self-check: `docker run --rm --entrypoint /app/scripts/docker-check.sh dsh-alioth`).

## Registry & database

The model source is **required** (`ALIOTH_MODEL_SOURCE`; the package's frozen `builtin` snapshot was
retired 2026-09-24 — there is no fallback). Assemble one with
`mise run alioth:model-source -- --source <model distribution> --out <dir>`: release content
(version anchor + the derived `isahl_meta-registry.sql` rows) plus the adapters the release does not ship.

The `isahl_meta` registry is bootstrapped from that source only (frozen structure baseline
+ the release's `isahl_meta-registry.sql` rows) into the database `ALIOTH_DATABASE_URL` names.
`mise run launch` and `mise run alioth:doctor` run the **same** bootstrap function:

- missing registry tables → **self-healed** on the first query either way (the bootstrap is lazy, so a
  serving console is not yet a bootstrapped one);
- a registry that is merely *stale* (model evolved) is never re-applied — use `mise run alioth:doctor --reset`;
- a database that **holds the Alioth model** (`regular tables under schema isahl`) is refused on every
  path — point the DSN at the plugin's own database.

Operator-facing details, the release-file contract and the troubleshooting table:
[`docs/registry-bootstrap.md`](docs/registry-bootstrap.md) (Chinese).

## Gates

CI (`.github/workflows/`): typecheck · lint · tests+coverage thresholds · knip ·
strip-only compatibility · vendor compliance (LICENSE/NOTICE) · version sync · semantic-dict
freshness · tree assembly · composition smoke · commitlint · audit · shellcheck ·
gitleaks · docker build+`--check`.

Local:

```sh
pnpm run typecheck  # packages + scripts + tests (strict)
pnpm run lint
pnpm run test:coverage
pnpm run check:strip-only       # Node strip-only compatibility (no parameter properties/enums/namespaces)
pnpm run check:vendor           # vendor LICENSE/NOTICE + sha256 provenance
pnpm run check:versions         # all workspace packages share the root version
pnpm run check:dicts            # semantic dicts anchored + fresh vs the model repo (ALIOTH_REPO)
pnpm run check:tree-assembly    # dsh --dump-config over all shipped compositions
mise run gates                  # everything above in one command
```

Pre-commit (lefthook): staged lint + strip-only + versions + vendor; commit-msg:
commitlint (Conventional Commits, English). Bypass: `git commit --no-verify`.

## License

Apache-2.0 (see `LICENSE`, `NOTICE`). Vendored model artifacts in
`packages/alioth/env-alioth/vendor/` are Apache-2.0 works of The Alioth Authors,
redistributed with attribution. The plugin group runs on the DeepSeek Harness
(MIT — Copyright (c) 2026 DeepSeek), and the container image bundles a built
harness checkout; it ships the license and notice files for both sides. The
aggregate third-party disclosure is `THIRD_PARTY_NOTICES.md`.

Both licenses are permissive — commercial use additionally requires
**authorization from this project's operator** (see `THIRD_PARTY_NOTICES.md`
§ Commercial use).
