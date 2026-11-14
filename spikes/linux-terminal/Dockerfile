# Azulejo spike 0.7: Linux terminal path (GTK4 + VTE + herdr attach, libghostty-vt probe)
FROM ubuntu:24.04

ARG HERDR_VERSION=v0.9.5
ARG ZIG_VERSION=0.17.0
ARG GHOSTTY_COMMIT=0d1d5e884b88a711f1ccf47387ab41e8fa28a755

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8 \
    TERM=xterm-256color

RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential pkg-config curl ca-certificates git xz-utils python3 \
      libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev \
      xvfb x11-apps imagemagick dbus-x11 libgl1-mesa-dri \
      fonts-dejavu-core fonts-jetbrains-mono \
      vim htop procps \
    && rm -rf /var/lib/apt/lists/*

# Rust
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
ENV PATH=/root/.cargo/bin:$PATH

# herdr
RUN arch=$(uname -m) \
    && curl -sSfL -o /usr/local/bin/herdr \
       https://github.com/herdrdev/herdr/releases/download/${HERDR_VERSION}/herdr-linux-${arch} \
    && chmod +x /usr/local/bin/herdr

# Zig (for libghostty-vt)
RUN arch=$(uname -m) \
    && curl -sSfL https://ziglang.org/download/${ZIG_VERSION}/zig-${arch}-linux-${ZIG_VERSION}.tar.xz \
       | tar -xJ -C /opt \
    && ln -s /opt/zig-${arch}-linux-${ZIG_VERSION}/zig /usr/local/bin/zig

# libghostty-vt, pinned
RUN git init /opt/ghostty && cd /opt/ghostty \
    && git remote add origin https://github.com/ghostty-org/ghostty.git \
    && git fetch --depth 1 origin ${GHOSTTY_COMMIT} && git checkout FETCH_HEAD \
    && zig build -Demit-lib-vt -Doptimize=ReleaseFast --prefix /opt/ghostty-vt

# Pre-fetch Rust deps for the VTE app (source is mounted at runtime)
COPY vte/Cargo.toml /tmp/vte-deps/Cargo.toml
RUN mkdir -p /tmp/vte-deps/src && echo 'fn main(){}' > /tmp/vte-deps/src/main.rs \
    && cd /tmp/vte-deps && cargo fetch

WORKDIR /spike

# Input injection for scroll/takeover tests (separate layer keeps the Ghostty build cached)
RUN apt-get update && apt-get install -y --no-install-recommends xdotool && rm -rf /var/lib/apt/lists/*
