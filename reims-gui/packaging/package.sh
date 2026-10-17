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

# .rpm from the same tree, for Rocky Linux / RHEL 10 and Fedora 40+ (the binary needs glibc 2.39).
# Requires names what winit/wgpu dlopen; the system QEMU is only recommended: RHEL's qemu-kvm
# lacks isa-applesmc and vmware-svga, so there macOS VMs run on Reims vGPU's own QEMU.
top=$work/rpm
mkdir -p "$top/SPECS"
cat > "$top/SPECS/reims-gui.spec" <<EOF
Name:           reims-gui
Version:        $version
Release:        1
Summary:        macOS virtual machines on QEMU/KVM with Reims vGPU
License:        GPL-3.0-or-later
URL:            https://github.com/jniltinho/macos-kvm-gui
BuildArch:      x86_64
AutoReqProv:    no
Requires:       glibc >= 2.39, libgcc, libX11, libX11-xcb, libXcursor, libXi, libXrandr, libxkbcommon, libxkbcommon-x11, libwayland-client, libwayland-cursor, vulkan-loader, mesa-libEGL, mesa-libGL, qemu-img
Recommends:     mesa-vulkan-drivers, (qemu-system-x86 or qemu-kvm)

%description
reims-gui creates, runs and shows macOS virtual machines on QEMU/KVM, with the
Reims vGPU display. One binary holds the GUI, the CLI and the MCP server.
Distributed under GPL-3.0-or-later.

%install
cp -a $root/usr %{buildroot}/

%files
/usr/bin/reims-gui
/usr/share/applications/reims-gui.desktop
/usr/share/icons/hicolor/scalable/apps/reims-gui.svg
%doc /usr/share/doc/reims-gui
EOF
rpm=reims-gui-$version-1.x86_64.rpm
HOME=$work rpmbuild --quiet -bb --define "_topdir $top" --define "_build_id_links none" \
    --define "__strip /bin/true" --define "_binary_payload w9.xzdio" "$top/SPECS/reims-gui.spec" >/dev/null
mv "$top/RPMS/x86_64/$rpm" "$dist/"

cd "$dist"
for f in "$base.tar.gz" "$deb" "$rpm"; do
    sha256sum "$f" > "$f.sha256"
done
