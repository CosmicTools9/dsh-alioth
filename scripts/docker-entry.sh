#!/bin/sh
# dsh-alioth container entry point.
# Default: launch the web GUI with the Alioth plugin group mounted.
# `--check`: keyless composition self-check (dump-config + doctor), exit code.
set -eu

PATCH_FILE="${ALIOTH_PATCH:-/app/packages/alioth/bundle-alioth/cordis.patch.yml}"

# Start PostgreSQL 18.6 (PGDG) on /data/pg unless an external database URL is
# given. The alioth superuser/database mirror the embedded defaults so
# ALIOTH_DATABASE_URL semantics are identical.
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
  if ! psql -h 127.0.0.1 -p 5432 -U alioth -d postgres -tAc "SELECT 1 FROM pg_database WHERE datname = 'alioth'" | grep -q 1; then
    createdb -h 127.0.0.1 -p 5432 -U alioth alioth
  fi
  export ALIOTH_DATABASE_URL="postgres://alioth:${PGPASSWORD}@127.0.0.1:5432/alioth"
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
