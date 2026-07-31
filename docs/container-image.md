# Container image

The OpenKache server is published as a minimal, non-root image at
`ghcr.io/openkache/openkache`. The image is a preview distribution of the
server; the production release gate in the repository README still applies.

## Image tags

The public workflow builds Linux `amd64` and `arm64` images from the same
locked source revision.

| Tag | Meaning |
|---|---|
| `latest` | The current `main` branch build |
| `main` | The current `main` branch build |
| `1.2.3`, `1.2`, `1` | Version tags produced from a `v1.2.3` Git tag |
| `sha-<commit>` | An immutable source-revision tag |

Use a version or commit tag for a deployment that must not change underneath
you. `latest` is useful for evaluation and follows ongoing development.

## Build locally

The build context must be the repository root so Cargo can resolve the public
workspace. Docker is shown first because it is the most common OCI workflow:

```bash
docker build \
  --file server/Dockerfile \
  --tag localhost/openkache:dev \
  .
```

Podman is a compatible alternative:

```bash
podman build \
  --format docker \
  --file server/Dockerfile \
  --tag localhost/openkache:dev \
  .
```

Nix runs only inside the pinned builder image; the host needs Docker or Podman,
not a Nix installation.
Cross-architecture local builds need an arm64-capable host or equivalent
QEMU/binfmt setup; the publication workflow configures QEMU automatically.

The Dockerfile builds the `openkache-server` binary with the matching musl
target and runs the protocol Smithy/Bun generator inside the build stage.
The release command is defined once as `cargo server-build` in
`.cargo/config.toml`; the Dockerfile invokes that same alias with its target
triple. To build the server without a container, install Rust, Bun, and Smithy
CLI, then run `cargo server-build` from the repository root.
BuildKit cache mounts keep the Cargo registry and target directory out of the
image layers.

The runtime image is intentionally shell-less and runs as UID/GID `65532`.
There is no package manager or diagnostic shell in the final image. Use
`docker image inspect localhost/openkache:dev` (or
`podman image inspect localhost/openkache:dev`) and server logs for basic
verification.
The server requires a Linux host and an OCI runtime that exposes `io_uring`.
Some default seccomp profiles still deny `io_uring_setup`,
`io_uring_enter`, and `io_uring_register`, returning `ENOSYS` even when the
kernel supports io_uring. If logs report `Function not implemented`, allow
those three syscalls with a narrowly scoped custom profile. The examples below
use `seccomp=unconfined` as a compatibility fallback; do not use that broad
fallback for a hardened production deployment.

## Persistent storage

Mount `/var/lib/openkache` to durable local or block storage. The server stores
Segment files, the generated storage key, and the running-process marker there.
Do not use an ephemeral container layer for data that must survive a restart.
The storage directory must be owned or writable by UID/GID `65532`; named
Docker and Podman volumes satisfy this when first created.

## Isolated local development

The production command requires a mounted PKI bundle. For an isolated local
test, explicitly replace the image command with insecure development mode.
Docker:

```bash
docker volume create openkache-data
docker run --rm \
  --name openkache \
  --security-opt seccomp=unconfined \
  --publish 4433:4433/udp \
  --volume openkache-data:/var/lib/openkache \
  ghcr.io/openkache/openkache:latest \
  --listen 0.0.0.0:4433 \
  --insecure-development \
  --directory /var/lib/openkache \
  --certificate-out /var/lib/openkache/certificate.local.der
```

Podman:

```bash
podman volume create openkache-data
podman run --rm \
  --name openkache \
  --security-opt seccomp=unconfined \
  --publish 4433:4433/udp \
  --volume openkache-data:/var/lib/openkache:Z \
  ghcr.io/openkache/openkache:latest \
  --listen 0.0.0.0:4433 \
  --insecure-development \
  --directory /var/lib/openkache \
  --certificate-out /var/lib/openkache/certificate.local.der
```

`--insecure-development` disables client authentication and is only suitable
for a private, trusted test network. The generated certificate is written into
the storage volume; trust it from the client exactly as described in the
server's local-development documentation.

## Production mTLS deployment

Create the internal PKI on an operator workstation. The CA private key must
remain offline. Docker:

```bash
mkdir -p pki
docker run --rm \
  --user "$(id -u):$(id -g)" \
  --volume ./pki:/pki \
  ghcr.io/openkache/openkache:latest \
  pki --workspace /pki init
docker run --rm \
  --user "$(id -u):$(id -g)" \
  --volume ./pki:/pki \
  ghcr.io/openkache/openkache:latest \
  pki --workspace /pki issue-server --dns cache.example.com
docker run --rm \
  --user "$(id -u):$(id -g)" \
  --volume ./pki:/pki \
  ghcr.io/openkache/openkache:latest \
  pki --workspace /pki issue-admin operator-01
```

Podman:

