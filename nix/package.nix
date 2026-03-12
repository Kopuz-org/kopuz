{ lib
, stdenv
, rustPlatform
, pkg-config
, openssl
, tailwindcss_4
, dioxus-cli
, src
# Linux only
, wrapGAppsHook3 ? null
, webkitgtk_4_1 ? null
, gtk3 ? null
, libsoup_3 ? null
, glib-networking ? null
, alsa-lib ? null
, xdotool ? null
, wayland ? null
, dbus ? null
, extraBuildInputs ? []
}:

rustPlatform.buildRustPackage {
  pname = "rusic";
  version = (builtins.fromTOML (builtins.readFile ../rusic/Cargo.toml)).package.version;

  inherit src;

  cargoLock = {
    lockFile = ../Cargo.lock;
  };

  nativeBuildInputs = [
    pkg-config
    tailwindcss_4
    dioxus-cli
  ] ++ lib.optionals stdenv.isLinux [
    wrapGAppsHook3
  ];

  buildInputs = lib.optionals stdenv.isLinux [
    webkitgtk_4_1
    gtk3
    libsoup_3
    glib-networking
    alsa-lib
    openssl
    xdotool
    wayland
    dbus
  ] ++ extraBuildInputs;

  doCheck = false;

  buildPhase = ''
    runHook preBuild

    tailwindcss -i tailwind.css -o rusic/assets/tailwind.css --minify

    dx build --release --platform desktop -p rusic --offline --frozen

    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall

    mkdir -p $out/bin

    ${if stdenv.isLinux then ''
      cp -r target/dx/rusic/release/linux/app/* $out/bin/

      install -Dm644 data/com.temidaradev.rusic.desktop \
        $out/share/applications/com.temidaradev.rusic.desktop
      substituteInPlace $out/share/applications/com.temidaradev.rusic.desktop \
        --replace-fail "Exec=rusic" "Exec=$out/bin/rusic"

      install -Dm644 data/com.temidaradev.rusic.metainfo.xml \
        $out/share/metainfo/com.temidaradev.rusic.metainfo.xml

      install -Dm644 rusic/assets/logo.png \
        $out/share/icons/hicolor/256x256/apps/com.temidaradev.rusic.png
    '' else ''
      cp -r target/dx/rusic/release/macos/app/rusic.app $out/bin/rusic.app
      ln -s $out/bin/rusic.app/Contents/MacOS/rusic $out/bin/rusic
    ''}

    runHook postInstall
  '';

  preFixup = lib.optionalString stdenv.isLinux ''
    gappsWrapperArgs+=(--chdir $out/bin)
  '';

  meta = with lib; {
    description = "Rusic - A modern music player";
    license = licenses.mit;
    platforms = platforms.linux ++ platforms.darwin;
    mainProgram = "rusic";
  };
}
