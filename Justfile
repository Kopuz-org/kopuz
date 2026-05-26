default:
    @just --list

tailwind:
    tailwindcss -i ./tailwind.css -o ./crates/kopuz/assets/tailwind.css --content './crates/kopuz/**/*.rs,./crates/components/**/*.rs,./crates/pages/**/*.rs,./crates/hooks/**/*.rs,./crates/player/**/*.rs,./crates/reader/**/*.rs'

serve: tailwind
    dx serve

build: tailwind
    dx build --package kopuz --release
    @echo ""
    @echo "Build complete!"

run-release: build
    target/dx/kopuz/release/linux/app/kopuz

clean:
    cargo clean
    rm -rf target/dx dist build-dir .flatpak-builder

flatpak:
    @chmod +x packaging/flatpak/build-flatpak.sh
    ./packaging/flatpak/build-flatpak.sh

flatpak-install: flatpak

flatpak-run:
    flatpak run com.temidaradev.kopuz

# --- Mobile -----------------------------------------------------------------

android_src := "android-src"
android_release_base := "target/dx/kopuz/release/android/app/app/src/main"
ios_app_path := "target/dx/kopuz/release/ios/kopuz.app"
ios_ipa_dir := "target/ipa"

# Build the Android app (debug runtime), patch in our Kotlin sources + manifest, assemble APK.
android-patch:
    @echo "Building Android project (dx)..."
    dx build --package kopuz --platform android --release
    @echo "Patching Kotlin sources..."
    mkdir -p {{android_release_base}}/java/com/temidaradev/kopuz {{android_release_base}}/kotlin/dev/dioxus/main
    cp -rv {{android_src}}/java/com/temidaradev/kopuz/. {{android_release_base}}/java/com/temidaradev/kopuz/
    cp -rv {{android_src}}/kotlin/dev/dioxus/main/. {{android_release_base}}/kotlin/dev/dioxus/main/
    @echo "Patching manifest and icons..."
    python3 {{android_src}}/patch_manifest.py {{android_release_base}}/AndroidManifest.xml
    @echo "Building APK..."
    cd target/dx/kopuz/release/android/app && ./gradlew assembleDebug
    @echo "Done. APK under target/dx/kopuz/release/android/app/app/build/outputs/apk/debug/"

ios-build-sim:
    dx build --ios --package kopuz --release

ios-build-device:
    dx build --ios --package kopuz --release --target aarch64-apple-ios

# Patch Info.plist for on-device install: APPL type, min OS, background audio, platform.
ios-fix-plist:
    #!/usr/bin/env bash
    set -euo pipefail
    PLIST="{{ios_app_path}}/Info.plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundlePackageType APPL" "$PLIST" 2>/dev/null || /usr/libexec/PlistBuddy -c "Add :CFBundlePackageType string APPL" "$PLIST"
    /usr/libexec/PlistBuddy -c "Set :CFBundleInfoDictionaryVersion 6.0" "$PLIST" 2>/dev/null || /usr/libexec/PlistBuddy -c "Add :CFBundleInfoDictionaryVersion string 6.0" "$PLIST"
    /usr/libexec/PlistBuddy -c "Set :MinimumOSVersion 15.0" "$PLIST" 2>/dev/null || /usr/libexec/PlistBuddy -c "Add :MinimumOSVersion string 15.0" "$PLIST"
    /usr/libexec/PlistBuddy -c "Delete :UILaunchStoryboardName" "$PLIST" 2>/dev/null || true
    /usr/libexec/PlistBuddy -c "Add :UILaunchScreen dict" "$PLIST" 2>/dev/null || true
    /usr/libexec/PlistBuddy -c "Add :UILaunchScreen:UIColorName string" "$PLIST" 2>/dev/null || true
    /usr/libexec/PlistBuddy -c "Add :UIBackgroundModes array" "$PLIST" 2>/dev/null || true
    /usr/libexec/PlistBuddy -c "Add :UIBackgroundModes:0 string audio" "$PLIST" 2>/dev/null || true
    /usr/libexec/PlistBuddy -c "Delete :CFBundleSupportedPlatforms" "$PLIST" 2>/dev/null || true
    /usr/libexec/PlistBuddy -c "Add :CFBundleSupportedPlatforms array" "$PLIST"
    /usr/libexec/PlistBuddy -c "Add :CFBundleSupportedPlatforms:0 string iPhoneOS" "$PLIST"

# Unsigned IPA for sideloading (Sideloadly/AltStore re-sign on install).
ios-ipa-sideloadly: ios-build-device ios-fix-plist
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

# Signed IPA. Pass APPLE_SIGN_IDENTITY + IOS_MOBILEPROVISION (and optional IOS_ENTITLEMENTS) as env vars.
ios-ipa-signed: ios-fix-plist ios-build-device
    #!/usr/bin/env bash
    set -euo pipefail
    : "${APPLE_SIGN_IDENTITY:?APPLE_SIGN_IDENTITY is required (e.g. 'Apple Development: Name (TEAMID)')}"
    : "${IOS_MOBILEPROVISION:?IOS_MOBILEPROVISION is required (path to a .mobileprovision file)}"
    [ -f "$IOS_MOBILEPROVISION" ] || { echo "Provisioning profile not found: $IOS_MOBILEPROVISION"; exit 1; }
    cp "$IOS_MOBILEPROVISION" "{{ios_app_path}}/embedded.mobileprovision"
    if [ -n "${IOS_ENTITLEMENTS:-}" ]; then
        codesign --force --deep --sign "$APPLE_SIGN_IDENTITY" --entitlements "$IOS_ENTITLEMENTS" --timestamp=none "{{ios_app_path}}"
    else
        codesign --force --deep --sign "$APPLE_SIGN_IDENTITY" --timestamp=none "{{ios_app_path}}"
    fi
    codesign -vv "{{ios_app_path}}"
    rm -rf {{ios_ipa_dir}}/Payload
    mkdir -p {{ios_ipa_dir}}/Payload
    cp -R {{ios_app_path}} {{ios_ipa_dir}}/Payload/
    rm -f {{ios_ipa_dir}}/Kopuz-signed.ipa
    cd {{ios_ipa_dir}} && zip -qry Kopuz-signed.ipa Payload
    echo "Signed IPA created at {{ios_ipa_dir}}/Kopuz-signed.ipa"
