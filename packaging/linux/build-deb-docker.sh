#!/usr/bin/env bash
# Builds the .deb inside a Debian container, from any machine with Docker (also macOS).
#   packaging/linux/build-deb-docker.sh [amd64|arm64]   (default: amd64)
# Output: target/deb/desktop-pilot_<version>_<arch>.deb
set -euo pipefail

ARCH="${1:-amd64}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
IMAGE="desktop-pilot-deb-builder:$ARCH"

docker build --platform "linux/$ARCH" -t "$IMAGE" - <<'DOCKERFILE'
FROM debian:bookworm
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential cmake clang pkg-config curl ca-certificates git dpkg-dev \
      libasound2-dev libxkbcommon-dev libxcb1-dev libwayland-dev libgl1-mesa-dev \
    && rm -rf /var/lib/apt/lists/*
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
ENV PATH=/root/.cargo/bin:$PATH
DOCKERFILE

# The Linux build gets its own target dir so it does not mix with macOS builds.
docker run --rm --platform "linux/$ARCH" \
    -v "$ROOT":/src -w /src \
    -e CARGO_TARGET_DIR=/src/target/linux-$ARCH \
    -e CARGO="${CARGO_IN_CONTAINER:-cargo}" \
    "$IMAGE" packaging/linux/build-deb.sh
ls -la "$ROOT"/target/deb/*.deb
