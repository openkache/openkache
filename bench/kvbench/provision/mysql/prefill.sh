#!/usr/bin/env bash
# prefill.sh -- bulk-load TABLE_SIZE rows into kvbench.kv (once).
# key = "kvbench:" + 24-digit zero-padded index; value = 100 bytes.
# Fast path: generate a TSV with awk, then LOAD DATA LOCAL INFILE (single
# sequential PK insert since keys are ascending). Idempotent: skips if the
# table already has >= TABLE_SIZE rows.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$HERE/env.sh"

N="${TABLE_SIZE}"
have="$("$MYSQL" --no-defaults -u root -S "$SOCKET" -N -e \
  "SELECT COUNT(*) FROM ${DB_NAME}.kv" 2>/dev/null || echo 0)"
if [ "${have:-0}" -ge "$N" ]; then
  echo "[prefill] kv already has $have rows (>= $N); skipping."
  exit 0
fi

TSV="$DATADIR/kv_load.tsv"
echo "[prefill] generating $N rows -> $TSV ..."
# value = exactly 100 bytes (fixed content; irrelevant to read cost, only size).
awk -v n="$N" 'BEGIN{
  v=""; for(i=0;i<100;i++) v=v "x";
  for(i=0;i<n;i++) printf "kvbench:%024d\t%s\n", i, v;
}' > "$TSV"
echo "[prefill] TSV size: $(du -h "$TSV" | cut -f1)"

echo "[prefill] LOAD DATA LOCAL INFILE ..."
"$MYSQL" --no-defaults --local-infile=1 -u root -S "$SOCKET" "$DB_NAME" <<SQL
SET foreign_key_checks=0; SET unique_checks=0;
LOAD DATA LOCAL INFILE '${TSV}' INTO TABLE kv
  FIELDS TERMINATED BY '\t' LINES TERMINATED BY '\n' (k, v);
SQL

rows="$("$MYSQL" --no-defaults -u root -S "$SOCKET" -N -e "SELECT COUNT(*) FROM ${DB_NAME}.kv")"
echo "[prefill] loaded; kv now has $rows rows."
rm -f "$TSV"
echo "[prefill] on-disk kv .ibd size:"
find "$DATADIR" -name 'kv*.ibd' -exec du -h {} \; 2>/dev/null || true
