#!/bin/sh
# Keyless container self-check (entry `--check`):
#   1. composition smoke — full plugin group on a real Context (it asserts the
#      model-facing tool set itself, so the count is never stale here),
#      builtin env ready, schema_info round-trip, doctor core green
#   2. doctor — model snapshot + PostgreSQL 18 (started by docker-entry.sh) + isahl_meta health
# Exits non-zero on any failure. No LLM key, no network needed.
set -eu

echo "== dsh-alioth container self-check =="
echo "--- composition smoke (group mount, tools, round-trip) ---"
node --import tsx /app/scripts/smoke-composition.ts
echo "--- doctor (builtin model, environment PostgreSQL) ---"
ALIOTH_DATA_ROOT="${ALIOTH_DATA_ROOT:-/tmp/alioth-check}" node --import tsx /app/scripts/alioth-doctor.ts
echo "== self-check OK =="
