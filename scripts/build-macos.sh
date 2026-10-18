#!/usr/bin/env bash
# Construit « OZ Mapping.app » et une image disque .dmg dans dist/.
#
#   scripts/build-macos.sh              # pour l'architecture de cette machine
#   scripts/build-macos.sh --universal  # Apple Silicon + Intel (cible x86_64-apple-darwin requise)
#
# L'app utilise ffmpeg/ffprobe installés sur la machine (brew install ffmpeg).
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
NAME="OZ Mapping"
BIN="ozmapping"
BUNDLE_ID="com.azpery.ozmapping"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d '"' -f2)"
DIST="$ROOT/dist"
APP="$DIST/$NAME.app"

# --- Compilation
if [[ "${1:-}" == "--universal" ]]; then
    rustup target add aarch64-apple-darwin x86_64-apple-darwin
    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin
    mkdir -p "$ROOT/target/universal"
    lipo -create -output "$ROOT/target/universal/$BIN" \
        "$ROOT/target/aarch64-apple-darwin/release/$BIN" \
        "$ROOT/target/x86_64-apple-darwin/release/$BIN"
    EXE="$ROOT/target/universal/$BIN"
else
    cargo build --release
    EXE="$ROOT/target/release/$BIN"
fi

# --- Bundle .app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$EXE" "$APP/Contents/MacOS/$BIN"

# Icône : PNG 1024 → jeu de tailles → .icns
ICONSET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$ICONSET"
swift "$ROOT/scripts/make-icon.swift" "$ROOT/assets/Phosphor.ttf" "$ICONSET/icon_512x512@2x.png"
sips -z 1024 1024 "$ICONSET/icon_512x512@2x.png" >/dev/null # le rendu se fait à l'échelle Retina
for s in 16 32 128 256 512; do
    sips -z $s $s "$ICONSET/icon_512x512@2x.png" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    d=$((s * 2))
    sips -z $d $d "$ICONSET/icon_512x512@2x.png" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$NAME</string>
    <key>CFBundleDisplayName</key><string>$NAME</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleExecutable</key><string>$BIN</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.video</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSHumanReadableCopyright</key><string>OZ Mapping</string>
</dict>
</plist>
PLIST

# Signature ad hoc : nécessaire pour lancer l'app sur Apple Silicon (pas une signature Apple Developer).
codesign --force --deep --sign - "$APP"

# --- Image disque
DMG="$DIST/OZ-Mapping-$VERSION.dmg"
STAGE="$(mktemp -d)"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -quiet -volname "$NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
rm -rf "$STAGE"

echo "App  : $APP"
echo "DMG  : $DMG"
