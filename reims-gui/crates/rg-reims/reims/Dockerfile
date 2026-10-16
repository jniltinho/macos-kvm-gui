# syntax=docker/dockerfile:1
# Builds Reims vGPU's QEMU (github.com/steelbrain/reims-vgpu) for one Ubuntu release with
# upstream's own scripts/install-ctl.sh, at a pinned commit. Used by `macos-kvm-gui reims build`
# and by .github/workflows/reims-release.yml, which publishes the packages `reims install` uses.
#
#   docker build --build-arg UBUNTU=24.04 -t reims-build:24.04 - < Dockerfile
#   docker run --rm -e HOST_UID=$(id -u) -e HOST_GID=$(id -g) -v <dir>:/build reims-build:24.04
#
# Output: <dir>/share/reims-ctl/qemu/<date>-<commit>/ (bin/qemu-system-x86_64 and its data,
# stripped); <dir>/cache keeps the library's and QEMU's builds for the next run.
ARG UBUNTU=24.04
FROM ubuntu:${UBUNTU}
ARG REIMS_COMMIT=c099dd5a4d61107a5527efc64cf1d1ab2f3a9b85
ARG RUST=1.98.1
ENV DEBIAN_FRONTEND=noninteractive
# install-ctl.sh's apt package list (it installs what is missing, so it must all be here), plus
# sudo, which it calls.
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential git curl ca-certificates pkg-config python3 python3-venv ninja-build \
    flex bison bzip2 cmake clang lld binutils libglib2.0-dev libpixman-1-dev libslirp-dev \
    zlib1g-dev libfdt-dev libpipewire-0.3-dev libpulse-dev libasound2-dev libvulkan1 \
    libx11-dev libxext-dev libxrandr-dev libxcursor-dev libxi-dev libxfixes-dev libxss-dev \
    libxtst-dev libxkbcommon-dev libwayland-dev libwayland-bin libdecor-0-dev \
    libegl-dev libgl-dev libdrm-dev libgbm-dev libdbus-1-dev libudev-dev libgtk-3-dev \
    sudo && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain ${RUST}
# Upstream, QEMU's source (vendor/qemu) and the Rust crates, fetched once into the image.
RUN git clone https://github.com/steelbrain/reims-vgpu /src && git -C /src checkout ${REIMS_COMMIT} \
    && git -C /src submodule update --init --depth 1 vendor/qemu
WORKDIR /src
# Not --locked: the committed Cargo.lock leaves out the git sources, which cargo adds.
RUN cargo fetch
# install-ctl.sh up to its reims-ctl part (the app runs QEMU itself and needs neither reims-ctl
# nor reims-util), with the library's build kept in the mounted cache, then stripped binaries.
# Everything in /build is the caller's on any exit, a failed build's too.
RUN cat > /usr/local/bin/build-reims <<'EOF'
#!/bin/bash
set -euo pipefail
trap 'chown -R "${HOST_UID:-0}:${HOST_GID:-0}" /build' EXIT
cd /src
grep -q '^# -* reims-ctl$' scripts/install-ctl.sh || { echo "install-ctl.sh changed: no reims-ctl section" >&2; exit 1; }
sed '/^# -* reims-ctl$/,$d' scripts/install-ctl.sh > scripts/install-qemu.sh
mkdir -p /build/share /build/cache
XDG_DATA_HOME=/build/share XDG_CACHE_HOME=/build/cache CARGO_TARGET_DIR=/build/cache/target \
  bash scripts/install-qemu.sh --yes
q="$(readlink -f /build/share/reims-ctl/qemu/current)"
find "$q/bin" "$q/libexec" -type f -perm -u+x -exec strip --strip-unneeded {} + 2>/dev/null || true
echo "Built $q"
EOF
RUN chmod +x /usr/local/bin/build-reims
CMD ["build-reims"]
