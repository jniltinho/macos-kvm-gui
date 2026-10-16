#!/usr/bin/env bash
# Rebuilds internal/assets/files from the sources in assets.env, verifying every download.
# The result is committed to the repository and embedded in the binary at build time.
# Override sources with environment variables or a local .env (precedence: env > .env > assets.env).
# Needs: curl, sha256sum, unzip, dpkg-deb. Run from anywhere: scripts/fetch-assets.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/internal/assets/files"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# load_env reads KEY=value lines without executing them. Only the asset keys are accepted
# (never LD_PRELOAD, PATH and the like), they are not exported, and values already set win.
load_env() {
  [ -f "$1" ] || return 0
  while IFS='=' read -r key value; do
    [[ "$key" =~ ^(OPENCORE|LILU|VIRTUALSMC|WHATEVERGREEN|OVMF)_(VERSION|URL|SHA256)$ ]] || continue
    [ -n "${!key+x}" ] || printf -v "$key" '%s' "$value"
  done <"$1"
}
load_env "$ROOT/.env"
load_env "$ROOT/assets.env"

fetch() { # fetch NAME FILE: downloads $NAME_URL into $TMP/FILE and checks $NAME_SHA256
  local url="${1}_URL" sum="${1}_SHA256"
  echo "fetching ${!url}"
  curl -sSfL -o "$TMP/$2" "${!url}"
  echo "${!sum}  $TMP/$2" | sha256sum -c --quiet
}
fetch OPENCORE opencore.zip
fetch LILU lilu.zip
fetch VIRTUALSMC virtualsmc.zip
fetch WHATEVERGREEN whatevergreen.zip
fetch OVMF ovmf.deb

rm -rf "$OUT"
EFI="$OUT/EFI"
mkdir -p "$EFI/BOOT" "$EFI/OC/Drivers" "$EFI/OC/Kexts" "$EFI/OC/ACPI" "$OUT/ovmf"

unzip -q "$TMP/opencore.zip" -d "$TMP/oc"
cp "$TMP/oc/X64/EFI/BOOT/BOOTx64.efi" "$EFI/BOOT/"
cp "$TMP/oc/X64/EFI/OC/OpenCore.efi" "$EFI/OC/"
cp "$TMP/oc/X64/EFI/OC/Drivers/OpenRuntime.efi" "$TMP/oc/X64/EFI/OC/Drivers/OpenHfsPlus.efi" "$EFI/OC/Drivers/"

for k in lilu:Lilu.kext virtualsmc:Kexts/VirtualSMC.kext whatevergreen:WhateverGreen.kext; do
  zip="${k%%:*}" path="${k#*:}"
  unzip -q "$TMP/$zip.zip" -d "$TMP/$zip"
  cp -R "$TMP/$zip/$path" "$EFI/OC/Kexts/"
done

dpkg-deb -x "$TMP/ovmf.deb" "$TMP/ovmf"
cp "$TMP/ovmf/usr/share/OVMF/OVMF_CODE_4M.fd" "$TMP/ovmf/usr/share/OVMF/OVMF_VARS_4M.fd" "$OUT/ovmf/"
cp "$TMP/ovmf/usr/share/doc/ovmf/copyright" "$OUT/ovmf/copyright"

# Versions shown by the app.
cat >"$OUT/VERSIONS" <<EOF
OpenCore $OPENCORE_VERSION
Lilu $LILU_VERSION
VirtualSMC $VIRTUALSMC_VERSION
WhateverGreen $WHATEVERGREEN_VERSION
OVMF $OVMF_VERSION
EOF

# OpenCore's validator for the tests (git-ignored, not embedded).
install -D -m 0755 "$TMP/oc/Utilities/ocvalidate/ocvalidate.linux" "$ROOT/internal/core/testdata/ocvalidate"

echo "assets ready in $OUT ($(du -sh "$OUT" | cut -f1))"
