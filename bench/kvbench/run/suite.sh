#!/usr/bin/env bash
# Unified benchmark suite: OpenKache, PostgreSQL, MySQL in one identical box.
#   box    = taskset cores 0,1 + systemd-run MemoryMax=$MEM (cg-run.sh)
#   loadgen= kvbench on cores 2-5, native protocol per engine
#   phases = wipe -> prefill N -> storage(du) -> cache-cold -> throughput
#            -> cache-cold -> latency
# Dataset >> cap so all engines are SSD-bound (neutralizes cache-ratio).
# Args: <N-keys> <MEM> <outfile>
set -uo pipefail
N="${1:?N}"; MEM="${2:?MEM}"; OUT="${3:?outfile}"
H=127.0.0.1
KVB="$HOME/kvbench_bin"
RUN="$HOME/kvbench/run"
PROV="$HOME/kvbench/provision"
log(){ echo "[$(date +%H:%M:%S)] $*" | tee -a "$OUT"; }

evict(){ # drop leaked page cache from an UNCAPPED process (needs no root)
  python3 - <<'PY' 2>/dev/null || true
c=[]
for i in range(64):
    b=bytearray(256*1024*1024)
    for o in range(0,len(b),4096): b[o]=1
    c.append(b)
PY
}
dubytes(){ du -s --block-size=1 "$1" 2>/dev/null | awk '{print $1}'; }  # allocated blocks, not sparse apparent size
rd(){ awk '$3=="sda"{print $4}' /proc/diskstats; }

# --- throughput measure via kvbench (conn 80, pipe 32), returns line to OUT ---
measure_tput(){ local be="$1" addr="$2" label="$3"
  evict
  local r0=$(rd)
  local o=$(taskset -c 2-5 "$KVB" --backend "$be" --addr "$addr" --keys "$N" \
      --connections 80 --pipeline 32 --warmup-ms 3000 --measure-ms 15000 --phase measure 2>&1)
  local r1=$(rd)
  local tp=$(echo "$o"|awk '/throughput/{print $2}')
  local ops=$(echo "$o"|awk '/measured_ops/{print $2}')
  local rpo=$(awk -v a="$((r1-r0))" -v b="$ops" 'BEGIN{if(b>0)printf "%.3f",a/b; else print "na"}')
  log "TPUT $label ops/s=$tp reads/op=$rpo"
}

# --- latency measure via kvbench (conn 1, pipe 1) ---
measure_lat(){ local be="$1" addr="$2" label="$3"
  evict
  local o=$(taskset -c 2-5 "$KVB" --backend "$be" --addr "$addr" --keys "$N" \
      --connections 1 --pipeline 1 --warmup-ms 3000 --measure-ms 15000 --phase measure 2>&1)
  local mean=$(echo "$o"|awk '/mean_us/{print $2}')
  local p50=$(echo "$o"|awk '/p50_us/{print $2}')
  local p99=$(echo "$o"|awk '/p99_us/{print $2}')
  local p999=$(echo "$o"|awk '/p99.9_us/{print $2}')
  log "LAT  $label avg_us=$mean p50=$p50 p99=$p99 p99.9=$p999"
}

log "=== SUITE START N=$N MEM=$MEM ==="

########## OpenKache ##########
run_openkache(){
  log "--- OpenKache ---"
  pkill -9 openkache-server-bench 2>/dev/null; sleep 1
  rm -f "$HOME/.bench/openkache.data"
  local u=$(CG_UNIT=ok-box "$RUN/cg-run.sh" "$MEM" 0,1 -- "$HOME/openkache-server-bench" --config "$HOME/.bench/bench-suite.toml" "$H:7711" 0 1 2>/dev/null; echo)
  sleep 3
  ss -ltn|grep -q :7711 || { log "OpenKache FAILED to start"; return 1; }
  log "prefill $N + flush"
  taskset -c 2-5 "$KVB" --backend openkache --addr "$H:7711" --keys "$N" --value-len 100 --connections 50 --pipeline 32 --flush-after-prefill --phase prefill >/dev/null 2>&1
  local disk=$(dubytes "$HOME/.bench/openkache.data")
  log "STORE OpenKache disk_bytes=$disk bytes/kv=$(awk -v d="$disk" -v n="$N" 'BEGIN{printf "%.1f",d/n}')"
  measure_tput openkache "$H:7711" OpenKache
  measure_lat  openkache "$H:7711" OpenKache
  CG_UNIT=ok-box "$RUN/cg-stop.sh" ok-box >/dev/null 2>&1 || pkill -9 openkache-server-bench
  sleep 2
}

