#!/usr/bin/env bash
set -euo pipefail

node /app/cloak-sidecar/server.mjs &
SIDECAR_PID=$!

# Wait for sidecar to accept connections before starting the main server.
# Image has bash (not necessarily curl); /dev/tcp works in bash.
for i in $(seq 1 60); do
  if (echo > /dev/tcp/127.0.0.1/3102) >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done

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
