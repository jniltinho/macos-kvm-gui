#!/bin/sh
# check-gnome.sh <24.04|26.04>: GNOME's compositor (Mutter, headless, Wayland + XWayland) in a
# clean ubuntu:<release> container with dist/'s .deb and /dev/kvm:
#   1. `reims-gui --self-test` on XWayland and on native Wayland;
#   2. the GUI on XWayland starts a VMware SVGA VM (dummy disk and media: OVMF runs, enough for
#      a window) and embeds QEMU's GTK window in the Screen tab, inside Mutter's frame;
#   3. SIGTERM to the GUI lets go of QEMU's window first and Xwayland survives.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
case "${1:-}" in 24.04|26.04) rel=$1 ;; *) echo "usage: $0 <24.04|26.04>" >&2; exit 2 ;; esac
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)
deb=reims-gui_${version}_ubuntu${rel}_amd64.deb
[ -f "$root/dist/$deb" ] || { echo "missing dist/$deb: run scripts/build.sh $rel" >&2; exit 1; }
[ -c /dev/kvm ] || { echo "needs /dev/kvm" >&2; exit 1; }
out=${OUT:-$root/target/check-gnome-$rel}
rm -rf "$out"; mkdir -p "$out"

docker run --rm -i --device /dev/kvm -v "$root/dist:/dist:ro" -v "$out:/out" \
    -e DEBIAN_FRONTEND=noninteractive -e DEB="$deb" "ubuntu:$rel" sh -eu <<'EOF'
apt-get update -qq
apt-get install -y -qq "/dist/$DEB" mutter xwayland dbus-daemon xdotool x11-utils imagemagick \
    mesa-vulkan-drivers >/dev/null 2>&1
chmod 666 /dev/kvm; chmod 777 /out
mkdir -p /tmp/.X11-unix; chmod 1777 /tmp/.X11-unix
mutter --version | head -n 1

cat > /tmp/session.sh <<'IN'
#!/bin/sh
# Runs inside Mutter.
o=/out
echo "WAYLAND_DISPLAY=$WAYLAND_DISPLAY DISPLAY=$DISPLAY"
reims-gui --self-test 2>&1 | grep '^self-test' | sed 's/^/xwayland: /'
env -u DISPLAY reims-gui --self-test 2>&1 | grep '^self-test' | sed 's/^/wayland: /'
RG_DEBUG=1 reims-gui > $o/gui.log 2>&1 &
gui=$!
W=$(xdotool search --sync --pid $gui | tail -n 1)
xdotool windowsize $W 1280 800 windowmove $W 0 0; sleep 2
xdotool mousemove --window $W 461 111 click 1; sleep 1   # Screen tab
xdotool mousemove --window $W 287 60 click 1             # Start
sleep 20
import -window $W $o/screen.png
xwininfo -root -tree > $o/tree.txt
kill $gui; sleep 4
kill -0 $gui 2>/dev/null && echo "gui still running" || echo "gui exited"
xwininfo -root >/dev/null 2>&1 && echo "xwayland alive" || echo "xwayland gone"
IN

su ubuntu -c 'export HOME=/home/ubuntu XDG_RUNTIME_DIR=/tmp/xdg-ubuntu
    mkdir -p -m 700 $XDG_RUNTIME_DIR; cd
    reims-gui vm create T --version sequoia >/dev/null
    reims-gui config set display=app >/dev/null
    qemu-img create -q -f qcow2 macOS-VMs/t/mac_hdd_ng.img 1G
    truncate -s 1M macOS-VMs/t/BaseSystem.img
    timeout 120 dbus-run-session -- mutter --headless --wayland --virtual-monitor 1280x800 \
        -- sh /tmp/session.sh > /out/session.log 2>&1 || true'
EOF

log=$out/session.log
grep -E '^(WAYLAND_DISPLAY|xwayland:|wayland:|gui |xwayland )' "$log" || true
fail=0
check() { if eval "$2"; then echo "ok: $1"; else echo "FAIL: $1"; fail=1; fi; }
check "self-test on XWayland" "grep -q '^xwayland: self-test: the window drew' $log"
check "self-test on Wayland" "grep -q '^wayland: self-test: the window drew' $log"
check "QEMU's window embedded" "grep -q '\[embed\] embed ' $out/gui.log && ! grep -q 'embedded window died' $out/gui.log"
# The window tree: QEMU's window under the app's (app > container > QEMU).
check "QEMU's window inside the app" "grep -A4 '\"reims-gui\": (\"\" ' $out/tree.txt | grep -q '\"QEMU'"
check "released on SIGTERM" "grep -q 'release child' $out/gui.log && grep -q '^gui exited' $log"
check "Xwayland survives" "grep -q '^xwayland alive' $log && ! grep -q 'Segmentation fault' $log"
echo "logs and screenshot: $out"
exit $fail
