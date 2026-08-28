#!/usr/bin/env bash
# init.sh -- build MySQL 8.4 (if needed), initialize a fresh datadir
# (--initialize-insecure), start the server, create the kvbench DB/user/table,
# then leave the server running. Safe to re-run after wipe.sh. Refuses on top
# of a live datadir.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$HERE/env.sh"

mkdir -p "$BENCH_ROOT"

# 1. Ensure MySQL 8.4 is in the store + GC-rooted symlink exists.
if [ ! -x "$MYSQLD" ]; then
  echo "[init] building MySQL 8.4 from nixpkgs ..."
  nix build nixpkgs#mysql84 -o "$MYSQL_LINK"
fi
BASEDIR="$(readlink -f "$MYSQL_LINK")"
echo "[init] basedir = $BASEDIR"
echo "[init] version = $("$MYSQLD" --version)"

# 2. Fresh datadir via --initialize-insecure (empty root@localhost password).
if [ -e "$DATADIR/mysql" ] || [ -e "$DATADIR/ibdata1" ]; then
  echo "[init] ERROR: $DATADIR already initialized. Run wipe.sh first." >&2
  exit 1
fi
mkdir -p "$DATADIR"
echo "[init] initializing data directory ..."
"$MYSQLD" --no-defaults --initialize-insecure \
  --datadir="$DATADIR" --basedir="$BASEDIR" 2>&1 | tail -3

# 3. Start the server (background) and wait for the socket.
echo "[init] starting server for bootstrap ..."
"$HERE/start.sh"

# 4. Create DB, user (mysql_native_password for broad client compat), table.
echo "[init] creating database/user/table ..."
"$MYSQL" --no-defaults -u root -S "$SOCKET" <<SQL
CREATE DATABASE IF NOT EXISTS \`${DB_NAME}\`;
CREATE USER IF NOT EXISTS '${DB_USER}'@'127.0.0.1'  IDENTIFIED WITH mysql_native_password BY '${DB_PASS}';
CREATE USER IF NOT EXISTS '${DB_USER}'@'localhost'  IDENTIFIED WITH mysql_native_password BY '${DB_PASS}';
GRANT ALL PRIVILEGES ON \`${DB_NAME}\`.* TO '${DB_USER}'@'127.0.0.1';
GRANT ALL PRIVILEGES ON \`${DB_NAME}\`.* TO '${DB_USER}'@'localhost';
FLUSH PRIVILEGES;
USE \`${DB_NAME}\`;
CREATE TABLE IF NOT EXISTS kv (
  k CHAR(32) CHARACTER SET ascii NOT NULL,
  v VARBINARY(100) NOT NULL,
  PRIMARY KEY (k)
) ENGINE=InnoDB;
SQL

echo "[init] done. Server on ${BIND_ADDR}:${PORT} (socket $SOCKET)."
echo "[init] connect: mysql://${DB_USER}:${DB_PASS}@${BIND_ADDR}:${PORT}/${DB_NAME}"
