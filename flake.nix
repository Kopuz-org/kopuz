{
  description = "Kopuz - A modern music player";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      nixpkgsFor = forAllSystems (system: import nixpkgs {
        inherit system;
        overlays = [ (import rust-overlay) ];
      });
    in
    {
      devShells = forAllSystems (system:
        let
          pkgs = nixpkgsFor.${system};
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
            extensions = [ "rust-src" "rust-analyzer" ];
          };
        in
        {
          default = pkgs.mkShell {
            inherit buildInputs;

            nativeBuildInputs = with pkgs; [
              pkg-config
              cmake
              clang
              lld
              mold
              flatpak
              flatpak-builder
              rustToolchain
              dioxus-cli
              nodejs_22
            ];

            shellHook = ''
              export RUSTFLAGS="-C link-arg=-fuse-ld=lld"
              export GIO_MODULE_DIR="${pkgs.glib-networking}/lib/gio/modules/"
              export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath buildInputs}:$LD_LIBRARY_PATH"
              export WEBKIT_DISABLE_COMPOSITING_MODE="1"
            '';
          };
        }
      );

      packages = forAllSystems (system:
        let
          pkgs = nixpkgsFor.${system};
          filteredSrc = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = path: type:
              let baseName = builtins.baseNameOf (toString path); in
              baseName != "node_modules" &&
              baseName != "target" &&
              baseName != "cache" &&
              baseName != ".github" &&
              (pkgs.lib.cleanSourceFilter path type);
          };
        in
        {
          default = pkgs.callPackage ./nix/package.nix {
            src = filteredSrc;
            extraBuildInputs = [];
          };
        }
      );

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/kopuz";
        };
      });
    };
}
