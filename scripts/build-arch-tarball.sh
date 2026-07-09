#!/usr/bin/env bash
# Build the Linux release tarball that the kopuz-bin AUR package sources.
#
# Runs inside archlinux:base-devel (CI job container, or `docker run` locally).
# Building on Arch is the whole point: the binary then links against the distro's
# CURRENT sonames (e.g. libxdo.so.4), instead of a Debian .deb's stale libxdo.so.3
# which crashes on updated Arch installs (issue #512).
#
# Output: dist/kopuz_v${VERSION}_x86_64-linux.tar.gz, laid out exactly as the
# PKGBUILD expects (a kopuz-linux-x86_64/ dir).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

VERSION="$(grep -m1 -oP '^version = "\K[^"]+' Cargo.toml)"
echo "==> Building kopuz $VERSION for x86_64 on $(source /etc/os-release; echo "$PRETTY_NAME")"

# --- system deps ------------------------------------------------------------
# Runtime libs double as build/link deps on Arch (headers ship in the main pkg).
echo "==> Installing system dependencies (pacman)"
pacman -Syu --noconfirm --needed \
  base-devel git curl cmake pkgconf \
  rustup nodejs npm \
  webkit2gtk-4.1 gtk3 libsoup3 glib-networking \
  alsa-lib openssl xdotool dbus opus libayatana-appindicator \
  pango cairo gdk-pixbuf2

# --- rust toolchain (pinned by rust-toolchain.toml) -------------------------
# We build with plain `cargo`, not `dx`: on Linux the app has no `asset!()`-
# collected assets (the only asset!() is Windows-only toolbar icons), and CSS/
# fonts are `include_str!`-embedded into the binary, so `cargo build` yields the
# same self-contained executable dx would -- without dx, its nightly unit-graph
# probe, or the bundler. Stable toolchain only.
echo "==> Setting up Rust toolchain"
toolchain="$(grep -m1 -oP 'channel = "\K[^"]+' rust-toolchain.toml)"
rustup toolchain install "$toolchain" --profile minimal --no-self-update
rustup default "$toolchain"
export PATH="$HOME/.cargo/bin:$PATH"

# --- tailwind (same invocation as CI / Justfile) ----------------------------
# tailwind.css is `include_str!`-baked into the binary at compile time, so it
# must exist and be current *before* cargo build.
echo "==> Building Tailwind CSS"
npm install
npx @tailwindcss/cli -i ./tailwind.css -o ./crates/kopuz/assets/tailwind.css \
  --content './crates/kopuz/**/*.rs,./crates/components/**/*.rs,./crates/pages/**/*.rs,./crates/hooks/**/*.rs,./crates/player/**/*.rs,./crates/reader/**/*.rs'

# --- build ------------------------------------------------------------------
# kopuz's Cargo.toml already pins `dioxus = { features = ["desktop"] }`, so a
# plain release build of the package is the desktop binary.
echo "==> cargo build --release -p kopuz"
cargo build --release -p kopuz
bin="target/release/kopuz"
[[ -x "$bin" ]] || { echo "::error::built binary not found at $bin"; exit 1; }

# --- assemble tarball (kopuz-linux-x86_64/ layout the PKGBUILD unpacks) ------
echo "==> Assembling tarball"
stage="$(mktemp -d)/kopuz-linux-x86_64"
mkdir -p "$stage"
install -m755 "$bin"                                   "$stage/kopuz"
install -m644 data/com.temidaradev.kopuz.desktop       "$stage/"
install -m644 data/com.temidaradev.kopuz.metainfo.xml  "$stage/"
install -m644 crates/kopuz/assets/logo.png             "$stage/logo.png"
install -m644 LICENSE                                   "$stage/LICENSE"

mkdir -p dist
out="dist/kopuz_v${VERSION}_x86_64-linux.tar.gz"
tar --sort=name --owner=0 --group=0 --numeric-owner \
    --mtime="@${SOURCE_DATE_EPOCH:-0}" \
    -czf "$out" -C "$(dirname "$stage")" kopuz-linux-x86_64

# --- self-check: the #512 regression must not reappear ----------------------
echo "==> Verifying dynamic linkage"
needed_xdo="$(readelf -d "$stage/kopuz" | grep -oP 'libxdo\.so\.\d+' || true)"
echo "    libxdo NEEDED: ${needed_xdo:-<none>}"
[[ "$needed_xdo" == "libxdo.so.4" ]] \
  || { echo "::error::expected libxdo.so.4, got '${needed_xdo:-<none>}' -- Arch soname drift"; exit 1; }
if ldd "$stage/kopuz" | grep -i 'not found'; then
  echo "::error::binary has unresolved shared libraries on Arch"; exit 1
fi

sha="$(sha256sum "$out" | cut -d' ' -f1)"
echo "==> Built $out"
echo "    sha256=$sha"
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  { echo "version=$VERSION"; echo "tarball=$out"; echo "sha256=$sha"; } >> "$GITHUB_OUTPUT"
fi
