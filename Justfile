default:
    @just --list

tailwind:
    npx @tailwindcss/cli -i ./tailwind.css -o ./crates/kopuz/assets/tailwind.css

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
    flatpak run moe.kopuz.kopuz

# --- Mobile -----------------------------------------------------------------

android_project := "target/dx/kopuz/release/android/app"
ios_app_path := "target/dx/kopuz/release/ios/kopuz.app"
ios_ipa_dir := "target/ipa"

# Debug-signed APK for sideloading onto a dev device. The Kotlin sources,
# manifest patches, launcher icons and R8 keep rules are all injected by
# crates/kopuz/build.rs during `dx build` — nothing to copy in afterwards.
android-patch:
    dx build --package kopuz --platform android --release
    cd {{android_project}} && ./gradlew assembleDebug
    @echo "APK: {{android_project}}/app/build/outputs/apk/debug/app-debug.apk"

# Release APK, signed when the KOPUZ_ANDROID_KEYSTORE env vars are set.
android-release:
    ./packaging/android/build-apk.sh

# Install the most recently built APK on the connected device.
android-install:
    adb install -r "$(ls -t target/android/*.apk {{android_project}}/app/build/outputs/apk/debug/*.apk 2>/dev/null | head -1)"

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
ios-ipa-signed: ios-build-device ios-fix-plist
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
