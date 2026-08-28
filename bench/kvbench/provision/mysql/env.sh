# Shared environment for kvbench MySQL 8.4 provisioning (Phase 2).
# Source:  . "$(dirname "$0")/env.sh"
# Product: canonical MySQL 8.4.x from nixpkgs (mysql80 is EOL/removed).

export BENCH_ROOT="${BENCH_ROOT:-$HOME/.bench}"
export MYSQL_LINK="$BENCH_ROOT/mysql84"                 # GC-root symlink -> /nix/store/...-mysql-8.4.x
export BASEDIR="$(readlink -f "$MYSQL_LINK" 2>/dev/null || echo "$MYSQL_LINK")"
export BINDIR="$MYSQL_LINK/bin"

# Data + runtime layout (all on the SSD under $HOME).
export DATADIR="$BENCH_ROOT/mysqldata"
export SOCKET="$DATADIR/mysqld.sock"
export PIDFILE="$DATADIR/mysqld.pid"
export PORT="33061"
export BIND_ADDR="127.0.0.1"

# my.cnf lives next to these scripts.
export HERE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
export MYCNF="$HERE_DIR/my.cnf"

# Connection identity for clients (Rust mysql_async + sysbench).
export DB_NAME="kvbench"
export DB_USER="kvbench"
export DB_PASS="kvbench"

# Binaries (MySQL names, not MariaDB).
export MYSQLD="$BINDIR/mysqld"
export MYSQL="$BINDIR/mysql"
export MYSQLADMIN="$BINDIR/mysqladmin"

# Benchmark knobs.
export TABLE_SIZE="${TABLE_SIZE:-10000000}"   # >=8M; ~1.4GB on disk -> exceeds 1G box
export SYSBENCH="$BENCH_ROOT/sysbench/bin/sysbench"

export PATH="/nix/var/nix/profiles/default/bin:$PATH"
