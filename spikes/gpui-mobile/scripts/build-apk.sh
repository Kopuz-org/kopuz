#!/bin/sh
# Build libkopuz_mobile_spike.so for aarch64-linux-android and package it as
# build/kopuz-gpui-spike.apk: aapt2 link, the .so stored uncompressed, zipalign -P 16,
# apksigner with build/debug.keystore (made on the first run).
#
# usage: scripts/build-apk.sh [--release | --debug] [--package-only]
#   --debug         debug profile (default: release); the APK is also marked debuggable
#   --package-only  skip cargo; package the .so already in target/aarch64-linux-android/<profile>/
#                   (or the one SPIKE_SO names)
set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
SPIKE_DIR=$(dirname -- "$SCRIPT_DIR")
. "$SCRIPT_DIR/env.sh"

profile=release
package_only=0
for arg in "$@"; do
    case $arg in
        --release) profile=release ;;
        --debug) profile=debug ;;
        --package-only) package_only=1 ;;
        -h | --help) sed -n '2,9p' "$0"; exit 0 ;;
        *) echo "build-apk.sh: unknown argument $arg" >&2; exit 2 ;;
    esac
done

LIB=libkopuz_mobile_spike.so
BUILD="$SPIKE_DIR/build"
SO="${SPIKE_SO:-$SPIKE_DIR/target/$RUST_TARGET/$profile/$LIB}"
APK="$BUILD/kopuz-gpui-spike.apk"
STAGE="$BUILD/apk-stage"
UNSIGNED="$BUILD/kopuz-gpui-spike.unsigned.apk"
ALIGNED="$BUILD/kopuz-gpui-spike.aligned.apk"
KEYSTORE="$BUILD/debug.keystore"

mkdir -p "$BUILD"
# build/ holds the keystore and outputs; keep all of it out of git.
[ -f "$BUILD/.gitignore" ] || printf '*\n' >"$BUILD/.gitignore"

if [ "$package_only" = 0 ]; then
    echo "== cargo build --target $RUST_TARGET ($profile)"
    cd "$SPIKE_DIR"
    if [ "$profile" = release ]; then
        cargo build --target "$RUST_TARGET" --lib --release
    else
        cargo build --target "$RUST_TARGET" --lib
    fi
fi
[ -f "$SO" ] || { echo "build-apk.sh: no $SO; build it first (drop --package-only)" >&2; exit 1; }

# The ELF LOAD segments must be 16 KB aligned for 16 KB page devices.
if "$NDK_BIN/llvm-readelf" -lW "$SO" | awk '$1 == "LOAD" && $NF != "0x4000" { bad = 1 } END { exit bad }'; then
    echo "== $LIB: LOAD segments 16 KB aligned"
else
    echo "build-apk.sh: warning: $LIB has LOAD segments not aligned to 16 KB (max-page-size flag missing?)" >&2
fi

echo "== package"
rm -rf "$STAGE" "$UNSIGNED" "$ALIGNED" "$APK" "$APK.idsig"
mkdir -p "$STAGE/lib/arm64-v8a"
cp "$SO" "$STAGE/lib/arm64-v8a/$LIB"

set -- link -o "$UNSIGNED" -I "$PLATFORM_JAR" \
    --manifest "$SPIKE_DIR/android/AndroidManifest.xml" \
    --min-sdk-version "$ANDROID_API" --target-sdk-version "${PLATFORM#android-}"
[ "$profile" = debug ] && set -- "$@" --debug-mode
if [ -d "$SPIKE_DIR/android/res" ]; then
    "$AAPT2" compile --dir "$SPIKE_DIR/android/res" -o "$BUILD/res.zip"
    set -- "$@" "$BUILD/res.zip"
fi
"$AAPT2" "$@"

# Stored (-0), so zipalign can page-align the .so and the loader can map it from the APK.
(cd "$STAGE" && zip -q -0 -X -r "$UNSIGNED" lib)
"$ZIPALIGN" -f -P 16 4 "$UNSIGNED" "$ALIGNED"
"$ZIPALIGN" -c -P 16 4 "$ALIGNED"

if [ ! -f "$KEYSTORE" ]; then
    echo "== new debug keystore $KEYSTORE"
    keytool -genkeypair -noprompt -keystore "$KEYSTORE" -storetype PKCS12 \
        -storepass android -keypass android -alias androiddebugkey \
        -keyalg RSA -keysize 2048 -validity 10000 \
        -dname "CN=Android Debug,O=Android,C=US"
fi
"$APKSIGNER" sign --ks "$KEYSTORE" --ks-pass pass:android --key-pass pass:android \
    --ks-key-alias androiddebugkey --out "$APK" "$ALIGNED"
"$APKSIGNER" verify --min-sdk-version "$ANDROID_API" "$APK"
rm -rf "$STAGE" "$UNSIGNED" "$ALIGNED"

echo "== $APK ($(wc -c <"$APK" | tr -d ' ') bytes)"
unzip -v "$APK" | grep -E "lib/|Method" || true
"$AAPT2" dump badging "$APK"
