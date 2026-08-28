#!/usr/bin/env bash
# pg_session.sh - ONE exclusive, serialized PG measurement session.
# Run under the machine-wide lock:
#   flock -w 1200 -x /home/kimseojin111/.bench/server.lock bash ~/pg_session.sh
# Does: start DB (cg 1G/cores0,1) -> prefill 8M (once) -> ANALYZE ->
#       sweep configs via restart -> pgbench (cores 2-5) -> STOP DB.
set -uo pipefail

PGBIN="/nix/store/fdh93xn8lhlkdslwrgxzr8kd1qc8akga-postgresql-17.10/bin"
PGDATA="$HOME/.bench/pgdata"
HOST=127.0.0.1; PORT=55432; DB=kvbench; USR="$USER"
RUN="$HOME/kvbench/run"
SQLF="$HOME/kvbench/provision/postgres/point-select.sql"
N=8000000                      # rows (>1GiB on disk)
RES="$HOME/.bench/pg_results.txt"
: > "$RES"
CURUNIT=""

log(){ echo "[sess $(date +%H:%M:%S)] $*"; }
reads_now(){ set -- $(grep -w sda1 /proc/diskstats); echo "$4"; }  # reads completed

write_conf(){ # $1=shared_buffers $2=huge_pages(try|off)
  cat > "$PGDATA/bench.conf" <<EOF
listen_addresses = '$HOST'
port = $PORT
unix_socket_directories = '$PGDATA'
max_connections = 200
shared_buffers = $1
effective_cache_size = 768MB
work_mem = 4MB
maintenance_work_mem = 128MB
huge_pages = $2
max_wal_size = 4GB
min_wal_size = 512MB
checkpoint_timeout = 30min
autovacuum = off
jit = off
logging_collector = on
log_directory = 'log'
log_filename = 'postgresql.log'
log_rotation_age = 0
log_rotation_size = 0
log_line_prefix = '%m [%p] '
EOF
}

start_db(){ # backgrounded under cgroup 1G + cores 0,1
  mkdir -p "$PGDATA/log"
  local out
  out="$("$RUN/cg-run.sh" 1G 0,1 -- "$PGBIN/postgres" -D "$PGDATA" 2>&1)"
  CURUNIT="$(sed -n 's/^UNIT=//p' <<<"$out")"
  log "cg-run: $out"
  for i in $(seq 1 30); do
    "$PGBIN/pg_isready" -h "$HOST" -p "$PORT" -q && { log "db up (unit=$CURUNIT)"; return 0; }
    sleep 1
  done
  log "DB FAILED TO START"; tail -20 "$PGDATA/log/postgresql.log"; return 1
}

stop_db(){
  "$PGBIN/pg_ctl" -D "$PGDATA" -w -m fast stop >/dev/null 2>&1 || true
  [ -n "$CURUNIT" ] && "$RUN/cg-stop.sh" "$CURUNIT" >/dev/null 2>&1 || true
  CURUNIT=""
  sleep 1
}
trap 'stop_db' EXIT

psql_db(){ "$PGBIN/psql" -h "$HOST" -p "$PORT" -U "$USR" -d "$DB" -v ON_ERROR_STOP=1 "$@"; }

prefill(){
  local have
  have="$(psql_db -tAc "SELECT count(*) FROM kv" 2>/dev/null || echo 0)"
  if [ "${have:-0}" -ge "$N" ]; then log "prefill present ($have rows) - skip"; return 0; fi
  log "prefilling $N rows (batches of 1M)"
  psql_db -c "TRUNCATE kv;" >/dev/null
  local b
  for b in $(seq 0 7); do
    local lo=$(( b*1000000 )) hi=$(( b*1000000 + 999999 ))
    psql_db -c "INSERT INTO kv (k,v)
      SELECT 'kvbench:'||lpad(g::text,24,'0'),
             decode(md5(g::text)||md5((g+1)::text)||md5((g+2)::text)||substr(md5((g+3)::text),1,8),'hex')
      FROM generate_series($lo,$hi) g;" >/dev/null
    log "  batch $b done ($((hi+1)) rows)"
  done
  log "ANALYZE"; psql_db -c "ANALYZE kv;" >/dev/null
}

# run_bench <label> <clients> <threads> <secs>  -> appends a RESULT line
run_bench(){
  local label="$1" c="$2" t="$3" s="$4"
  local ldir; ldir="$(mktemp -d)"
  local r0 r1 out tps p99 tx dread rpo
  r0="$(reads_now)"
  out="$( cd "$ldir" && taskset -c 2-5 "$PGBIN/pgbench" \
        -h "$HOST" -p "$PORT" -U "$USR" -d "$DB" \
        -n -M prepared -f "$SQLF" -c "$c" -j "$t" -T "$s" -l 2>&1 )"
  r1="$(reads_now)"
  tps="$(sed -n 's/^tps = \([0-9.]*\).*without initial.*/\1/p' <<<"$out")"
  [ -z "$tps" ] && tps="$(sed -n 's/^tps = \([0-9.]*\).*/\1/p' <<<"$out" | head -1)"
  tx="$(sed -n 's/^number of transactions actually processed: \([0-9]*\).*/\1/p' <<<"$out")"
  p99="$(cat "$ldir"/pgbench_log.* 2>/dev/null | awk '{print $3}' | sort -n | \
        awk '{a[NR]=$1} END{if(NR>0) printf "%.3f", a[int(NR*0.99)]/1000}')"
  dread=$(( r1 - r0 ))
  rpo="$(awk -v d="$dread" -v x="${tx:-0}" 'BEGIN{if(x>0) printf "%.3f", d/x; else print "NA"}')"
  printf 'RESULT label=%s clients=%s tps=%s p99_ms=%s tx=%s disk_reads=%s reads_per_op=%s\n' \
    "$label" "$c" "${tps:-NA}" "${p99:-NA}" "${tx:-NA}" "$dread" "$rpo" | tee -a "$RES"
  rm -rf "$ldir"
}

sweep_conf(){ # $1=sb $2=hp $3=label ; restart under new conf, run 1 bench
  write_conf "$1" "$2"; start_db || return 1
  # size + plan proof (once, cheap)
  psql_db -tAc "SELECT 'onexk_size='||pg_size_pretty(pg_total_relation_size('kv'))" | tee -a "$RES"
  run_bench "$3" 32 4 20
  stop_db
}

# ---- session ----
log "=== PG tuning session start (N=$N) ==="
# initial start with a middle config to prefill
write_conf 256MB off; start_db || exit 1
prefill
psql_db -tAc "EXPLAIN (COSTS OFF) SELECT v FROM kv WHERE k='kvbench:'||lpad('123'::text,24,'0')" | tee -a "$RES"
stop_db

# A) shared_buffers sweep (clients=32)
sweep_conf 128MB off sb128
sweep_conf 256MB off sb256
sweep_conf 512MB off sb512

# B) huge_pages=try on 256MB (expected fallback: none allocated)
sweep_conf 256MB try sb256_hptry

# C) client-count sweep on 256MB/off to find saturation
write_conf 256MB off; start_db || exit 1
run_bench cli16 16 4 20
run_bench cli32 32 4 20
run_bench cli64 64 4 20
run_bench cli96 96 4 20
stop_db

# D) FINAL longer run at best-looking config (256MB, best client), 30s
write_conf 256MB off; start_db || exit 1
run_bench FINAL 64 4 30
stop_db

log "=== session done ==="
echo "===== RESULTS ====="; cat "$RES"



