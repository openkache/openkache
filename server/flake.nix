{
  description = "Pinned inputs for the OpenKache server container toolchain";

  inputs = {
    nixpkgs.url = "https://flakehub.com/f/DeterminateSystems/nixpkgs-weekly/0.1";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  # `container.nix` is intentionally kept as a traditional nix-shell
  # expression so the Dockerfile can pass its target triple. It reads the
  # locked input revisions and hashes from this flake's flake.lock.
  outputs = { ... }: { };
}
