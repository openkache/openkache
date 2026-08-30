#!/bin/sh

set -eu

OPENKACHE_REPOSITORY="openkache/openkache"
OPENKACHE_INSTALL_DIR="${OPENKACHE_INSTALL_DIR:-${HOME}/.local/bin}"
OPENKACHE_RELEASE_ROOT_URL="${OPENKACHE_RELEASE_ROOT_URL:-https://github.com/${OPENKACHE_REPOSITORY}/releases/download}"
OPENKACHE_LATEST_RELEASE_URL="${OPENKACHE_LATEST_RELEASE_URL:-https://github.com/${OPENKACHE_REPOSITORY}/releases/latest}"
OPENKACHE_VERSION="${OPENKACHE_VERSION:-}"

say() {
    printf '%s\n' "$*"
}

fail() {
    printf 'openkache installer: %s\n' "$*" >&2
    exit 1
}

need_command() {
    command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

usage() {
    cat <<'EOF'
Install OpenKache server and CLI from a tagged GitHub release.

Usage:
  install.sh [--version VERSION] [--install-dir DIRECTORY]

Options:
  --version VERSION       Install a specific version instead of the latest release.
  --install-dir DIRECTORY Install binaries into DIRECTORY (default: ~/.local/bin).
  -h, --help              Show this help.

Environment variables:
  OPENKACHE_VERSION
  OPENKACHE_INSTALL_DIR
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || fail "--version requires a value"
            OPENKACHE_VERSION="$2"
            shift 2
            ;;
        --install-dir)
            [ "$#" -ge 2 ] || fail "--install-dir requires a value"
            OPENKACHE_INSTALL_DIR="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            fail "unknown option: $1"
            ;;
    esac
done

for command_name in awk cat curl grep install mkdir mktemp rm tar uname; do
    need_command "$command_name"
done

if [ -z "$OPENKACHE_VERSION" ]; then
    say "Finding the latest OpenKache release..."
    if ! latest_release_url="$(
        curl --fail --silent --show-error --location \
            --output /dev/null \
            --write-out '%{url_effective}' \
            "$OPENKACHE_LATEST_RELEASE_URL"
    )"; then
        fail "could not find the latest tagged GitHub release"
    fi
    latest_release_tag="${latest_release_url##*/}"
    case "$latest_release_tag" in
        server-v*) OPENKACHE_VERSION="${latest_release_tag#server-v}" ;;
        *) fail "latest GitHub release does not use a server-v<version> tag" ;;
    esac
fi

case "$OPENKACHE_VERSION" in
    server-v*) OPENKACHE_VERSION="${OPENKACHE_VERSION#server-v}" ;;
    v*) OPENKACHE_VERSION="${OPENKACHE_VERSION#v}" ;;
esac

if ! printf '%s\n' "$OPENKACHE_VERSION" |
    grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'; then
    fail "invalid version: $OPENKACHE_VERSION"
fi

operating_system="$(uname -s)"
machine_architecture="$(uname -m)"

case "${operating_system}-${machine_architecture}" in
    Linux-x86_64|Linux-amd64)
        openkache_target="linux-x86_64-musl"
        ;;
    Linux-aarch64|Linux-arm64)
        openkache_target="linux-aarch64-musl"
        ;;
    Darwin-arm64)
        openkache_target="macos-arm64"
        ;;
    Darwin-x86_64)
        if command -v sysctl >/dev/null 2>&1 &&
            [ "$(sysctl -in sysctl.proc_translated 2>/dev/null || true)" = "1" ]; then
            openkache_target="macos-arm64"
        else
            fail "Intel macOS has no native release; use the Docker quick start"
        fi
        ;;
    *)
        fail "unsupported platform: ${operating_system}-${machine_architecture}; use the Docker quick start"
        ;;
esac

archive_name="openkache-server-${OPENKACHE_VERSION}-${openkache_target}.tar.gz"
release_url="${OPENKACHE_RELEASE_ROOT_URL}/server-v${OPENKACHE_VERSION}"

temporary_base="${TMPDIR:-/tmp}"
temporary_base="${temporary_base%/}"
temporary_directory="$(mktemp -d "${temporary_base}/openkache-install.XXXXXX")"
case "$temporary_directory" in
    "${temporary_base}"/openkache-install.*) ;;
    *) fail "mktemp returned an unexpected directory: $temporary_directory" ;;
esac

cleanup() {
    rm -rf "$temporary_directory"
}
trap cleanup 0
trap 'exit 1' 1 2 15

archive_path="${temporary_directory}/${archive_name}"
checksum_path="${archive_path}.sha256"

say "Downloading OpenKache ${OPENKACHE_VERSION} for ${openkache_target}..."
curl --fail --silent --show-error --location \
    --output "$archive_path" \
    "${release_url}/${archive_name}"
curl --fail --silent --show-error --location \
    --output "$checksum_path" \
    "${release_url}/${archive_name}.sha256"

expected_checksum="$(awk 'NR == 1 { print $1 }' "$checksum_path")"
case "$expected_checksum" in
    ''|*[!0123456789abcdef]*) fail "release checksum is not valid SHA-256" ;;
esac
[ "${#expected_checksum}" -eq 64 ] || fail "release checksum is not valid SHA-256"

if command -v sha256sum >/dev/null 2>&1; then
    actual_checksum="$(sha256sum "$archive_path" | awk '{ print $1 }')"
elif command -v shasum >/dev/null 2>&1; then
    actual_checksum="$(shasum -a 256 "$archive_path" | awk '{ print $1 }')"
else
    fail "sha256sum or shasum is required to verify the release"
fi

[ "$actual_checksum" = "$expected_checksum" ] || fail "release checksum verification failed"
say "Verified SHA-256 checksum."

tar -xzf "$archive_path" -C "$temporary_directory"
package_directory="${temporary_directory}/openkache-server-${OPENKACHE_VERSION}-${openkache_target}"
server_binary="${package_directory}/openkache-server"
cli_binary="${package_directory}/openkache-cli"

[ -f "$server_binary" ] || fail "release archive does not contain openkache-server"
[ -f "$cli_binary" ] || fail "release archive does not contain openkache-cli"

mkdir -p "$OPENKACHE_INSTALL_DIR"
install -m 0755 "$server_binary" "${OPENKACHE_INSTALL_DIR}/openkache-server"
install -m 0755 "$cli_binary" "${OPENKACHE_INSTALL_DIR}/openkache-cli"

say "Installed OpenKache ${OPENKACHE_VERSION}:"
say "  ${OPENKACHE_INSTALL_DIR}/openkache-server"
say "  ${OPENKACHE_INSTALL_DIR}/openkache-cli"

case ":${PATH}:" in
    *":${OPENKACHE_INSTALL_DIR}:"*)
        say "Run 'openkache-server' to start the server."
        ;;
    *)
        say "Add the install directory to PATH with:"
        say "  export PATH=\"${OPENKACHE_INSTALL_DIR}:\$PATH\""
        ;;
esac
