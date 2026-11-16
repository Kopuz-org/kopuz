# GPUI Mobile

Mobile platform layer for [GPUI](https://github.com/zed-industries/zed) — run Rust UI apps natively on **iOS** and **Android**.

[![Build & Test](https://github.com/longbridge/gpui-mobile/actions/workflows/ci.yml/badge.svg)](https://github.com/longbridge/gpui-mobile/actions/workflows/ci.yml)

## Overview

Implements the `gpui::Platform` trait for mobile targets, following the same architecture as Zed's desktop platform crates.

| Platform | Renderer | Text |
|----------|----------|------|
| **iOS** | Metal via wgpu | CoreText |
| **Android** | Vulkan/GL via wgpu | cosmic-text + swash |

**Highlights:** GPU-accelerated rendering, touch input with momentum scrolling, keyboard support, safe area insets, dark mode, and emoji rendering.

## GPUI pre-release dependencies

This fork publishes as `gpui-pre-mobile` and uses the crates.io GPUI snapshots
from GPUI Kit. Version `0.1.0` targets `gpui-pre` and `gpui-pre-wgpu` **0.3.4**.
Both are pinned together so the platform, renderer, and application share the
same GPUI types. No Zed Git checkout or consumer Cargo patches are needed.

```toml
[dependencies]
gpui = { package = "gpui-pre", version = "=0.3.4", default-features = false }
gpui-mobile = { package = "gpui-pre-mobile", version = "0.1.0" }
```

The Rust library name remains `gpui_mobile`, including its native library
filenames. Existing `use gpui_mobile::...` imports continue to work. Native app
entry points and build steps are demonstrated in `example/`.

This is an experimental platform release. Switching to `gpui-pre` does not
complete mobile IME composition, accessibility, or the mobile lifecycle hooks.

Custom Android hosts should extend `dev.gpui.mobile.GpuiInputActivity` instead of
`android.app.NativeActivity` to receive multistage IME composition through a
native `InputConnection`. Include `GpuiInputActivity.java` and retain the usual
`android.app.lib_name` metadata. For GPUI file prompts, also package
`GpuiPathPicker.java` and register its non-exported Activity as shown in the
example manifest. The picker imports documents into app cache before returning
filesystem paths; it does not require broad storage permissions.

## Quick Start

### Prerequisites

- **Rust** 1.75+
- **iOS**: macOS + Xcode 15+, [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`)
- **Android**: Android SDK + NDK r25+, [cargo-ndk](https://github.com/nickelc/cargo-ndk) (`cargo install cargo-ndk`)

### Build & Run

```bash
cd example

# iOS
./build.sh ios --device        # or --simulator
./build.sh ios --device --release

# Android
./build.sh android --device    # or --emulator
./build.sh android --device --release
```

### Manual Build

```bash
# iOS
rustup target add aarch64-apple-ios
cd example && cargo build --target aarch64-apple-ios
cd ios && xcodegen generate --spec project.yml
xcodebuild -project GpuiExample.xcodeproj -scheme GpuiExample build

# Android
rustup target add aarch64-linux-android
cd example
cargo ndk -t arm64-v8a -P 31 -o android/gradle/app/src/main/jniLibs build
cd android/gradle && ./gradlew assembleDebug
```

## Platform Support

| Platform | Status | Min Version | GPU Backend |
|----------|--------|-------------|-------------|
| iOS (device + sim) | ✅ | iOS 13.0+ | Metal |
| Android (arm64) | ✅ | API 26+ | Vulkan (preferred), GL ES 3.0 |
| Android (armv7/x86_64) | ⚠️ Untested | API 26+ | Vulkan / GL ES |

## Screenshots

Running on a Motorola Edge 50 Pro (Android, Vulkan):

| Home | Counter | Settings | About |
|------|---------|----------|-------|
| ![Home](screenshots/home.png) | ![Counter](screenshots/counter.png) | ![Settings](screenshots/settings.png) | ![About](screenshots/about.png) |

A demo video is available at [`screenshots/demo.mp4`](screenshots/demo.mp4).

## Example App

The example app includes screens for Home, Counter, About, Settings, Components (Apple Glass + Material Design), Animations, and Shaders.

## Contributing

1. Fork & create a feature branch
2. Ensure all targets compile:
   ```bash
   cargo check
   cargo check --target aarch64-apple-ios
   cargo check --target aarch64-linux-android
   cargo test
   cargo fmt --all
   ```
3. Open a Pull Request

## License

Licensed under any one of:

- [GPL-3.0-or-later](LICENSE-GPL)
- [AGPL-3.0-or-later](LICENSE-AGPL)
- [Apache-2.0](LICENSE-APACHE)

## Acknowledgements

[Zed Industries](https://github.com/zed-industries/zed) (GPUI framework) · [wgpu](https://wgpu.rs/) · [cosmic-text](https://github.com/pop-os/cosmic-text) · [swash](https://github.com/dfrg/swash) · [Google Noto Fonts](https://github.com/googlefonts/noto-emoji)
