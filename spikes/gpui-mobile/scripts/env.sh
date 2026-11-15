# Android toolchain environment for the GPUI mobile spike (POSIX sh, macOS).
#
#   . spikes/gpui-mobile/scripts/env.sh     # source it; status is 1 when a path is missing
#   sh spikes/gpui-mobile/scripts/env.sh    # run it to print what it sets
#
# Overridable: ANDROID_HOME, ANDROID_NDK_VERSION, ANDROID_API, BUILD_TOOLS_VERSION, PLATFORM.
# The C compiler, C++ compiler and archiver are exported under their per-target
# names (CC_aarch64_linux_android, ...), which the cc crate reads only for the
# Android target; a bare CC would also be used for host build scripts. The same
# paths are in ANDROID_CC, ANDROID_CXX and ANDROID_AR for direct use.

: "${ANDROID_HOME:=$HOME/Library/Android/sdk}"
: "${ANDROID_NDK_VERSION:=26.3.11579264}"
: "${ANDROID_API:=31}"
: "${BUILD_TOOLS_VERSION:=36.1.0}"
: "${PLATFORM:=android-34}"

RUST_TARGET=aarch64-linux-android
ANDROID_NDK_HOME="$ANDROID_HOME/ndk/$ANDROID_NDK_VERSION"
ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"
NDK_BIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64/bin"
ANDROID_CC="$NDK_BIN/aarch64-linux-android$ANDROID_API-clang"
ANDROID_CXX="$NDK_BIN/aarch64-linux-android$ANDROID_API-clang++"
ANDROID_AR="$NDK_BIN/llvm-ar"
BUILD_TOOLS="$ANDROID_HOME/build-tools/$BUILD_TOOLS_VERSION"
PLATFORM_JAR="$ANDROID_HOME/platforms/$PLATFORM/android.jar"
AAPT2="$BUILD_TOOLS/aapt2"
ZIPALIGN="$BUILD_TOOLS/zipalign"
APKSIGNER="$BUILD_TOOLS/apksigner"
ADB="$ANDROID_HOME/platform-tools/adb"
EMULATOR="$ANDROID_HOME/emulator/emulator"

CC_aarch64_linux_android="$ANDROID_CC"
CXX_aarch64_linux_android="$ANDROID_CXX"
AR_aarch64_linux_android="$ANDROID_AR"
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ANDROID_CC"
# 16 KB pages: Android 15+ devices and the ps16k system images need it; harmless on 4 KB.
CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"
# gpui-pre-mobile's own .cargo/config.toml sets this; keep it as a safeguard.
RUST_FONTCONFIG_DLOPEN=on

export ANDROID_HOME ANDROID_NDK_VERSION ANDROID_API BUILD_TOOLS_VERSION PLATFORM RUST_TARGET \
    ANDROID_NDK_HOME ANDROID_NDK_ROOT NDK_BIN ANDROID_CC ANDROID_CXX ANDROID_AR \
    BUILD_TOOLS PLATFORM_JAR AAPT2 ZIPALIGN APKSIGNER ADB EMULATOR \
    CC_aarch64_linux_android CXX_aarch64_linux_android AR_aarch64_linux_android \
    CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS \
    RUST_FONTCONFIG_DLOPEN

kopuz_env_ok=1
for kopuz_env_path in "$ANDROID_HOME" "$ANDROID_NDK_HOME" "$NDK_BIN" "$BUILD_TOOLS" "$PLATFORM_JAR"; do
    if [ ! -e "$kopuz_env_path" ]; then
        echo "env.sh: missing $kopuz_env_path" >&2
        kopuz_env_ok=0
    fi
done
for kopuz_env_path in "$ANDROID_CC" "$ANDROID_CXX" "$ANDROID_AR" "$AAPT2" "$ZIPALIGN" "$APKSIGNER"; do
    if [ ! -x "$kopuz_env_path" ]; then
        echo "env.sh: missing or not executable: $kopuz_env_path" >&2
        kopuz_env_ok=0
    fi
done
if [ "$kopuz_env_ok" != 1 ]; then
    echo "env.sh: install the Android SDK/NDK pieces above, or point ANDROID_HOME, ANDROID_NDK_VERSION, BUILD_TOOLS_VERSION or PLATFORM at what is installed" >&2
fi

# Run directly (not sourced): show the result. zsh names the sourced file in $0,
# so its eval context tells the two apart there.
case "$0:${ZSH_EVAL_CONTEXT:-}" in
    *:*file*) ;;
    *env.sh:*)
        for kopuz_env_var in ANDROID_HOME ANDROID_NDK_HOME NDK_BIN BUILD_TOOLS PLATFORM PLATFORM_JAR RUST_TARGET \
            CC_aarch64_linux_android CXX_aarch64_linux_android AR_aarch64_linux_android \
            CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS \
            AAPT2 ZIPALIGN APKSIGNER ADB EMULATOR RUST_FONTCONFIG_DLOPEN; do
            eval "printf '%s=%s\n' \"\$kopuz_env_var\" \"\${$kopuz_env_var}\""
        done
        ;;
esac
unset kopuz_env_path kopuz_env_var

# Last command: the status a caller of `. env.sh` sees.
[ "$kopuz_env_ok" = 1 ]
