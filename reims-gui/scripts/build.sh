#!/bin/sh
# build.sh: release build in the Ubuntu 24.04 image, then the .tar.gz and .deb (with .sha256
# files) in reims-gui/dist/. Built against 24.04's libraries, the same package installs and runs
# on 26.04 (scripts/check-package.sh and check-gnome.sh check both). The host needs only Docker.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
rel=24.04
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }
target=target/release-$rel

CARGO_TARGET_DIR=$target "$root/scripts/cargo-docker.sh" "$rel" cargo build --release --locked -p reims-gui

# winit and wgpu dlopen these, so dpkg-shlibdeps cannot see them. Use the release's t64 name
# where it has one (asked of apt in the image, as root: the build image keeps no apt lists).
libs="libx11-6 libx11-xcb1 libxcursor1 libxi6 libxrandr2 libxkbcommon0 libxkbcommon-x11-0 \
libwayland-client0 libwayland-cursor0 libvulkan1 libegl1 libgl1"
# shellcheck disable=SC2086 # $libs splits into the package names on purpose
runtime=$(docker run --rm "reims-gui-build:$rel" sh -c '
    apt-get update -qq >/dev/null
    for p; do
        if apt-cache show "${p}t64" >/dev/null 2>&1; then printf "%s " "${p}t64"
        elif apt-cache show "$p" >/dev/null 2>&1; then printf "%s " "$p"
        else echo "no package $p" >&2; exit 1; fi
    done' sh $libs)
runtime=${runtime% }

CARGO_TARGET_DIR=$target "$root/scripts/cargo-docker.sh" "$rel" \
    sh packaging/package.sh "$version" "$target/release/reims-gui" "$runtime"
ls -l "$root/dist"
