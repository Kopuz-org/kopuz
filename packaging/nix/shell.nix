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
  ffmpeg,
  glib-networking,
  glib,
  gtk3,
  libayatana-appindicator,
  mesa,
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

    nodejs_22
    ffmpeg
  ]
  ++ lib.optionals stdenv.hostPlatform.isLinux [
    flatpak
    flatpak-builder
    appstream
  ];

  env = lib.optionalAttrs stdenv.hostPlatform.isLinux {
    GIO_MODULE_DIR = "${glib-networking}/lib/gio/modules/";
    GSETTINGS_SCHEMA_DIR = "${glib.getSchemaPath gtk3}";
    LD_LIBRARY_PATH = "${lib.makeLibraryPath kopuzPkg.buildInputs}:${libayatana-appindicator}/lib:$LD_LIBRARY_PATH";
    WEBKIT_DISABLE_COMPOSITING_MODE = "1";
    RUSTFLAGS = "-C link-arg=-fuse-ld=lld";
  };

  shellHook = lib.optionalString stdenv.hostPlatform.isLinux ''
    export __EGL_VENDOR_LIBRARY_DIRS="''${__EGL_VENDOR_LIBRARY_DIRS-/run/opengl-driver/share/glvnd/egl_vendor.d:/etc/glvnd/egl_vendor.d:/usr/share/glvnd/egl_vendor.d:${mesa}/share/glvnd/egl_vendor.d}"
    export GBM_BACKENDS_PATH="''${GBM_BACKENDS_PATH-/run/opengl-driver/lib/gbm:${mesa}/lib/gbm}"
  '';
}
