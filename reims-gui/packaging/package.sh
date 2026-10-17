#!/bin/sh
# package.sh <version> <binary> "<runtime deps>": runs in the reims-gui build image
# (scripts/build.sh calls it through cargo-docker.sh) and writes the .tar.gz, the .deb and their
# .sha256 files to dist/. <runtime deps> are the dlopened libraries, already named for the build
# release by build.sh. The build release is the oldest supported one (24.04), so the same
# package runs on 26.04 too.
set -eu
version=$1 bin=$2 runtime=$3
here=packaging
dist=dist
umask 022
mkdir -p "$dist"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cargo metadata --format-version 1 --locked --filter-platform x86_64-unknown-linux-gnu > "$work/metadata.json"
python3 "$here/third-party-licenses.py" "$work/metadata.json" > "$work/THIRD_PARTY_LICENSES"
cp "$work/THIRD_PARTY_LICENSES" "$dist/THIRD_PARTY_LICENSES"

# License texts THIRD_PARTY_NOTICES.md points to: boot assets, icons, fonts.
third=$work/third-party
mkdir -p "$third"
cp assets/BOOT_ASSETS_NOTICES.md crates/rg-ui/assets/lucide/LICENSE-lucide.txt \
    crates/rg-ui/assets/fonts/OFL-NotoSans.txt crates/rg-ui/assets/fonts/OFL-JetBrainsMono.txt "$third/"
cp assets/files/ovmf/copyright "$third/OVMF-copyright"

base=reims-gui-$version-x86_64

# Tarball: fixed order, owner and mtime keep it independent of the build host.
tree=$work/tar/$base
mkdir -p "$tree"
cp "$bin" "$tree/reims-gui"
cp -R LICENSE LICENSES THIRD_PARTY_NOTICES.md "$third" "$work/THIRD_PARTY_LICENSES" "$here/README.md" \
    "$here/reims-gui.desktop" "$here/reims-gui.svg" "$tree/"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime=@0 -C "$work/tar" -cf - "$base" \
    | gzip -9n > "$dist/$base.tar.gz"

# .deb: Depends = what the ELF links (dpkg-shlibdeps) + what winit/wgpu dlopen + QEMU.
root=$work/deb
doc=$root/usr/share/doc/reims-gui
mkdir -p "$root/DEBIAN" "$root/usr/bin" "$root/usr/share/applications" \
    "$root/usr/share/icons/hicolor/scalable/apps" "$doc"
install -m 0755 "$bin" "$root/usr/bin/reims-gui"
install -m 0644 "$here/reims-gui.desktop" "$root/usr/share/applications/reims-gui.desktop"
install -m 0644 "$here/reims-gui.svg" "$root/usr/share/icons/hicolor/scalable/apps/reims-gui.svg"
cp -R "$here/copyright" LICENSE LICENSES THIRD_PARTY_NOTICES.md "$third" "$work/THIRD_PARTY_LICENSES" \
    "$here/README.md" "$doc/"
chmod -R u+rwX,go+rX,go-w "$root/usr"

# dpkg-shlibdeps wants a debian/control next to it.
mkdir -p "$work/src/debian"
printf 'Source: reims-gui\n\nPackage: reims-gui\nArchitecture: amd64\n' > "$work/src/debian/control"
shlibs=$(cd "$work/src" && dpkg-shlibdeps -O "$root/usr/bin/reims-gui" | sed -n 's/^shlibs:Depends=//p')
depends="$shlibs, $(echo "$runtime" | sed 's/  */, /g'), qemu-system-x86, qemu-system-gui, qemu-utils"

cat > "$root/DEBIAN/control" <<EOF
Package: reims-gui
Version: $version
Architecture: amd64
Maintainer: Nilton OS <jniltinho@gmail.com>
Installed-Size: $(du -sk "$root/usr" | cut -f1)
Depends: $depends
Recommends: mesa-vulkan-drivers
Section: admin
Priority: optional
Homepage: https://github.com/jniltinho/macos-kvm-gui
Description: macOS virtual machines on QEMU/KVM with Reims vGPU
 reims-gui creates, runs and shows macOS virtual machines on QEMU/KVM, with the
 Reims vGPU display. One binary holds the GUI, the CLI and the MCP server.
 Distributed under GPL-3.0-or-later.
EOF
deb=reims-gui_${version}_amd64.deb
SOURCE_DATE_EPOCH=0 dpkg-deb --root-owner-group -Zxz --build "$root" "$dist/$deb" >/dev/null
echo "Depends: $depends"

cd "$dist"
for f in "$base.tar.gz" "$deb"; do
    sha256sum "$f" > "$f.sha256"
done
