# Maintainer: Adrien <pestouille@gmail.com>
pkgname=omniman
pkgver=0.1.0
pkgrel=1
pkgdesc="Spotlight-like launcher with AI and clipboard history for Wayland/GNOME"
arch=('x86_64')
url="https://github.com/apestel/omniman"
license=('MIT')
depends=('gtk4' 'libadwaita' 'wl-clipboard' 'dbus' 'xdg-utils' 'glib2')
makedepends=('rust' 'cargo' 'zstd')
install=omniman.install
source=("$pkgname-$pkgver.tar.gz::https://github.com/apestel/$pkgname/archive/v$pkgver.tar.gz")
sha256sums=('SKIP')

prepare() {
    cd "$pkgname-$pkgver"
    cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
    cd "$pkgname-$pkgver"
    export RUSTUP_TOOLCHAIN=stable
    export CARGO_TARGET_DIR=target
    # Clear user RUSTFLAGS to prevent personal linker overrides (e.g. lld)
    # from breaking static archive linking in offline builds.
    export RUSTFLAGS=""
    cargo build --release --workspace --frozen --offline
}

check() {
    cd "$pkgname-$pkgver"
    cargo test --release --workspace --frozen --offline
}

package() {
    cd "$pkgname-$pkgver"

    # Binaries
    install -Dm755 target/release/omnimand  "$pkgdir/usr/bin/omnimand"
    install -Dm755 target/release/omniman   "$pkgdir/usr/bin/omniman"

    # Desktop entry (NoDisplay=true — used for autostart, not app launcher)
    install -Dm644 data/omniman.desktop \
        "$pkgdir/usr/share/applications/omniman.desktop"

    # Systemd user unit
    install -Dm644 data/systemd/omnimand.service \
        "$pkgdir/usr/lib/systemd/user/omnimand.service"

    # D-Bus session service (enables dbus-daemon auto-activation)
    install -Dm644 data/dbus/org.adrien.OmnimanDaemon.service \
        "$pkgdir/usr/share/dbus-1/services/org.adrien.OmnimanDaemon.service"

    # Helper scripts (shortcut registration)
    install -Dm755 data/setup-shortcut.sh  "$pkgdir/usr/share/omniman/setup-shortcut.sh"
    install -Dm755 data/remove-shortcut.sh "$pkgdir/usr/share/omniman/remove-shortcut.sh"

    # License
    install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE" 2>/dev/null || true
}
