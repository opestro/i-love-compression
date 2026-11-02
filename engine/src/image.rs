//! Image settings, ported from Image Compressor Pro (Pillow) with better encoders:
//!   JPEG  mozjpeg (trellis quantisation, progressive) at the chosen quality
//!   PNG   quality 100 = lossless (oxipng); below 100 = palette quantisation (libimagequant,
//!         the pngquant/TinyPNG technique) then oxipng: usually 60-80% smaller
//!   WebP  libwebp, method 6 (slowest/smallest); quality 100 = lossless
//!   AVIF  rav1e via ravif, speed 6
//!   ICO   16-256 px PNG frames
//! Pixels are always turned upright from the EXIF orientation; kept EXIF gets orientation 1.

use crate::Event;
use image::{imageops::FilterType, DynamicImage, ImageDecoder, ImageReader};
use img_parts::{Bytes, DynImage, ImageEXIF, ImageICC};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    Jpeg,
    Png,
    Webp,
    Avif,
    Ico,
}

impl Format {
    pub fn parse(s: &str) -> Option<Format> {
        Some(match s.to_ascii_lowercase().as_str() {
            "jpeg" | "jpg" => Format::Jpeg,
            "png" => Format::Png,
            "webp" => Format::Webp,
            "avif" => Format::Avif,
            "ico" => Format::Ico,
            _ => return None,
        })
    }
    fn extension(self) -> &'static str {
        match self {
            Format::Jpeg => "jpg",
            Format::Png => "png",
            Format::Webp => "webp",
            Format::Avif => "avif",
            Format::Ico => "ico",
        }
    }
}

pub struct Options {
    pub format: Format,
    pub quality: u8,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub stretch: bool, // exact size instead of fitting inside the box
    pub keep_metadata: bool,
    pub out_dir: Option<PathBuf>,
    pub name: String, // pattern: {original_name} {number} {date} {width} {height}
    pub number: u32,
}

pub fn compress(input: &Path, index: usize, opts: &Options) -> Event {
    if is_own_output(input) {
        let size = crate::size(input);
        return Event::done(input, size, size, "Already a compressed copy, skipped").with_index(index);
    }
    match try_compress(input, index, opts) {
        Ok((output, before, after)) => {
            let note = if after > before { "Bigger than the original" } else { "" };
            Event::done(&output, before, after, note).with_index(index)
        }
        Err(message) => Event::failed(message).with_index(index),
    }
}

fn try_compress(input: &Path, index: usize, opts: &Options) -> Result<(PathBuf, u64, u64), String> {
    let source = std::fs::read(input).map_err(|e| format!("Couldn't read the file: {e}"))?;
    let mut decoder = ImageReader::new(std::io::Cursor::new(&source))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_decoder()
        .map_err(|e| format!("Not a supported image: {e}"))?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    let img = resize(img, opts);

    let encoded = encode(&img, opts)?;
    let encoded = if opts.keep_metadata { with_metadata(&source, encoded) } else { encoded };

    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    let name = output_name(&opts.name, &stem, opts.number + index as u32, &today(), img.width(), img.height());
    let dir = opts.out_dir.clone().unwrap_or_else(|| input.parent().unwrap_or(Path::new(".")).to_path_buf());
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create the output folder: {e}"))?;
    let output = unique_path(&dir, &name, opts.format.extension(), input);
    std::fs::write(&output, &encoded).map_err(|e| format!("Couldn't save: {e}"))?;
    Ok((output, source.len() as u64, encoded.len() as u64))
}

/// Only ever shrinks: a bigger image is never smaller on disk.
fn resize(img: DynamicImage, opts: &Options) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    let (bw, bh) = (opts.width.unwrap_or(w).min(w), opts.height.unwrap_or(h).min(h));
    if (bw, bh) == (w, h) || bw == 0 || bh == 0 {
        return img;
    }
    if opts.stretch {
        img.resize_exact(bw, bh, FilterType::Lanczos3)
    } else {
        img.resize(bw, bh, FilterType::Lanczos3) // keeps aspect, fits inside the box
    }
}

