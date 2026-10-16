#!/usr/bin/env bash
# Packages a built binary into dist/: a .tar.gz, a .deb (with menu entry and icon) and SHA-256 files.
# Usage: scripts/package.sh <version, e.g. v0.1.1> <path to binary>
set -euo pipefail

VERSION="${1:?version}"
BIN="${2:?binary}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$ROOT/dist"
NAME="macos-kvm-gui-$VERSION-linux-amd64"
DEBVER="${VERSION#v}"
mkdir -p "$DIST"
# Remove earlier packages only: the binary may live in dist/ too.
rm -rf "$DIST"/*.tar.gz "$DIST"/*.deb "$DIST"/*.sha256 "$DIST/$NAME" "$DIST/deb"

# Tarball: binary plus README and licenses.
mkdir -p "$DIST/$NAME"
install -m 0755 "$BIN" "$DIST/$NAME/macos-kvm-gui"
cp "$ROOT/README.md" "$ROOT/LICENSE" "$ROOT/THIRD_PARTY_NOTICES.md" "$DIST/$NAME/"
tar -C "$DIST" -czf "$DIST/$NAME.tar.gz" "$NAME"
rm -rf "$DIST/$NAME"

# Debian package.
PKG="$DIST/deb"
install -D -m 0755 "$BIN" "$PKG/usr/bin/macos-kvm-gui"
install -D -m 0644 "$ROOT/internal/assets/logo/logo.svg" "$PKG/usr/share/icons/hicolor/scalable/apps/macos-kvm-gui.svg"
install -D -m 0644 "$ROOT/LICENSE" "$PKG/usr/share/doc/macos-kvm-gui/copyright"
install -D -m 0644 "$ROOT/THIRD_PARTY_NOTICES.md" "$PKG/usr/share/doc/macos-kvm-gui/THIRD_PARTY_NOTICES.md"
install -D -m 0644 /dev/stdin "$PKG/usr/share/applications/macos-kvm-gui.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=macOS KVM
Comment=Install and run macOS on KVM (home lab use)
Exec=macos-kvm-gui
Icon=macos-kvm-gui
Terminal=false
Categories=System;Emulator;
EOF
mkdir -p "$PKG/DEBIAN"
cat >"$PKG/DEBIAN/control" <<EOF
Package: macos-kvm-gui
Version: $DEBVER
Section: utils
Priority: optional
Architecture: amd64
Depends: libc6 (>= 2.38), libgl1, libx11-6, libwayland-client0, qemu-system-x86, qemu-utils, pkexec
Maintainer: Nilton OS <jniltinho@gmail.com>
Homepage: https://github.com/jniltinho/macos-kvm-gui
Description: Desktop app to install and run macOS on KVM (home lab use)
 Single binary with built-in OpenCore and OVMF: checks the host, downloads
 and verifies the macOS recovery from Apple, creates the disk and controls
 the VM. macOS's license only allows it on Apple-branded hardware.
EOF
dpkg-deb --build --root-owner-group "$PKG" "$DIST/macos-kvm-gui_${DEBVER}_amd64.deb" >/dev/null
rm -rf "$PKG"

cd "$DIST"
for f in *.tar.gz *.deb; do sha256sum "$f" >"$f.sha256"; done
ls -lh "$DIST"
