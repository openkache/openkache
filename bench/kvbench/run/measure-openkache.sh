#!/usr/bin/env bash
# OpenKache GET measurement under the same fair box (2 cores, 1G cap).
# OpenKache uses O_DIRECT so page cache never serves its reads, but we still run
# under the identical cgroup box and cache-cold for parity with the competitors.
set -euo pipefail
BIN="$HOME/openkache-server-bench"
CFG="$HOME/.bench/bench20m.toml"
KVB="$HOME/kvbench_bin"
ADDR=127.0.0.1:7711
KEYS=20000000

rd() { awk -v d="$1" '$3==d{print $4}' /proc/diskstats; }
evict() { python3 -c '
c=[]
for i in range(68):
    b=bytearray(256*1024*1024)
    for o in range(0,len(b),4096): b[o]=1
    c.append(b)
'; }

pkill -9 openkache-server-bench 2>/dev/null || true
sleep 1
rm -f /home/kimseojin111/.bench/openkache.data

# Start OpenKache under 1G cap, cores 0,1 (net cpu 0, storage cpu 1).
bash "$HOME/kvbench/run/cg-run.sh" 1G 0,1 -- "$BIN" --config "$CFG" "$ADDR" 0 1 >/dev/null 2>&1
sleep 3
echo "server up: $(pgrep -f openkache-server-bench | head -1)"

echo "=== prefill $KEYS keys + FLUSH to SSD (kvbench, cores 2-5) ==="
taskset -c 2-5 "$KVB" --backend openkache --addr "$ADDR" --keys "$KEYS" \
  --value-len 100 --connections 50 --pipeline 32 --flush-after-prefill \
  --phase prefill 2>&1 | tail -3

echo "=== cache-cold evict (parity) ==="
evict
echo "cached_after_evict_kB=$(awk '/^Cached/{print $2}' /proc/meminfo)"

echo "=== measure GET (warmup 3s + 20s, 80 conns pipeline 32, cores 2-5) ==="
S0=$(rd sda)
taskset -c 2-5 "$KVB" --backend openkache --addr "$ADDR" --keys "$KEYS" \
  --connections 80 --pipeline 32 --warmup-ms 3000 --measure-ms 20000 \
  --phase measure 2>&1 | grep -iE "throughput|p50_us|p99_us|p99.9_us|measured_ops|hits"
S1=$(rd sda)
echo "reads_sda_delta=$((S1-S0)) over 20s (compare to measured_ops => reads/GET)"

pkill -9 openkache-server-bench 2>/dev/null || true
echo DONE
