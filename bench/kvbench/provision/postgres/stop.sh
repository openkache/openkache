#!/usr/bin/env bash
# stop.sh - clean fast shutdown. Run on serveroptima1.
set -euo pipefail
cd "$(dirname "$0")"
. ./env.sh

if [ ! -s "$PGDATA/postmaster.pid" ]; then
  echo "[stop] no postmaster.pid at $PGDATA - not running?"
  exit 0
fi

"$PGBIN/pg_ctl" -D "$PGDATA" -w -m fast stop
echo "[stop] stopped."
