{
  lib,
  stdenv,
  craneLib,
  fetchurl,
  pkg-config,
  cmake,
  git,
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
  libayatana-appindicator,
  # Darwin deps
  libopus,
}:
let
  pname = "kopuz";
  version = "0.9.0";

  # `deno_core` pulls in the `v8` crate, whose build script fetches a prebuilt
  # librusty_v8 archive — impossible in the network-less Nix sandbox. Fetch it as
  # a fixed-output derivation and pass it via RUSTY_V8_ARCHIVE (build.rs
  # decompresses the .gz itself). Keep rustyV8Version in sync with Cargo.lock.
  rustyV8Version = "130.0.7";
  rustyV8Target = stdenv.hostPlatform.rust.rustcTarget;
  rustyV8Hashes = {
    "aarch64-apple-darwin" = "1sh0y3dq0llz6hfx8qgx13sc6vjbw1xzzwfrl236wx8f9w7x1nzn";
    "x86_64-apple-darwin" = "1h0j3qw5ad2c83mh36pr18s1vp598rscj3zzxpl7vynzj6q321s3";
    "aarch64-unknown-linux-gnu" = "0nli54vqcrfh9nkz7ma7230k0xmhcrk0jmfbyxcp3rxybarygvxy";
    "x86_64-unknown-linux-gnu" = "0pdp6h7vbjvq5l9lh25qilmp6xrxg7mj8m263h44f0lv9swnqix6";
  };
  librustyV8 = fetchurl {
    url = "https://github.com/denoland/rusty_v8/releases/download/v${rustyV8Version}/librusty_v8_release_${rustyV8Target}.a.gz";
    sha256 =
      rustyV8Hashes.${rustyV8Target}
        or (throw "no prebuilt librusty_v8 hash for target ${rustyV8Target}");
  };

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
            (s + /crates)
            (s + /data)

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
        --prefix LD_LIBRARY_PATH : ${libayatana-appindicator}/lib
      )
    '';

    meta = {
      description = "Fast, modern music player with Jellyfin and local library support";
      homepage = "https://github.com/temidaradev/kopuz";
      license = lib.licenses.mit;
      maintainers = with lib.maintainers; [
        temidaradev
        NotAShelf
      ];
      platforms = lib.platforms.linux ++ lib.platforms.darwin;
      mainProgram = "kopuz";
    };
  }
)
