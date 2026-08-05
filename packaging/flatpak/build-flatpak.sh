#!/bin/bash
set -e

echo "=== Kopuz Flatpak Builder ==="
# TODO: remove com.temidaradev.kopuz later
echo "[0/2] Cleaning up previous builds..."
rm -rf .flatpak-builder build-dir dist
rm -f target/dx/kopuz/release/linux/app/kopuz
flatpak uninstall --user -y com.temidaradev.kopuz || true
flatpak uninstall --system -y com.temidaradev.kopuz || true
flatpak uninstall --user -y moe.kopuz.kopuz || true
flatpak uninstall --system -y moe.kopuz.kopuz || true
rm -f ~/.local/bin/kopuz
rm -f ~/.local/share/applications/kopuz.desktop
rm -f ~/.local/share/applications/com.temidaradev.kopuz.desktop
rm -f ~/.local/share/applications/moe.kopuz.kopuz.desktop
update-desktop-database ~/.local/share/applications || true

echo "[1/2] Build Flatpak..."
flatpak-builder --user --install --force-clean build-dir packaging/flatpak/moe.kopuz.kopuz.json

echo "[2/2] Creating bundle file..."
mkdir -p dist
flatpak build-bundle ~/.local/share/flatpak/repo dist/kopuz.flatpak moe.kopuz.kopuz

echo
echo "✅ Flatpak build complete!"
echo "Run with: flatpak run moe.kopuz.kopuz"
echo "Bundle file: dist/kopuz.flatpak"

