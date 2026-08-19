#!/bin/sh
# dsh-alioth container entry point.
# Default: launch the web GUI with the Alioth plugin group mounted.
# `--check`: keyless composition self-check (dump-config + doctor), exit code.
set -eu

PATCH_FILE="${ALIOTH_PATCH:-/app/packages/alioth/bundle-alioth/cordis.patch.yml}"

if [ "${1:-}" = "--check" ]; then
  exec /app/scripts/docker-check.sh
fi

if [ -z "${DEEPSEEK_API_KEY:-}" ]; then
  echo "dsh-alioth: DEEPSEEK_API_KEY is not set — the web UI will refuse model calls." >&2
fi

echo "dsh-alioth: launching web profile with patch ${PATCH_FILE}"
exec /app/node_modules/.bin/dsh --profile web --patch "${PATCH_FILE}"
