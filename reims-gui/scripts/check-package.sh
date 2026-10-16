#!/bin/sh
# check-package.sh <24.04|26.04>: installs dist/'s .deb for that release with apt in a clean
# ubuntu:<release> container (so its Depends must resolve there) and runs `reims-gui --version`
# and `reims-gui --self-test` under Xvfb, with lavapipe from the Recommends.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
case "${1:-}" in 24.04|26.04) rel=$1 ;; *) echo "usage: $0 <24.04|26.04>" >&2; exit 2 ;; esac
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)
deb=reims-gui_${version}_ubuntu${rel}_amd64.deb
[ -f "$root/dist/$deb" ] || { echo "missing dist/$deb: run scripts/build.sh $rel" >&2; exit 1; }

docker run --rm -v "$root/dist:/dist:ro" -e DEBIAN_FRONTEND=noninteractive "ubuntu:$rel" sh -euc "
    apt-get update -qq
    apt-get install -y -qq /dist/$deb xvfb xauth >/dev/null
    dpkg -s reims-gui | grep -E '^(Version|Depends|Recommends):'
    test -f /usr/share/applications/reims-gui.desktop
    # The ubuntu image's dpkg excludes /usr/share/doc, so ask the package list, not the disk.
    dpkg -L reims-gui | grep -qx /usr/share/doc/reims-gui/THIRD_PARTY_LICENSES
    reims-gui --version
    xvfb-run -a reims-gui --self-test
"
echo "ubuntu:$rel: $deb ok"
