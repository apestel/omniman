# Maintainer: Adrien <pestouille@gmail.com>
pkgname=omniman
pkgver=0.1.0
pkgrel=1
pkgdesc="Spotlight-like launcher with AI and clipboard history for Wayland/GNOME"
arch=('x86_64')
url="https://github.com/adrien/omniman"
license=('MIT')
depends=('gtk4' 'libadwaita' 'wl-clipboard' 'dbus' 'xdg-utils')
makedepends=('rust' 'cargo')
source=("$pkgname-$pkgver.tar.gz::https://github.com/adrien/$pkgname/archive/v$pkgver.tar.gz")
sha256sums=('SKIP')

prepare() {
    cd "$pkgname-$pkgver"
    cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
    cd "$pkgname-$pkgver"
    export RUSTUP_TOOLCHAIN=stable
    export CARGO_TARGET_DIR=target
    cargo build --release --workspace --frozen --offline
}

check() {
    cd "$pkgname-$pkgver"
    cargo test --release --workspace --frozen --offline
}

package() {
    cd "$pkgname-$pkgver"
    install -Dm755 target/release/omnimand  "$pkgdir/usr/bin/omnimand"
    install -Dm755 target/release/omniman   "$pkgdir/usr/bin/omniman"
    install -Dm644 data/omniman.desktop     "$pkgdir/usr/share/applications/omniman.desktop"
    install -Dm644 data/systemd/omnimand.service \
        "$pkgdir/usr/lib/systemd/user/omnimand.service"
    install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE" 2>/dev/null || true
}
