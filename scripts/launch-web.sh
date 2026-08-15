#!/usr/bin/env bash
# Boot the DeepSeek Harness web profile with the alioth-agent patch and open the
# browser. Needs DEEPSEEK_API_KEY for real model calls; ALIOTH_PRE_PROC_ROOT
# overrides the default Pre-Proc root.
set -euo pipefail

HOST="${DSH_WEB_HOST:-127.0.0.1}"
PORT="${DSH_WEB_PORT:-3100}"
URL="http://${HOST}:${PORT}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

pnpm exec dsh --profile web --patch examples/alioth-agent/web.patch.yml --host "$HOST" --port "$PORT" &
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
