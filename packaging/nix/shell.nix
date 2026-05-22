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
  deno,
  glib-networking,
  glib,
  gtk3,
}:
let
  kopuzPkg = self.packages.${stdenv.hostPlatform.system}.kopuz;
in
mkShell {
  name = "kopuz-dev";
  inputsFrom = [ kopuzPkg ];

  nativeBuildInputs = [
    # Dev
    just
    deno

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
    LD_LIBRARY_PATH = "${lib.makeLibraryPath kopuzPkg.buildInputs}:$LD_LIBRARY_PATH";
    WEBKIT_DISABLE_COMPOSITING_MODE = "1";
  };
}
