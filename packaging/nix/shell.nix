{
  self,
  lib,
  mkShell,
  stdenv,
  just,
  flatpak,
  flatpak-builder,
  appstream,
  nodejs_22,
  yt-dlp,
  glib-networking,
  glib,
  gtk3,
  vulkan-loader,
  libGL,
  wayland,
  libxkbcommon,
  xorg,
}:
let
  kopuzPkg = self.packages.${stdenv.hostPlatform.system}.kopuz;
  # Runtime graphics libs the native (wgpu) renderer dlopens at startup. The
  # active GPU's Vulkan ICD / GL driver live in /run/opengl-driver/lib on
  # NixOS, which a nix devShell doesn't expose by default — without it wgpu
  # finds no compatible adapter and KOPUZ_BLITZ=1 panics in RequestAdapter.
  graphicsLibs = [
    vulkan-loader
    libGL
    wayland
    libxkbcommon
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
    xorg.libxcb
  ];
in
mkShell {
  name = "kopuz-dev";
  inputsFrom = [ kopuzPkg ];

  nativeBuildInputs = [
    # Dev
    just

    nodejs_22
    yt-dlp
  ]
  ++ lib.optionals stdenv.hostPlatform.isLinux [
    flatpak
    flatpak-builder
    appstream
  ];

  env = {
    GIO_MODULE_DIR = "${glib-networking}/lib/gio/modules/";
    GSETTINGS_SCHEMA_DIR = "${glib.getSchemaPath gtk3}";
    LD_LIBRARY_PATH = "${lib.makeLibraryPath kopuzPkg.buildInputs}:$LD_LIBRARY_PATH";
    WEBKIT_DISABLE_COMPOSITING_MODE = "1";
  }
  // lib.optionalAttrs stdenv.hostPlatform.isLinux {
    RUSTFLAGS = "-C link-arg=-fuse-ld=lld";
    # Prepend the NixOS hardware-driver path + windowing libs so the wgpu
    # native renderer (KOPUZ_BLITZ=1) can find a GPU adapter.
    LD_LIBRARY_PATH = "/run/opengl-driver/lib:${
      lib.makeLibraryPath (kopuzPkg.buildInputs ++ graphicsLibs)
    }:$LD_LIBRARY_PATH";
  };
}
