#!/usr/bin/env bash
# cg-stop.sh <unit>
# Cleanly stop a cgroup/command started by cg-run.sh.
# <unit> may be given with or without the .service/.scope suffix,
# e.g. "cgrun-1234-567", "cgrun-1234-567.service", or a full CGROUP path.
set -euo pipefail

ARG="${1:?usage: cg-stop.sh <unit-name|.service|.scope|cgroup-path>}"

# Allow passing the CGROUP path printed by cg-run.sh
ARG="$(basename "$ARG")"

U="$(id -u)"
BASE="/sys/fs/cgroup/user.slice/user-${U}.slice/user@${U}.service/app.slice"

stop_unit() {
  local unit="$1"
  if systemctl --user status "$unit" >/dev/null 2>&1; then
    systemctl --user stop "$unit" && echo "stopped $unit" && return 0
  fi
  return 1
}

# Try both suffixes
if [[ "$ARG" == *.service || "$ARG" == *.scope ]]; then
  stop_unit "$ARG" && exit 0
else
  stop_unit "${ARG}.service" && exit 0
  stop_unit "${ARG}.scope" && exit 0
fi

# Fallback: kill any PIDs still in the cgroup directory, then remove it.
for suf in .service .scope ""; do
  CG="${BASE}/${ARG}${suf}"
  if [ -f "${CG}/cgroup.procs" ]; then
    echo "fallback: killing PIDs in ${CG}"
    while read -r p; do [ -n "$p" ] && kill "$p" 2>/dev/null || true; done < "${CG}/cgroup.procs"
    sleep 1
    while read -r p; do [ -n "$p" ] && kill -9 "$p" 2>/dev/null || true; done < "${CG}/cgroup.procs"
    rmdir "$CG" 2>/dev/null || true
    echo "cleaned ${CG}"
    exit 0
  fi
done

echo "cg-stop: no running unit/cgroup found for '$ARG'" >&2
exit 1
