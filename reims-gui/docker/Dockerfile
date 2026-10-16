# reims-gui build image: one per Ubuntu release, so binaries link that release's libraries.
# The host needs only Docker; scripts/cargo-docker.sh runs cargo in it as the calling user.
ARG UBUNTU=24.04
FROM ubuntu:${UBUNTU}
ARG RUST=1.99.0
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl git build-essential pkg-config python3 dpkg-dev fakeroot file \
        libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev \
        libvulkan1 mesa-vulkan-drivers libegl1 libgl1 xvfb xauth \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal \
        --default-toolchain ${RUST} -c clippy,rustfmt \
    && chmod -R a+rX /opt/rustup /opt/cargo
WORKDIR /work
