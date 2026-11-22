#!/usr/bin/env bash
# Builds "Desktop Pilot.app" and "Desktop-Pilot-<version>-macos-<arch>.dmg" in target/dmg/.
#
#   packaging/macos/build-dmg.sh
#
# Environment:
#   CARGO          cargo command to use (default: cargo)
#   SIGN_IDENTITY  codesign identity (default: "-" = ad-hoc, runs on this Mac only;
#                  a "Developer ID Application" identity + notarization is needed to
#                  share the app with other Macs without a Gatekeeper warning)
set -euo pipefail

CARGO="${CARGO:-cargo}"
SIGN_IDENTITY="${SIGN_IDENTITY:--}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/target/dmg"
APP="$OUT/Desktop Pilot.app"
VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)"
ARCH="$(uname -m)"
DMG="$OUT/Desktop-Pilot-$VERSION-macos-$ARCH.dmg"

cd "$ROOT"
echo "==> Building desktop-pilot $VERSION (release)"
$CARGO build --release -p desktop-pilot

echo "==> Rendering the app icon"
rm -rf "$OUT/AppIcon.iconset"
DP_ICONSET_DIR="$OUT/AppIcon.iconset" $CARGO test -q -p desktop-pilot write_iconset_when_asked >/dev/null
iconutil -c icns "$OUT/AppIcon.iconset" -o "$OUT/AppIcon.icns"

echo "==> Assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/target/release/desktop-pilot" "$APP/Contents/MacOS/desktop-pilot"
cp "$OUT/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
sed "s/__VERSION__/$VERSION/g" "$ROOT/packaging/macos/Info.plist" > "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist" >/dev/null

echo "==> Signing ($SIGN_IDENTITY)"
codesign --force --deep --sign "$SIGN_IDENTITY" "$APP"
codesign --verify --deep --strict "$APP"

echo "==> Creating $DMG"
STAGE="$OUT/dmg-stage"
rm -rf "$STAGE" "$DMG"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "Desktop Pilot" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE" "$OUT/AppIcon.iconset"
hdiutil verify "$DMG" >/dev/null

echo "==> Done: $DMG ($(du -h "$DMG" | cut -f1))"
