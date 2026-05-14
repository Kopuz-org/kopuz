{ lib
, stdenv
, rustPlatform
, pkg-config
, cmake
, openssl
, tailwindcss_4
, dioxus-cli
, yt-dlp
, fetchFromGitHub
, libopus
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
}:

rustPlatform.buildRustPackage rec {
  pname = "kopuz";
  version = "0.5.5";

  src = fetchFromGitHub {
    owner = "temidaradev";
    repo = "kopuz";
    rev = "v${version}";
    hash = "sha256-ktioWgKu1JNMOfiJpHQBhY3IVn6OaYrUJuSUrYw/OW8=";
  };

  useFetchCargoVendor = true;
  cargoHash = "sha256-fvcNXNz8sN75eTGNcK0NVnryN0zItaWRFzMi+9hehq8=";

  nativeBuildInputs = [
    pkg-config
    cmake
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
    yt-dlp
    libopus
  ] ++ lib.optionals stdenv.isDarwin [
    libopus
  ];

  doCheck = false;

  buildPhase = ''
    runHook preBuild

    tailwindcss -i tailwind.css -o kopuz/assets/tailwind.css --minify

    ${lib.optionalString stdenv.isDarwin ''
      mkdir -p "$TMPDIR/fake-bin"
      cat > "$TMPDIR/fake-bin/codesign" << 'CODESIGN_EOF'
#!/bin/sh
exec true
CODESIGN_EOF
      chmod +x "$TMPDIR/fake-bin/codesign"
      export PATH="$TMPDIR/fake-bin:$PATH"
    ''}

    dx build --release --platform desktop -p kopuz --offline --frozen

    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall

    mkdir -p $out/bin

    ${if stdenv.isLinux then ''
      cp -r target/dx/kopuz/release/linux/app/* $out/bin/

      install -Dm644 data/com.temidaradev.kopuz.desktop \
        $out/share/applications/com.temidaradev.kopuz.desktop
      substituteInPlace $out/share/applications/com.temidaradev.kopuz.desktop \
        --replace-fail "Exec=kopuz" "Exec=$out/bin/kopuz"

      install -Dm644 data/com.temidaradev.kopuz.metainfo.xml \
        $out/share/metainfo/com.temidaradev.kopuz.metainfo.xml

      install -Dm644 kopuz/assets/logo.png \
        $out/share/icons/hicolor/256x256/apps/com.temidaradev.kopuz.png
    '' else ''
      # Dioxus outputs the bundle at macos/Kopuz.app (capitalised, no app/ subdir)
      cp -r target/dx/kopuz/release/macos/Kopuz.app $out/bin/kopuz.app
      # Symlink whatever binary dioxus placed in MacOS/ (name may differ in case)
      macBin=$(find $out/bin/kopuz.app/Contents/MacOS -maxdepth 1 -type f | head -1)
      ln -s "$macBin" $out/bin/kopuz
    ''}

    runHook postInstall
  '';

  preFixup = lib.optionalString stdenv.isLinux ''
    gappsWrapperArgs+=(
      --chdir $out/bin
      --prefix PATH : ${lib.makeBinPath [ yt-dlp ]}
    )
  '';

  meta = {
    description = "Fast, modern music player with Jellyfin and local library support";
    homepage = "https://github.com/temidaradev/kopuz";
    license = lib.licenses.mit;
    maintainers = with lib.maintainers; [ temidaradev ];
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
    mainProgram = "kopuz";
  };
}
