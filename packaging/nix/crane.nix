{
  lib,
  stdenv,
  craneLib,
  gitRev ? null,
  callPackage,
  pkg-config,
  cmake,
  git,
  openssl,
  tailwindcss_4,
  dioxus-cli,
  # Runtime deps
  ffmpeg,
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
  libayatana-appindicator,
  gst_all_1,
  # Darwin deps
  libopus,
}:
let
  pname = "kopuz-legacy";
  version = "0.22.0";

  librustyV8 = callPackage ./rusty-v8.nix { };

  nativeBuildInputs = [
    pkg-config
    cmake
    tailwindcss_4
    dioxus-cli
    git
  ]
  ++ lib.optionals stdenv.isLinux [ wrapGAppsHook3 ];

  buildInputs = [
    libopus
  ]
  ++ lib.optionals stdenv.isLinux [
    webkitgtk_4_1
    gtk3
    libsoup_3
    glib-networking
    alsa-lib
    openssl
    xdotool
    wayland
    dbus
    libayatana-appindicator
    libopus
    # WebKit plays the music video through GStreamer and aborts when it finds no
    # audio sink. wrapGAppsHook3 puts these on GST_PLUGIN_SYSTEM_PATH_1_0.
    gst_all_1.gst-plugins-base
    gst_all_1.gst-plugins-good
    gst_all_1.gst-plugins-bad
    gst_all_1.gst-libav
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
    # Only what the package ships; the GPUI app and kopuzd are packaging/nix/kopuz.nix.
    cargoExtraArgs = "--locked -p kopuz-legacy";

    # On commonArgs so both the deps-only and final derivations see it.
    RUSTY_V8_ARCHIVE = librustyV8;

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
            # build.rs embeds Android configuration even for desktop builds.
            (s + /android-src)
            (s + /shared)
            (s + /daemon)
            (s + /apps)
            # Cargo resolves the workspace's [patch] even for packages that never use it.
            (s + /vendor)
            (s + /data)
            (s + /packaging/systemd)

            (s + /Cargo.toml)
            (s + /Cargo.lock)
            (s + /Dioxus.toml)
            (s + /tailwind.css)
            (s + /tailwind.config.js)

            (s + /.clippy.toml)
          ]
        );
      };
  }
  // lib.optionalAttrs (gitRev != null) {
    KOPUZ_GIT_COMMIT = gitRev;
  };

  # Pre-build all external deps, this derivation is cached across source changes
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
craneLib.mkCargoDerivation (
  commonArgs
  // {
    inherit cargoArtifacts;
    dontWrapGApps = true;

    buildPhaseCargoCommand = ''
      tailwindcss -i tailwind.css -o apps/legacy/kopuz/assets/tailwind.css --minify

      ${lib.optionalString stdenv.isDarwin ''
              mkdir -p "$TMPDIR/fake-bin"
              cat > "$TMPDIR/fake-bin/codesign" <<'CODESIGN_EOF'
        #!/bin/sh
        exec true
        CODESIGN_EOF
              chmod +x "$TMPDIR/fake-bin/codesign"
              export PATH="$TMPDIR/fake-bin:$PATH"
      ''}

      dx build --release --platform desktop -p kopuz-legacy --offline --frozen
    '';

    installPhase = ''
      runHook preInstall

      mkdir -p $out/bin

      ${
        if stdenv.isLinux then
          ''
            cp -r target/dx/kopuz-legacy/release/linux/app/* $out/bin/

            install -Dm644 data/moe.kopuz.legacy.desktop \
              $out/share/applications/moe.kopuz.legacy.desktop
            substituteInPlace $out/share/applications/moe.kopuz.legacy.desktop \
              --replace-fail "Exec=kopuz-legacy" "Exec=$out/bin/kopuz-legacy"

            install -Dm644 apps/legacy/kopuz/assets/logo.png \
              $out/share/icons/hicolor/256x256/apps/moe.kopuz.legacy.png
          ''
        else
          ''
            cp -r target/dx/kopuz-legacy/release/macos/KopuzLegacy.app $out/bin/kopuz-legacy.app
            macBin=$(find $out/bin/kopuz-legacy.app/Contents/MacOS -maxdepth 1 -type f | head -1)
            ln -s "$macBin" $out/bin/kopuz-legacy
          ''
      }

      runHook postInstall
    '';

    preFixup = lib.optionalString stdenv.isLinux ''
      wrapProgram $out/bin/kopuz-legacy \
        "''${gappsWrapperArgs[@]}" \
        --chdir $out/bin \
        --prefix PATH : ${lib.makeBinPath [ ffmpeg ]} \
        --prefix LD_LIBRARY_PATH : ${libayatana-appindicator}/lib
    '';

    meta = {
      description = "The legacy Dioxus interface of the Kopuz music player, with the daemon built in";
      homepage = "https://github.com/Kopuz-org/kopuz";
      license = lib.licenses.eupl12;
      maintainers = with lib.maintainers; [
        temidaradev
        NotAShelf
      ];
      platforms = lib.platforms.linux ++ lib.platforms.darwin;
      mainProgram = "kopuz-legacy";
    };
  }
)
