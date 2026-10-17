#!/bin/sh
# sync-assets.sh: copies the Go app's boot files and icons (../internal/assets, built by
# scripts/fetch-assets.sh) and its THIRD_PARTY_NOTICES.md into reims-gui/assets, which rg-assets embeds. The build container
# mounts only reims-gui/, so the copy is committed; run this again after fetch-assets.sh.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
src=$root/../internal/assets
dst=$root/assets
rm -rf "$dst"
mkdir -p "$dst"
cp -a "$src/files" "$src/icons" "$src/logo" "$src/config.plist.tmpl" "$dst/"
# Their licenses, shipped in the packages as third-party/BOOT_ASSETS_NOTICES.md.
cp "$root/../THIRD_PARTY_NOTICES.md" "$dst/BOOT_ASSETS_NOTICES.md"
