#!/usr/bin/env bash
#
# Points a dev build at a backend. With no argument it detects this machine's
# LAN address, which is what you want when the stack runs here and the client
# runs on another device on the same network.
#
#   bash scripts/set-dev-url.sh              # detect the LAN IP
#   bash scripts/set-dev-url.sh 10.0.0.5     # a specific host
#   bash scripts/set-dev-url.sh localhost    # everything on this machine
#
# Writes config/local.json, which dev and staging builds read and production
# ignores entirely.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/config/local.json"
HOST="${1:-}"

if [ -z "$HOST" ]; then
    if [[ "$OSTYPE" == darwin* ]]; then
        HOST=$(ifconfig 2>/dev/null | grep "inet " | grep -v 127.0.0.1 | awk '{print $2}' | head -n 1 || true)
    else
        HOST=$(hostname -I 2>/dev/null | awk '{print $1}' || true)
    fi
    if [ -z "$HOST" ]; then
        echo "could not detect a LAN address; pass one explicitly" >&2
        exit 1
    fi
    echo "detected LAN address: $HOST"
fi

mkdir -p "$ROOT/config"
cat > "$OUT" <<JSON
{
  "api_base_url": "http://${HOST}:8080/api",
  "ws_url": "ws://${HOST}:8085",
  "public_site_url": "http://${HOST}:3000"
}
JSON

echo "wrote $OUT"
sed 's/^/  /' "$OUT"
echo
echo "RISE_API_URL and RISE_WS_URL still win over this file if they are set."
