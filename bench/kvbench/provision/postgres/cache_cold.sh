#!/usr/bin/env bash
# cache_cold.sh [gib] - unprivileged page-cache eviction.
# drop_caches needs root (unavailable here), so we create memory pressure:
# touch a large anonymous buffer so the kernel reclaims CLEAN file pages
# (no swap on this host => anon can't be paged out => file cache is evicted).
# Frees the buffer on exit. Run OUTSIDE the DB cgroup (plain ssh shell).
set -euo pipefail
GIB="${1:-16}"
before="$(awk '/^Cached:/{print $2}' /proc/meminfo)"
python3 - "$GIB" <<'PY'
import sys, ctypes
gib = int(sys.argv[1]); n = gib*(1024**3)
buf = bytearray(n)                    # commit anon memory
step = 4096
for i in range(0, n, step):           # touch every page => resident
    buf[i] = 1
PY
after="$(awk '/^Cached:/{print $2}' /proc/meminfo)"
echo "cache_cold: Cached ${before}kB -> ${after}kB (freed ~$(( (before-after)/1024 ))MB)"
