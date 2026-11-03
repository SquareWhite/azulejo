# Ubuntu 24.04 dev image for the GTK4 shell (GTK 4.14, libadwaita 1.5).
FROM ubuntu:24.04
ARG RUST_TOOLCHAIN
ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential pkg-config curl ca-certificates \
      libgtk-4-dev libadwaita-1-dev \
      xvfb xdotool imagemagick dbus fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/*
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal \
      --default-toolchain "${RUST_TOOLCHAIN}" -c clippy,rustfmt
