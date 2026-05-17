#!/usr/bin/env bash
set -euo pipefail

node /app/cloak-sidecar/server.mjs &
SIDECAR_PID=$!

/usr/local/bin/book-forge-server &
SERVER_PID=$!

shutdown() {
  kill -TERM "$SIDECAR_PID" "$SERVER_PID" 2>/dev/null || true
}
trap shutdown TERM INT

wait -n "$SIDECAR_PID" "$SERVER_PID"
EXIT_CODE=$?
shutdown
wait || true
exit "$EXIT_CODE"
