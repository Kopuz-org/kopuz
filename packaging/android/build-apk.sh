#!/bin/bash
# Build a release APK. Signs it when a keystore is configured, otherwise leaves
# the unsigned Gradle output in place so CI can still publish a build artifact.
#
#   KOPUZ_ANDROID_KEYSTORE           path to a JKS/PKCS12 keystore
#   KOPUZ_ANDROID_KEYSTORE_PASSWORD  store password
#   KOPUZ_ANDROID_KEY_ALIAS          key alias inside the store
#   KOPUZ_ANDROID_KEY_PASSWORD       key password (defaults to the store password)
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"
if [[ ! -d "$sdk" ]]; then
  echo "Android SDK not found — set ANDROID_HOME" >&2
  exit 1
fi

# apksigner/zipalign live in a versioned build-tools dir; take the newest.
build_tools="$(ls -1 "$sdk/build-tools" | sort -V | tail -1)"
apksigner="$sdk/build-tools/$build_tools/apksigner"
zipalign="$sdk/build-tools/$build_tools/zipalign"

version="$(
  cargo metadata --no-deps --format-version 1 |
    jq -r '.packages[] | select(.name == "kopuz") | .version'
)"

gradle_project="target/dx/kopuz/release/android/app"
unsigned="$gradle_project/app/build/outputs/apk/release/app-release-unsigned.apk"
out_dir="target/android"
out="$out_dir/kopuz-$version-arm64-v8a.apk"

echo "[1/4] Compiling Rust and generating the Gradle project..."
dx build --package kopuz --platform android --release

echo "[2/4] Assembling the release APK..."
(cd "$gradle_project" && ./gradlew assembleRelease)

# Google Play rejects 4 KB-aligned native libs (16 KB page mandate). The linker
# flag lives in .cargo/config.toml but an exported RUSTFLAGS silently overrides
# it, so verify the shipped library instead of trusting the config.
readelf_bin="$(command -v llvm-readelf || command -v readelf || true)"
if [[ -z "$readelf_bin" && -n "${ANDROID_NDK_HOME:-}" ]]; then
  for cand in "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin/llvm-readelf; do
    [[ -x "$cand" ]] && readelf_bin="$cand" && break
  done
fi
if [[ -n "$readelf_bin" ]]; then
  align_tmp="$(mktemp -d)"
  unzip -o -q "$unsigned" 'lib/*/*.so' -d "$align_tmp"
  while IFS= read -r so; do
    if "$readelf_bin" -l -W "$so" | awk '$1=="LOAD" && $NF=="0x1000" {bad=1} END{exit bad}'; then
      :
    else
      echo "$so is 4 KB-aligned; RUSTFLAGS overrode the 16 KB max-page-size flag" >&2
      exit 1
    fi
  done < <(find "$align_tmp" -name '*.so')
  rm -rf "$align_tmp"
else
  echo "warning: no readelf found; skipping the 16 KB alignment check" >&2
fi

mkdir -p "$out_dir"
rm -f "$out"

if [[ -z "${KOPUZ_ANDROID_KEYSTORE:-}" ]]; then
  echo "[3/4] No KOPUZ_ANDROID_KEYSTORE — leaving the APK unsigned."
  echo "[4/4] Unsigned APK: $unsigned"
  cp "$unsigned" "${out%.apk}-unsigned.apk"
  echo "      copied to ${out%.apk}-unsigned.apk"
  exit 0
fi

echo "[3/4] Aligning..."
"$zipalign" -p -f 4 "$unsigned" "$out"

echo "[4/4] Signing..."
# `export`, not a bare assignment: apksigner reads this out of its own
# environment, and the fallback would otherwise only exist in this shell.
export KOPUZ_ANDROID_KEY_PASSWORD="${KOPUZ_ANDROID_KEY_PASSWORD:-$KOPUZ_ANDROID_KEYSTORE_PASSWORD}"
"$apksigner" sign \
  --ks "$KOPUZ_ANDROID_KEYSTORE" \
  --ks-pass "env:KOPUZ_ANDROID_KEYSTORE_PASSWORD" \
  --ks-key-alias "$KOPUZ_ANDROID_KEY_ALIAS" \
  --key-pass "env:KOPUZ_ANDROID_KEY_PASSWORD" \
  "$out"
"$apksigner" verify --print-certs "$out"

echo "Signed APK: $out"
