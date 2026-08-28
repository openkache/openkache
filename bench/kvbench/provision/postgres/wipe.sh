#!/usr/bin/env bash
# wipe.sh - stop (if running) + delete the datadir for a fresh run.
# Run on serveroptima1.
set -euo pipefail
cd "$(dirname "$0")"
. ./env.sh

# Best-effort stop; ignore if already down.
if [ -s "$PGDATA/postmaster.pid" ]; then
  "$PGBIN/pg_ctl" -D "$PGDATA" -w -m fast stop || true
fi

# Safety: refuse to rm an empty/unset path.
case "$PGDATA" in
  ""|"/"|"$HOME") echo "[wipe] refusing to remove '$PGDATA'" >&2; exit 1 ;;
esac

echo "[wipe] removing $PGDATA"
rm -rf "$PGDATA"
echo "[wipe] done."
