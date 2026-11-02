//! Runs the real ilc-engine binary. Image tests need nothing else; the video tests need
//! ffmpeg/ffprobe on PATH or next to the binary (`cargo test -- --include-ignored`).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn engine() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ilc-engine"))
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ilc-smoke-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs to completion with stdin held open (closing it would cancel) and returns the events.
fn run(cmd: &mut Command) -> Vec<serde_json::Value> {
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let events = BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();
    child.wait().unwrap();
    events
}

fn photo(dir: &Path) -> (PathBuf, PathBuf) {
    // Gradient + soft circle + sensor-like grain + a transparent corner. Without the grain,
    // lossless PNG is unrealistically small (a real lecture frame: lossless -12%, q70 -74%).
    let mut seed = 1u32;
    let img = image::RgbaImage::from_fn(640, 480, |x, y| {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let grain = (seed >> 28) as i32 - 8;
        let d = ((x as f32 - 320.0).powi(2) + (y as f32 - 240.0).powi(2)).sqrt();
        let px = |v: i32| (v + grain).clamp(0, 255) as u8;
        let a = if x < 80 && y < 80 { 0 } else { 255 };
        image::Rgba([px((x / 3) as i32), px((y / 2) as i32), px(255 - d.min(255.0) as i32), a])
    });
    let png = dir.join("photo.png");
    img.save(&png).unwrap();
    let jpg = dir.join("photo.jpg");
    image::DynamicImage::ImageRgba8(img).to_rgb8().save(&jpg).unwrap();
    (png, jpg)
}

#[test]
fn compresses_images_to_every_format() {
    let dir = temp_dir("images");
    let (png, jpg) = photo(&dir);
    for format in ["jpeg", "png", "webp", "avif", "ico"] {
        let events = run(engine().args(["image", "--format", format, "--quality", "75", "--name", &format!("out-{format}")]).arg(&png).arg(&jpg));
        assert_eq!(events.len(), 2, "{format}: {events:?}");
        for e in &events {
            assert_eq!(e["event"], "done", "{format}: {e}");
            let out = PathBuf::from(e["output"].as_str().unwrap());
            assert!(out.exists() && e["after"].as_u64().unwrap() > 0, "{format}: {e}");
            if format != "avif" {
                image::open(&out).unwrap_or_else(|err| panic!("{format} output doesn't decode: {err}"));
            }
        }
    }
    // Two inputs with the same name pattern never overwrite each other or the source.
    let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    assert!(names.contains(&"out-png.png".to_string()) && names.contains(&"out-png (2).png".to_string()), "{names:?}");
    assert!(png.exists() && jpg.exists());
}

#[test]
fn lossy_png_is_much_smaller_and_resize_fits_the_box() {
    let dir = temp_dir("png");
    let (png, _) = photo(&dir);
    let lossless = run(engine().args(["image", "--format", "png", "--quality", "100", "--name", "lossless"]).arg(&png));
    let lossy = run(engine().args(["image", "--format", "png", "--quality", "70", "--name", "lossy", "--width", "320"]).arg(&png));
    let (a, b) = (lossless[0]["after"].as_u64().unwrap(), lossy[0]["after"].as_u64().unwrap());
    assert!(b * 2 < a, "lossy {b} should be well under half of lossless {a}");
    let resized = image::open(lossy[0]["output"].as_str().unwrap()).unwrap();
    assert_eq!((resized.width(), resized.height()), (320, 240));
}

#[test]
fn bad_input_fails_cleanly() {
    let dir = temp_dir("bad");
    let bad = dir.join("not-an-image.jpg");
    std::fs::write(&bad, b"hello").unwrap();
    let events = run(engine().args(["image", "--format", "webp"]).arg(&bad));
    assert_eq!(events[0]["event"], "failed");
    assert_eq!(events[0]["index"], 0);
}

fn clip(dir: &Path, seconds: u32) -> PathBuf {
    let clip = dir.join("clip.mp4");
    let ok = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", &format!("testsrc2=s=640x360:r=30:d={seconds}")])
        .args(["-f", "lavfi", "-i", &format!("sine=d={seconds}"), "-ac", "2", "-c:v", "mpeg4", "-c:a", "aac"])
        .arg(&clip)
        .status()
        .expect("ffmpeg on PATH");
    assert!(ok.success());
    clip
}

#[test]
#[ignore]
fn compresses_video_on_the_cpu() {
    let dir = temp_dir("video");
    let input = clip(&dir, 2);
    for codec in ["hevc", "av1"] {
        let events = run(engine().args(["video", "--codec", codec]).arg(&input));
        let last = events.last().unwrap();
        assert_eq!(last["event"], "done", "{codec}: {events:?}");
        assert!(Path::new(last["output"].as_str().unwrap()).exists());
    }
    let probe = run(engine().arg("probe"));
    assert!(probe[0].get("nvenc_hevc").is_some());
}

#[test]
#[ignore]
fn closing_stdin_stops_and_cleans_up() {
    let dir = temp_dir("cancel");
    let input = clip(&dir, 60);
    let mut child = engine().args(["video", "--codec", "av1"]).arg(&input).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let first = lines.next().unwrap().unwrap();
    assert!(first.contains("progress"), "{first}");
    drop(child.stdin.take()); // the app quitting looks exactly like this
    let last: serde_json::Value = serde_json::from_str(&lines.map(Result::unwrap).last().unwrap()).unwrap();
    child.wait().unwrap();
    assert_eq!(last["message"], "Stopped");
    assert!(!dir.join("clip.av1.mp4").exists(), "partial output left behind");
}
