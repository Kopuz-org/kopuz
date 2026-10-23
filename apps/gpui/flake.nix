{
  description = "Kopuz GPUI frontend";

  inputs = {
    nixpkgs.url = "https://channels.nixos.org/nixos-unstable/nixexprs.tar.xz";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system}.extend rust-overlay.overlays.default;
          runtimeLibs = with pkgs; [
            wayland
            vulkan-loader
            libxcb
            libxkbcommon
            libx11
            fontconfig
            freetype
          ];
        in
        {
          default = pkgs.mkShell {
            name = "kopuz-gpui-dev";
            nativeBuildInputs = with pkgs; [
              (rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
              pkg-config
              clang
              lld
            ];
            buildInputs = runtimeLibs ++ [ pkgs.vulkan-headers ];
            env = {
              LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
              RUSTFLAGS = "-C link-arg=-fuse-ld=lld";
            };
          };
        }
      );
    };
}
