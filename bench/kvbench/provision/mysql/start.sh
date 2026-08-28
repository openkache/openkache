#!/usr/bin/env bash
# start.sh -- start mysqld against the datadir on the chosen port + socket.
# Backgrounds the server and waits until it accepts connections.
# For the FAIR BOX, session.sh launches mysqld via cg-run.sh instead; this
# plain start.sh is for standalone / non-boxed use. Extra mysqld overrides can
# be appended as args (e.g. start.sh --innodb_buffer_pool_size=256M).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$HERE/env.sh"
BASEDIR="$(readlink -f "$MYSQL_LINK")"

if [ ! -e "$DATADIR/ibdata1" ]; then
  echo "[start] ERROR: datadir $DATADIR not initialized. Run init.sh first." >&2
  exit 1
fi

if [ -S "$SOCKET" ] && "$MYSQLADMIN" --no-defaults -u root -S "$SOCKET" ping >/dev/null 2>&1; then
  echo "[start] already running on socket $SOCKET"
  exit 0
fi

echo "[start] launching mysqld ..."
"$MYSQLD" \
  --defaults-file="$MYCNF" \
  --basedir="$BASEDIR" \
  --datadir="$DATADIR" \
  "$@" \
  > "$DATADIR/mysqld.out.log" 2>&1 &
echo $! > "$DATADIR/mysqld.shell.pid"

for i in $(seq 1 120); do
  if "$MYSQLADMIN" --no-defaults -u root -S "$SOCKET" ping >/dev/null 2>&1; then
    echo "[start] up: ${BIND_ADDR}:${PORT}  socket=$SOCKET  pid=$(cat "$DATADIR/mysqld.shell.pid")"
    exit 0
  fi
  sleep 0.5
done

echo "[start] ERROR: server did not become ready; tail of log:" >&2
tail -n 40 "$DATADIR/mysqld.out.log" >&2
exit 1
