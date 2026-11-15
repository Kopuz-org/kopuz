# Upstream mobile GPUI crates vs kopuz (research for the Android spike)

Job jb897aa19, 2026-11-15. All crate sources were downloaded from crates.io and read, not built (the spike
was built and run afterwards; the results are in REPORT.md).
Paths: `M/` = gpui-pre-mobile 0.1.0 (`src/...`), `G/` = gpui-pre 0.3.5, `W/` = gpui-pre-wgpu 0.3.5,
`K/` = gpui-kit 0.6.4. `EX/` = `example/` in github.com/longbridge/gpui-mobile at commit
e65684f2 (the commit the crate was published from; `example/` is not in the .crate file).

## 1. Verdict

* **The task brief is wrong about the version.** The README of gpui-pre-mobile 0.1.0 says it
  "targets gpui-pre and gpui-pre-wgpu 0.3.4" (M/README.md:21, :27). Its real manifest pins
  **`gpui-pre = "=0.3.5"`** (default-features off) and **`gpui-pre-wgpu = "=0.3.5"`** (android and ios
  targets), both in Cargo.toml.orig and in the crates.io dependencies API. The code agrees: it
  implements `Platform::prevent_idle_sleep` / `on_system_sleep` (M/android/platform.rs:1176, :1122)
  and `PlatformWindow::visibility` (M/android/window.rs:1150). Those are trait methods that were
  added between 0.3.4 and 0.3.5, so the crate would not compile against 0.3.4. The README is just stale.
  The dependency lines, from the cargo registry copy
  (`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-pre-mobile-0.1.0/`):

  ```toml
  # Cargo.toml.orig
  66:  gpui = { package = "gpui-pre", version = "=0.3.5", default-features = false }
  83:  gpui_wgpu = { package = "gpui-pre-wgpu", version = "=0.3.5", features = ["font-kit"] }   # iOS
  114: gpui_wgpu = { package = "gpui-pre-wgpu", version = "=0.3.5" }                          # Android
  # Cargo.toml (normalized by cargo publish)
  119: [dependencies.gpui]
  120: version = "=0.3.5"
  121: default-features = false
  122: package = "gpui-pre"
  146: [target.'cfg(target_os = "android")'.dependencies.gpui_wgpu]
  147: version = "=0.3.5"
  148: package = "gpui-pre-wgpu"
  ```
* **kopuz is already on 0.3.5** (Cargo.lock: gpui-pre and every gpui-pre-* crate at 0.3.5, wgpu
  29.0.4, cosmic-text 0.19.0, gpui-kit 0.6.4). So the gap is **zero**, not 0.3.4 against 0.3.5.
* gpui-kit 0.6.4, gpui-component 0.6.4 and gpui-base 0.6.4 all ask for `gpui-pre ^0.3.5` (a caret
  requirement), so `=0.3.5` from gpui-pre-mobile fits. gpui-pre 0.3.5 pins its own family at `=0.3.5`.
  **One dependency graph works with no fork and no [patch].**
* **gpui-kit 0.6.4 is already mobile-aware.** `gpui-pre-platform` (the desktop backends: linux,
  macos, windows) is only a dependency under `cfg(not(any(target_os="ios", target_os="android")))`
  (K/Cargo.toml, `[target...dependencies.gpui_platform]`). `gpui_kit::platform` and
  `gpui_kit::application()` are gated the same way (K/src/lib.rs:107-108, :148-149). On Android
  the app passes the platform itself. gpui-component has only optional tree-sitter (gated
  not-wasm) and `notify` 7 (inotify, which works on Android). gpui-base's only OS deps are `ashpd` on
  `target_os="linux"` (false on Android) and objc2 on macOS. gpui-base already switches on mobile
  behaviour for android (`scroll_bounce.rs:114`, `lib.rs:210`). The upstream example uses gpui-kit
  on Android (`EX/src/lib.rs:232-233`, `gpui_kit::init`, `Theme::change`, `Root`) from gpui-kit git
  rev 9504b46. GitHub's compare API says gpui-kit 0.6.4 (vcs sha 3c387ae) is **38 commits
  ahead of and 0 behind** that rev, so 0.6.4 contains everything the example relied on.
  Not proven by a build here. Upstream CI only runs `cargo check` for Android
  (M/.github/workflows/ci.yml:97-146) and never links or packages.
