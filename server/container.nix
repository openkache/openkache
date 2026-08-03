{ target ? "x86_64-unknown-linux-musl" }:

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
  nixpkgs = locked-input "nixpkgs";
  rust-overlay = locked-input "rust-overlay";
  # The Dockerfile evaluates this expression on the target platform, so the
  # native static compiler below emits the selected target without a separate
  # cross toolchain.
  pkgs = import nixpkgs {
    system = builtins.currentSystem;
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
in
pkgs.mkShell.override { stdenv = pkgs.clangStdenv; } {
  packages = [
    rust
    pkgs.bun
    pkgs.smithy-cli
    pkgs.gnumake
    pkgs.pkgsStatic.clangStdenv.cc
    pkgs.llvmPackages.llvm
    pkgs.mold
  ];

  shellHook = ''
    export CARGO_BUILD_TARGET=${target}
    export CARGO_TARGET_${target-env}_LINKER=${pkgs.pkgsStatic.clangStdenv.cc}/bin/${target-cc}
    export CC=clang
    export CXX=clang++
    export AR=llvm-ar
    export RANLIB=llvm-ranlib
    export CC_${target-env}=${pkgs.pkgsStatic.clangStdenv.cc}/bin/${target-cc}
    export CXX_${target-env}=${pkgs.pkgsStatic.clangStdenv.cc}/bin/${target-cc}++
    export AR_${target-env}=${pkgs.pkgsStatic.clangStdenv.cc}/bin/${target}-ar
    export RANLIB_${target-env}=${pkgs.pkgsStatic.clangStdenv.cc}/bin/${target}-ranlib
    export RUSTFLAGS="-C link-arg=-fuse-ld=mold"
  '';
}
