#!/bin/bash

set -e

if [ "$EUID" -eq 0 ]; then
    echo "Don't run this as root"
    exit 1
fi

detect_distro() {
    if [ -f /etc/os-release ]; then
        . /etc/os-release
        echo "$ID" | tr '[:upper:]' '[:lower:]'
    else
        uname -s | tr '[:upper:]' '[:lower:]'
    fi
}

install_deps() {
    distro=$(detect_distro)
    echo "Installing deps for $distro..."
    
    case "$distro" in
        ubuntu|debian|pop|linuxmint)
            sudo apt-get update
            sudo apt-get install -y curl wget build-essential libssl-dev pkg-config \
                libasound2-dev libgtk-3-dev libwebkit2gtk-4.0-dev libayatana-appindicator3-dev \
                librsvg2-dev libfuse2 nodejs npm
            ;;
        fedora|rhel|centos)
            sudo dnf install -y curl wget gcc gcc-c++ openssl-devel pkg-config \
                alsa-lib-devel gtk3-devel webkit2gtk4.1-devel libappindicator-gtk3-devel \
                librsvg2-devel fuse nodejs npm
            ;;
        arch|manjaro)
            sudo pacman -Sy --noconfirm --needed curl wget base-devel openssl pkg-config \
                alsa-lib gtk3 webkit2gtk libappindicator-gtk3 librsvg fuse2 nodejs npm
            ;;
        opensuse*|suse)
            sudo zypper install -y curl wget gcc gcc-c++ libopenssl-devel pkg-config \
                alsa-devel gtk3-devel webkit2gtk3-devel libappindicator3-devel \
                librsvg-devel fuse nodejs npm
            ;;
        *)
            echo "Warning: unknown distro"
            echo "You'll need: curl wget build-essential libssl pkg-config alsa gtk3 webkit2gtk libappindicator librsvg fuse nodejs npm"
            read -p "Continue? (y/N) " -r
            [[ ! $REPLY =~ ^[Yy]$ ]] && exit 1
            ;;
    esac
}

check_rust() {
    if ! command -v cargo &> /dev/null; then
        echo "Installing rust..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source "$HOME/.cargo/env"
    fi
}

echo "=== Rusic Installer ==="
echo

read -p "Install dependencies? (Y/n) " -r
if [[ ! $REPLY =~ ^[Nn]$ ]]; then
    install_deps
fi

check_rust
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"

if ! command -v dx &> /dev/null; then
    echo "Installing dioxus-cli..."
    cargo install --locked dioxus-cli
fi

INSTALL_DIR="$HOME/.local/share/rusic"
BIN_DIR="$HOME/.local/bin"
DESKTOP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/512x512/apps"

mkdir -p "$INSTALL_DIR" "$BIN_DIR" "$DESKTOP_DIR" "$ICON_DIR"

echo "Installing node dependencies..."
npm install || exit 1

echo "Building..."
make build || exit 1

APP_DIR="target/dx/rusic/release/linux/app"
[ ! -d "$APP_DIR" ] && echo "Build failed" && exit 1

echo "Installing to $INSTALL_DIR..."
rm -rf "$INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
cp -r "$APP_DIR"/* "$INSTALL_DIR/"
chmod +x "$INSTALL_DIR/rusic"

cat > "$BIN_DIR/rusic" << 'EOF'
#!/bin/bash
cd "$HOME/.local/share/rusic"
exec ./rusic "$@"
EOF
chmod +x "$BIN_DIR/rusic"

[ -f "rusic/assets/logo.png" ] && cp "rusic/assets/logo.png" "$ICON_DIR/com.temidaradev.rusic.png"
[ -f "rusic/assets/logo.png" ] && cp "rusic/assets/logo.png" "$INSTALL_DIR/logo.png"

cp "data/com.temidaradev.rusic.desktop" "$DESKTOP_DIR/"
sed -i "/^Path=/d" "$DESKTOP_DIR/com.temidaradev.rusic.desktop"
sed -i "s|Icon=com.temidaradev.rusic|Icon=$INSTALL_DIR/logo.png|g" "$DESKTOP_DIR/com.temidaradev.rusic.desktop"

cat > "$BIN_DIR/rusic-desktop" << EOF
#!/bin/bash
cd "$INSTALL_DIR"
exec ./rusic "\$@"
EOF
chmod +x "$BIN_DIR/rusic-desktop"

sed -i "s|Exec=rusic|Exec=$BIN_DIR/rusic-desktop|g" "$DESKTOP_DIR/com.temidaradev.rusic.desktop"

command -v update-desktop-database &>/dev/null && update-desktop-database "$DESKTOP_DIR" 2>/dev/null
command -v gtk-update-icon-cache &>/dev/null && gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" 2>/dev/null

echo
echo "Done! You can now run 'rusic' or find it in your app menu"
[[ ":$PATH:" != *":$BIN_DIR:"* ]] && echo "Note: add ~/.local/bin to your PATH"
echo
