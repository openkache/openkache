{
  description = "Pinned inputs for the OpenKache server container toolchain";

  inputs = {
    nixpkgs.url = "https://flakehub.com/f/DeterminateSystems/nixpkgs-weekly/0.1";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  # `container.nix` is intentionally kept as a traditional nix-shell
  # expression so the Dockerfile can pass its target triple. It reads the
  # locked input revisions and hashes from this flake's flake.lock.
  outputs = { nixpkgs, rust-overlay, ... }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      target-for-system = system:
        if system == "x86_64-linux"
        then "x86_64-unknown-linux-musl"
        else if system == "aarch64-linux"
        then "aarch64-unknown-linux-musl"
        else throw "unsupported OpenKache container cache-check system: ${system}";
    in {
      cacheManifestSystems = systems;
      cacheManifests = builtins.listToAttrs (map (system: {
        name = system;
        value = {
          container = import ./container.nix {
            inherit system;
            nixpkgs-input = nixpkgs;
            rust-overlay-input = rust-overlay;
            target = target-for-system system;
            cache-manifest = true;
          };
        };
      }) systems);
      packages = builtins.listToAttrs (map (system: {
        name = system;
        value = {
          nixpkgs-cache-check = import ./container.nix {
            inherit system;
            nixpkgs-input = nixpkgs;
            rust-overlay-input = rust-overlay;
            target = target-for-system system;
            cache-check = true;
          };
        };
      }) systems);
    };
}
