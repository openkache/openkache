#!/usr/bin/env bash
# Install OpenKache's git hooks into this clone.
#
# Git does not track files under .git/hooks, so each clone installs them
# once. Run from anywhere in the repo:
#
#     ./scripts/install-hooks.sh
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
hooks_dir="$(git rev-parse --git-common-dir)/hooks"
mkdir -p "$hooks_dir"

install -m 0755 "${repo_root}/scripts/pre-commit" "${hooks_dir}/pre-commit"
echo "Installed pre-commit hook -> ${hooks_dir}/pre-commit"
