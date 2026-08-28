# Shared environment for kvbench PostgreSQL provisioning.
# Sourced by init.sh / start.sh / stop.sh / wipe.sh.
# All server-side; run these on serveroptima1 (via `ssh openkache-remote`).

# PostgreSQL 17.10 from the nix store (already present on the host).
export PGBIN="/nix/store/fdh93xn8lhlkdslwrgxzr8kd1qc8akga-postgresql-17.10/bin"

# Data directory on the SSD (/dev/sda1 ext4, mounted under /home).
export PGDATA="${PGDATA:-$HOME/.bench/pgdata}"

# Connection parameters.
export PGHOST="127.0.0.1"
export PGPORT="55432"
export PGDATABASE="kvbench"
# Unix socket lives inside the datadir (see unix_socket_directories in conf).
export PGSOCKDIR="$PGDATA"

# Log file (postgres logging_collector writes here: $PGDATA/log/postgresql.log).
export PGLOGDIR="log"
export PGLOGFILE="postgresql.log"

# Fail fast in scripts that opt in with `set -euo pipefail`.
export PATH="$PGBIN:$PATH"