########## PostgreSQL ##########
run_postgres(){
  log "--- PostgreSQL ---"
  . "$PROV/postgres/env.sh"
  pkill -9 postgres 2>/dev/null; sleep 2
  bash "$PROV/postgres/wipe.sh" >/dev/null 2>&1 || true
  bash "$PROV/postgres/init.sh" >/dev/null 2>&1
  CG_UNIT=pg-box "$RUN/cg-run.sh" "$MEM" 0,1 -- "$PGBIN/postgres" -D "$PGDATA" >/dev/null 2>&1
  for i in $(seq 1 40); do ss -ltn|grep -q :55432 && break; sleep 1; done
  ss -ltn|grep -q :55432 || { log "PG FAILED to start"; return 1; }
  local base=$(dubytes "$PGDATA")
  log "prefill $N"
  taskset -c 2-5 "$KVB" --backend postgres --addr "$H:55432" --keys "$N" --value-len 100 --connections 50 --pipeline 1 --phase prefill >/dev/null 2>&1
  "$PGBIN/psql" -h "$H" -p 55432 -U "$USER" -d kvbench -c "CHECKPOINT;" >/dev/null 2>&1
  local disk=$(dubytes "$PGDATA")
  log "STORE PostgreSQL disk_bytes=$disk delta=$((disk-base)) bytes/kv=$(awk -v d="$disk" -v n="$N" 'BEGIN{printf "%.1f",d/n}')"
  measure_tput postgres "$H:55432" PostgreSQL
  measure_lat  postgres "$H:55432" PostgreSQL
  CG_UNIT=pg-box "$RUN/cg-stop.sh" pg-box >/dev/null 2>&1 || pkill -9 postgres
  sleep 2
}

########## MySQL ##########
run_mysql(){
  log "--- MySQL ---"
  . "$PROV/mysql/env.sh"
  pkill -9 mysqld 2>/dev/null; sleep 2
  bash "$PROV/mysql/wipe.sh" >/dev/null 2>&1 || true
  bash "$PROV/mysql/init.sh" >/dev/null 2>&1
  CG_UNIT=my-box "$RUN/cg-run.sh" "$MEM" 0,1 -- "$MYSQLD" --defaults-file="$MYCNF" --basedir="$BASEDIR" --datadir="$DATADIR" >/dev/null 2>&1
  for i in $(seq 1 60); do ss -ltn|grep -q :33061 && break; sleep 1; done
  ss -ltn|grep -q :33061 || { log "MySQL FAILED to start"; return 1; }
  local base=$(dubytes "$DATADIR")
  log "prefill $N"
  taskset -c 2-5 "$KVB" --backend mysql --addr "$H:33061" --keys "$N" --value-len 100 --connections 50 --pipeline 1 --phase prefill >/dev/null 2>&1
  local disk=$(dubytes "$DATADIR")
  log "STORE MySQL disk_bytes=$disk delta=$((disk-base)) bytes/kv=$(awk -v d="$disk" -v n="$N" 'BEGIN{printf "%.1f",d/n}')"
  measure_tput mysql "$H:33061" MySQL
  measure_lat  mysql "$H:33061" MySQL
  CG_UNIT=my-box "$RUN/cg-stop.sh" my-box >/dev/null 2>&1 || pkill -9 mysqld
  sleep 2
}

run_openkache || log "OpenKache stage error"
run_postgres  || log "PostgreSQL stage error"
run_mysql     || log "MySQL stage error"
log "=== SUITE DONE ==="
