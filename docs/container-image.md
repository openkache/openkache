# Container image

The OpenKache preview image is published at `ghcr.io/openkache/openkache` for
Linux `amd64` and `arm64`. It contains one static `openkache-server` binary and
runs as UID/GID `65532` without a shell or package manager.

## Build

Build from the repository root:

```bash
docker build \
  --file server/Dockerfile \
  --tag localhost/openkache:dev \
  .
```

Podman can use the same build context and Dockerfile.

## Run

```bash
docker volume create openkache-data
docker run --rm \
  --name openkache \
  --security-opt seccomp=unconfined \
  --publish 4433:4433/tcp \
  --publish 4433:4433/udp \
  --volume openkache-data:/var/lib/openkache \
  localhost/openkache:dev
```

The image listens on `0.0.0.0:4433`, with networking pinned to CPU 0 and
storage pinned to CPU 1. Override the command when the container receives a
different CPU set:

```bash
docker run --rm \
  --security-opt seccomp=unconfined \
  --cpuset-cpus 2,3 \
  --publish 4433:4433/tcp \
  --publish 4433:4433/udp \
  --volume openkache-data:/var/lib/openkache \
  localhost/openkache:dev \
  0.0.0.0:4433 2 3
```

TCP serves RESP and UDP serves OpenKache Gate 0 over QUIC. The server generates
an ephemeral self-signed certificate and does not authenticate clients, so the
image is currently suitable only for development or isolated evaluation.

## Storage

The image runs in `/var/lib/openkache` and creates `openkache.data` there. The
file has a fixed logical size of 16 GiB and is truncated on every process
start. A volume therefore provides writable space but not restart recovery in
the current preview.

## `io_uring`

The server requires `io_uring_setup`, `io_uring_enter`, and
`io_uring_register`. Some container seccomp profiles deny these calls. The
examples use `seccomp=unconfined` as a compatibility fallback; use a narrowly
scoped profile when running outside an isolated development environment.

## Tags and publication

| Tag | Meaning |
| --- | --- |
| `main` | Current `main` build |
| `sha-<commit>` | Immutable source revision |
| `1.2.3`, `1.2`, `1` | Tags generated from `v1.2.3` |

Pull requests build without publishing. Pushes to `main` and matching version
tags publish a multi-platform manifest with provenance and an SBOM.

The Nix toolchain inputs used by `server/Dockerfile` are locked in
`server/flake.lock` and updated by the scheduled container-input workflow.
