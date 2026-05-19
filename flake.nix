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
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsForEach system;

          # Scoped sharedArgs in one so to not leak into 'pkgs'
          sharedArgs = {
            buildInputs = with pkgs; [
              webkitgtk_4_1
              gtk3
              libsoup_3
              glib-networking
              wayland
              alsa-lib
              xdotool
              openssl
            ];

            rustToolchain = pkgs.rust-bin.stable.latest.default.override {
              extensions = [
                "rust-src"
                "rust-analyzer"
              ];
            };
          };
        in
        {
          default = pkgs.callPackage ./packaging/nix/shell.nix { inherit sharedArgs; };
        }
      );

      packages = forAllSystems (
        system:
        let
          pkgs = pkgsForEach system;
          craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;
          rustToolchain = pkgs.rust-bin.stable.latest.default.override {
            extensions = [
              "rust-src"
              "rust-analyzer"
            ];
          };
        in
        {
          default = pkgs.callPackage ./packaging/nix/crane.nix { inherit craneLib; };
        }
      );

      checks = forAllSystems (system: {
        default = self.packages.${system}.default;
      });

      # Provides the default formatter for 'nix fmt'. For maximum compatibility, nixfmt
      # has been selected here. The -tree variant is a wrapper script that formats all
      # Nix files automatically.
      formatter = forAllSystems (system: nixpkgs.legacyPackages.${system}.nixfmt-tree);
    };
}
