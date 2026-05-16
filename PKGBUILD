# Maintainer: Adrien <pestouille@gmail.com>
pkgname=omniman
pkgver=0.1.0
pkgrel=1
pkgdesc="Spotlight-like launcher with AI and clipboard history for Wayland/GNOME"
arch=('x86_64')
url="https://github.com/apestel/omniman"
license=('MIT')
depends=('gtk4' 'libadwaita' 'wl-clipboard' 'dbus' 'xdg-utils' 'glib2' 'sqlite')
makedepends=('rust' 'cargo')
install=omniman.install
source=("$pkgname-$pkgver.tar.gz::https://github.com/apestel/$pkgname/archive/v$pkgver.tar.gz")
sha256sums=('SKIP')

prepare() {
    cd "$pkgname-$pkgver"
    # Override user's ~/.cargo/config.toml to prevent lld and pkg-config zstd
    # from bleeding into this offline build.
    mkdir -p .cargo
    cat > .cargo/config.toml << 'EOF'
[target.x86_64-unknown-linux-gnu]
linker = "gcc"
rustflags = []
EOF
    cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
    cd "$pkgname-$pkgver"
    export RUSTUP_TOOLCHAIN=stable
    export CARGO_TARGET_DIR=target
    export RUSTFLAGS=""
    # Force bundled static zstd; system libzstd.so causes DSO link errors
    # with --as-needed when the dynamic lib isn't listed explicitly.
    export ZSTD_SYS_USE_PKG_CONFIG=0
    # Strip -flto from makepkg's CFLAGS: libsqlite3-sys's bundled sqlite3.c
    # build emits LTO objects, which causes rustc to drop the -l static=sqlite3
    # directive at final link → undefined sqlite3_* references.
    export CFLAGS="${CFLAGS//-flto=auto/}"
    export CXXFLAGS="${CXXFLAGS//-flto=auto/}"
    export LDFLAGS="${LDFLAGS//-flto=auto/}"
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
