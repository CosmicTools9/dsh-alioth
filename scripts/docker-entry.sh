#!/bin/sh
# dsh-alioth container entry point.
# Default: launch the web GUI with the Alioth plugin group mounted.
# `--check`: keyless composition self-check (dump-config + doctor), exit code.
set -eu

PATCH_FILE="${ALIOTH_PATCH:-/app/packages/alioth/bundle-alioth/cordis.patch.yml}"

# Start PostgreSQL 18.6 (PGDG) on /data/pg unless an external database URL is
# given. This is the database env-alioth uses — the plugin never provisions a
# cluster of its own, so a container with neither fails loud.
ensure_pg() {
  if [ -n "${ALIOTH_DATABASE_URL:-}" ]; then
    echo "dsh-alioth: using external database ${ALIOTH_DATABASE_URL%%@*}@${ALIOTH_DATABASE_URL##*@}"
    return
  fi
  PGDATA=/data/pg
  if [ ! -f "${PGDATA}/PG_VERSION" ]; then
    echo "dsh-alioth: initializing PostgreSQL 18.6 at ${PGDATA}"
    printf '%s\n' "${PGPASSWORD}" > /tmp/pgpw
    initdb -D "${PGDATA}" -U alioth --auth=password --pwfile=/tmp/pgpw -E UTF8 --locale=C.UTF-8
    rm -f /tmp/pgpw
  fi
  pg_ctl -D "${PGDATA}" -l /data/pg.log -o "-p 5432 -k /tmp" start
  # The registry database is this deployment's own; it must never be a database that
  # carries the Alioth model (env-alioth refuses that — see packages/alioth/env-alioth/src/bootstrap.ts).
  if ! psql -h 127.0.0.1 -p 5432 -U alioth -d postgres -tAc "SELECT 1 FROM pg_database WHERE datname = 'dsh_alioth'" | grep -q 1; then
    # A volume created before 2026-09-24 named it `alioth` (the naming this contract retired):
    # rename in place so the existing registry and its `dsh_alioth*` schemas survive.
    if psql -h 127.0.0.1 -p 5432 -U alioth -d postgres -tAc "SELECT 1 FROM pg_database WHERE datname = 'alioth'" | grep -q 1; then
      echo "dsh-alioth: renaming legacy database alioth -> dsh_alioth"
      psql -h 127.0.0.1 -p 5432 -U alioth -d postgres -c 'ALTER DATABASE alioth RENAME TO dsh_alioth'
    else
      createdb -h 127.0.0.1 -p 5432 -U alioth dsh_alioth
    fi
  fi
  export ALIOTH_DATABASE_URL="postgres://alioth:${PGPASSWORD}@127.0.0.1:5432/dsh_alioth"
  trap 'pg_ctl -D "${PGDATA}" stop -m fast >/dev/null 2>&1 || true' EXIT
  echo "dsh-alioth: PostgreSQL 18.6 ready at ${ALIOTH_DATABASE_URL}"
}

if [ "${1:-}" = "--check" ]; then
  ensure_pg
  /app/scripts/docker-check.sh
  rc=$?
  exit "${rc}"
fi

if [ -z "${DEEPSEEK_API_KEY:-}" ]; then
  echo "dsh-alioth: DEEPSEEK_API_KEY is not set — the web UI will refuse model calls." >&2
fi

ensure_pg
echo "dsh-alioth: launching web profile with patch ${PATCH_FILE}"
exec /app/node_modules/.bin/dsh --profile web --patch "${PATCH_FILE}"
