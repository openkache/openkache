#!/usr/bin/env bash
# diskstats.sh [dev]
# Print reads-completed and sectors-read for <dev> (default sda1) from
# /proc/diskstats. Used to prove that reads are hitting the SSD.
#
# /proc/diskstats layout: major minor name f1 f2 f3 f4 f5 f6 ...
#   f1 (col 4) = reads completed successfully
#   f3 (col 6) = sectors read  (multiply by 512 for bytes)
# Sample twice around a workload and diff to measure read I/O.
set -euo pipefail

DEV="${1:-sda1}"
line="$(grep -w -- "$DEV" /proc/diskstats || true)"
if [ -z "$line" ]; then
  echo "diskstats.sh: device '$DEV' not found in /proc/diskstats" >&2
  exit 1
fi

# shellcheck disable=SC2086
set -- $line
reads_completed="$4"     # field f1
sectors_read="$6"        # field f3
bytes_read=$(( sectors_read * 512 ))

echo "dev=${DEV} reads_completed=${reads_completed} sectors_read=${sectors_read} bytes_read=${bytes_read}"
