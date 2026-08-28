#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 4 ]]; then
  echo "usage: render-formula.sh <version> <archive-url> <sha256> <output>" >&2
  exit 2
fi

version="$1"
archive_url="$2"
sha256="$3"
output="$4"
template="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/openkache.rb.in"

if [[ ! "${version}" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "version must be a stable three-part SemVer value" >&2
  exit 2
fi
if [[ ! "${sha256}" =~ ^[0-9a-f]{64}$ ]]; then
  echo "sha256 must contain exactly 64 lowercase hexadecimal characters" >&2
  exit 2
fi
if [[ -z "${archive_url}" ]]; then
  echo "archive URL must not be empty" >&2
  exit 2
fi

escaped_url="${archive_url//&/\\&}"
escaped_url="${escaped_url//|/\\|}"
output_dir="$(dirname -- "${output}")"
mkdir -p "${output_dir}"

sed \
  -e "s|@VERSION@|${version}|g" \
  -e "s|@URL@|${escaped_url}|g" \
  -e "s|@SHA256@|${sha256}|g" \
  "${template}" > "${output}"

if grep -Eq '@(VERSION|URL|SHA256)@' "${output}"; then
  echo "formula template still contains an unresolved placeholder" >&2
  exit 1
fi
