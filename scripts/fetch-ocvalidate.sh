#!/usr/bin/env bash
# Downloads OpenCore's ocvalidate (used by the tests, never committed) into internal/core/testdata.
# Uses the OpenCore release pinned in assets.env, with the same overrides as fetch-assets.sh.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/internal/core/testdata/ocvalidate"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

load_env() {
  [ -f "$1" ] || return 0
  while IFS='=' read -r key value; do
    [[ "$key" =~ ^OPENCORE_(URL|SHA256)$ ]] || continue
    [ -n "${!key+x}" ] || printf -v "$key" '%s' "$value"
  done <"$1"
}
load_env "$ROOT/.env"
load_env "$ROOT/assets.env"

curl -sSfL -o "$TMP/opencore.zip" "$OPENCORE_URL"
echo "$OPENCORE_SHA256  $TMP/opencore.zip" | sha256sum -c --quiet
unzip -q -j "$TMP/opencore.zip" Utilities/ocvalidate/ocvalidate.linux -d "$TMP"
install -D -m 0755 "$TMP/ocvalidate.linux" "$DEST"
echo "ocvalidate ready in $DEST"
