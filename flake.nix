{
  inputs = {
    # Track `nixpkgs-unstable` branch instead of the default branch to avoid
    # package build cache misses.
    # https://discourse.nixos.org/t/nix-flakes-input-repository-branches-conventions/26772/2
    nixpkgs.url = "https://flakehub.com/f/DeterminateSystems/nixpkgs-weekly/0.1";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
          config.allowUnfree = true;
        };
        rust-toolchain = pkgs.rust-bin.nightly.latest.default.override {
          targets = [ "x86_64-unknown-linux-musl" "aarch64-unknown-linux-musl" ];
          extensions = [ "llvm-tools-preview" "rust-src" ];
        };
      in {
        devShell = pkgs.mkShell {
          buildInputs = [
            # TODO: Migrate the followings
            # curl?
            pkgs.bashInteractive             # `bash` command
            pkgs.git                         # `git` command
            pkgs.openssh                     # `ssh` command
            pkgs.nodejs                      # `node` command
            pkgs.bun  # `bun` command
            pkgs.typst # `typst` command
            pkgs.pandoc # `pandoc` command
#            pkgs.nodePackages.pnpm           # `pnpm` command
            pkgs.biome
            pkgs.font-awesome                       # `biome` command
            pkgs.d2 # `d2` command
            pkgs.poppler-utils # `pdftoppm` for visual inspection of PDFs
            pkgs.jq # `jq` command
            pkgs.pv # `pv` command for progress viewing in Brunch OS flashing
            pkgs.vboot_reference # `cgpt` utility for ChromeOS partition handling
            rust-toolchain
            pkgs.cargo-watch
            pkgs.cargo-flamegraph
            pkgs.sqlite
            pkgs.cargo-zigbuild
            pkgs.cargo-llvm-cov
            pkgs.zig
            pkgs.qemu-user
            # gemini-cli expect s `agy` binary to detect Antigravity IDE.
            # Reference: https://github.com/google-gemini/gemini-cli/issues/15553
            (pkgs.writeShellScriptBin "aipl" ''
              PRJ_ROOT=$(git rev-parse --show-toplevel)
              exec "$PRJ_ROOT/target/release/aiplang" "$@"
            '')
            (pkgs.writeShellScriptBin "agy" ''
              exec antigravity "$@"
            '')
            pkgs.python3
            pkgs.uv
            pkgs.go
            pkgs.mariadb.client
            pkgs.mitmproxy
            pkgs.httptoolkit
            pkgs.z3
            pkgs.llvm_21.dev
            pkgs.llvm_21.lib
            pkgs.llvmPackages_21.mlir
            pkgs.llvmPackages_21.mlir.dev
            pkgs.cmake
            pkgs.libclang
            pkgs.libxml2
            pkgs.pkg-config
            pkgs.openssl
            pkgs.claude-code
            pkgs.redis
            pkgs.awscli2 # `aws` command
            pkgs.libgbm
            pkgs.libdrm
            pkgs.libglvnd
            pkgs.libxkbcommon
          ];

          hardeningDisable = [ "fortify" ];
          RUSTC_BOOTSTRAP = "1";

          shellHook = ''
          # inside shellHook

          # Add wrapper scripts to PATH to prevent npm/npx usage
          export PATH="$PWD/.devshell:$PATH"

          export NODE_OPTIONS=""
          # Some nodejs programs (e.g., `vite build`, `eslint`) encounter out of memory without the following option.
          export NODE_OPTIONS="--max-old-space-size=16384 $NODE_OPTIONS"
          # p-queue package needed ESM https://github.com/sindresorhus/p-queue/issues/134
          # It's more future proof use ESM anyways.
          export NODE_OPTIONS="--experimental-vm-modules $NODE_OPTIONS"
          export TYPST_FONT_PATHS="${pkgs.font-awesome}/share/fonts/opentype/"

          # Python Virtual Environment Setup
          if [ ! -d ".venv" ]; then
            echo "Creating Python virtual environment with uv..."
            uv venv
          fi

          # source .venv/bin/activate
          # Instead of 'source .venv/bin/activate', we manually set the vars:
          export VIRTUAL_ENV="$PWD/.venv"
          export PATH="$PWD/.venv/bin:$PATH"

          # Unset PYTHONHOME to avoid conflicts with the Nix-provided Python
          unset PYTHONHOME

          if [ -f "pyproject.toml" ]; then
              echo "Syncing dependencies with uv..."
              uv sync
          fi
          '';
        };
      }
    );
}
