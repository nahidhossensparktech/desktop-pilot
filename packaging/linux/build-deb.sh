#!/usr/bin/env bash
# Builds target/deb/desktop-pilot_<version>_<arch>.deb. Run on Debian/Ubuntu (or use
# packaging/linux/build-deb-docker.sh from any machine with Docker).
#
# Build dependencies (Ubuntu 22.04+ / Debian 12+):
#   sudo apt-get install -y build-essential cmake clang pkg-config libasound2-dev \
#     libxkbcommon-dev libxcb1-dev libwayland-dev libgl1-mesa-dev dpkg-dev
#
# Environment: CARGO (default: cargo).
set -euo pipefail

CARGO="${CARGO:-cargo}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
OUT="$ROOT/target/deb"
VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)"
ARCH="$(dpkg --print-architecture)"
PKG="$OUT/desktop-pilot_${VERSION}_${ARCH}"

cd "$ROOT"
echo "==> Building desktop-pilot $VERSION (release, $ARCH)"
$CARGO build --release -p desktop-pilot

echo "==> Rendering icons"
rm -rf "$OUT/icons"
DP_ICONSET_DIR="$OUT/icons" $CARGO test -q -p desktop-pilot write_iconset_when_asked >/dev/null

echo "==> Assembling $PKG"
rm -rf "$PKG"
install -Dm755 "$TARGET_DIR/release/desktop-pilot" "$PKG/usr/bin/desktop-pilot"
install -Dm644 "$ROOT/packaging/linux/desktop-pilot.desktop" \
    "$PKG/usr/share/applications/desktop-pilot.desktop"
for size in 32 128 256 512; do
    install -Dm644 "$OUT/icons/icon_${size}x${size}.png" \
        "$PKG/usr/share/icons/hicolor/${size}x${size}/apps/desktop-pilot.png"
done
install -Dm644 "$ROOT/LICENSE" "$PKG/usr/share/doc/desktop-pilot/copyright"
INSTALLED_KB="$(du -sk "$PKG/usr" | cut -f1)"
mkdir -p "$PKG/DEBIAN"
cat > "$PKG/DEBIAN/control" <<CONTROL
Package: desktop-pilot
Version: $VERSION
Architecture: $ARCH
Section: utils
Priority: optional
Installed-Size: $INSTALLED_KB
Depends: libc6, libgcc-s1, libstdc++6, libasound2t64 | libasound2, libgl1, libxkbcommon0, libwayland-client0, libx11-6, libxcursor1, libxi6, libxrandr2
Recommends: fonts-noto-core, speech-dispatcher | espeak-ng
Maintainer: Desktop Pilot <packages@desktoppilot.invalid>
Homepage: https://desktoppilot.invalid
Description: AI desktop assistant: control your computer with words and voice
 Desktop Pilot sees your screen and uses the mouse and keyboard for you, powered
 by Claude. It includes a voice assistant (local Whisper speech recognition,
 Bangla and English) and a Siri-style chat window.
 .
 On Linux, screen capture and mouse/keyboard control are not implemented yet;
 chat and the voice assistant work.
CONTROL

echo "==> Building the package"
dpkg-deb --build --root-owner-group "$PKG" "$PKG.deb" >/dev/null
dpkg-deb --info "$PKG.deb" | sed -n '1,12p'
rm -rf "$PKG" "$OUT/icons"
echo "==> Done: $PKG.deb ($(du -h "$PKG.deb" | cut -f1))"
