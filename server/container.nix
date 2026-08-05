{ target ? "x86_64-unknown-linux-musl"
, system ? builtins.currentSystem
, cache-check ? false
, nixpkgs-input ? null
, rust-overlay-input ? null
}:

let
  # Keep the container build independent from a host Nix installation. The
  # flake lock is the single source of truth for both revisions and content
  # hashes; `nix flake update` updates the pair atomically.
  lock = builtins.fromJSON (builtins.readFile ./flake.lock);
  locked-input = name:
    let
      input = builtins.getAttr name lock.nodes;
      locked = input.locked;
      url =
        if builtins.hasAttr "url" locked
        then locked.url
        else if locked.type == "github"
        then "https://github.com/${locked.owner}/${locked.repo}/archive/${locked.rev}.tar.gz"
        else throw "unsupported locked input type for ${name}: ${locked.type}";
    in
      builtins.fetchTarball {
        inherit url;
        sha256 = locked.narHash;
      };
  nixpkgs =
    if nixpkgs-input == null
    then locked-input "nixpkgs"
    else nixpkgs-input;
  rust-overlay =
    if rust-overlay-input == null
    then locked-input "rust-overlay"
    else rust-overlay-input;
  # The Dockerfile evaluates this expression on the target platform, so the
  # native static compiler below emits the selected target without a separate
  # cross toolchain.
  pkgs = import nixpkgs {
    inherit system;
    overlays = [(import rust-overlay)];
  };
  rust = pkgs.rust-bin.nightly."2026-07-27".minimal.override {
    targets = [ target ];
  };
  target-cc = if target == "x86_64-unknown-linux-musl"
    then "x86_64-unknown-linux-musl-clang"
    else if target == "aarch64-unknown-linux-musl"
    then "aarch64-unknown-linux-musl-clang"
    else throw "unsupported OpenKache container target: ${target}";
  target-env = builtins.replaceStrings ["-" "."] ["_" "_"] (pkgs.lib.toUpper target);
  target-cc-env = builtins.replaceStrings ["-" "."] ["_" "_"] target;
  host-env = builtins.replaceStrings ["-" "."] ["_" "_"]
    (pkgs.lib.toUpper pkgs.stdenv.hostPlatform.rust.rustcTarget);
  # Keep the Clang wrapper on the system BFD linker.  The musl Clang wrapper
  # cannot use mold here because mold's Nix setup expects a GCC `gcc_s`
  # runtime directory, while this container intentionally exposes no GCC
  # compiler driver.
  native-clang-stdenv = pkgs.clangStdenv;
  static-clang-stdenv =
    if target == "x86_64-unknown-linux-musl"
    then pkgs.pkgsCross.musl64.clangStdenv
    else pkgs.pkgsCross."aarch64-multiplatform-musl".clangStdenv;
  container-packages = [
    rust
    pkgs.bun
    pkgs.smithy-cli
    pkgs.gnumake
    pkgs.binutils
    static-clang-stdenv.cc
    pkgs.llvmPackages.llvm
  ];
  # The Rust overlay is intentionally excluded: it is not a nixpkgs output and
  # is built from the pinned overlay toolchain when the container is compiled.
  cacheable-container-packages = [
    pkgs.bun
    pkgs.smithy-cli
    pkgs.gnumake
    pkgs.binutils
    pkgs.clangStdenv.cc
    static-clang-stdenv.cc
    pkgs.llvmPackages.llvm
  ];
  nixpkgs-cache-check = pkgs.linkFarm "openkache-container-nixpkgs-cache-check"
    (pkgs.lib.imap0 (index: package: {
      name = "package-${toString index}";
      path = package;
    }) cacheable-container-packages) // {
      cachePaths = map toString cacheable-container-packages;
    };
in
if cache-check
then nixpkgs-cache-check
else pkgs.mkShell.override { stdenv = native-clang-stdenv; } {
  packages = container-packages;

  shellHook = ''
    # The prebuilt Rust package carries a GCC runtime closure as a propagated
    # Nix input. Keep that runtime available, but remove GNU compiler drivers
    # and host compiler directories so native actions can only select Clang.
    openkache_compiler_path=""
    openkache_previous_ifs="$IFS"
    IFS=:
    for openkache_path_entry in $PATH; do
      case "$openkache_path_entry" in
        *gcc-*/bin|/bin|/sbin|/usr/bin|/usr/sbin|/usr/local/bin|/usr/local/sbin)
          continue
          ;;
      esac
      if [ -z "$openkache_compiler_path" ]; then
        openkache_compiler_path="$openkache_path_entry"
      else
        openkache_compiler_path="$openkache_compiler_path:$openkache_path_entry"
      fi
    done
    IFS="$openkache_previous_ifs"
    export PATH="$openkache_compiler_path"
    unset openkache_compiler_path openkache_previous_ifs openkache_path_entry
    if command -v gcc >/dev/null 2>&1 || command -v g++ >/dev/null 2>&1; then
      echo "ERROR: GNU GCC compiler drivers remain available in the container Nix shell." >&2
      echo "Why: the container build must use the pinned static Clang toolchain." >&2
      echo "Fix: remove the inherited GCC paths and re-enter nix-shell." >&2
      exit 1
    fi
    openkache_gnu_driver=""
    openkache_previous_ifs="$IFS"
    IFS=:
    for openkache_path_entry in $PATH; do
      for openkache_driver in \
        "$openkache_path_entry"/gcc \
        "$openkache_path_entry"/g++ \
        "$openkache_path_entry"/*-gcc \
        "$openkache_path_entry"/*-g++; do
        if [ -x "$openkache_driver" ]; then
          openkache_gnu_driver="$openkache_driver"
          break 2
        fi
      done
    done
    IFS="$openkache_previous_ifs"
    unset openkache_previous_ifs openkache_path_entry openkache_driver
    if [ -n "$openkache_gnu_driver" ]; then
      echo "ERROR: target-prefixed GNU compiler driver remains available: $openkache_gnu_driver" >&2
      echo "Why: cross build scripts must select the pinned target Clang wrapper, never a GCC driver." >&2
      echo "Fix: remove the inherited GCC compiler path and re-enter nix-shell." >&2
      exit 1
    fi
    unset openkache_gnu_driver
    export CARGO_BUILD_TARGET=${target}
    export CARGO_TARGET_${host-env}_LINKER=${pkgs.clangStdenv.cc}/bin/clang
    export CARGO_TARGET_${target-env}_LINKER=${static-clang-stdenv.cc}/bin/${target-cc}
    export CC=clang
    export CXX=clang++
    export AR=llvm-ar
    export RANLIB=llvm-ranlib
    export CMAKE_AR=llvm-ar
    export CMAKE_RANLIB=llvm-ranlib
    export CC_${target-cc-env}=${static-clang-stdenv.cc}/bin/${target-cc}
    export CXX_${target-cc-env}=${static-clang-stdenv.cc}/bin/${target-cc}++
    export AR_${target-cc-env}=${pkgs.llvmPackages.llvm}/bin/llvm-ar
    export RANLIB_${target-cc-env}=${pkgs.llvmPackages.llvm}/bin/llvm-ranlib
    export RUSTFLAGS="-C link-arg=-fuse-ld=bfd"
  '';
}
