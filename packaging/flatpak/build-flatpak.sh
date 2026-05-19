#!/bin/bash
set -e

echo "=== Kopuz Flatpak Builder ==="

echo "[0/4] Cleaning up previous builds..."
rm -rf .flatpak-builder build-dir dist
rm -f target/dx/kopuz/release/linux/app/kopuz
flatpak uninstall --user -y com.temidaradev.kopuz || true
flatpak uninstall --system -y com.temidaradev.kopuz || true
rm -f ~/.local/bin/kopuz
rm -f ~/.local/share/applications/kopuz.desktop
rm -f ~/.local/share/applications/com.temidaradev.kopuz.desktop
update-desktop-database ~/.local/share/applications || true

echo "[1/4] Building with dx cli..."
npx @tailwindcss/cli -i ./tailwind.css -o ./crates/kopuz/assets/tailwind.css --content './crates/kopuz/**/*.rs,./crates/components/**/*.rs,./crates/pages/**/*.rs,./crates/hooks/**/*.rs,./crates/player/**/*.rs,./crates/reader/**/*.rs'
dx build --release --package kopuz

echo "[2/4] Verifying binary..."
BINARY_PATH="target/dx/kopuz/release/linux/app/kopuz"
if [ ! -f "$BINARY_PATH" ]; then
    echo "❌ Error: Binary not found at $BINARY_PATH"
    exit 1
fi

echo "[3/4] Packaging Flatpak..."
flatpak-builder --user --install --force-clean build-dir packaging/flatpak/com.temidaradev.kopuz.json

echo "[4/4] Creating bundle file..."
mkdir -p dist
flatpak build-bundle ~/.local/share/flatpak/repo dist/kopuz.flatpak com.temidaradev.kopuz

echo
echo "✅ Flatpak build complete!"
echo "Run with: flatpak run com.temidaradev.kopuz"
echo "Bundle file: dist/kopuz.flatpak"

