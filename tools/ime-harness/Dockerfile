# syntax=docker/dockerfile:1
# Linux IME test harness: Xvfb, headless sway, fcitx5 and IBus with CJK and Vietnamese engines,
# and an egui test app. See README.md.

FROM debian:trixie-slim AS base

ENV DEBIAN_FRONTEND=noninteractive LANG=C.UTF-8
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl git build-essential pkg-config \
        libx11-dev libxkbcommon-dev libwayland-dev \
        libxkbcommon-x11-0 libxcursor1 libxrandr2 libxi6 libwayland-cursor0 libwayland-egl1 \
        libgl1 libegl1 libegl-mesa0 libglx-mesa0 libgl1-mesa-dri \
        xvfb xauth x11-utils xdotool \
        sway wtype wlrctl \
        dbus dbus-x11 dconf-cli dconf-service libglib2.0-bin procps \
        fcitx5 fcitx5-chinese-addons fcitx5-hangul fcitx5-unikey fcitx5-anthy \
        ibus ibus-libpinyin ibus-hangul ibus-unikey ibus-anthy \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/opt/rust/rustup CARGO_HOME=/opt/rust/cargo PATH=/opt/rust/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable \
    && chmod -R a+rX /opt/rust

FROM base AS harness

COPY app /src/app
RUN --mount=type=cache,target=/opt/rust/cargo/registry \
    --mount=type=cache,target=/src/target \
    cd /src/app \
    && CARGO_TARGET_DIR=/src/target cargo build --locked \
    && cp /src/target/debug/ime-harness-app /usr/local/bin/

COPY harness /usr/local/lib/ime-harness
RUN ln -s /usr/local/lib/ime-harness/ime-harness /usr/local/bin/ime-harness

# Runs as the container's root user: under rootless docker that is the invoking host user, so
# files written to a mounted results directory belong to them.
ENTRYPOINT ["ime-harness"]
CMD ["list"]
