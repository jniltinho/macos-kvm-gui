# reims-gui build image: one per Ubuntu release, so binaries link that release's libraries.
# The host needs only Docker; scripts/cargo-docker.sh runs cargo in it as the calling user.
ARG UBUNTU=24.04
FROM ubuntu:${UBUNTU}
ARG RUST=1.99.0
ENV DEBIAN_FRONTEND=noninteractive
# libxkbcommon-x11-0: winit on X11 (the GUI self-test). rg-x11 tests: cmake/libx*-dev build SDL3 for the helper window, python3-gi runs GTK3's.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl git build-essential pkg-config python3 dpkg-dev fakeroot file \
        libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev \
        libvulkan1 mesa-vulkan-drivers libegl1 libgl1 libxkbcommon-x11-0 xvfb xauth \
        cmake libxext-dev libxss-dev libxtst-dev python3-gi gir1.2-gtk-3.0 \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal \
        --default-toolchain ${RUST} -c clippy,rustfmt \
    && chmod -R a+rX /opt/rustup /opt/cargo
WORKDIR /work
