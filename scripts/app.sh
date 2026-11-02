#!/usr/bin/env bash
# Build and start the local visual app (« PDF Edit ») on http://127.0.0.1:<port>.
# Usage: scripts/app.sh [port] [--no-open]   (port defaults to $PORT, then 7878)
# An older copy of the app still listening on that port is stopped first, so you always get this build.
set -euo pipefail

cd "$(dirname "$0")/.."

PORT="${PORT:-7878}"
if [[ "${1:-}" =~ ^[0-9]+$ ]]; then
  PORT="$1"
  shift
fi

echo "1/2 Building p2d-cli (release)..."
cargo build --release -p p2d-cli

for pid in $(lsof -tiTCP:"$PORT" -sTCP:LISTEN 2>/dev/null || true); do
  name="$(ps -p "$pid" -o comm= 2>/dev/null || true)"
  if [[ "$name" == *p2d-cli ]]; then
    echo "Stopping the previous app (pid $pid) on port $PORT..."
    kill "$pid"
    while kill -0 "$pid" 2>/dev/null; do sleep 0.1; done
  else
    echo "Port $PORT is used by another program ($name). Try: pnpm app 7979" >&2
    exit 1
  fi
done

echo "2/2 Starting the app on http://127.0.0.1:$PORT (Ctrl+C to stop)..."
exec ./target/release/p2d-cli app "$PORT" "$@"
