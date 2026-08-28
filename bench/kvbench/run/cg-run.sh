#!/usr/bin/env bash
# cg-run.sh <mem> <core_spec> -- <command...>
# Run <command...> constrained to memory.max=<mem> and CPU cores <core_spec>.
#
# Mechanism (serveroptima1, verified):
#   cpuset is NOT delegated to the unprivileged user cgroup subtree, so
#   AllowedCPUs=/cpuset.cpus cannot be set without root. We therefore:
#     * cap memory via systemd-run --user (MemoryMax + MemorySwapMax=0), and
#     * pin CPUs via `taskset -c <core_spec>` (inherited by all children).
#   memory.max includes page cache (memory_recursiveprot enabled), which is
#   what forces reads to hit the SSD once the dataset exceeds <mem>.
#
# <mem>       : memory ceiling, e.g. 256M, 1G, or raw bytes (268435456)
# <core_spec> : taskset core list, default "0,1"
#
# Env vars:
#   CG_UNIT=<name>   systemd unit name (default cgrun-<pid>-<rand>)
#   CG_FG=1          run in foreground as a --scope (blocks); else background service
#
# Background (default): starts a --user service, returns immediately, prints
#   the unit name and cgroup path. Good for launching a long-lived DB server.
# Foreground (CG_FG=1): runs as a --scope and blocks until the command exits.
set -euo pipefail

MEM="${1:?usage: cg-run.sh <mem> <core_spec> -- <command...>}"; shift
CORES="0,1"
if [ "${1:-}" != "--" ]; then CORES="${1:?missing core_spec or --}"; shift; fi
[ "${1:-}" = "--" ] || { echo "cg-run.sh: expected -- before command" >&2; exit 2; }
shift
[ "$#" -ge 1 ] || { echo "cg-run.sh: no command given" >&2; exit 2; }

U="$(id -u)"
UNIT="${CG_UNIT:-cgrun-$$-${RANDOM}}"
BASE="/sys/fs/cgroup/user.slice/user-${U}.slice/user@${U}.service/app.slice"

if [ "${CG_FG:-0}" = "1" ]; then
  echo "cg-run: unit=${UNIT}.scope cgroup=${BASE}/${UNIT}.scope mem=${MEM} cores=${CORES} (foreground)" >&2
  exec systemd-run --user --scope --unit="${UNIT}" \
    -p MemoryMax="${MEM}" -p MemorySwapMax=0 \
    -- taskset -c "${CORES}" "$@"
fi

systemd-run --user --unit="${UNIT}" \
  -p MemoryMax="${MEM}" -p MemorySwapMax=0 \
  -- taskset -c "${CORES}" "$@" >&2
echo "UNIT=${UNIT}.service"
echo "CGROUP=${BASE}/${UNIT}.service"
