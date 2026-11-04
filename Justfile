# The GPUI frontend: `just gpui run`, `just gpui check`, and the rest.
mod gpui "apps/kopuz/justfile"

default:
    @just --list

tailwind:
    npx @tailwindcss/cli -i ./tailwind.css -o ./apps/legacy/kopuz/assets/tailwind.css

serve: tailwind
    dx serve --package kopuz-legacy

build: tailwind
    dx build --package kopuz-legacy --release
    @echo ""
    @echo "Build complete!"

# The app, hosting the daemon core and serving it on the socket. Extra flags
# reach cargo, so `just run --release` builds and runs it released.
run *FLAGS: tailwind
    cargo run {{FLAGS}} -p kopuz-legacy

daemon *FLAGS:
    cargo run {{FLAGS}} -p kopuz-kopuzd --bin kopuzd

ctl *ARGS:
    cargo run -p kopuz-ctl -- {{ARGS}}

run-release: build
    target/dx/kopuz-legacy/release/linux/app/kopuz-legacy

clean:
    cargo clean
    rm -rf target/dx dist build-dir .flatpak-builder

flatpak:
    @chmod +x packaging/build-flatpak.sh
    ./packaging/build-flatpak.sh

flatpak-install: flatpak

flatpak-run:
    flatpak run moe.kopuz.kopuz

# --- Mobile -----------------------------------------------------------------

android_project := "target/dx/kopuz-legacy/release/android/app"
ios_app_path := "target/dx/kopuz-legacy/release/ios/KopuzLegacy.app"
ios_ipa_dir := "target/ipa"

# Debug-signed APK for sideloading onto a dev device. The Kotlin sources,
# manifest patches, launcher icons and R8 keep rules are all injected by
# apps/legacy/kopuz/build.rs during `dx build` — nothing to copy in afterwards.
android-patch:
    dx build --package kopuz-legacy --platform android --release
    cd {{android_project}} && ./gradlew assembleDebug
    @echo "APK: {{android_project}}/app/build/outputs/apk/debug/app-debug.apk"

# Release APK, signed when the KOPUZ_ANDROID_KEYSTORE env vars are set.
android-release:
    ./packaging/android/build-apk.sh

# Install the most recently built APK on the connected device.
android-install:
    adb install -r "$(ls -t target/android/*.apk {{android_project}}/app/build/outputs/apk/debug/*.apk 2>/dev/null | head -1)"

# The nix shell points DEVELOPER_DIR at a macOS-only SDK and wraps the C
# compiler for macOS, so iOS builds run in a clean environment on the system
# Xcode. Override with XCODE_DEVELOPER_DIR.
ios_env := 'env -i HOME="$HOME" TERM="${TERM:-dumb}" PATH="$HOME/.cargo/bin:$(dirname "$(command -v dx)"):$(dirname "$(command -v cmake)"):/usr/bin:/bin:/usr/sbin" DEVELOPER_DIR="${XCODE_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}" IPHONEOS_DEPLOYMENT_TARGET=16.0 SQLX_OFFLINE=true'

ios-build-sim:
    {{ios_env}} dx build --ios --package kopuz-legacy --release --target aarch64-apple-ios-sim

ios-build-device:
    {{ios_env}} dx build --ios --package kopuz-legacy --release --target aarch64-apple-ios

# Build, install and launch on a simulator (IOS_SIMULATOR, default "iPhone 17 Pro").
ios-sim: ios-build-sim
    #!/usr/bin/env bash
    set -euo pipefail
    export DEVELOPER_DIR="${XCODE_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
    SIM="${IOS_SIMULATOR:-iPhone 17 Pro}"
    /usr/bin/xcrun simctl boot "$SIM" 2>/dev/null || true
    open -a Simulator
    /usr/bin/xcrun simctl install "$SIM" "{{ios_app_path}}"
    /usr/bin/xcrun simctl launch --console-pty --terminate-running-process "$SIM" moe.kopuz.kopuz

# Sign the device build with IOS_SIGN_IDENTITY and IOS_MOBILEPROVISION; entitlements come from the profile.
ios-sign: ios-build-device
    #!/usr/bin/env bash
    set -euo pipefail
    : "${IOS_SIGN_IDENTITY:?IOS_SIGN_IDENTITY is required (see: security find-identity -v -p codesigning)}"
    : "${IOS_MOBILEPROVISION:?IOS_MOBILEPROVISION is required (path to a .mobileprovision file)}"
    ENTITLEMENTS="$(mktemp -d)/entitlements.plist"
    security cms -D -i "$IOS_MOBILEPROVISION" | plutil -extract Entitlements xml1 -o "$ENTITLEMENTS" -
    cp "$IOS_MOBILEPROVISION" "{{ios_app_path}}/embedded.mobileprovision"
    codesign --force --sign "$IOS_SIGN_IDENTITY" --entitlements "$ENTITLEMENTS" --timestamp=none "{{ios_app_path}}"
    codesign -vv "{{ios_app_path}}"

# Install and launch on a connected iPhone (IOS_DEVICE, default the first one connected).
ios-device: ios-sign
    #!/usr/bin/env bash
    set -euo pipefail
    export DEVELOPER_DIR="${XCODE_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
    DEVICE="${IOS_DEVICE:-$(/usr/bin/xcrun devicectl list devices 2>/dev/null | awk '/physical/ { for (i = 2; i <= NF; i++) if ($i == "(UDID)") { print $(i - 1); exit } }')}"
    : "${DEVICE:?no paired iPhone found; set IOS_DEVICE}"
    /usr/bin/xcrun devicectl device install app --device "$DEVICE" "{{ios_app_path}}"
    /usr/bin/xcrun devicectl device process launch --console --terminate-existing --device "$DEVICE" moe.kopuz.kopuz

# Unsigned IPA for sideloading (Sideloadly/AltStore re-sign on install).
ios-ipa-sideloadly: ios-build-device
    #!/usr/bin/env bash
    set -euo pipefail
    codesign --remove-signature "{{ios_app_path}}" 2>/dev/null || true
    rm -f "{{ios_app_path}}/embedded.mobileprovision"
    rm -rf "{{ios_app_path}}/_CodeSignature"
    rm -rf {{ios_ipa_dir}}/Payload
    mkdir -p {{ios_ipa_dir}}/Payload
    cp -R {{ios_app_path}} {{ios_ipa_dir}}/Payload/
    rm -f {{ios_ipa_dir}}/Kopuz-sideloadly.ipa
    cd {{ios_ipa_dir}} && zip -qry Kopuz-sideloadly.ipa Payload
    echo "Sideloadly IPA created at {{ios_ipa_dir}}/Kopuz-sideloadly.ipa"

# Signed IPA, with the same IOS_SIGN_IDENTITY and IOS_MOBILEPROVISION as ios-sign.
ios-ipa-signed: ios-sign
    #!/usr/bin/env bash
    set -euo pipefail
    rm -rf {{ios_ipa_dir}}/Payload
    mkdir -p {{ios_ipa_dir}}/Payload
    cp -R {{ios_app_path}} {{ios_ipa_dir}}/Payload/
    rm -f {{ios_ipa_dir}}/Kopuz-signed.ipa
    cd {{ios_ipa_dir}} && zip -qry Kopuz-signed.ipa Payload
    echo "Signed IPA created at {{ios_ipa_dir}}/Kopuz-signed.ipa"
