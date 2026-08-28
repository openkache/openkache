#!/usr/bin/env bash
# wipe.sh -- stop the server (if up) and delete the datadir. Destructive.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$HERE/env.sh"

# Best-effort clean stop.
"$HERE/stop.sh" || true

# Extra guard: kill any stray mysqld bound to our datadir.
if [ -f "$DATADIR/mysqld.shell.pid" ]; then
  pid="$(cat "$DATADIR/mysqld.shell.pid" 2>/dev/null || true)"
  if [ -n "${pid:-}" ] && kill -0 "$pid" 2>/dev/null; then
    kill "$pid" 2>/dev/null || true
    sleep 2
  fi
fi

if [ -d "$DATADIR" ]; then
  echo "[wipe] removing $DATADIR ..."
  rm -rf "$DATADIR"
  echo "[wipe] done."
else
  echo "[wipe] nothing to remove ($DATADIR absent)."
fi