```bash
podman run --rm \
  --userns=keep-id \
  --user "$(id -u):$(id -g)" \
  --volume ./pki:/pki:Z \
  ghcr.io/openkache/openkache:latest \
  pki --workspace /pki init
podman run --rm \
  --userns=keep-id \
  --user "$(id -u):$(id -g)" \
  --volume ./pki:/pki:Z \
  ghcr.io/openkache/openkache:latest \
  pki --workspace /pki issue-server --dns cache.example.com
podman run --rm \
  --userns=keep-id \
  --user "$(id -u):$(id -g)" \
  --volume ./pki:/pki:Z \
  ghcr.io/openkache/openkache:latest \
  pki --workspace /pki issue-admin operator-01
```

The Docker command uses the caller's numeric UID/GID. Podman's
`--userns=keep-id` and `:Z` bind-mount label keep the same ownership behavior
in rootless and SELinux-enabled environments. The issuing commands temporarily
mount this offline workspace into a short-lived PKI utility; never bake
`pki/authority/ca.key` into an image or mount it into the long-running server.

Generate application client identities as needed with
`pki --workspace /pki issue-client <name>`. Distribute only the client bundles
and the deployable `pki/server` directory.

Run the server with the deployable server bundle mounted read-only. The
bind-mounted data directory keeps the example writable when the PKI files are
owned by the operator. Docker:

```bash
mkdir -p openkache-data
docker run --detach \
  --name openkache \
  --user "$(id -u):$(id -g)" \
  --security-opt seccomp=unconfined \
  --publish 4433:4433/udp \
  --volume ./openkache-data:/var/lib/openkache \
  --volume ./pki/server:/etc/openkache/pki:ro \
  --read-only \
  --cap-drop=ALL \
  --security-opt=no-new-privileges \
  ghcr.io/openkache/openkache:1.2.3
```

Podman:

```bash
mkdir -p openkache-data
podman run --detach \
  --name openkache \
  --userns=keep-id \
  --user "$(id -u):$(id -g)" \
  --security-opt seccomp=unconfined \
  --publish 4433:4433/udp \
  --volume ./openkache-data:/var/lib/openkache:Z \
  --volume ./pki/server:/etc/openkache/pki:ro,Z \
  --read-only \
  --cap-drop=ALL \
  --security-opt=no-new-privileges \
  ghcr.io/openkache/openkache:1.2.3
```

The image's default command listens on `0.0.0.0:4433`, reads
`/etc/openkache/pki`, and writes cache data under `/var/lib/openkache`. The
published port is UDP because the default protocol is QUIC. Publish TCP as
well only when deliberately running the development RESP mode.

For a TOML configuration, mount it read-only and replace the default command,
for example. Docker:

```bash
mkdir -p openkache-data
docker run --detach \
  --user "$(id -u):$(id -g)" \
  --security-opt seccomp=unconfined \
  --volume ./openkache-data:/var/lib/openkache \
  --volume ./pki/server:/etc/openkache/pki:ro \
  --volume ./openkache.toml:/etc/openkache/openkache.toml:ro \
  --publish 4433:4433/udp \
  ghcr.io/openkache/openkache:1.2.3 \
  --config /etc/openkache/openkache.toml \
  --listen 0.0.0.0:4433 \
  --pki-directory /etc/openkache/pki \
  --directory /var/lib/openkache
```

Podman:

```bash
podman run --detach \
  --userns=keep-id \
  --user "$(id -u):$(id -g)" \
  --security-opt seccomp=unconfined \
  --volume ./openkache-data:/var/lib/openkache:Z \
  --volume ./pki/server:/etc/openkache/pki:ro,Z \
  --volume ./openkache.toml:/etc/openkache/openkache.toml:ro,Z \
  --publish 4433:4433/udp \
  ghcr.io/openkache/openkache:1.2.3 \
  --config /etc/openkache/openkache.toml \
  --listen 0.0.0.0:4433 \
  --pki-directory /etc/openkache/pki \
  --directory /var/lib/openkache
```

The server still enforces its normal sizing, storage ownership, and mTLS
validation inside the container. Use the same worker and Segment layout when
reopening an existing storage volume.

## Pin maintenance

The public repository pins base images and GitHub Actions to immutable
digests/commits. Dependabot opens a weekly grouped update PR for
`server/Dockerfile` and `.github/workflows`, so container and action pins stay
current without silently changing a deployment.

The Dockerfile is the canonical source for the image's static OCI product
identity: title, description, documentation, license, source, and URL. The
publication workflow deliberately removes those fields from the metadata
action's repository-derived defaults, then adds only dynamic release labels
such as creation time, revision, version, and tag. This keeps local and
published images aligned even if the GitHub repository description changes.

The Nix inputs used by `server/container.nix` are locked in
`server/flake.lock`. The scheduled `update-container-inputs` workflow runs
`nix flake update` inside the same pinned Nix builder image and opens a reviewable
PR, updating each revision and content hash as one atomic change. No Nix
installation is needed on a developer or deployment host. Review the generated
PR and let the container build validate the new toolchain before merging it.

## CI publication

`.github/workflows/publish-container.yml` builds pull requests without
publishing, then publishes `main` and semver tag builds to GHCR. It attaches
BuildKit provenance and an SBOM, uses immutable action revisions, and publishes
both supported Linux architectures.
