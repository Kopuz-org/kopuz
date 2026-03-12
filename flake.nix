{
  description = "Rusic - A modern music player";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };

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

        nativeBuildInputs = with pkgs; [
          pkg-config
          clang
          lld
          mold
          flatpak
          flatpak-builder
        ];
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
        devShells.default = pkgs.mkShell {
          inherit buildInputs;

          nativeBuildInputs = nativeBuildInputs ++ (with pkgs; [
            rustToolchain
            dioxus-cli
            nodejs_22
            nodePackages.npm
            flatpak
            flatpak-builder
          ]);

          shellHook = ''
            export RUSTFLAGS="-C link-arg=-fuse-ld=lld"
            export GIO_MODULE_DIR="${pkgs.glib-networking}/lib/gio/modules/"
            export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath buildInputs}:$LD_LIBRARY_PATH"
            export WEBKIT_DISABLE_COMPOSITING_MODE="1"
          '';
        };

        packages.default = pkgs.callPackage ./nix/package.nix {
          src = filteredSrc;
          extraBuildInputs = pkgs.lib.optionals pkgs.stdenv.isDarwin (
            with pkgs.darwin.apple_sdk.frameworks; [
              WebKit
              AppKit
              CoreAudio
              AVFoundation
              CoreFoundation
              Security
              SystemConfiguration
            ]
          );
        };

        apps.default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/rusic";
        };
      }
    );
}
