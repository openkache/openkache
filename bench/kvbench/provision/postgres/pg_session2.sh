#!/usr/bin/env bash
# pg_session2.sh - CORRECTED fair measurement.
# Cold the OS page cache ONCE, then run the client sweep inside a SINGLE
# live instance (256MB shared_buffers). While one instance stays alive its
# faulted pages are charged to its own 1G cap (table 1.34GB+sb > 1G => stays
# SSD-bound); reparent-leak only happens after an instance dies.
# Run under the lock, same as pg_session.sh.
set -uo pipefail
PGBIN="/nix/store/fdh93xn8lhlkdslwrgxzr8kd1qc8akga-postgresql-17.10/bin"
PGDATA="$HOME/.bench/pgdata"; HOST=127.0.0.1; PORT=55432; DB=kvbench; USR="$USER"
RUN="$HOME/kvbench/run"; SQLF="$HOME/kvbench/provision/postgres/point-select.sql"
COLD="$HOME/kvbench/provision/postgres/cache_cold.sh"
RES="$HOME/.bench/pg_results2.txt"; : > "$RES"; CURUNIT=""
log(){ echo "[s2 $(date +%H:%M:%S)] $*"; }
reads_now(){ set -- $(grep -w sda1 /proc/diskstats); echo "$4"; }
start_db(){ mkdir -p "$PGDATA/log"
  local o; o="$("$RUN/cg-run.sh" 1G 0,1 -- "$PGBIN/postgres" -D "$PGDATA" 2>&1)"
  CURUNIT="$(sed -n 's/^UNIT=//p' <<<"$o")"; log "$o"
  for i in $(seq 1 30); do "$PGBIN/pg_isready" -h $HOST -p $PORT -q && { log "up $CURUNIT"; return 0; }; sleep 1; done
  log FAIL; return 1; }
stop_db(){ "$PGBIN/pg_ctl" -D "$PGDATA" -w -m fast stop >/dev/null 2>&1 || true
  [ -n "$CURUNIT" ] && "$RUN/cg-stop.sh" "$CURUNIT" >/dev/null 2>&1 || true; CURUNIT=""; sleep 1; }
trap stop_db EXIT
run_bench(){ local label="$1" c="$2" t="$3" s="$4"; local d; d="$(mktemp -d)"
  local r0 r1 out tps p99 tx dr rpo; r0="$(reads_now)"
  out="$(cd "$d" && taskset -c 2-5 "$PGBIN/pgbench" -h $HOST -p $PORT -U $USR -d $DB \
        -n -M prepared -f "$SQLF" -c "$c" -j "$t" -T "$s" -l 2>&1)"
  r1="$(reads_now)"
  tps="$(sed -n 's/^tps = \([0-9.]*\).*/\1/p' <<<"$out"|head -1)"
  tx="$(sed -n 's/^number of transactions actually processed: \([0-9]*\).*/\1/p' <<<"$out")"
  p99="$(cat "$d"/pgbench_log.* 2>/dev/null|awk '{print $3}'|sort -n|awk '{a[NR]=$1}END{if(NR)printf "%.3f",a[int(NR*0.99)]/1000}')"
  dr=$((r1-r0)); rpo="$(awk -v x=$dr -v y=${tx:-0} 'BEGIN{if(y)printf "%.3f",x/y;else print "NA"}')"
  printf 'RESULT label=%s clients=%s tps=%s p99_ms=%s tx=%s disk_reads=%s reads_per_op=%s\n' \
    "$label" "$c" "${tps:-NA}" "${p99:-NA}" "${tx:-NA}" "$dr" "$rpo"|tee -a "$RES"; rm -rf "$d"; }

log "=== corrected session: cold cache then single-instance sweep ==="
bash "$COLD" 16 | tee -a "$RES"
start_db || exit 1
run_bench cold_cli16 16 4 30
run_bench cold_cli24 24 4 30
run_bench cold_cli32 32 4 30
run_bench cold_cli48 48 4 30
stop_db
log "=== done ==="; echo "== RESULTS2 =="; cat "$RES"
