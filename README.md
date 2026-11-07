# kiln

Declarative package profiles for Arch Linux and Kiln Linux. Pacman remains
responsible for every package transaction. Kiln manages named package sets
written in TOML, records what it installed, and protects packages shared by
active profiles.

```sh
kiln profile list
kiln plan developer
sudo kiln apply developer
sudo kiln sync --purge
sudo kiln remove developer --purge
kiln doctor
```

Profiles are searched in `~/.config/kiln/profiles`, `/etc/kiln/profiles`, and
`/usr/share/kiln/profiles`, or passed as a `.toml` path. The `profiles/`
directory contains examples and test fixtures; the Kiln Linux distro ships
its own profiles.

## Build and test

```sh
cargo test --locked
cargo clippy -- -D warnings
cargo build --release --locked
```

On Arch, `makepkg` builds a pacman package from this checkout. Install it
with `sudo pacman -U kiln-*.pkg.tar.zst`. Do not run makepkg as root.

Tags named `v<version>` run the tests and publish an x86_64 Arch package as
a GitHub release asset. Keep Cargo.toml and PKGBUILD versions in sync before
tagging. Kiln Linux's `kiln-update` service uses these release packages and
verifies GitHub's SHA-256 asset digest before asking pacman to install them.

Kiln Linux: https://github.com/kilnlinux/Kiln-Linux

MIT license.
