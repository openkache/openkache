-- pgbench point-select: random exact-key GET over the prefilled range.
-- Run with -M prepared so :idx is bound as a parameter ($1) and the
-- statement is prepared once per session (fair, PG-native protocol).
-- Key is rebuilt server-side (pgbench has no client-side string format);
-- this puts a tiny constant lpad/concat cost on PG => CONSERVATIVE for PG.
-- Range 0..7,999,999 => every key exists => 100% hit, every GET returns 100B.
\set idx random(0, 7999999)
SELECT v FROM kv WHERE k = 'kvbench:' || lpad((:idx)::text, 24, '0');
