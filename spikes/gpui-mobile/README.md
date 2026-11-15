# gpui-mobile spike: kopuz's GPUI app on Android

The question this spike answers: can kopuz's GPUI stack render on Android and drive the kopuz daemon?
If it can, the GPUI app can later replace the legacy Dioxus app (`apps/legacy/*`), which today is the
only kopuz frontend for Android (it also covers macOS).

## What it is

- **One screen**: a library list (`api.tracks(...)`, loaded once, no live event stream), where tapping a
  row plays it (`set_queue` with `QueueMode::Replace`), and a play/pause toggle (`player_command(Toggle)`).
- Rendered by **gpui-pre 0.3.5 + gpui-pre-mobile 0.1.0** on `android.app.NativeActivity`, through wgpu on
  **Vulkan**. On the emulator, wgpu rejects the GL ES translator and picks the llvmpipe (CPU) Vulkan
  adapter, so emulator rendering is software; a real phone would use its GPU driver.
- Connected to a **real `kopuzd`** over gRPC/TCP with `client::GrpcApi::connect_tcp("127.0.0.1:50777")`.
  The daemon runs **on the Mac**. The device reaches it through `adb reverse`, so nothing listens beyond localhost.
- A standalone Cargo workspace (its own `[workspace]` and `Cargo.lock`). It touches nothing in kopuz's root
  workspace, its `[patch]` or the legacy app.

## What it is not

- Not the product app. It does not use `gpui-kit`, `link`, `store` or any other `apps/kopuz` UI crate (see
  `docs/ANALYSIS.md` section 4, `docs/UPSTREAM.md` section 1 (versions and gpui-kit) and the Versions section
  of `docs/REPORT.md` for why, and for how they can come later).
- No daemon on the device, no foreground service, no media notification, no background playback. Audio
  plays **on the Mac**, where kopuzd runs. `docs/FOREGROUND_SERVICE.md` has the plan for the on-device daemon.
- No Java or Kotlin (`android:hasCode="false"`), so no soft keyboard/IME, permissions, clipboard or intents.
- Not a Gradle project and not signed for release. The APK is built with `aapt2`, `zipalign` and
  `apksigner` and signed with a throwaway debug key.
- arm64-v8a only.

## Layout

```
spikes/gpui-mobile/
  Cargo.toml, Cargo.lock   standalone crate kopuz-mobile-spike, cdylib libkopuz_mobile_spike.so  (app builder)
  src/                     android_main, the GPUI view, the tokio/GrpcApi bridge                (app builder)
  android/                 AndroidManifest.xml (NativeActivity, lib_name, INTERNET)             (packager)
  scripts/env.sh           SDK, NDK, build-tools and linker environment, sourced by the others  (packager)
  scripts/build-apk.sh     cross-compiles the .so and packages, aligns and signs the APK        (packager)
  scripts/test-emulator.sh end-to-end run: host kopuzd + the running emulator + APK               (packager)
  docs/                    REPORT.md, ANALYSIS.md, UPSTREAM.md, FOREGROUND_SERVICE.md
  target/, build/          build output and the debug keystore (git-ignored)
```

## Prerequisites (already on the Mac; nothing is installed by the scripts)

- Android SDK with platform-tools (`adb`), the emulator, **platform android-34** and **build-tools 36.1.0**
  (`aapt2`, `zipalign` with `-P 16`, `apksigner`).
- **NDK 26.3.11579264** (`aarch64-linux-android<API>-clang`, `llvm-ar`).
- **Java 17** (for `keytool` and `apksigner`).
- **Rust 1.95**, pinned by the repo's `rust-toolchain.toml` (it applies inside `spikes/`), with the
  `aarch64-linux-android` target installed.
- For the end-to-end test: an emulator that is **already running** (for example the `Pixel_8_Pro` AVD,
  API 35, arm64, started from Android Studio). The `.so` is linked with `-z max-page-size=16384` and the
  APK aligned with `zipalign -P 16`, so 16 KB-page images work too.

`scripts/env.sh` finds these tools. Check it if your SDK is not in the default place.

## Build

From `spikes/gpui-mobile`:

```sh
scripts/build-apk.sh
```

