#!/bin/sh
# cargo-docker.sh [24.04|26.04] <command...>: runs a command (default: cargo build) in the
# reims-gui build image for that Ubuntu release, as the calling user, with the workspace at
# /work. Parallel agents set CARGO_TARGET_DIR=target/<name> to keep their builds apart.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
rel=24.04
case "${1:-}" in 24.04|26.04) rel=$1; shift ;; esac
image=reims-gui-build:$rel
# Always through docker build: cached it is instant, and the image follows Dockerfile edits.
docker build -q --build-arg UBUNTU="$rel" -t "$image" "$root/docker" >/dev/null
[ $# -eq 0 ] && set -- cargo build --workspace
tty=""; [ -t 0 ] && tty="-t"
# RG_CARGO_HOME (a host folder) shares the crates registry between worktrees.
cargo_home=${RG_CARGO_HOME:-$root/target/cargo-home}
mkdir -p "$cargo_home" "$root/target"
exec docker run --rm -i $tty --user "$(id -u):$(id -g)" -v "$root:/work" -v "$cargo_home:/cargo-home" -w /work \
    -e HOME=/work/target/home -e CARGO_HOME=/cargo-home \
    -e CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/ubuntu$rel}" -e CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}" \
    -e RUST_BACKTRACE=1 -e RUSTUP_HOME=/opt/rustup -e PATH=/opt/cargo/bin:/usr/bin:/bin \
    "$image" "$@"
