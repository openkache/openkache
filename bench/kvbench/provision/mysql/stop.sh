#!/usr/bin/env bash
# stop.sh -- clean shutdown via mysqladmin (InnoDB flushes + checkpoints).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$HERE/env.sh"

if [ ! -S "$SOCKET" ]; then
  echo "[stop] no socket at $SOCKET; server not running."
  exit 0
fi

echo "[stop] shutting down ..."
"$MYSQLADMIN" --no-defaults -u root -S "$SOCKET" shutdown || true

for i in $(seq 1 120); do
  [ -S "$SOCKET" ] || { echo "[stop] stopped cleanly."; exit 0; }
  sleep 0.5
done
echo "[stop] WARNING: socket still present after 60s." >&2
exit 1
