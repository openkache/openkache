#!/usr/bin/env bash
# start.sh - run postgres in the FOREGROUND from the nix-store binary.
# Foreground so a cgroup runner (systemd-run/cgexec) can wrap this PID
# directly. Config (port, socket, buffers) all live in $PGDATA/bench.conf.
# Run on serveroptima1.
set -euo pipefail
cd "$(dirname "$0")"
. ./env.sh

if [ ! -s "$PGDATA/PG_VERSION" ]; then
  echo "[start] no cluster at $PGDATA - run ./init.sh first" >&2
  exit 1
fi

echo "[start] postgres foreground: $PGHOST:$PGPORT db=$PGDATABASE data=$PGDATA"
# exec so signals (and the cgroup) target postgres directly, not the shell.
exec "$PGBIN/postgres" -D "$PGDATA"
