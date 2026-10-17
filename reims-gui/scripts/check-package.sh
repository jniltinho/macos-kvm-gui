#!/bin/sh
# check-package.sh <24.04|26.04>: installs dist/'s .deb (built on 24.04, one for both releases)
# with apt in a clean ubuntu:<release> container (so its Depends must resolve there) and runs
# `reims-gui --version` and `reims-gui --self-test` under Xvfb, with lavapipe from the Recommends.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
case "${1:-}" in 24.04|26.04) rel=$1 ;; *) echo "usage: $0 <24.04|26.04>" >&2; exit 2 ;; esac
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)
deb=reims-gui_${version}_amd64.deb
[ -f "$root/dist/$deb" ] || { echo "missing dist/$deb: run scripts/build.sh" >&2; exit 1; }

# The license texts THIRD_PARTY_NOTICES.md points to, in both packages.
notices=$(printf 'third-party/%s ' BOOT_ASSETS_NOTICES.md OVMF-copyright LICENSE-lucide.txt \
    OFL-NotoSans.txt OFL-JetBrainsMono.txt)
tarball=reims-gui-$version-x86_64
[ -f "$root/dist/$tarball.tar.gz" ] || { echo "missing dist/$tarball.tar.gz" >&2; exit 1; }
listing=$(tar tzf "$root/dist/$tarball.tar.gz")
for f in THIRD_PARTY_LICENSES THIRD_PARTY_NOTICES.md $notices; do
    echo "$listing" | grep -qx "$tarball/$f" || { echo "$tarball.tar.gz lacks $f" >&2; exit 1; }
done
if grep -n '\.\./' "$root/THIRD_PARTY_NOTICES.md"; then
    echo "THIRD_PARTY_NOTICES.md must not point outside the package" >&2; exit 1
fi

docker run --rm -v "$root/dist:/dist:ro" -e DEBIAN_FRONTEND=noninteractive "ubuntu:$rel" sh -euc "
    apt-get update -qq
    apt-get install -y -qq /dist/$deb xvfb xauth >/dev/null
    dpkg -s reims-gui | grep -E '^(Version|Depends|Recommends):'
    test -f /usr/share/applications/reims-gui.desktop
    # The ubuntu image's dpkg excludes /usr/share/doc, so ask the package list, not the disk.
    for f in THIRD_PARTY_LICENSES THIRD_PARTY_NOTICES.md $notices; do
        dpkg -L reims-gui | grep -qx /usr/share/doc/reims-gui/\$f || { echo \"deb lacks \$f\" >&2; exit 1; }
    done
    reims-gui --version
    xvfb-run -a reims-gui --self-test
"
echo "ubuntu:$rel: $deb ok"
