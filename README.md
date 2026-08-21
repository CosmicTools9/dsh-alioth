# dsh-alioth

**Dialogue-driven enterprise app generator for the [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) (dsh), built on the Alioth v10 data model.**

A user describes an app in dialogue; the plugin group registers business entities
in the Alioth entity registry and emits AliothStudio-importable artifacts
(`app.json`, `module.json`, `extensions/`, prototype, `Sources/` skeleton)
through a **deterministic, programmatic pipeline** — the LLM supplies structured
parameters and semantic decisions only, never artifact text.

- Pure consumer of the published Alioth model ([CosmicTools9/Alioth](https://github.com/CosmicTools9/Alioth), Apache-2.0) — never advances it.
- Zero-network first boot: frozen model artifacts vendored, embedded PostgreSQL 18 auto-provisioned.
- B/S deliverable: register/login, per-user `U-<username>` namespace isolation over a shared workspace.

## Packages

| Package | Role |
|---|---|
| `env-alioth` | Model snapshot sync, embedded PG lifecycle, `isahl_meta` bootstrap, doctor |
| `tool-alioth` | `alioth_app_list` / `alioth_app_inspect` / `alioth_app_write` / `alioth_app_configure` / `alioth_app_delete` — contract-validated artifact tools (discover, create, grow, enrich, retire/delete) |
| `tool-alioth-meta` | `alioth_schema_info` / `alioth_schema_semantic_search` / `alioth_entity_write` — registry + embedding search (bge-small-zh, deterministic) |
| `tool-alioth-workflow` | `alioth_workflow_step` / `alioth_workflow_complete` — AppAgent track/step/gate bridge |
| `tool-alioth-orchestrator` | `alioth_app_create` — PTC pipeline: validate → entity → app → verify (atomic, zero LLM) |
| `auth-alioth` | B/S auth + workspace: scrypt passwords, token sessions, namespace guard, env auto-detect, `Pre-Proc/{ns}`+`Deploy/{ns}` workspace bootstrap |
| `skill-alioth` | Adapter state machine, entity-validate (real dictionary data), 9-stage AppAgent machine |
| `gen-alioth` | Artifact JSON-Schema contracts + pure generators |
| `bundle-alioth` | One `cordis.patch.yml` mounting the whole group |

## Quickstart

```sh
pnpm install                 # registry pinned to npmjs (see .npmrc)
pnpm run test                # 152 tests, real embedded PostgreSQL
mise run dev                 # headless dialogue (needs DEEPSEEK_API_KEY)
mise run launch              # web GUI on :3100
mise run alioth:doctor       # environment self-check (exit 0 = green)

dsh --profile headless --patch packages/alioth/bundle-alioth/cordis.patch.yml "<task>"
```

Docker: `docker build -t dsh-alioth . && docker run --rm -p 3100:3100 -e DEEPSEEK_API_KEY=... -v alioth-data:/data dsh-alioth`
(keyless self-check: `docker run --rm --entrypoint /app/scripts/docker-check.sh dsh-alioth`).

## Gates

CI (`.github/workflows/`): typecheck · lint · tests+coverage thresholds · knip ·
strip-only compatibility · vendor provenance · version sync · semantic-dict
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
redistributed with attribution.
