#!/usr/bin/env bash
# Builds "build/I Love Compression.app" (Apple Silicon): the SwiftUI app plus ilc-engine
# and ffmpeg next to its executable. `--dmg` also makes build/I-Love-Compression.dmg.
# ffmpeg comes from FFMPEG_DIR (CI: scripts/fetch-ffmpeg-macos.sh); defaults to Homebrew's.
set -euo pipefail
cd "$(dirname "$0")/.."

swift build -c release --package-path macos
cargo build --release --manifest-path engine/Cargo.toml

app="build/I Love Compression.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp macos/.build/release/ILoveCompression "$app/Contents/MacOS/"
cp engine/target/release/ilc-engine "$app/Contents/MacOS/"
ffmpeg_dir="${FFMPEG_DIR:-/opt/homebrew/bin}"
cp -L "$ffmpeg_dir/ffmpeg" "$app/Contents/MacOS/"
cp macos/Info.plist "$app/Contents/"
cp assets/AppIcon.icns "$app/Contents/Resources/"
codesign --force --deep -s - "$app"
echo "built $app"

if [ "${1:-}" = "--dmg" ]; then
  hdiutil create -volname "I Love Compression" -srcfolder "$app" -ov -format UDZO build/I-Love-Compression.dmg
fi
