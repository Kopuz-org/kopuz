#!/bin/sh
# kopuz installer — downloads a prebuilt binary + assets, no compiling.
#   curl -fsSL https://raw.githubusercontent.com/Kopuz-org/kopuz/master/install.sh | sh
#
# The kopuz binary resolves its assets relative to its own location, so the
# binary and the assets/ directory must always live in the same folder. This
# script keeps them together in INSTALL_DIR and puts a tiny launcher on PATH.
#
# Env overrides:
#   KOPUZ_VERSION=v0.6.5   install a specific tag instead of latest
# Flags:
#   --uninstall            remove kopuz
set -eu

REPO="Kopuz-org/kopuz"
APP="kopuz"
APP_ID="com.temidaradev.kopuz"

DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
INSTALL_DIR="$DATA_DIR/$APP"
BIN_DIR="$HOME/.local/bin"
DESKTOP_DIR="$DATA_DIR/applications"
ICON_DIR="$DATA_DIR/icons/hicolor/512x512/apps"

if [ "${1:-}" = "--uninstall" ]; then
	rm -rf "$INSTALL_DIR"
	rm -f "$BIN_DIR/$APP"
	rm -f "$DESKTOP_DIR/$APP_ID.desktop"
	rm -f "$ICON_DIR/$APP_ID.png"
	echo "kopuz uninstalled."
	exit 0
fi

os="$(uname -s)"
arch="$(uname -m)"
if [ "$os" != "Linux" ]; then
	echo "error: this installer supports Linux only (detected $os)." >&2
	echo "       macOS/Windows: see the release page or build from source." >&2
	exit 1
fi
if [ "$arch" != "x86_64" ]; then
	echo "error: only x86_64 is supported (detected $arch)." >&2
	exit 1
fi
target="x86_64-linux"

VERSION="${KOPUZ_VERSION:-}"
if [ -z "$VERSION" ]; then
	VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
		| grep '"tag_name"' | head -1 | cut -d'"' -f4)"
fi
if [ -z "$VERSION" ]; then
	echo "error: could not determine the latest release version." >&2
	exit 1
fi

tarball="$APP-$VERSION-$target.tar.gz"
url="https://github.com/$REPO/releases/download/$VERSION/$tarball"

echo "Installing $APP $VERSION ($target)..."

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if ! curl -fSL "$url" -o "$tmp/$tarball"; then
	echo "error: download failed: $url" >&2
	exit 1
fi

sums_url="https://github.com/$REPO/releases/download/$VERSION/SHA256SUMS"
if ! curl -fSL "$sums_url" -o "$tmp/SHA256SUMS"; then
	echo "error: could not download checksums: $sums_url" >&2
	exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
	actual="$(sha256sum "$tmp/$tarball" | cut -d' ' -f1)"
elif command -v shasum >/dev/null 2>&1; then
	actual="$(shasum -a 256 "$tmp/$tarball" | cut -d' ' -f1)"
else
	echo "error: no sha256 tool (sha256sum/shasum) found to verify download." >&2
	exit 1
fi

expected="$(awk -v f="$tarball" '$2 == f || $2 == "*" f {print $1; exit}' "$tmp/SHA256SUMS")"
if [ -z "$expected" ]; then
	echo "error: no checksum entry for $tarball in SHA256SUMS." >&2
	exit 1
fi
if [ "$actual" != "$expected" ]; then
	echo "error: checksum mismatch for $tarball — refusing to install." >&2
	echo "  expected: $expected" >&2
	echo "  actual:   $actual" >&2
	exit 1
fi
echo "✓ checksum verified"

rm -rf "$INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
tar -xzf "$tmp/$tarball" -C "$INSTALL_DIR"

if [ ! -x "$INSTALL_DIR/$APP" ]; then
	echo "error: binary not found in archive." >&2
	exit 1
fi

mkdir -p "$BIN_DIR"
cat >"$BIN_DIR/$APP" <<EOF
#!/bin/sh
exec "$INSTALL_DIR/$APP" "\$@"
EOF
chmod +x "$BIN_DIR/$APP"

if [ -f "$INSTALL_DIR/$APP_ID.desktop" ]; then
	mkdir -p "$DESKTOP_DIR"
	sed "s|^Exec=.*|Exec=$BIN_DIR/$APP|" \
		"$INSTALL_DIR/$APP_ID.desktop" >"$DESKTOP_DIR/$APP_ID.desktop"
fi
if [ -f "$INSTALL_DIR/icon.png" ]; then
	mkdir -p "$ICON_DIR"
	cp "$INSTALL_DIR/icon.png" "$ICON_DIR/$APP_ID.png"
fi

echo "✓ installed to $INSTALL_DIR"
echo "✓ launcher    $BIN_DIR/$APP"

case ":$PATH:" in
*":$BIN_DIR:"*) ;;
*)
	echo "⚠ $BIN_DIR is not on your PATH. Add this to your shell rc:"
	echo "    export PATH=\"$BIN_DIR:\$PATH\""
	;;
esac

echo "Run:  $APP"
echo "Needs system libs: webkit2gtk-4.1, gtk3, libsoup3, alsa, opus, xdotool."
