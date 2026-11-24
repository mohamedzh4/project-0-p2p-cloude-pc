# Ubuntu 24.04 (same as the Pro guest) with FFmpeg dev libraries, Xvfb and
# PulseAudio: builds every feature and runs a real capture -> encode ->
# stream -> decode check with no GPU (libx264). See scripts/linux-check.sh.
# Run from stream/: docker build -f scripts/linux-check.Dockerfile --target check .
FROM ubuntu:24.04 AS base
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl build-essential pkg-config clang libclang-dev \
      libavcodec-dev libavformat-dev libavdevice-dev libavutil-dev libavfilter-dev \
      libswscale-dev libswresample-dev \
      libasound2-dev libudev-dev libxkbcommon-dev libwayland-dev \
      xvfb x11-utils xterm pulseaudio python3 \
    && rm -rf /var/lib/apt/lists/*
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.95.0
ENV PATH=/root/.cargo/bin:$PATH
WORKDIR /src

# docker build -f scripts/linux-check.Dockerfile --target check .
FROM base AS check
COPY . /src
RUN --mount=type=cache,target=/root/.cargo/registry --mount=type=cache,target=/src/target \
    find proto server client -name "*.rs" -exec touch {} + && bash scripts/linux-check.sh
