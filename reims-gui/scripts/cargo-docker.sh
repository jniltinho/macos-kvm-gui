#!/bin/sh
# cargo-docker.sh [24.04|26.04] <command...>: runs a command (default: cargo build) in the
# reims-gui build image for that Ubuntu release, as the calling user, with the workspace at
# /work. Parallel agents set CARGO_TARGET_DIR=target/<name> to keep their builds apart.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
rel=24.04
case "${1:-}" in 24.04|26.04) rel=$1; shift ;; esac
image=reims-gui-build:$rel
docker image inspect "$image" >/dev/null 2>&1 || docker build --build-arg UBUNTU="$rel" -t "$image" "$root/docker"
[ $# -eq 0 ] && set -- cargo build --workspace
tty=""; [ -t 0 ] && tty="-t"
mkdir -p "$root/target/cargo-home"
exec docker run --rm -i $tty --user "$(id -u):$(id -g)" -v "$root:/work" -w /work \
    -e HOME=/work/target/home -e CARGO_HOME=/work/target/cargo-home \
    -e CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/ubuntu$rel}" -e CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}" \
    -e RUST_BACKTRACE=1 -e RUSTUP_HOME=/opt/rustup -e PATH=/opt/cargo/bin:/usr/bin:/bin \
    "$image" "$@"