fn encode(img: &DynamicImage, opts: &Options) -> Result<Vec<u8>, String> {
    let (w, h) = (img.width(), img.height());
    let q = opts.quality.clamp(1, 100);
    let opaque = !img.color().has_alpha();
    match opts.format {
        Format::Jpeg => {
            // JPEG has no alpha: flatten onto white rather than black.
            let rgb = flatten_on_white(img);
            std::panic::catch_unwind(|| -> std::io::Result<Vec<u8>> {
                let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
                comp.set_size(w as usize, h as usize);
                comp.set_quality(q as f32);
                let mut started = comp.start_compress(Vec::new())?;
                started.write_scanlines(&rgb)?;
                started.finish()
            })
            .map_err(|_| "JPEG encoder failed".to_string())?
            .map_err(|e| e.to_string())
        }
        Format::Png => {
            let rgba: Vec<rgb::RGBA8> = img.to_rgba8().pixels().map(|p| rgb::RGBA8::new(p[0], p[1], p[2], p[3])).collect();
            let raw = if q >= 100 {
                let bytes: Vec<u8> = rgba.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();
                oxipng::RawImage::new(w, h, oxipng::ColorType::RGBA, oxipng::BitDepth::Eight, bytes)
            } else {
                let (palette, indices) = quantize(rgba, w, h, q)?;
                oxipng::RawImage::new(w, h, oxipng::ColorType::Indexed { palette }, oxipng::BitDepth::Eight, indices)
            }
            .map_err(|e| e.to_string())?;
            raw.create_optimized_png(&oxipng::Options::from_preset(2)).map_err(|e| e.to_string())
        }
        Format::Webp => {
            let pixels = if opaque { img.to_rgb8().into_raw() } else { img.to_rgba8().into_raw() };
            let encoder = if opaque {
                webp::Encoder::from_rgb(&pixels, w, h)
            } else {
                webp::Encoder::from_rgba(&pixels, w, h)
            };
            if q >= 100 {
                return Ok(encoder.encode_lossless().to_vec());
            }
            let mut config = webp::WebPConfig::new().map_err(|_| "WebP encoder failed".to_string())?;
            config.quality = q as f32;
            config.method = 6;
            config.lossless = 0;
            encoder.encode_advanced(&config).map(|m| m.to_vec()).map_err(|e| format!("WebP encoder failed: {e:?}"))
        }
        Format::Avif => {
            let enc = ravif::Encoder::new().with_quality(q as f32).with_alpha_quality(q as f32).with_speed(6);
            let result = if opaque {
                let px: Vec<ravif::RGB8> = img.to_rgb8().pixels().map(|p| ravif::RGB8::new(p[0], p[1], p[2])).collect();
                enc.encode_rgb(ravif::Img::new(&px[..], w as usize, h as usize))
            } else {
                let px: Vec<ravif::RGBA8> = img.to_rgba8().pixels().map(|p| ravif::RGBA8::new(p[0], p[1], p[2], p[3])).collect();
                enc.encode_rgba(ravif::Img::new(&px[..], w as usize, h as usize))
            };
            result.map(|r| r.avif_file).map_err(|e| e.to_string())
        }
        Format::Ico => {
            let mut frames = Vec::new();
            for size in [16u32, 32, 48, 64, 128, 256] {
                let frame = img.resize(size, size, FilterType::Lanczos3).to_rgba8();
                frames.push(
                    image::codecs::ico::IcoFrame::as_png(frame.as_raw(), frame.width(), frame.height(), image::ExtendedColorType::Rgba8)
                        .map_err(|e| e.to_string())?,
                );
            }
            let mut out = Vec::new();
            image::codecs::ico::IcoEncoder::new(&mut out).encode_images(&frames).map_err(|e| e.to_string())?;
            Ok(out)
        }
    }
}

fn quantize(rgba: Vec<rgb::RGBA8>, w: u32, h: u32, quality: u8) -> Result<(Vec<rgb::RGBA8>, Vec<u8>), String> {
    let mut liq = imagequant::new();
    liq.set_speed(4).map_err(|e| e.to_string())?;
    liq.set_quality(0, quality).map_err(|e| e.to_string())?;
    let mut image = liq.new_image(rgba, w as usize, h as usize, 0.0).map_err(|e| e.to_string())?;
    let mut result = liq.quantize(&mut image).map_err(|e| e.to_string())?;
    result.set_dithering_level(1.0).map_err(|e| e.to_string())?;
    result.remapped(&mut image).map_err(|e| e.to_string())
}

fn flatten_on_white(img: &DynamicImage) -> Vec<u8> {
    if !img.color().has_alpha() {
        return img.to_rgb8().into_raw();
    }
    img.to_rgba8()
        .pixels()
        .flat_map(|p| {
            let a = p[3] as u32;
            [0, 1, 2].map(|c| ((p[c] as u32 * a + 255 * (255 - a)) / 255) as u8)
        })
        .collect()
}

/// Copies ICC profile and EXIF from the source into the encoded file (JPEG, PNG, WebP).
/// Pixels were already turned upright, so EXIF orientation is reset to 1.
fn with_metadata(source: &[u8], encoded: Vec<u8>) -> Vec<u8> {
    let Ok(Some(src)) = DynImage::from_bytes(Bytes::copy_from_slice(source)) else { return encoded };
    let Ok(Some(mut out)) = DynImage::from_bytes(Bytes::from(encoded.clone())) else { return encoded };
    if let Some(icc) = src.icc_profile() {
        out.set_icc_profile(Some(icc));
    }
    if let Some(exif) = src.exif() {
        let mut exif = exif.to_vec();
        reset_orientation(&mut exif);
        out.set_exif(Some(Bytes::from(exif)));
    }
    let mut buf = Vec::new();
    match out.encoder().write_to(&mut buf) {
        Ok(_) => buf,
        Err(_) => encoded,
    }
}

