{
  sharedArgs,
  lib,
  mkShell,
  dioxus-cli,
  pkg-config,
  cmake,
  clang,
  lld,
  mold,
  flatpak,
  flatpak-builder,
  appstream,
  nodejs_22,
  yt-dlp,
  glib-networking,
  glib,
  gtk3,
}:
mkShell {
  inherit (sharedArgs) buildInputs;

  nativeBuildInputs = [
    # Build deps
    sharedArgs.rustToolchain
    dioxus-cli
    pkg-config
    cmake
    clang
    lld
    mold

    # Packaging
    flatpak
    flatpak-builder

    appstream
    nodejs_22
    yt-dlp
  ];

  env = {
    RUSTFLAGS = "-C link-arg=-fuse-ld=lld";
    GIO_MODULE_DIR = "${glib-networking}/lib/gio/modules/";
    GSETTINGS_SCHEMA_DIR = "${glib.getSchemaPath gtk3}";
    LD_LIBRARY_PATH = "${lib.makeLibraryPath sharedArgs.buildInputs}:$LD_LIBRARY_PATH";
    WEBKIT_DISABLE_COMPOSITING_MODE = "1";
  };
}
