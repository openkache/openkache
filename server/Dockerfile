# syntax=docker/dockerfile:1.7

# The builder is pinned by digest and contains the complete Nix installation.
# Users only need Docker or Podman; Nix is never required on the host.
# Keep this stage on the target platform (the default): `container.nix` uses
# that platform's native static musl compiler. CI schedules one native runner
# per published architecture, so no emulation is needed.
FROM nixos/nix:2.35.1@sha256:377d4887aca98f0dfa12971c1ea6d6a625a435d8b610d4c95a436843da6fbfd1 AS builder

ARG TARGETARCH
SHELL ["/bin/sh", "-euxo", "pipefail", "-c"]

# `container.nix` pins nixpkgs, rust-overlay, Rust, Bun, and Smithy. Keep it in
# its own layer so source changes do not invalidate the toolchain evaluation.
WORKDIR /workspace
COPY server/container.nix server/flake.lock server/flake.nix server/

RUN case "${TARGETARCH}" in \
      amd64) target="x86_64-unknown-linux-musl" ;; \
      arm64) target="aarch64-unknown-linux-musl" ;; \
      *) echo "Unsupported TARGETARCH=${TARGETARCH}; publish linux/amd64 or linux/arm64 images." >&2; exit 1 ;; \
    esac; \
    nix-shell --pure server/container.nix --argstr target "${target}" --run \
      "printf 'Using Nix toolchain: %s\\n' \"\$(rustc --version)\"; printf 'Bun: %s\\n' \"\$(bun --version)\"; printf 'Smithy: %s\\n' \"\$(smithy --version | head -n 1)\""

# Keep dependency resolution in its own cacheable layer. All workspace
# manifests are copied because Cargo resolves the complete workspace even
# though only the server package is compiled.
COPY Cargo.toml Cargo.lock ./
COPY LICENSE LICENSE
COPY .cargo/config.toml .cargo/config.toml
COPY scripts/package.json scripts/bun.lock scripts/generate-third-party-notices.ts scripts/
COPY protocol/Cargo.toml protocol/Cargo.toml
COPY server/Cargo.toml server/Cargo.toml
COPY clients/core/Cargo.toml clients/core/Cargo.toml
COPY clients/value/Cargo.toml clients/value/Cargo.toml
COPY clients/rust/Cargo.toml clients/rust/Cargo.toml
COPY clients/cli/Cargo.toml clients/cli/Cargo.toml
COPY clients/typescript/native/Cargo.toml clients/typescript/native/Cargo.toml

# Cargo validates that every workspace member has a target before it can fetch
# dependencies, even when `-p openkache-server` selects only the server package.
# These metadata-only placeholders avoid copying or compiling client sources;
# the protocol and server placeholders are replaced by real sources below, and
# none of these files reaches the runtime image.
RUN mkdir -p \
      protocol/src \
      server/src \
      clients/core/src \
      clients/value/src \
      clients/rust/src \
      clients/cli/src \
      clients/typescript/native/src \
    && touch \
      protocol/src/lib.rs \
      server/src/main.rs \
      clients/core/src/lib.rs \
      clients/value/src/lib.rs \
      clients/rust/src/lib.rs \
      clients/cli/src/main.rs \
      clients/typescript/native/src/lib.rs

RUN --mount=type=cache,id=openkache-cargo-registry,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,id=openkache-cargo-git,target=/root/.cargo/git,sharing=locked \
    case "${TARGETARCH}" in \
      amd64) target="x86_64-unknown-linux-musl" ;; \
      arm64) target="aarch64-unknown-linux-musl" ;; \
      *) echo "Unsupported TARGETARCH=${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    nix-shell --pure server/container.nix --argstr target "${target}" --run 'cargo fetch --locked'

COPY protocol protocol
COPY server server

RUN --mount=type=cache,id=openkache-cargo-registry,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,id=openkache-cargo-git,target=/root/.cargo/git,sharing=locked \
    --mount=type=cache,id=openkache-target,target=/workspace/target,sharing=locked \
    case "${TARGETARCH}" in \
      amd64) target="x86_64-unknown-linux-musl" ;; \
      arm64) target="aarch64-unknown-linux-musl" ;; \
      *) echo "Unsupported TARGETARCH=${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    nix-shell --pure server/container.nix --argstr target "${target}" --run \
      "cargo server-build --target ${target}; \
       bun install --cwd scripts --frozen-lockfile --production; \
       bun ./scripts/generate-third-party-notices.ts \
         --artifact server --output /out/THIRD-PARTY-NOTICES.txt"; \
    install -D -m 0755 \
      "target/${target}/release/openkache-server" \
      /out/openkache-server; \
    install -D -m 0644 LICENSE /out/LICENSE; \
    install -d -m 0750 /out/var/lib/openkache

# A static musl binary lets the final image contain no shell, package manager,
# compiler, or runtime libraries.
FROM gcr.io/distroless/static-debian12:nonroot@sha256:f5b485ea962d9bd1186b2f6b3a061191539b905b82ec395de78cbfae51f20e35 AS runtime

ARG VERSION=dev
ARG VCS_REF=unknown

# Canonical OCI product identity. The publication workflow preserves these
# values and adds only release-specific labels such as revision and version.
LABEL org.opencontainers.image.title="OpenKache server" \
      org.opencontainers.image.description="SSD-backed RESP and QUIC cache server" \
      org.opencontainers.image.documentation="https://github.com/openkache/openkache/blob/main/docs/container-image.md" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later" \
      org.opencontainers.image.source="https://github.com/openkache/openkache" \
      org.opencontainers.image.url="https://github.com/openkache/openkache" \
      org.opencontainers.image.revision="${VCS_REF}" \
      org.opencontainers.image.version="${VERSION}"

COPY --from=builder --chown=65532:65532 /out/openkache-server /usr/local/bin/openkache-server
COPY --from=builder --chown=65532:65532 /out/LICENSE /usr/share/doc/openkache/LICENSE
COPY --from=builder --chown=65532:65532 /out/THIRD-PARTY-NOTICES.txt /usr/share/doc/openkache/THIRD-PARTY-NOTICES.txt
COPY --from=builder --chown=65532:65532 /out/var/lib/openkache /var/lib/openkache

WORKDIR /var/lib/openkache
VOLUME ["/var/lib/openkache"]
EXPOSE 4433/udp
EXPOSE 4433/tcp
STOPSIGNAL SIGTERM
USER 65532:65532

ENTRYPOINT ["/usr/local/bin/openkache-server"]
CMD ["0.0.0.0:4433", "0", "1"]
