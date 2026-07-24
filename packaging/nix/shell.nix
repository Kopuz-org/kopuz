{
  self,
  lib,
  mkShell,
  stdenv,
  just,
  bash,
  bazelisk,
  writeShellScriptBin,
  flatpak,
  flatpak-builder,
  appstream,
  nodejs_22,
  yt-dlp,
  glib-networking,
  glib,
  gtk3,
  libayatana-appindicator,
}:
let
  kopuzPkg = self.packages.${stdenv.hostPlatform.system}.kopuz;
  nixTarget = lib.replaceStrings [ "-" ] [ "_" ] stdenv.hostPlatform.config;
  bazel = writeShellScriptBin "bazel" ''
    case "$1" in
      build|run|test|coverage|aquery)
        command="$1"
        shift
        exec ${lib.getExe bazelisk} "$command" \
          --shell_executable=${lib.getExe bash} \
          --action_env=PATH="$PATH" \
          --action_env=PKG_CONFIG_PATH="$PKG_CONFIG_PATH" \
          --action_env=NIX_LDFLAGS="$NIX_LDFLAGS" \
          --action_env=NIX_CC_WRAPPER_TARGET_HOST_${nixTarget}=1 \
          --action_env=NIX_BINTOOLS_WRAPPER_TARGET_HOST_${nixTarget}=1 \
          "$@"
        ;;
      *)
        exec ${lib.getExe bazelisk} "$@"
        ;;
    esac
  '';
in
mkShell {
  name = "kopuz-dev";
  inputsFrom = [ kopuzPkg ];

  nativeBuildInputs = [
    # Dev
    just
    bazel
    bazelisk

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
    LD_LIBRARY_PATH = "${lib.makeLibraryPath kopuzPkg.buildInputs}:${libayatana-appindicator}/lib:$LD_LIBRARY_PATH";
    WEBKIT_DISABLE_COMPOSITING_MODE = "1";
  }
  // lib.optionalAttrs stdenv.hostPlatform.isLinux {
    RUSTFLAGS = "-C link-arg=-fuse-ld=lld";
  };
}
