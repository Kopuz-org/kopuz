# Maintainer: temidaradev <temidaradev@proton.me>
pkgname=kopuz
pkgver=0.4.2
pkgrel=1
pkgdesc="A modern music player"
arch=('x86_64')
url="https://github.com/temidaradev/kopuz"
license=('MIT')
depends=(
    'webkit2gtk-4.1'
    'gtk3'
    'libsoup3'
    'glib-networking'
    'alsa-lib'
    'openssl'
    'xdotool'
    'dbus'
)
makedepends=(
    'rust'
    'cargo'
    'git'
)
# dioxus-cli must be installed manually or from AUR at version matching dioxus 0.7.x:
#   cargo install dioxus-cli --version "^0.7"
source=("$pkgname-$pkgver.tar.gz::https://github.com/temidaradev/kopuz/archive/refs/tags/v$pkgver.tar.gz")
sha256sums=('0935d1e7ea1e5377ea669ed460f530ca48f45d0448da88518951a1cf45705aeb')

build() {
    cd "$pkgname-$pkgver"

    export CARGO_HOME="$srcdir/cargo-home"
    export RUSTUP_TOOLCHAIN=stable

    # tailwind.css is pre-built and included in the release tarball
    dx build --release --platform desktop -p kopuz
}

package() {
    cd "$pkgname-$pkgver"

    local appdir="target/dx/kopuz/release/linux/app"

    install -Dm755 "$appdir/kopuz" "$pkgdir/usr/bin/kopuz"

    # Install bundled assets alongside binary
    if [[ -d "$appdir/assets" ]]; then
        cp -r "$appdir/assets" "$pkgdir/usr/bin/"
    fi

    install -Dm644 "data/com.temidaradev.kopuz.desktop" \
        "$pkgdir/usr/share/applications/com.temidaradev.kopuz.desktop"
    sed -i "s|Exec=kopuz|Exec=/usr/bin/kopuz|" \
        "$pkgdir/usr/share/applications/com.temidaradev.kopuz.desktop"

    install -Dm644 "kopuz/assets/logo.png" \
        "$pkgdir/usr/share/icons/hicolor/256x256/apps/com.temidaradev.kopuz.png"

    install -Dm644 "data/com.temidaradev.kopuz.metainfo.xml" \
        "$pkgdir/usr/share/metainfo/com.temidaradev.kopuz.metainfo.xml"

    install -Dm644 "LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
