{
  lib,
  craneLib,
  gitRev ? null,
  callPackage,
  makeWrapper,
  pkg-config,
  cmake,
  git,
  openssl,
  alsa-lib,
  libopus,
  ffmpeg,
  libcdio,
  libcdio-paranoia,
  wayland,
  vulkan-loader,
  libxcb,
  libxkbcommon,
  libx11,
  fontconfig,
  freetype,
}:
let
  # The GPUI app, with kopuzd and kopuzctl beside it in bin/, where the app looks for the daemon.
  pname = "kopuz";
  inherit ((lib.importTOML ../../Cargo.toml).workspace.package) version;

  # Linked or dlopened by gpui at runtime.
  gpuiLibs = [
    wayland
    vulkan-loader
    libxcb
    libxkbcommon
    libx11
    fontconfig
    freetype
  ];

  commonArgs = {
    inherit pname version;
    nativeBuildInputs = [
      pkg-config
      cmake
      makeWrapper
      # rookie's build script shells out to git.
      git
    ];
    buildInputs = [
      openssl
      alsa-lib
      libopus
    ]
    ++ gpuiLibs;
    strictDeps = true;
    doCheck = false;
    cargoExtraArgs = "--locked -p kopuz -p kopuz-kopuzd -p kopuz-ctl";
    RUSTY_V8_ARCHIVE = callPackage ./rusty-v8.nix { };

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
            (s + /shared)
            (s + /daemon)
            # Every workspace member's manifest has to be there, including the legacy app's.
            (s + /apps)
            (s + /vendor)
            (s + /data)
            (s + /packaging/systemd)
            (s + /Cargo.toml)
            (s + /Cargo.lock)
            (s + /.clippy.toml)
          ]
        );
      };
  };

  # crane stubs every crate in src for the deps build, but the patched gpui platform crate is a dependency.
  dummySrc = craneLib.mkDummySrc {
    inherit (commonArgs) src;
    extraDummyScript = ''
      rm -rf $out/vendor/gpui-pre-linux
      cp -r ${lib.cleanSource ../../vendor/gpui-pre-linux} $out/vendor/gpui-pre-linux    '';
  };

  cargoArtifacts = craneLib.buildDepsOnly (commonArgs // { inherit dummySrc; });
in
craneLib.buildPackage (
  commonArgs
  # Only on the final build, so the deps build keeps one hash across commits and Cachix can serve it.
  // lib.optionalAttrs (gitRev != null) {
    KOPUZ_GIT_COMMIT = gitRev;
  }
  // {
    inherit cargoArtifacts;

    postInstall = ''
      install -Dm644 packaging/systemd/kopuzd.service $out/share/systemd/user/kopuzd.service
      substituteInPlace $out/share/systemd/user/kopuzd.service \
        --replace-fail "ExecStart=kopuzd" "ExecStart=$out/bin/kopuzd"

      install -Dm644 data/moe.kopuz.kopuz.desktop $out/share/applications/moe.kopuz.kopuz.desktop
      substituteInPlace $out/share/applications/moe.kopuz.kopuz.desktop \
        --replace-fail "Exec=kopuz" "Exec=$out/bin/kopuz"
      install -Dm644 data/moe.kopuz.kopuz.metainfo.xml $out/share/metainfo/moe.kopuz.kopuz.metainfo.xml
      install -Dm644 apps/legacy/kopuz/assets/logo.png \
        $out/share/icons/hicolor/256x256/apps/moe.kopuz.kopuz.png

      wrapProgram $out/bin/kopuz --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath gpuiLibs}
      # kopuzd dlopens libcdio for audio CDs, so it is found at runtime rather than linked.
      wrapProgram $out/bin/kopuzd --prefix PATH : ${lib.makeBinPath [ ffmpeg ]} \
        --prefix LD_LIBRARY_PATH : ${
          lib.makeLibraryPath [
            libcdio
            libcdio-paranoia
          ]
        }
    '';

    meta = {
      description = "Fast, modern music player with Jellyfin and local library support";
      homepage = "https://github.com/Kopuz-org/kopuz";
      license = lib.licenses.eupl12;
      maintainers = with lib.maintainers; [
        temidaradev
        NotAShelf
      ];
      # gpui compiles its Metal shaders with xcrun, which the Darwin sandbox does not have.
      platforms = lib.platforms.linux;
      mainProgram = "kopuz";
    };
  }
)
