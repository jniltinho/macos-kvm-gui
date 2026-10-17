#!/bin/sh
# check-rpm.sh <rocky10|fedora42>: installs dist/'s .rpm with dnf in a clean container (so its
# Requires must resolve there) and runs `reims-gui --version` and `--self-test`: on Rocky Linux 10
# under a headless weston with XWayland (RHEL 10 has no Xvfb), on Fedora 42 under Xvfb.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)
rpm=reims-gui-$version-1.x86_64.rpm
[ -f "$root/dist/$rpm" ] || { echo "missing dist/$rpm: run scripts/build.sh" >&2; exit 1; }
case "${1:-}" in
rocky10)
    docker run --rm -v "$root/dist:/dist:ro" rockylinux/rockylinux:10 sh -euc "
        dnf -y -q install epel-release dnf-plugins-core >/dev/null
        dnf config-manager --set-enabled crb
        dnf -y -q install /dist/$rpm weston xorg-x11-server-Xwayland >/dev/null
        rpm -q reims-gui
        reims-gui --version
        export XDG_RUNTIME_DIR=/tmp/rt; mkdir -p -m 700 \$XDG_RUNTIME_DIR; mkdir -p -m 1777 /tmp/.X11-unix
        weston --backend=headless --socket=wl0 --xwayland >/tmp/weston.log 2>&1 &
        sleep 4
        DISPLAY=:0 WAYLAND_DISPLAY=wl0 reims-gui --self-test"
    ;;
fedora42)
    docker run --rm -v "$root/dist:/dist:ro" fedora:42 sh -euc "
        dnf -y -q install /dist/$rpm xorg-x11-server-Xvfb >/dev/null
        rpm -q reims-gui
        reims-gui --version
        Xvfb :7 -screen 0 1280x1024x24 >/dev/null 2>&1 &
        sleep 2
        DISPLAY=:7 reims-gui --self-test"
    ;;
*) echo "usage: $0 <rocky10|fedora42>" >&2; exit 2 ;;
esac
echo "$1: $rpm ok"
