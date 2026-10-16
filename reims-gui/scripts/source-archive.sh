#!/bin/sh
# source-archive.sh: dist/reims-gui-<version>-source.tar.gz, the corresponding source of the
# release binaries: reims-gui/ at HEAD (docker/Dockerfile and packaging/ included) plus every
# crate from `cargo vendor` and a .cargo/config.toml that builds from them offline.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)
base=reims-gui-$version-source
out=target/source
rm -rf "${root:?}/$out"
mkdir -p "$root/$out/$base/.cargo" "$root/dist"

"$root/scripts/cargo-docker.sh" 24.04 cargo vendor --locked --versioned-dirs "$out/vendor" \
    | sed "s|\"$out/vendor\"|\"vendor\"|" > "$root/$out/config.toml"
# Run from reims-gui/, git archive takes that folder only, paths relative to it.
git -C "$root" archive --format=tar -o "$root/$out/git.tar" HEAD .
tar -xf "$root/$out/git.tar" -C "$root/$out/$base"
mv "$root/$out/vendor" "$root/$out/$base/vendor"
mv "$root/$out/config.toml" "$root/$out/$base/.cargo/config.toml"
grep -q '^directory = "vendor"$' "$root/$out/$base/.cargo/config.toml"

tar --sort=name --owner=0 --group=0 --numeric-owner --mtime=@0 -C "$root/$out" -cf - "$base" \
    | gzip -9n > "$root/dist/$base.tar.gz"
cd "$root/dist" && sha256sum "$base.tar.gz" > "$base.tar.gz.sha256"
ls -l "$root/dist/$base.tar.gz"
