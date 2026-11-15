#!/bin/sh
# Build libkopuz_mobile_spike.so for aarch64-linux-android and package it as
# build/kopuz-gpui-spike.apk: aapt2 link, the .so stored uncompressed, zipalign -P 16,
# apksigner with build/debug.keystore (made on the first run).
#
# usage: scripts/build-apk.sh [--release | --debug] [--package-only] [--app spike | kopuz]
#   --debug         debug profile (default: release); the APK is also marked debuggable
#   --package-only  skip cargo; package the .so already in target/aarch64-linux-android/<profile>/
#                   (or the one SPIKE_SO names)
#   --app kopuz     the real kopuz GPUI app (package kopuz-android, libkopuz_android.so,
#                   android/kopuz-app/AndroidManifest.xml) as build/kopuz-gpui-app.apk;
#                   default: the spike (kopuz-mobile-spike) as build/kopuz-gpui-spike.apk
set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
SPIKE_DIR=$(dirname -- "$SCRIPT_DIR")
. "$SCRIPT_DIR/env.sh"

profile=release
package_only=0
app=spike
while [ $# -gt 0 ]; do
    case $1 in
        --release) profile=release ;;
        --debug) profile=debug ;;
        --package-only) package_only=1 ;;
        --app) shift; app=${1:-} ;;
        --app=*) app=${1#--app=} ;;
        -h | --help) sed -n '2,12p' "$0"; exit 0 ;;
        *) echo "build-apk.sh: unknown argument $1" >&2; exit 2 ;;
    esac
    shift
done

case $app in
    spike)
        CRATE=kopuz-mobile-spike
        LIB=libkopuz_mobile_spike.so
        NAME=kopuz-gpui-spike
        ANDROID_DIR="$SPIKE_DIR/android"
        ;;
    kopuz)
        CRATE=kopuz-android
        LIB=libkopuz_android.so
        NAME=kopuz-gpui-app
        ANDROID_DIR="$SPIKE_DIR/android/kopuz-app"
        ;;
    *) echo "build-apk.sh: unknown app '$app' (spike or kopuz)" >&2; exit 2 ;;
esac

BUILD="$SPIKE_DIR/build"
SO="${SPIKE_SO:-$SPIKE_DIR/target/$RUST_TARGET/$profile/$LIB}"
APK="$BUILD/$NAME.apk"
STAGE="$BUILD/apk-stage-$NAME"
UNSIGNED="$BUILD/$NAME.unsigned.apk"
ALIGNED="$BUILD/$NAME.aligned.apk"
KEYSTORE="$BUILD/debug.keystore"

mkdir -p "$BUILD"
# build/ holds the keystore and outputs; keep all of it out of git.
[ -f "$BUILD/.gitignore" ] || printf '*\n' >"$BUILD/.gitignore"

if [ "$package_only" = 0 ]; then
    echo "== cargo build -p $CRATE --target $RUST_TARGET ($profile)"
    cd "$SPIKE_DIR"
    if [ "$profile" = release ]; then
        cargo build -p "$CRATE" --target "$RUST_TARGET" --lib --release
    else
        cargo build -p "$CRATE" --target "$RUST_TARGET" --lib
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
# The whole app's line tables make a ~700 MB library; the symbol table stays for backtraces.
if [ "$app" = kopuz ]; then
    "$NDK_BIN/llvm-strip" --strip-debug "$STAGE/lib/arm64-v8a/$LIB"
fi

set -- link -o "$UNSIGNED" -I "$PLATFORM_JAR" \
    --manifest "$ANDROID_DIR/AndroidManifest.xml" \
    --min-sdk-version "$ANDROID_API" --target-sdk-version "${PLATFORM#android-}"
[ "$profile" = debug ] && set -- "$@" --debug-mode
if [ -d "$ANDROID_DIR/res" ]; then
    "$AAPT2" compile --dir "$ANDROID_DIR/res" -o "$BUILD/$NAME-res.zip"
    set -- "$@" "$BUILD/$NAME-res.zip"
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
