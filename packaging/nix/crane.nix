{
  lib,
  stdenv,
  craneLib,
  pkg-config,
  cmake,
  openssl,
  tailwindcss_4,
  dioxus-cli,
  # Runtime deps
  yt-dlp,
  # Linux Deps
  wrapGAppsHook3,
  webkitgtk_4_1,
  gtk3,
  libsoup_3,
  glib-networking,
  alsa-lib,
  xdotool,
  wayland,
  dbus,
  # Darwin deps
  libopus,
}:
let
  pname = "kopuz";
  version = "0.6.0";

  nativeBuildInputs = [
    pkg-config
    cmake
    tailwindcss_4
    dioxus-cli
  ]
  ++ lib.optionals stdenv.isLinux [ wrapGAppsHook3 ];

  buildInputs =
    lib.optionals stdenv.isLinux [
      webkitgtk_4_1
      gtk3
      libsoup_3
      glib-networking
      alsa-lib
      openssl
      xdotool
      wayland
      dbus
      libopus
    ]
    ++ lib.optionals stdenv.isDarwin [
      libopus
    ];

  commonArgs = {
    inherit
      pname
      version
      nativeBuildInputs
      buildInputs
      ;
    strictDeps = true;
    doCheck = false;

    src =
      let
        fs = lib.fileset;
        s = ../../.;
      in
      fs.toSource {
        root = s;
        fileset = fs.intersection (fs.fromSource (lib.sources.cleanSource s)) (
          fs.unions [
            (s + /.cargo)
            (s + /crates)
            (s + /data)
            (s + /locales)

            (s + /Cargo.toml)
            (s + /Cargo.lock)
            (s + /Dioxus.toml)
            (s + /tailwind.css)
            (s + /tailwind.config.js)

            (s + /.clippy.toml)
          ]
        );
      };
  };

  # Pre-build all external deps, this derivation is cached across source changes
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
craneLib.mkCargoDerivation (
  commonArgs
  // {
    inherit cargoArtifacts;

    buildPhaseCargoCommand = ''
      tailwindcss -i tailwind.css -o crates/kopuz/assets/tailwind.css --minify

      ${lib.optionalString stdenv.isDarwin ''
              mkdir -p "$TMPDIR/fake-bin"
              cat > "$TMPDIR/fake-bin/codesign" <<'CODESIGN_EOF'
        #!/bin/sh
        exec true
        CODESIGN_EOF
              chmod +x "$TMPDIR/fake-bin/codesign"
              export PATH="$TMPDIR/fake-bin:$PATH"
      ''}

      dx build --release --platform desktop -p kopuz --offline --frozen
    '';

    installPhase = ''
      runHook preInstall

      mkdir -p $out/bin

      ${
        if stdenv.isLinux then
          ''
            cp -r target/dx/kopuz/release/linux/app/* $out/bin/

            install -Dm644 data/com.temidaradev.kopuz.desktop \
              $out/share/applications/com.temidaradev.kopuz.desktop
            substituteInPlace $out/share/applications/com.temidaradev.kopuz.desktop \
              --replace-fail "Exec=kopuz" "Exec=$out/bin/kopuz"

            install -Dm644 data/com.temidaradev.kopuz.metainfo.xml \
              $out/share/metainfo/com.temidaradev.kopuz.metainfo.xml

            install -Dm644 crates/kopuz/assets/logo.png \
              $out/share/icons/hicolor/256x256/apps/com.temidaradev.kopuz.png
          ''
        else
          ''
            cp -r target/dx/kopuz/release/macos/Kopuz.app $out/bin/kopuz.app
            macBin=$(find $out/bin/kopuz.app/Contents/MacOS -maxdepth 1 -type f | head -1)
            ln -s "$macBin" $out/bin/kopuz
          ''
      }

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
)
