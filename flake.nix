{
  description = "Kopuz - A modern music player";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      crane,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsForEach = system: nixpkgs.legacyPackages.${system}.extend rust-overlay.overlays.default;
      mkCraneLib =
        pkgs:
        (crane.mkLib pkgs).overrideToolchain (
          p:
          # We use the rust-overlay to get the stable Rust toolchain for various targets.
          # This is not exactly necessary, but it allows for compiling for various targets
          # with the least amount of friction. Using a rust-toolchain.toml also allows us
          # to provide a stable toolchain for non-NixOS users as well.
          (p.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
        );
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsForEach system;
        in
        {
          default = pkgs.callPackage ./packaging/nix/shell.nix { inherit self; };
        }
      );

      packages = forAllSystems (
        system:
        let
          pkgs = pkgsForEach system;
          craneLib = mkCraneLib pkgs;
        in
        {
          kopuz = pkgs.callPackage ./packaging/nix/crane.nix { inherit craneLib; };
          default = self.packages.${system}.kopuz;
        }
      );

      checks = forAllSystems (system: {
        default = self.packages.${system}.default;
      });

      # Provides the default formatter for 'nix fmt'. For maximum compatibility, nixfmt
      # has been selected here. The -tree variant is a wrapper script that formats all
      # Nix files automatically.
      formatter = forAllSystems (
        system:
        let
          pkgs = pkgsForEach system;
        in
        pkgs.writeShellApplication {
          name = "nix3-fmt-wrapper";

          runtimeInputs = [
            pkgs.nixfmt
            pkgs.fd
            pkgs.deno
          ];

          text = ''
            # Format Nix files with nixfmt
            fd "$@" -t f -e nix -x nixfmt -q '{}'

            # Format Markdown files with Deno
            fd "$@" -t f -e md -e js -x deno fmt '{}'
          '';
        }
      );
    };
}
