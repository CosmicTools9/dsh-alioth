#!/usr/bin/env bash
# Boot the DeepSeek Harness web profile with the full Alioth plugin group
# (bundle patch: env + 4 tool plugins + auth trio + billing + feedback) and
# open the browser. Needs DEEPSEEK_API_KEY for real model calls;
# ALIOTH_PRE_PROC_ROOT overrides the default Pre-Proc root. A previous dsh
# instance still holding the port is stopped automatically (SIGTERM, then
# SIGKILL); a non-dsh process on the port aborts the launch instead.
set -euo pipefail

HOST="${DSH_WEB_HOST:-127.0.0.1}"
PORT="${DSH_WEB_PORT:-3100}"
URL="http://${HOST}:${PORT}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Stop a previous dsh web instance on $PORT. Only processes whose command line
# contains "/dsh/" are reaped; anything else must be freed by the operator.
free_port() {
  local pids
  pids="$(lsof -nP -tiTCP:"$PORT" -sTCP:LISTEN 2>/dev/null || true)"
  [ -z "$pids" ] && return 0
  local pid cmd
  for pid in $pids; do
    cmd="$(ps -p "$pid" -o command= 2>/dev/null || true)"
    if [ -z "$cmd" ] || ! printf '%s' "$cmd" | grep -q '/dsh/'; then
      echo "port $PORT is held by a non-dsh process (pid $pid): $cmd" >&2
      exit 1
    fi
  done
  echo "stopping previous dsh web instance on port $PORT: $pids" >&2
  kill $pids 2>/dev/null || true
  for _ in $(seq 1 10); do
    if ! lsof -nP -tiTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then return 0; fi
    sleep 0.3
  done
  echo "previous instance did not stop gracefully, forcing" >&2
  kill -9 $pids 2>/dev/null || true
  sleep 0.3
}

free_port

pnpm exec dsh --profile web --patch packages/alioth/bundle-alioth/cordis.patch.yml --host "$HOST" --port "$PORT" &
SERVER_PID=$!
cleanup() { kill "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT INT TERM

ready=false
for _ in $(seq 1 120); do
  if curl -sf "$URL" >/dev/null 2>&1; then
    ready=true
    break
  fi
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "dsh web server exited before becoming ready" >&2
    exit 1
  fi
  sleep 0.5
done

if [ "$ready" != true ]; then
  echo "dsh web server did not become ready on $URL" >&2
  exit 1
fi

echo "dsh web serving at $URL"
if [ "${DSH_OPEN:-1}" != "0" ]; then
  open "$URL"
  echo "opened $URL in the default browser"
fi

wait "$SERVER_PID"
