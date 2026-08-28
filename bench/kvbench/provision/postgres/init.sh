#!/usr/bin/env bash
# init.sh - initdb + config + createdb + schema. Idempotent-ish:
# safe to re-run; safe to run after wipe.sh. Run on serveroptima1.
set -euo pipefail
cd "$(dirname "$0")"
. ./env.sh

# 1) initdb if the cluster does not already exist.
if [ ! -s "$PGDATA/PG_VERSION" ]; then
  echo "[init] running initdb at $PGDATA"
  mkdir -p "$PGDATA"
  chmod 700 "$PGDATA"
  "$PGBIN/initdb" \
    --pgdata="$PGDATA" \
    --encoding=UTF8 \
    --locale=C \
    --auth-local=trust \
    --auth-host=trust \
    --username="$USER" >/dev/null
else
  echo "[init] cluster already present at $PGDATA (skipping initdb)"
fi

# 2) Write benchmark config to a dedicated file and include it once.
cat > "$PGDATA/bench.conf" <<EOF
# --- kvbench point-lookup read benchmark config ---
listen_addresses = '$PGHOST'
port = $PGPORT
unix_socket_directories = '$PGSOCKDIR'
max_connections = 200

# TUNED (Phase 2): 256MB is the measured throughput optimum under the fair
# box (cores 0,1 + memory.max=1G). It caches the hot upper PK-btree levels in
# postgres' own pool while leaving ~700MB of the 1G cap for OS page cache, so
# the 1.34GB dataset still can't fit -> GETs stay SSD-bound (reads_per_op~1.1).
# 128MB gave ~19.3k tps, 256MB ~20.1k (cold-cache peak 17.4k @ cli24),
# 512MB ~19.9k (steals page-cache room, heap reads rise). See tuning table.
shared_buffers = 256MB
# Advertise the ~1G cap as available cache (planner still always picks the PK
# index for exact match; this keeps estimates honest, no plan change).
effective_cache_size = 768MB
work_mem = 4MB
maintenance_work_mem = 128MB           # bounded under the 1G cap during prefill

# Read benchmark: leave fsync/synchronous_commit at safe defaults.
# Fewer checkpoints during the (client-driven) bulk prefill / read phase.
max_wal_size = 4GB
min_wal_size = 512MB
checkpoint_timeout = 30min

# No writes during measurement -> keep autovacuum out of the read path.
# (Run ANALYZE kv manually after prefill for good stats.)
autovacuum = off

jit = off                              # trivial point queries; JIT is pure overhead
# huge_pages: host has HugePages_Total=0 and no root to reserve any, so 'on'
# refuses to start and 'try' silently falls back to none -> leave off.
huge_pages = off

# Log to a file inside the datadir.
logging_collector = on
log_directory = '$PGLOGDIR'
log_filename = '$PGLOGFILE'
log_rotation_age = 0
log_rotation_size = 0
log_truncate_on_rotation = off
log_line_prefix = '%m [%p] '
EOF

INCLUDE_LINE="include = 'bench.conf'"
if ! grep -qxF "$INCLUDE_LINE" "$PGDATA/postgresql.conf"; then
  printf '\n# kvbench overrides\n%s\n' "$INCLUDE_LINE" >> "$PGDATA/postgresql.conf"
fi

# 3) Bring the server up briefly to create the DB + schema, then stop it.
mkdir -p "$PGDATA/$PGLOGDIR"
echo "[init] starting server (temporary) to create schema"
"$PGBIN/pg_ctl" -D "$PGDATA" -w -l "$PGDATA/$PGLOGDIR/$PGLOGFILE" start

# createdb is not idempotent; guard it.
if ! "$PGBIN/psql" -h "$PGSOCKDIR" -p "$PGPORT" -U "$USER" -d postgres \
      -tAc "SELECT 1 FROM pg_database WHERE datname='$PGDATABASE'" | grep -q 1; then
  echo "[init] creating database $PGDATABASE"
  "$PGBIN/createdb" -h "$PGSOCKDIR" -p "$PGPORT" -U "$USER" "$PGDATABASE"
else
  echo "[init] database $PGDATABASE already exists"
fi

# Schema. TEXT COLLATE "C" -> byte-wise memcmp equality on the PK btree,
# which is the fastest exact-match lookup for fixed 32-byte ASCII keys.
"$PGBIN/psql" -h "$PGSOCKDIR" -p "$PGPORT" -U "$USER" -d "$PGDATABASE" -v ON_ERROR_STOP=1 <<'SQL'
CREATE TABLE IF NOT EXISTS kv (
  k TEXT COLLATE "C" PRIMARY KEY,
  v BYTEA NOT NULL
);
SQL

echo "[init] stopping temporary server"
"$PGBIN/pg_ctl" -D "$PGDATA" -w -m fast stop
echo "[init] done. schema ready in database '$PGDATABASE'."
