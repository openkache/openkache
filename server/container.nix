{ target ? "x86_64-unknown-linux-musl" }:

let
  # Keep the container build independent from a host Nix installation. These
  # are the same pinned inputs used by the repository's development flake.
  nixpkgs = builtins.fetchTarball {
    url = "https://api.flakehub.com/f/pinned/DeterminateSystems/nixpkgs-weekly/0.1.1032869%2Brev-e7a3ca8092b61ff85b6a45bf863ea2b2d6a661b3/019f8355-02b4-7432-8b0c-3d57029bf5e6/source.tar.gz";
    sha256 = "sha256-UgCQzxeWI75XM8G+hPrPh+MKzEPjG3SpAj7dtqSbksA=";
  };
  rust-overlay = builtins.fetchTarball {
    url = "https://github.com/oxalica/rust-overlay/archive/c67ce00525464a710971351c183ce67acb6ca827.tar.gz";
    sha256 = "sha256-VNbQv2P0zgaNh96mT4LrnX7hdXgiC5nBH+uvyrrVX7U=";
  };
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
    then "x86_64-unknown-linux-musl-gcc"
    else if target == "aarch64-unknown-linux-musl"
    then "aarch64-unknown-linux-musl-gcc"
    else throw "unsupported OpenKache container target: ${target}";
  target-env = builtins.replaceStrings ["-" "."] ["_" "_"] (pkgs.lib.toUpper target);
in
pkgs.mkShell {
  packages = [
    rust
    pkgs.bun
    pkgs.smithy-cli
    pkgs.gnumake
    pkgs.pkgsStatic.stdenv.cc
  ];

  shellHook = ''
    export CARGO_BUILD_TARGET=${target}
    export CARGO_TARGET_${target-env}_LINKER=${pkgs.pkgsStatic.stdenv.cc}/bin/${target-cc}
    export CC_${target-env}=${pkgs.pkgsStatic.stdenv.cc}/bin/${target-cc}
  '';
}
