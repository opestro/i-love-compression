# I Love Compression

Compresses **videos** and **images** on macOS and Windows. Open it, choose Video or Image,
set the options, drop your files, press Compress. Compressed copies are saved next to the
originals (or in a folder you pick) and never overwrite anything.

| | Video | Image |
|---|---|---|
| Formats | HEVC (plays everywhere) or AV1 (smallest) | JPEG, PNG, WebP, AVIF, ICO |
| Typical saving | lectures 20–30× smaller | 70–93% |
| Speed | 18.7 min lecture: 3m39s on an M1 Pro, 54 s on an RTX 3060 Ti | well under a second per photo |

## How it's built

```
engine/    ilc-engine (Rust): every compression setting lives here, plus the tests
macos/     native SwiftUI app (Liquid Glass on macOS 26)
windows/   native WPF app (.NET 10, Windows 11 Fluent theme)
```

Both apps only draw the UI and run `ilc-engine`, which ships next to them with ffmpeg:

- **Video**: x265 / SVT-AV1 on the CPU, NVIDIA NVENC on the GPU (Windows, "GPU · fastest").
  15 fps and mono speech audio, tuned with VMAF on lecture recordings
  (numbers in `engine/src/video.rs`).
- **Image**: mozjpeg, libimagequant + oxipng (lossy PNG, like TinyPNG), libwebp, AVIF (rav1e).
  Photos are turned upright from EXIF; metadata can be kept.

`ilc-engine` also works on its own:

```bash
ilc-engine image photo.jpg --format avif --quality 60 --width 1920 --height 1080
ilc-engine video lecture.mp4 --codec hevc --engine gpu
```

## Download

Get the latest version from **[Releases](https://github.com/opestro/i-love-compression/releases/latest)**:
- **macOS** (Apple Silicon): `I-Love-Compression-X.Y.Z-macOS-arm64.dmg`
- **Windows** (x64): `I-Love-Compression-X.Y.Z-Windows-x64.zip`, unzip and run `ILoveCompression.exe`

They aren't signed yet:
- **Mac:** open the app once, then System Settings → Privacy & Security → **Open Anyway**.
- **Windows:** on the SmartScreen warning click **More info → Run anyway**.

GPU encoding needs an NVIDIA driver 570 or newer; AV1 on the GPU needs an RTX 40 series.

## Build

```bash
cd engine && cargo test                       # add -- --include-ignored with ffmpeg on PATH
macos/build.sh                                # Mac: build/I Love Compression.app (Homebrew ffmpeg)
```

Windows builds run in CI (`.github/workflows/build.yml`). To tune the NVIDIA setting on a
new GPU, run `scripts/calibrate-nvenc.ps1 -Video lecture.mp4` from the Windows app folder.

## Releasing

Versions follow `MAJOR.MINOR.PATCH`: bump the last number for fixes, the middle one for
features. One command sets the version in all three parts, tags and pushes:

```bash
scripts/release.sh 0.1.1
```

CI then builds both apps and publishes the GitHub release with the downloads and a list of
the changes since the previous version.

## License

[GPL-3.0](LICENSE). The apps bundle FFmpeg (built with x265, GPL) and link libimagequant
(GPL-3.0), so the whole project is GPL-3.0.
