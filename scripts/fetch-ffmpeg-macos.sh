#!/usr/bin/env bash
# Downloads static Apple Silicon ffmpeg (ffmpeg.martin-riedl.de, signed) into
# vendor/ffmpeg/, for `FFMPEG_DIR=vendor/ffmpeg macos/build.sh`. Used by CI.
set -euo pipefail
cd "$(dirname "$0")/.."

[ "$(uname -s)-$(uname -m)" = "Darwin-arm64" ] || { echo "macOS arm64 only" >&2; exit 1; }
dest=vendor/ffmpeg
mkdir -p "$dest"
tmp=$(mktemp -d)
for tool in ffmpeg; do
  curl -fsSL -o "$tmp/$tool.zip" "https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/release/$tool.zip"
  unzip -o -q "$tmp/$tool.zip" -d "$tmp/$tool"
  install -m 755 "$tmp/$tool/$tool" "$dest/$tool"
done

encoders=$("$dest/ffmpeg" -hide_banner -encoders)
for need in libx265 libsvtav1 libopus; do
  grep -q " $need " <<<"$encoders" || { echo "this ffmpeg build lacks $need" >&2; exit 1; }
done
echo "ffmpeg ready: $("$dest/ffmpeg" -version | head -1)"
