#!/bin/sh
# Keyless container self-check (entry `--check`):
#   1. composition smoke — full plugin group on a real Context (it asserts the
#      model-facing tool set itself, so the count is never stale here),
#      assembled model source ready, schema_info round-trip, doctor core green
#   2. doctor — model snapshot + PostgreSQL 18 (started by docker-entry.sh) + isahl_meta health
# Exits non-zero on any failure. No LLM key, no network needed.
set -eu

echo "== dsh-alioth container self-check =="
echo "--- composition smoke (group mount, tools, round-trip) ---"
node --import tsx /app/scripts/smoke-composition.ts
echo "--- console build artifacts (web face built, not just the libraries) ---"
for f in /deepseek-harness/apps/web/dist/index.html /deepseek-harness/.dsh-build/client-build-environment.json; do
  if [ ! -f "$f" ]; then
    echo "FAIL: $f missing — the console would serve missing/stale client assets (Failed to load plugins)";
    exit 1;
  fi
done
echo "console artifacts OK: $(find /deepseek-harness/apps/web/dist -type f | wc -l | tr -d ' ') files in dist, record commit=$(sed -n 's/.*\"DSH_CLIENT_COMMIT_HASH\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p' /deepseek-harness/.dsh-build/client-build-environment.json | head -n 1)"
echo "--- doctor (assembled fixture model source, environment PostgreSQL) ---"
# `builtin` is retired and a container self-check must stay keyless: assemble a fixture source
# (vendored kit + a tiny registry seed) for the check itself. A real deployment passes its own
# ALIOTH_MODEL_SOURCE (see the Dockerfile header).
node --import tsx /app/scripts/assemble-model-source.ts --out /tmp/alioth-check/model-source --fixture-seed
# A freshly created /data has no semantic index: it rebuilds on the first semantic_search,
# so its absence must not fail the container check. Every other check still gates.
ALIOTH_DATA_ROOT="${ALIOTH_DATA_ROOT:-/tmp/alioth-check}" \
ALIOTH_MODEL_SOURCE=/tmp/alioth-check/model-source \
  node --import tsx /app/scripts/alioth-doctor.ts --allow-unbuilt-semantic-index
echo "== self-check OK =="