/// Sets EXIF tag 0x0112 (Orientation) in IFD0 to 1. Returns false if there's no such tag.
fn reset_orientation(exif: &mut [u8]) -> bool {
    let base = if exif.starts_with(b"Exif\0\0") { 6 } else { 0 };
    let t = &mut exif[base..];
    if t.len() < 8 {
        return false;
    }
    let le = &t[0..2] == b"II";
    let u16_at = |t: &[u8], i: usize| if le { u16::from_le_bytes([t[i], t[i + 1]]) } else { u16::from_be_bytes([t[i], t[i + 1]]) };
    let u32_at = |t: &[u8], i: usize| {
        let b = [t[i], t[i + 1], t[i + 2], t[i + 3]];
        if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) }
    };
    let ifd = u32_at(t, 4) as usize;
    if ifd + 2 > t.len() {
        return false;
    }
    for i in 0..u16_at(t, ifd) as usize {
        let entry = ifd + 2 + i * 12;
        if entry + 12 > t.len() {
            return false;
        }
        if u16_at(t, entry) == 0x0112 {
            let one = if le { 1u16.to_le_bytes() } else { 1u16.to_be_bytes() };
            t[entry + 8..entry + 10].copy_from_slice(&one);
            return true;
        }
    }
    false
}

pub fn output_name(pattern: &str, original: &str, number: u32, date: &str, width: u32, height: u32) -> String {
    let name = pattern
        .replace("{original_name}", original)
        .replace("{number}", &format!("{number:03}"))
        .replace("{date}", date)
        .replace("{width}", &width.to_string())
        .replace("{height}", &height.to_string());
    let clean: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    if clean.trim().is_empty() { original.to_string() } else { clean }
}

/// photo-compressed.webp / "photo-compressed (2).webp": made with the default name pattern,
/// e.g. picked up again by dropping a whole folder. Custom patterns can't be recognised.
pub fn is_own_output(input: &Path) -> bool {
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    let base = match stem.rsplit_once(" (") {
        Some((base, n)) if n.len() > 1 && n.ends_with(')') && n[..n.len() - 1].bytes().all(|b| b.is_ascii_digit()) => base,
        _ => &stem,
    };
    base.ends_with("-compressed")
}

/// Never overwrites anything, least of all the original: adds " (2)", " (3)", ... when taken.
pub fn unique_path(dir: &Path, name: &str, ext: &str, input: &Path) -> PathBuf {
    let mut path = dir.join(format!("{name}.{ext}"));
    let mut n = 2;
    while path.exists() || same_file(&path, input) {
        path = dir.join(format!("{name} ({n}).{ext}"));
        n += 1;
    }
    path
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y) || a == b
}

/// Today as YYYYMMDD (UTC) without a date crate: days since 1970 -> civil date (H. Hinnant).
fn today() -> String {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}{m:02}{d:02}")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naming() {
        assert_eq!(output_name("{original_name}-compressed", "IMG 1", 1, "20261102", 10, 20), "IMG 1-compressed");
        assert_eq!(output_name("trip_{number}_{width}x{height}_{date}", "x", 7, "20261102", 1920, 1080), "trip_007_1920x1080_20261102");
        assert_eq!(output_name("a/b:c", "x", 1, "d", 1, 1), "a_b_c");
        assert_eq!(output_name("  ", "orig", 1, "d", 1, 1), "orig");
        assert!(is_own_output(Path::new("/a/f1-compressed.webp")) && is_own_output(Path::new("/a/f1-compressed (12).png")));
        assert!(!is_own_output(Path::new("/a/f1.png")) && !is_own_output(Path::new("/a/f1-compressed (x).png")));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_759), (2026, 11, 2));
    }

    #[test]
    fn never_overwrites_the_original() {
        let dir = std::env::temp_dir().join("ilc-unique-test");
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("photo.jpg");
        std::fs::write(&original, b"x").unwrap();
        assert_eq!(unique_path(&dir, "photo", "jpg", &original), dir.join("photo (2).jpg"));
        assert_eq!(unique_path(&dir, "photo", "webp", &original), dir.join("photo.webp"));
    }

    #[test]
    fn orientation_reset() {
        // Little-endian TIFF with one IFD0 entry: Orientation = 6 (rotate 90).
        let mut exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec();
        assert!(reset_orientation(&mut exif));
        assert_eq!(&exif[6 + 8 + 2 + 8..6 + 8 + 2 + 10], &[1, 0]);
        assert!(!reset_orientation(&mut b"short".to_vec()));
    }

    #[test]
    fn resize_only_shrinks() {
        let img = DynamicImage::new_rgb8(4000, 3000);
        let mut opts = Options {
            format: Format::Jpeg, quality: 80, width: Some(1920), height: Some(1080), stretch: false,
            keep_metadata: false, out_dir: None, name: String::new(), number: 1,
        };
        let fitted = resize(img.clone(), &opts);
        assert_eq!((fitted.width(), fitted.height()), (1440, 1080));
        opts.width = Some(8000);
        opts.height = None;
        assert_eq!(resize(img, &opts).width(), 4000);
    }
}
