# Build from this repository's checked-out source; releases pin that checkout by tag.
pkgname=kiln
pkgver=0.1.0
pkgrel=1
pkgdesc='Declarative package profiles for Kiln Linux, on top of pacman'
arch=('x86_64')
url='https://github.com/kilnlinux/kiln-package-manager'
license=('MIT')
# Refuse updates on systems older than the libraries used to build this binary.
depends=('pacman' "glibc>=$(pacman -Q glibc | cut -d' ' -f2)" "gcc-libs>=$(pacman -Q gcc-libs | cut -d' ' -f2)")
makedepends=('rust')
options=('!debug')
# Keep makepkg's cleanbuild directory separate from Cargo's tracked src/.
[[ $BUILDDIR != "$startdir" ]] || BUILDDIR="$startdir/.makepkg"

build() {
    cargo build --release --locked --manifest-path "$startdir/Cargo.toml"
}

check() {
    cargo test --locked --manifest-path "$startdir/Cargo.toml"
}

package() {
    install -Dm755 "$startdir/target/release/kiln" "$pkgdir/usr/bin/kiln"
    install -Dm644 "$startdir/LICENSE" "$pkgdir/usr/share/licenses/kiln/LICENSE"
}
