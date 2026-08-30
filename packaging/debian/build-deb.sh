#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 5 ]]; then
  echo "usage: build-deb.sh <version> <amd64|arm64> <server> <cli> <output-dir>" >&2
  exit 2
fi

version="$1"
architecture="$2"
server_binary="$3"
cli_binary="$4"
output_dir="$5"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(cd "${script_dir}/../.." && pwd)"

if [[ ! "${version}" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "version must be a stable three-part SemVer value" >&2
  exit 2
fi
case "${architecture}" in
  amd64|arm64) ;;
  *)
    echo "unsupported Debian architecture: ${architecture}" >&2
    exit 2
    ;;
esac
for binary in "${server_binary}" "${cli_binary}"; do
  if [[ ! -x "${binary}" ]]; then
    echo "missing executable: ${binary}" >&2
    exit 2
  fi
done

package_root="$(mktemp -d)"
trap 'rm -rf "${package_root}"' EXIT

install -d \
  "${package_root}/DEBIAN" \
  "${package_root}/etc/openkache" \
  "${package_root}/usr/bin" \
  "${package_root}/usr/lib/systemd/system" \
  "${package_root}/usr/share/doc/openkache"
install -m 0755 "${server_binary}" "${package_root}/usr/bin/openkache-server"
install -m 0755 "${cli_binary}" "${package_root}/usr/bin/openkache-cli"
install -m 0644 "${script_dir}/openkache.toml" "${package_root}/etc/openkache/openkache.toml"
install -m 0644 "${script_dir}/openkache.service" \
  "${package_root}/usr/lib/systemd/system/openkache.service"
install -m 0644 "${repository_root}/README.md" "${package_root}/usr/share/doc/openkache/README.md"
install -m 0644 "${repository_root}/LICENSE" "${package_root}/usr/share/doc/openkache/copyright"
install -m 0644 "${script_dir}/conffiles" "${package_root}/DEBIAN/conffiles"

sed \
  -e "s/@VERSION@/${version}/g" \
  -e "s/@ARCH@/${architecture}/g" \
  "${script_dir}/control.in" > "${package_root}/DEBIAN/control"

mkdir -p "${output_dir}"
output="${output_dir}/openkache_${version}_${architecture}.deb"
dpkg-deb --build --root-owner-group "${package_root}" "${output}" >&2
(
  cd "${output_dir}"
  sha256sum "$(basename "${output}")" > "$(basename "${output}").sha256"
)
echo "${output}"