* kopuz's `[patch.crates-io] gpui-pre-linux` (root Cargo.toml:223-225) does not matter on Android.
  `target_os="linux"` is false there, and a standalone spike workspace does not see it anyway.
* Upstream has moved on: GitHub `main` (2026-11-03, #45) pins `=0.3.10`, adds TalkBack via
  `accesskit_android` (#25) and vendors a patched gpui-pre-wgpu. None of that is published.
  crates.io has only 0.1.0, which still matches kopuz. gpui-pre 0.3.10 would need gpui-kit 0.8.1
  (gpui-kit 0.6.6 `=0.3.6`, 0.7.3/0.8.0 `=0.3.9`, 0.8.1 `=0.3.10`, 0.9.0 `=0.3.11`, 0.9.1 `=0.3.12`).
  If kopuz upgrades gpui-kit before gpui-pre-mobile republishes, the two drift apart.

## 2. The crates

| | gpui-pre-mobile 0.1.0 | gpui-mobile 0.1.0 |
|---|---|---|
| Published | 2026-09-15, Longbridge fork (longbridge/gpui-mobile, a fork of itsbalamurali/gpui-mobile) | 2026-03-03, itsbalamurali |
| Content | Full Platform impl for iOS and Android, about 9.2k lines in src/android, plus 30 "packages" | **Name placeholder.** Cargo.toml has `[dependencies]` empty; lib.rs only exports `momentum`. The real code (src/android/*, Cargo.toml.full) targets zed git `gpui 0.2.2` and wgpu 28 |
| GPUI | gpui-pre =0.3.5 | zed's gpui from git (not on crates.io) |
| License | GPL-3.0+ OR AGPL-3.0+ OR Apache-2.0 (Apache works for kopuz's EUPL) | same |
| Maturity | 110 downloads, one release, README says "experimental platform release" and that "IME composition, accessibility, mobile lifecycle hooks" are not done | 120 downloads; upstream repo has 526 stars |

**Use gpui-pre-mobile.** gpui-mobile cannot be used: it holds no platform code and its source
targets zed's gpui, not gpui-pre. kopuz uses gpui-pre.

## 3. How the platform is chosen

* gpui-pre 0.3.5 has no built-in platform. The app calls `Application::with_platform(Rc<dyn Platform>)`
  (G/src/app.rs:175). `new_inaccessible` (app.rs:190) does the same with AccessKit off.
* `gpui-pre-platform::current_platform()` (gpui_platform.rs:57) only has macos, windows,
  linux/freebsd and wasm branches. It has **no android branch**. This is why gpui-kit gates it out.
* gpui-pre-mobile provides `gpui_mobile::current_platform(headless)` (M/android/mod.rs:184). For a
  real app the documented path is `jni::init_platform(&app)` (M/android/jni.rs:1030), then
  `jni::shared_platform()` (jni.rs:321), then `SharedPlatform::into_rc()` (platform.rs:1285-1296),
  passed to `Application::with_platform`.
* `Platform::run` **blocks** and drives the android-activity loop (platform.rs:481-505, `run_event_loop`
  jni.rs:534). The `run(|cx| ...)` closure is deferred until `InitWindow` has produced the
  primary window (jni.rs:756-781). `cx.open_window` wraps that window (platform.rs:1028-1053) and fails if
  called earlier. Android has exactly one fullscreen window.

## 4. Android entry point and rendering

* Uses `android-activity 0.6` with the **`native-activity`** feature (not GameActivity). jni.rs:1-36, :84.
  The app defines `#[no_mangle] fn android_main(app: AndroidApp)` (EX/src/lib.rs:83-133) and must
  depend on android-activity itself to name `AndroidApp`. With edition 2024 that is `#[unsafe(no_mangle)]`.
* crate-type `cdylib`. Manifest activity is `android.app.NativeActivity` (or a subclass) with
  `<meta-data android:name="android.app.lib_name" android:value="<lib name without lib/.so>">`.
  No `[package.metadata.android]` is needed. That table only matters to cargo-apk/xbuild, which upstream does not use.
* Rendering goes through `gpui_wgpu::WgpuRenderer` on the `ANativeWindow` (M/android/window.rs:1036-1052).
  The instance is created with `Backends::VULKAN | Backends::GL` (W/src/wgpu_context.rs:292) and
  adapters are sorted to prefer Vulkan over GL (wgpu_context.rs:345-352). GL ES is the fallback.
* Text uses `gpui_wgpu::CosmicTextSystem::new("Roboto")` (platform.rs:277). It explicitly loads
  /system/fonts/Roboto*, Noto* and DroidSans* (platform.rs:279+). Emoji can come from APK assets (platform.rs:227).
* Optional Java: `GpuiInputActivity` (IME), `GpuiActivity` (splash), and `Gpui*.java` for packages. JNI calls
  look these up through `find_app_class` (jni.rs:175) and fail soft (log only) when they are missing. **The spike needs no
  Java at all** (`android:hasCode="false"`).

## 5. Feature coverage (Android, crates.io 0.1.0)

| Feature | Status | Evidence |
|---|---|---|
| Touch to GPUI | Present | MotionEvent (jni.rs:381-472, per-pointer down/up :437-452) goes to `handle_touch` (window.rs:696) and then raw `PlatformInput::Touch` (window.rs:1336-1380). gpui-pre core recognizes gestures: tap becomes `ClickEvent::Touch`, pan becomes `ScrollWheelEvent`, plus pinch and long press (G/src/gestures.rs:1-11) |
| Scroll and fling | Present, iOS feel | Momentum is in gpui core. The platform does not override `Platform::gestures()` (default `None`, G/src/platform.rs:286), so `GestureTuning::default()` applies with `ScrollPhysics::ios()` (gestures.rs:134-141, used at G/src/window.rs:2066-2070). `ScrollPhysics::android()` (gestures.rs:189) is unused. `M/src/momentum.rs` is not used on Android |
| Hardware keys | Present | jni.rs:474-523, keyboard.rs |
| Soft keyboard | Partial | `gpui_mobile::show_keyboard()/hide_keyboard()` (lib.rs:102-146), then jni.rs:1208/:1240. Show calls `activity.gpuiShowKeyboard` and **needs GpuiInputActivity.java**, otherwise it only logs a warning. Hide falls back to `app.hide_soft_input`. Not tied to GPUI focus: the app must call it. `set_keyboard_height` has no Android caller, so keyboard height reads as 0 |
| IME and text input | Partial | `PlatformInputHandler` is stored (window.rs:1205). Composition uses `replace_and_mark_text_in_range` / `unmark_text` (text_input.rs:41-120), fed by `Java_dev_gpui_mobile_GpuiInputActivity_nativeIme` (jni.rs:1280). Needs the Java InputConnection proxy. The README admits composition is incomplete |
| Accessibility | Missing | No `a11y_*` overrides, so the gpui-pre no-op defaults apply (G/src/platform.rs:1066-1072). TODO.md lists "Assistive technologies". TalkBack exists only on unpublished main (0.3.10) |
| Lifecycle | Partial | Pause and resume are deferred flags (jni.rs:887-896, then :712-735, `did_enter_background`/`set_active`). Surface destroyed (jni.rs:608-620, `term_window` window.rs:499) and recreated (jni.rs:622-680, `replace_surface` keeps the atlas, window.rs:429-470). Destroy calls quit (jni.rs:919). **SaveState is only logged** (jni.rs:911). LowMemory is only logged (jni.rs:915); `on_memory_warning` is not implemented. Rendering runs continuously while active, with a 0.5 ms sleep (jni.rs:784-802): a battery cost. **No frame is forced after a surface replacement.** `handle_resize` returns early on an unchanged size (window.rs:538-541), and frame requests set `force_render` only for pending text (window.rs:1299-1306). In the run, the window stayed black after resume until a tap (REPORT.md) |
| Scale and density | Present at launch; runtime density change not applied | `density_dpi / 160` from AConfiguration (display.rs:69-89). Touch is converted to logical px (window.rs:1366-1370). The window's scale is set only in `AndroidWindow::new` (window.rs:336-371). ConfigChanged does not touch it (jni.rs:695-709), and `handle_resize` returns early on an unchanged pixel size (window.rs:538-541) and otherwise reuses the stored scale (window.rs:553) |
| Safe areas and insets | Partial | content_rect is read into `update_safe_area_from_content_rect` (window.rs:883-905). It is reachable only through `AndroidWindow::safe_area_insets_logical()` (window.rs:873). `PlatformWindow::insets()` is not implemented, so GPUI's `WindowInsets` stays zero (G/src/platform.rs:1035) |
| Dark mode | Partial | The window's appearance (`PlatformWindow::appearance`, window.rs:1176-1185) starts as **Light** (window.rs:373) whatever the system setting is. Night mode is read only on ConfigChanged (jni.rs:340, :695-709, the only caller of `set_appearance`), so a phone that is already in dark mode at launch reports Light (the run logged `appearance=Light`, logcat-spike.txt:57). Only the app-level `Platform::window_appearance` is hard-coded Dark (platform.rs:1056-1057), so the two can disagree |
| Clipboard | Stub | In-process string only (platform.rs:64-87, :1199-1213). A real one is in `packages/clipboard` (feature) and needs GpuiClipboard.java |
| open_url, open_with_system | Stub | platform.rs:1063, :1107 (logs only) |
| Thermal | Stub | platform.rs:813-816 |

Known gaps from `grep -rn -E 'TODO|FIXME|unimplemented|todo!|not yet|stub'` in src/android: the
clipboard and credential-store stubs (platform.rs:64, :89), "Intent launch not yet implemented"
(platform.rs:1063, :1107), thermal polling not wired (platform.rs:813), the keyboard mapper stub
(platform.rs:1253), and dispatcher priority not implemented (dispatcher.rs:505). There are no
`todo!()` or `unimplemented!()` in src/android. Literal TODO/FIXME markers are only in
packages/maps and in iOS files.

## 6. Build tooling

* Upstream uses **cargo-ndk plus Gradle**: `cargo ndk -t arm64-v8a -o .../jniLibs --platform 31 build`,
  then `./gradlew assembleDebug` (EX/build.sh:441-460). Gradle has minSdk 26, compile/targetSdk 34, abiFilters arm64-v8a,
  and AndroidX deps (core, splashscreen, biometric, media) that only the Java helpers need. There is no cargo-apk or xbuild,
  and no `[package.metadata.android]`. The crate's `.cargo/config.toml` sets `RUST_FONTCONFIG_DLOPEN=on`.
  The fontconfig/freetype entries in its lock come from the iOS-only zed-font-kit, so the setting is harmless
  on Android. Keep it as a safeguard.
* **This Mac has what is needed without installing anything.** NDK 26.3.11579264 (darwin-x86_64 prebuilt,
  universal binaries, so arm64 is native) has `aarch64-linux-android{26,30,31,34}-clang`, `llvm-ar` and `llvm-strip`.
  The SDK has platforms android-33/34/35/36/36.1/37.0 (android-34/android.jar is enough). build-tools 36.1.0
  has `aapt2` (2.20), `zipalign` with `-P 16` support, `apksigner` 0.9 and `d8`. There are Java 17 and `keytool`,
  plus platform-tools `adb`, and Rust 1.95.0 (pinned by kopuz's rust-toolchain.toml, which applies inside
  spikes/) with `aarch64-linux-android` installed. **cargo-ndk is optional.** It only sets the linker and
  CC env vars, which the recipe below sets by hand. **Gradle is not needed**: with `hasCode="false"` there
  is no Java/dex step.
* System images are all arm64-v8a: android-35 google_apis_playstore, **android-35
  google_apis_playstore_ps16k** (16 KB pages), android-36 tv, android-35-ext15 automotive, and Baklava ps16k. The
  Pixel_8_Pro AVD may use a 16 KB image. So link with `-z max-page-size=16384` and run `zipalign -P 16`.
  Both are harmless on 4 KB devices. (I did not read the AVD config, which is outside the SDK folder.)

## 7. Recipe for the builder

> **Note:** sections 7 and 8 are the notes written before the build, kept for the record. The real files are
> `scripts/env.sh`, `scripts/build-apk.sh`, `scripts/test-emulator.sh` and `android/AndroidManifest.xml`
> (plus `Cargo.toml` and `src/`). The results are in REPORT.md. The names below have been corrected to
> match the real files.

Layout: `spikes/gpui-mobile/{Cargo.toml, src/, android/AndroidManifest.xml, scripts/{env,build-apk,test-emulator}.sh, docs/}`.
Put `target/` and `build/` (which holds the debug keystore) in a spike `.gitignore`.
Before the first build, run `cp ../../Cargo.lock .` so the lock starts with kopuz's exact versions
(gpui-pre-* 0.3.5, wgpu 29.0.4, cosmic-text 0.19). Without that, gpui-kit's caret requirements may resolve
to newer crates. Cargo drops the unused entries.

```toml
# spikes/gpui-mobile/Cargo.toml
[package]
name = "kopuz-mobile-spike"
version = "0.0.0"
edition = "2024"
publish = false

[lib]
name = "kopuz_mobile_spike"   # = android.app.lib_name, output libkopuz_mobile_spike.so
crate-type = ["cdylib"]
path = "src/lib.rs"

[workspace]                   # standalone: outside kopuz's workspace and its [patch]

[dependencies]
gpui = { package = "gpui-pre", version = "=0.3.5", default-features = false }
gpui-mobile = { package = "gpui-pre-mobile", version = "=0.1.0", default-features = false } # drops the 30 package modules
log = "0.4"
# Step 2 (gpui-kit): gpui-kit = { version = "=0.6.4", default-features = false, features = ["component"] }
# Step 3 (kopuzd):   client = { package = "kopuz-client", path = "../../apps/common/client" }
#                    tokio  = { version = "1", features = ["rt-multi-thread"] }

[target.'cfg(target_os = "android")'.dependencies]
android-activity = { version = "0.6", features = ["native-activity"] }
android_logger = "0.15"

[profile.release]
opt-level = 3
lto = "thin"
debug = "line-tables-only"
```

```rust
// spikes/gpui-mobile/src/lib.rs  (API per gpui-pre 0.3.5; compile-check)
#![cfg(target_os = "android")]
use gpui::{App, Application, Context, SharedString, Window, WindowOptions, div, prelude::*, px, rgb};
use gpui_mobile::android::jni;

struct Library { tracks: Vec<SharedString>, playing: Option<usize> }

impl Render for Library {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().id("library").size_full().flex().flex_col().overflow_y_scroll()
            .bg(rgb(0x121212)).text_color(rgb(0xeeeeee))
            .children(self.tracks.iter().enumerate().map(|(i, title)| {
                div().id(("row", i)).flex().justify_between().items_center().p(px(16.))
                    .child(title.clone())
                    .child(div().id(("play", i)).px(px(14.)).py(px(8.)).rounded(px(6.)).bg(rgb(0x3355ff))
                        .child(if self.playing == Some(i) { "Playing" } else { "Play" })
                        .on_click(cx.listener(move |this, _, _, cx| { this.playing = Some(i); cx.notify(); })))
            }))
    }
}

#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    android_logger::init_once(android_logger::Config::default()
        .with_max_level(log::LevelFilter::Info).with_tag("kopuz-spike"));
    jni::install_panic_hook();
    jni::init_platform(&app);
    let Some(platform) = jni::shared_platform() else { return };
    Application::with_platform(platform.into_rc()).run(|cx: &mut App| {
        let tracks = (1..=40).map(|n| SharedString::from(format!("Track {n}"))).collect();
        cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| Library { tracks, playing: None }))
            .expect("open window");
    }); // returns only when the activity is destroyed
}
```

```xml
<!-- spikes/gpui-mobile/android/AndroidManifest.xml -->
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="moe.kopuz.gpuispike" android:versionCode="1" android:versionName="0.0.1">
  <uses-permission android:name="android.permission.INTERNET"/>
  <uses-feature android:name="android.hardware.vulkan.version" android:version="0x400003" android:required="false"/>
  <application android:label="Kopuz GPUI spike" android:hasCode="false" android:extractNativeLibs="false"
      android:theme="@android:style/Theme.Material.NoActionBar">
    <activity android:name="android.app.NativeActivity" android:exported="true"
        android:configChanges="orientation|screenSize|screenLayout|smallestScreenSize|keyboard|keyboardHidden|density|uiMode"
        android:windowSoftInputMode="adjustResize">
      <meta-data android:name="android.app.lib_name" android:value="kopuz_mobile_spike"/>
      <intent-filter><action android:name="android.intent.action.MAIN"/>
        <category android:name="android.intent.category.LAUNCHER"/></intent-filter>
    </activity>
  </application>
</manifest>
```

```sh
# spikes/gpui-mobile/scripts/build-apk.sh, with scripts/env.sh inlined  (run from spikes/gpui-mobile; sh, macOS)
set -eu
SDK="$HOME/Library/Android/sdk"; NDK="$SDK/ndk/26.3.11579264"
TC="$NDK/toolchains/llvm/prebuilt/darwin-x86_64/bin"; BT="$SDK/build-tools/36.1.0"; API=31
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$TC/aarch64-linux-android$API-clang"
export CC_aarch64_linux_android="$TC/aarch64-linux-android$API-clang"
export AR_aarch64_linux_android="$TC/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"
export RUST_FONTCONFIG_DLOPEN=on
cargo build --release --target aarch64-linux-android --lib
rm -rf build/apk build/*.apk; mkdir -p build/apk/lib/arm64-v8a
cp target/aarch64-linux-android/release/libkopuz_mobile_spike.so build/apk/lib/arm64-v8a/
"$BT/aapt2" link -o build/base.apk -I "$SDK/platforms/android-34/android.jar" \
  --manifest android/AndroidManifest.xml --min-sdk-version $API --target-sdk-version 34
(cd build/apk && zip -q -0 -r ../base.apk lib)          # stored, so zipalign can page-align it
"$BT/zipalign" -f -P 16 4 build/base.apk build/aligned.apk
[ -f build/debug.keystore ] || keytool -genkeypair -noprompt -keystore build/debug.keystore \
  -storepass android -keypass android -alias androiddebugkey -keyalg RSA -keysize 2048 \
  -validity 10000 -dname "CN=Android Debug,O=Android,C=US"
"$BT/apksigner" sign --ks build/debug.keystore --ks-pass pass:android --key-pass pass:android \
  --out build/kopuz-gpui-spike.apk build/aligned.apk
"$BT/apksigner" verify build/kopuz-gpui-spike.apk
```

Run against an emulator that is already running (for example the Pixel_8_Pro AVD, started from Android Studio;
`scripts/test-emulator.sh` never starts or stops one):
`adb install -r build/kopuz-gpui-spike.apk && adb shell am start -n moe.kopuz.gpuispike/android.app.NativeActivity`,
then `adb logcat -d -s kopuz-spike gpui-android AndroidRuntime DEBUG`.
The equivalent with cargo-ndk, kept inside the project: `cargo install --root target/tools cargo-ndk`, then
`ANDROID_NDK_HOME="$NDK" target/tools/bin/cargo-ndk -t arm64-v8a -P 31 -o build/apk/lib build --release`.

kopuzd connection (step 3): kopuzd serves plain gRPC over TCP with `--listen <addr>` (daemon/kopuzd/src/lib.rs:11,
:39). Run it on the Mac bound to `127.0.0.1:<port>`, then `adb reverse tcp:<port> tcp:<port>`, and the app calls
`client::GrpcApi::connect_tcp("127.0.0.1:<port>")` (apps/common/client/src/lib.rs:83). The emulator alternative is
`10.0.2.2:<port>`. Nothing listens beyond localhost. kopuz-proto builds with `protox` (pure Rust, so no protoc is needed
when cross-compiling). tonic needs a tokio runtime: start one on a background thread and send results to GPUI over a
channel, then call `cx.notify()`. `link` (spawning its own daemon over a unix socket) does not fit Android. Use the TCP path.

## 8. Open points

* When this was written, no build had been run. The spike has since been built and run (REPORT.md): the cdylib links,
  and the emulator renders on Vulkan (llvmpipe; the GL translator was rejected). The dependency graph of gpui-kit for
  aarch64-linux-android is still unverified, because the spike does not use gpui-kit.
* `cargo` path dependencies on kopuz crates (`kopuz-client`) from a separate workspace should inherit
  `workspace = true` keys from kopuz's root. Check that at step 3.
* For text entry: add GpuiInputActivity.java and compile it with `javac -cp android.jar` plus `d8`, or move to upstream main
  (but that needs gpui-pre 0.3.10 and gpui-kit 0.8.1).
* Before any real use: the clipboard stub, open_url, SaveState, safe-area plumbing into `WindowInsets`,
  a11y, and the continuous render loop.