It runs `cargo build --release --target aarch64-linux-android --lib`, puts `libkopuz_mobile_spike.so`
into `lib/arm64-v8a/`, links the manifest with `aapt2`, page-aligns, signs and verifies. The APK is
`build/kopuz-gpui-spike.apk`. `--debug` builds the debug profile; `--package-only` repackages an existing `.so`. The first build compiles wgpu, cosmic-text and tonic, so expect several minutes.

## End-to-end test (emulator)

```sh
scripts/test-emulator.sh
```

It does the following:

1. Builds a host `kopuzd` into the spike's own `target/` directory, not the repo's.
2. Runs it isolated from your real library: a throwaway database and socket under the spike's build
   directory, serving gRPC with `--listen 127.0.0.1:50777`.
3. Uses the emulator that is already running (the first `emulator-*` in `adb devices`, or
   `ANDROID_SERIAL`); it never starts or stops an emulator. It runs `adb reverse tcp:50777 tcp:50777`.
4. Installs the APK, launches it and drives it: tap a row, swipe, toggle, home and resume, density 280,
   a keyboard probe, and rotation. Screenshots go to `build/shots/`, logs to `build/logs/`, and a
   PASS/FAIL line per check to `build/logs/summary.txt`.
5. On exit it stops kopuzd and the app, removes its `adb reverse`, and restores density and rotation.
   The emulator keeps running; the APK stays installed unless you pass `--uninstall`.

While it runs, the isolated kopuzd plays the quiet test tones on the Mac and briefly owns Now Playing and
the media keys.

Read the script's header for its exact steps and output files.

## Manual run on a phone

With USB debugging on and a daemon serving TCP on the Mac:

```sh
kopuzd --listen 127.0.0.1:50777          # or the isolated one that test-emulator.sh starts
adb reverse tcp:50777 tcp:50777          # the phone's 127.0.0.1:50777 now reaches the Mac
adb install -r build/kopuz-gpui-spike.apk
adb shell am start -n moe.kopuz.gpuispike/android.app.NativeActivity
adb logcat -s kopuz-spike AndroidRuntime DEBUG
```

A plain `kopuzd` uses your real library. Pass a separate database (as the test script does) if that
matters. On the emulator, `10.0.2.2:50777` also reaches the Mac without `adb reverse`, but the app uses
`127.0.0.1:50777`.

## Docs

- [docs/ANALYSIS.md](docs/ANALYSIS.md): how the GPUI and legacy apps reach the daemon, the minimum API,
  and which `apps/kopuz` crates can be reused on mobile.
- [docs/UPSTREAM.md](docs/UPSTREAM.md): gpui-pre-mobile against kopuz's GPUI versions, Android feature
  coverage, and the build recipe.
- [docs/REPORT.md](docs/REPORT.md): the findings, the test results and the recommendation.
- [docs/FOREGROUND_SERVICE.md](docs/FOREGROUND_SERVICE.md): the plan for running the daemon on the device
  as a mediaPlayback foreground service (process model, lifecycle, Kotlin reuse, effort).

## Results

End-to-end run on 2026-11-15 on the running Pixel 8 Pro emulator (API 35). The results come from
`build/logs/summary.txt`, corrected where the screenshots show more than the script checks:

| Check | Result |
|---|---|
| APK install, launch, no crash | PASS |
| Connect to kopuzd, load 24 tracks | PASS |
| Tap a row to play (daemon reports `playing`) | PASS |
| Touch scroll (swipe) | PASS |
| Play/pause toggle | PASS |
| Home and resume | PARTIAL: the process survives and the lifecycle events fire (the script's PASS), but the window shows black after resume until the next input (`07-resumed.png`, 6 s after resume, and `08`). Likely cause: gpui-pre-mobile forces no frame after it replaces the surface (see `docs/REPORT.md`) |
| Rotation (new size 947x448) | PASS |
| Renderer | Vulkan (llvmpipe on the emulator) |
| Display density change (`wm density 280`) | FAIL: no new scale line. The source shows that gpui-pre-mobile sets the scale only when it creates the window; the `08` shot cannot confirm it (the window was still black) |
| Soft keyboard and text input | FAIL by design: the spike has no text field. The IME and keyboard gap comes from reading the source (gpui-pre-mobile needs its Java `GpuiInputActivity`); the run did not observe it |

The first run froze after loading the tracks, because the spike called a gpui-pre-mobile method that takes
the platform lock from inside a GPUI task. That is fixed in `src/lifecycle.rs`. See `docs/REPORT.md`.
