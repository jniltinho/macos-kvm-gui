#!/usr/bin/env bash
# Packages Reims vGPU's QEMU for this repository's Releases (.github/workflows/reims-release.yml),
# and runs the same way locally.
#
#   scripts/reims-package.sh pkg <qemu-dir> <release> <out-dir>
#       <qemu-dir> is a build from internal/core/reims/Dockerfile (share/reims-ctl/qemu/<build>);
#       writes reims-qemu-<commit>-ubuntu<release>-x86_64.tar.gz and its .sha256; a <release>
#       that does not start with a digit (rocky10, from Dockerfile.rocky) is used as it is:
#       reims-qemu-<commit>-rocky10-x86_64.tar.gz.
#   scripts/reims-package.sh src <image> <out-dir>
#       <image> is the Dockerfile's image (any release); writes reims-vgpu-src-<commit>.tar.gz
#       and its .sha256: the corresponding source of the packages' binaries (GPL-2.0).
#
# <commit> is the short (7-digit) upstream commit the Dockerfile pins, as in the release tag.
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
dockerfile="$here/internal/core/reims/Dockerfile"
commit="$(sed -n 's/^ARG REIMS_COMMIT=\([0-9a-f]\{7\}\).*/\1/p' "$dockerfile")"
[[ -n $commit ]] || { echo "no ARG REIMS_COMMIT in $dockerfile" >&2; exit 1; }

# Same bytes for the same files: sorted, no owners, fixed times, no gzip timestamp.
pack() { # <dir> <out.tar.gz> [paths...]
  local dir="$1" out="$2"
  shift 2
  tar --sort=name --owner=0 --group=0 --numeric-owner --mtime=@0 -C "$dir" -cf - "$@" | gzip -9n >"$out"
  (cd "$(dirname "$out")" && sha256sum "$(basename "$out")" >"$(basename "$out").sha256")
  ls -l "$out"
  cat "$out.sha256"
}

case "${1:-}" in
  pkg)
    [[ $# == 4 ]] || { echo "usage: $0 pkg <qemu-dir> <release> <out-dir>" >&2; exit 2; }
    qemu="$2" rel="$3" out="$4"
    [[ -x "$qemu/bin/qemu-system-x86_64" && -f "$qemu/share/qemu/ReimsVgpu.rom" ]] ||
      { echo "$qemu is not a Reims QEMU build" >&2; exit 1; }
    case $rel in [0-9]*) rel="ubuntu$rel" ;; esac
    mkdir -p "$out"
    pack "$qemu" "$(cd "$out" && pwd)/reims-qemu-$commit-$rel-x86_64.tar.gz" .
    ;;
  src)
    [[ $# == 3 ]] || { echo "usage: $0 src <image> <out-dir>" >&2; exit 2; }
    image="$2" out="$3"
    mkdir -p "$out"
    name="reims-vgpu-src-$commit"
    work="$(mktemp -d)"
    trap 'rm -rf "$work"' EXIT
    # Upstream at the commit, its vendor/qemu, the meson subprojects QEMU's configure fetches,
    # and every Rust crate (SDL3's source is the sdl3-src crate), gathered in the build image.
    docker run --rm -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" -e NAME="$name" \
      -v "$work:/out" -v "$dockerfile:/Dockerfile:ro" "$image" bash -euo pipefail -c '
        d=/out/$NAME
        mkdir -p "$d/vendor/qemu"
        git -C /src archive HEAD | tar -x -C "$d"
        git -C /src/vendor/qemu archive HEAD | tar -x -C "$d/vendor/qemu"
        cd "$d/vendor/qemu"
        whl=$(ls python/wheels/meson-*.whl)
        PYTHONPATH="$whl" python3 -m mesonbuild.mesonmain subprojects download >/dev/null
        rm -rf subprojects/*/.git
        cd "$d"
        cargo vendor --quiet vendor/cargo > cargo-vendor.toml
        mkdir -p macos-kvm-gui && cp /Dockerfile macos-kvm-gui/Dockerfile
        {
          echo "Reims vGPU QEMU, corresponding source"
          echo
          echo "reims-vgpu: https://github.com/steelbrain/reims-vgpu at $(git -C /src rev-parse HEAD)"
          echo "  licenses: LICENSE and LICENSES/ (GPL-2.0-or-later OR LGPL-3.0-or-later)"
          echo "vendor/qemu: $(git -C /src config -f .gitmodules --get submodule.vendor/qemu.url) at $(git -C /src/vendor/qemu rev-parse HEAD)"
          echo "  QEMU $(cat vendor/qemu/VERSION), license: vendor/qemu/COPYING (GPL-2.0) and vendor/qemu/LICENSE"
          echo "  vendor/qemu/subprojects: the meson subprojects its configure downloads"
          echo "  firmware blobs in vendor/qemu/pc-bios are built from the roms/ submodules below,"
          echo "  as QEMU distributes them:"
          git -C /src/vendor/qemu ls-tree HEAD roms/ | while read -r _ type sha path; do
            [[ $type == commit ]] || continue
            echo "    $path $(git -C /src/vendor/qemu config -f .gitmodules --get submodule.$path.url) $sha"
          done
          echo "vendor/cargo: every Rust crate the build uses (cargo vendor; cargo-vendor.toml holds the"
          echo "  source replacement for .cargo/config.toml); SDL3 is vendor/cargo/sdl3-src (zlib)"
          echo
          echo "Build: macos-kvm-gui/Dockerfile, the recipe the packages were built with: it runs"
          echo "scripts/install-ctl.sh up to its reims-ctl part, then strips the binaries."
        } > SOURCES.txt
        chown -R "$HOST_UID:$HOST_GID" /out'
    pack "$work" "$(cd "$out" && pwd)/$name.tar.gz" "$name"
    ;;
  *)
    sed -n '2,/^set -euo/p' "$0" | sed -e '$d' -e 's/^# \{0,1\}//' >&2
    exit 2
    ;;
esac
