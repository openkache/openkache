#!/usr/bin/env bash
# session.sh -- ENTIRE measurement session under one exclusive lock:
#   init(if needed) -> start DB in the FAIR BOX -> prefill(once) ->
#   config sweep (restart DB per config, run sysbench) -> stop DB.
# Run as:  flock -w 1200 -x ~/.bench/server.lock bash session.sh
#
# THE BOX: mysqld pinned to cores 0,1 + cgroup memory.max=1G via cg-run.sh.
#          sysbench pinned to cores 2-5 via taskset.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$HERE/env.sh"
RUN="$HOME/kvbench/run"
BASEDIR="$(readlink -f "$MYSQL_LINK")"

MEM="1G"; DBCORES="0,1"; LGCORES="2-5"
THREADS="${THREADS:-16}"; DURATION="${DURATION:-15}"
CUR_UNIT=""

log(){ echo "[session] $*"; }

box_start(){  # args: extra mysqld overrides
  local out
  out="$(CG_UNIT="mysqld-box-$RANDOM" "$RUN/cg-run.sh" "$MEM" "$DBCORES" -- \
        "$MYSQLD" --defaults-file="$MYCNF" --basedir="$BASEDIR" \
        --datadir="$DATADIR" "$@" )"
  CUR_UNIT="$(echo "$out" | sed -n 's/^UNIT=//p')"
  log "box unit=$CUR_UNIT  overrides: $*"
  local i
  for i in $(seq 1 160); do
    "$MYSQLADMIN" --no-defaults -u root -S "$SOCKET" ping >/dev/null 2>&1 && return 0
    sleep 0.5
  done
  log "ERROR: mysqld not ready"; tail -n 30 "$DATADIR/mysqld.out.log" 2>/dev/null || true
  return 1
}

box_stop(){
  [ -S "$SOCKET" ] && "$MYSQLADMIN" --no-defaults -u root -S "$SOCKET" shutdown >/dev/null 2>&1 || true
  for i in $(seq 1 120); do [ -S "$SOCKET" ] || break; sleep 0.5; done
  [ -n "$CUR_UNIT" ] && "$RUN/cg-stop.sh" "$CUR_UNIT" >/dev/null 2>&1 || true
  CUR_UNIT=""
}
trap 'box_stop' EXIT

# run sysbench point-select; echoes: "<label> tps=<> p99ms=<> reads_per_op=<> qps=<>"
bench(){  # args: label threads
  local label="$1" thr="$2"
  local s0 s1 qtot tps p99 dqs reads_per_op
  # Warmup (discarded): fills the buffer pool to steady-state hit ratio, since
  # each config restarts the box cold. Then the measured window reflects the
  # steady SSD-bound rate, not the cold ramp.
  taskset -c "$LGCORES" "$SYSBENCH" "$HERE/kv_point_select.lua" \
    --db-driver=mysql --mysql-host=127.0.0.1 --mysql-port="$PORT" \
    --mysql-user="$DB_USER" --mysql-password="$DB_PASS" --mysql-db="$DB_NAME" \
    --table_size="$TABLE_SIZE" --threads="$thr" --time="${WARMUP:-8}" \
    --rand-type=uniform --report-interval=0 run >/dev/null 2>&1
  # diskstats.sh prints: dev=sda1 reads_completed=<> sectors_read=<> bytes_read=<>
  s0="$("$RUN/diskstats.sh" sda1 | sed -n 's/.*sectors_read=\([0-9]*\).*/\1/p')"
  local out
  out="$(taskset -c "$LGCORES" "$SYSBENCH" "$HERE/kv_point_select.lua" \
        --db-driver=mysql --mysql-host=127.0.0.1 --mysql-port="$PORT" \
        --mysql-user="$DB_USER" --mysql-password="$DB_PASS" --mysql-db="$DB_NAME" \
        --table_size="$TABLE_SIZE" --threads="$thr" --time="$DURATION" \
        --percentile=99 --rand-type=uniform --report-interval=0 run 2>&1)"
  s1="$("$RUN/diskstats.sh" sda1 | sed -n 's/.*sectors_read=\([0-9]*\).*/\1/p')"
  tps="$(echo "$out" | sed -n 's/.*queries: *[0-9]* (\([0-9.]*\) per sec.*/\1/p')"
  [ -z "$tps" ] && tps="$(echo "$out" | sed -n 's/.*transactions: *[0-9]* (\([0-9.]*\) per sec.*/\1/p')"
  qtot="$(echo "$out" | sed -n 's/.*queries: *\([0-9]*\) .*/\1/p')"
  p99="$(echo "$out"  | sed -n 's/.*99th percentile: *\([0-9.]*\).*/\1/p')"
  dqs=$(( s1 - s0 ))
  if [ -n "$qtot" ] && [ "$qtot" -gt 0 ]; then
    reads_per_op="$(awk -v a="$dqs" -v q="$qtot" 'BEGIN{printf "%.3f", (a*512.0/16384.0)/q}')"
  else reads_per_op="NA"; fi
  echo "RESULT $label threads=$thr tps=$tps p99ms=$p99 qtot=$qtot sectors_delta=$dqs reads16k_per_op=$reads_per_op"
}

######## MAIN ########
mkdir -p "$BENCH_ROOT"

# 1. Fresh datadir + schema (NOT boxed: one-time setup). Wipe for a clean,
#    reproducible dataset each full session, then prefill exactly TABLE_SIZE rows.
if [ "${CLEAN:-1}" = "1" ] || [ ! -e "$DATADIR/ibdata1" ]; then
  log "clean init: wiping + initializing datadir + schema ..."
  "$HERE/wipe.sh" >/dev/null 2>&1 || true
  "$HERE/init.sh"
  "$HERE/stop.sh"
fi

# 2. Start DB in the box, prefill once (data persists across restarts).
box_start --innodb_buffer_pool_size=256M
"$HERE/prefill.sh"
box_stop

# 3. CONFIG SWEEP. Each entry: "label|mysqld overrides".
SWEEP=(
  "bp128            |--innodb_buffer_pool_size=128M"
  "bp256            |--innodb_buffer_pool_size=256M"
  "bp512            |--innodb_buffer_pool_size=512M"
  "bp256_rio4       |--innodb_buffer_pool_size=256M --innodb_read_io_threads=4"
  "bp256_rio16      |--innodb_buffer_pool_size=256M --innodb_read_io_threads=16"
  "bp256_toc1k      |--innodb_buffer_pool_size=256M --table_open_cache=1000"
  "bp256_ahi_off    |--innodb_buffer_pool_size=256M --innodb_adaptive_hash_index=OFF"
)
echo "======== SWEEP (threads=$THREADS duration=${DURATION}s) ========"
for entry in "${SWEEP[@]}"; do
  label="$(echo "${entry%%|*}" | xargs)"
  ov="${entry#*|}"
  box_start $ov || { box_stop; continue; }
  bench "$label" "$THREADS"
  box_stop
done

# 4. THREAD sweep at best pool (256M).
echo "======== THREAD SWEEP (bp256) ========"
box_start --innodb_buffer_pool_size=256M
for t in 8 16 32 64; do bench "bp256_t$t" "$t"; done
box_stop

log "session complete; DB stopped."

