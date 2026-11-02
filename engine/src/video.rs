//! Video settings. Numbers come from VMAF benchmarks on Easy University lectures
//! (whiteboard + small webcam), 60 s slice:
//!   M1 Pro: AV1 SVT preset 7 + hw decode = 1.43x faster than preset 6, +1.6% size, -0.06 VMAF
//!           HEVC x265 medium+rd4/subme3/ref4 CRF 29.5 = 3.1x faster than slow CRF 30, -0.6% size
//!   RTX 3060 Ti: NVENC p5 cq34, no multipass = 3.3x faster than x265, 4% smaller, -0.34 VMAF
//!                (full 18.7 min lecture: 54.5 s GPU vs 216.6 s CPU)
//! Rejected after measuring: Apple's hardware HEVC encoder (VMAF 92, bigger), chunked
//! parallel encodes (slower), keeping frames on the GPU with scale_cuda (only 0.3 s faster).

use crate::{command, Event};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    Hevc,
    Av1,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Cpu,
    Gpu,
}

/// What this machine's ffmpeg can do, probed once by the app at startup.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Hardware {
    pub nvenc_hevc: bool,
    pub nvenc_av1: bool,
    pub aac_at: bool,
}

const NVENC_HEVC_CQ: u32 = 34;
// ponytail: AV1 value is an uncalibrated guess; calibrate on an RTX 40 series card.
const NVENC_AV1_CQ: u32 = 40;

/// name.mp4 -> name.hevc.mp4 / name.av1.mp4, next to the original.
pub fn output_path(input: &Path, codec: Codec) -> PathBuf {
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    let suffix = match codec {
        Codec::Hevc => "hevc",
        Codec::Av1 => "av1",
    };
    input.with_file_name(format!("{stem}.{suffix}.mp4"))
}

pub fn args(input: &Path, output: &Path, codec: Codec, engine: Engine, hw: &Hardware, channels: u32) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-nostdin", "-loglevel", "error", "-progress", "pipe:1", "-nostats"]
        .map(String::from)
        .to_vec();
    // Hardware decoding (VideoToolbox on Mac, DXVA2/CUDA on Windows); ffmpeg falls back to
    // software when the GPU can't decode the source. Free ~11%, identical output.
    a.extend(["-hwaccel", "auto", "-i"].map(String::from));
    a.push(input.to_string_lossy().into_owned());
    // 15 fps: handwriting and a talking head look normal, halves the motion to encode.
    a.extend(["-vf", "fps=15", "-g", "300"].map(String::from));

    let video: &[&str] = match (codec, engine) {
        (Codec::Hevc, Engine::Cpu) => &[
            "-c:v", "libx265", "-preset", "medium", "-crf", "29.5",
            "-x265-params", "rd=4:subme=3:ref=4:log-level=error", "-pix_fmt", "yuv420p10le",
        ],
        // scm = AV1 screen-content tools (palette, intra copy) for text and UI.
        (Codec::Av1, Engine::Cpu) => &[
            "-c:v", "libsvtav1", "-preset", "7", "-crf", "44", "-svtav1-params", "scm=1", "-pix_fmt", "yuv420p10le",
        ],
        (Codec::Hevc, Engine::Gpu) => &[
            "-c:v", "hevc_nvenc", "-preset", "p5", "-tune", "hq", "-rc", "vbr", "-b:v", "0",
            "-multipass", "disabled", "-spatial-aq", "1", "-temporal-aq", "1", "-rc-lookahead", "32",
            "-bf", "4", "-b_ref_mode", "middle", "-profile:v", "main10", "-pix_fmt", "p010le",
        ],
        (Codec::Av1, Engine::Gpu) => &[
            "-c:v", "av1_nvenc", "-preset", "p5", "-tune", "hq", "-rc", "vbr", "-b:v", "0",
            "-multipass", "disabled", "-spatial-aq", "1", "-temporal-aq", "1", "-rc-lookahead", "32",
            "-pix_fmt", "p010le",
        ],
    };
    a.extend(video.iter().map(|s| s.to_string()));
    match (codec, engine) {
        (Codec::Hevc, Engine::Gpu) => a.extend(["-cq".into(), NVENC_HEVC_CQ.to_string()]),
        (Codec::Av1, Engine::Gpu) => a.extend(["-cq".into(), NVENC_AV1_CQ.to_string()]),
        _ => {}
    }
    if codec == Codec::Hevc {
        a.extend(["-tag:v", "hvc1"].map(String::from)); // Safari/QuickTime refuse HEVC without it
    }

    if channels == 0 {
        a.push("-an".into());
    } else {
        // Speech in mono. ffmpeg's -ac 1 keeps power, which on identical L/R adds +3 dB
        // and clipped the voice; averaging keeps the original level. -2 dB = codec headroom.
        let mono = if channels == 2 { "pan=mono|c0=0.5*c0+0.5*c1" } else { "aformat=channel_layouts=mono" };
        a.extend(["-af".into(), format!("{mono},volume=-2dB")]);
        let audio: &[&str] = match codec {
            Codec::Av1 => &["-c:a", "libopus", "-b:a", "64k", "-ar", "48000"],
            // AAC so every iPhone/Safari plays it; Apple's encoder when available.
            Codec::Hevc if hw.aac_at => &["-c:a", "aac_at", "-b:a", "96k"],
            Codec::Hevc => &["-c:a", "aac", "-b:a", "96k"],
        };
        a.extend(audio.iter().map(|s| s.to_string()));
    }

    a.extend(["-movflags", "+faststart", "-y"].map(String::from));
    a.push(output.to_string_lossy().into_owned());
    a
}

/// `-progress pipe:1` line "out_time_us=12345678" -> seconds.
pub fn parse_progress(line: &str) -> Option<f64> {
    line.strip_prefix("out_time_us=")?.trim().parse::<i64>().ok().map(|us| us.max(0) as f64 / 1e6)
}

/// What `ffmpeg -i` prints about the input -> (duration seconds, first audio stream's channels:
/// 0 = no audio, 1 mono, 2 stereo, 6 = any other layout). Saves shipping ffprobe (165 MB).
pub fn parse_input_info(text: &str) -> (f64, u32) {
    let duration = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("Duration: "))
        .and_then(|rest| {
            let parts: Vec<f64> = rest.split(',').next()?.split(':').filter_map(|p| p.trim().parse().ok()).collect();
            (parts.len() == 3).then(|| parts[0] * 3600.0 + parts[1] * 60.0 + parts[2])
        })
        .unwrap_or(0.0);
    let channels = text.lines().find(|l| l.contains("Stream #") && l.contains("Audio:")).map_or(0, |l| {
        if l.contains(", mono") {
            1
        } else if l.contains(", stereo") {
            2
        } else {
            6
        }
    });
    (duration, channels)
}

fn media_info(ffmpeg: &Path, input: &Path) -> (f64, u32) {
    // With no output file ffmpeg exits with an error after describing the input; that's fine.
    let out = command(ffmpeg).args(["-hide_banner", "-nostdin", "-i"]).arg(input).output();
    out.map(|o| parse_input_info(&String::from_utf8_lossy(&o.stderr))).unwrap_or((0.0, 0))
}

pub fn probe(ffmpeg: &Path) -> Hardware {
    let encoders = command(ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    // NVENC encoders are listed even without an NVIDIA GPU (or with a too-old driver), so try one frame.
    let works = |enc: &str| {
        encoders.contains(enc)
            && command(ffmpeg)
                .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=black:s=256x256:d=0.1", "-c:v", enc, "-f", "null", "-"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
    };
    Hardware { nvenc_hevc: works("hevc_nvenc"), nvenc_av1: works("av1_nvenc"), aac_at: encoders.contains("aac_at") }
}

/// Compresses one video, emitting progress; `cancelled` flips when the app closes stdin.
pub fn run(input: &Path, codec: Codec, engine: Engine, ffmpeg: &Path, cancel: &crate::Cancel) -> Event {
    let output = output_path(input, codec);
    let before = crate::size(input);
    if output.exists() {
        return Event::done(&output, before, crate::size(&output), "Already compressed");
    }
    let hw = probe_audio_only(ffmpeg);
    let (duration, channels) = media_info(ffmpeg, input);
    let spawned = command(ffmpeg)
        .args(args(input, &output, codec, engine, &hw, channels))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => return Event::failed(format!("Couldn't start ffmpeg: {e}")),
    };
    let stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    // Drain stderr on its own thread so ffmpeg can never block on a full pipe.
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let child = Arc::new(Mutex::new(child));
    cancel.on_cancel({
        let child = child.clone();
        move || {
            let _ = child.lock().unwrap().kill();
        }
    });

    let mut last_percent = -1;
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let (Some(t), true) = (parse_progress(&line), duration > 0.0) {
            let progress = (t / duration).min(1.0);
            let percent = (progress * 100.0) as i32;
            if percent != last_percent {
                last_percent = percent;
                Event::Progress { progress }.emit();
            }
        }
    }
    let status = child.lock().unwrap().wait();
    let errors = errors.join().unwrap_or_default();
    match status {
        Ok(s) if s.success() && !cancel.is_cancelled() => Event::done(&output, before, crate::size(&output), ""),
        _ => {
            let _ = std::fs::remove_file(&output); // a half-written file would look "already compressed"
            if cancel.is_cancelled() {
                Event::failed("Stopped".into())
            } else {
                Event::failed(errors.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("ffmpeg failed").to_string())
            }
        }
    }
}

/// Only the Apple AAC check: the app already chose CPU or GPU, so skip the NVENC test encodes.
fn probe_audio_only(ffmpeg: &Path) -> Hardware {
    let encoders = command(ffmpeg).args(["-hide_banner", "-encoders"]).output();
    let aac_at = encoders.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("aac_at"));
    Hardware { aac_at, ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(codec: Codec, engine: Engine, hw: &Hardware, ch: u32) -> String {
        args(Path::new("/v/in.mp4"), Path::new("/v/in.out.mp4"), codec, engine, hw, ch).join(" ")
    }

    #[test]
    fn profiles() {
        let hw = Hardware { aac_at: true, ..Default::default() };
        let hevc = joined(Codec::Hevc, Engine::Cpu, &hw, 2);
        assert!(hevc.contains("-hwaccel auto -i /v/in.mp4 -vf fps=15"));
        assert!(hevc.contains("libx265 -preset medium -crf 29.5 -x265-params rd=4:subme=3:ref=4"));
        assert!(hevc.contains("-tag:v hvc1") && hevc.contains("-c:a aac_at -b:a 96k"));
        assert!(hevc.ends_with("-y /v/in.out.mp4"));

        let av1 = joined(Codec::Av1, Engine::Cpu, &hw, 2);
        assert!(av1.contains("libsvtav1 -preset 7 -crf 44 -svtav1-params scm=1"));
        assert!(av1.contains("-c:a libopus -b:a 64k") && !av1.contains("hvc1"));

        let gpu = joined(Codec::Hevc, Engine::Gpu, &Hardware::default(), 2);
        assert!(gpu.contains("hevc_nvenc -preset p5") && gpu.contains("-multipass disabled") && gpu.contains("-cq 34"));
        assert!(gpu.contains("-c:a aac -b:a 96k"));
        assert!(joined(Codec::Av1, Engine::Gpu, &hw, 2).contains("av1_nvenc"));
    }

    #[test]
    fn audio_downmix() {
        let hw = Hardware::default();
        assert!(joined(Codec::Av1, Engine::Cpu, &hw, 2).contains("-af pan=mono|c0=0.5*c0+0.5*c1,volume=-2dB"));
        assert!(joined(Codec::Av1, Engine::Cpu, &hw, 1).contains("-af aformat=channel_layouts=mono,volume=-2dB"));
        let silent = joined(Codec::Av1, Engine::Cpu, &hw, 0);
        assert!(silent.contains("-an") && !silent.contains("-af"));
    }

    #[test]
    fn parsing() {
        assert_eq!(parse_progress("out_time_us=1500000"), Some(1.5));
        assert_eq!(parse_progress("out_time_us=N/A"), None);
        assert_eq!(parse_progress("frame=12"), None);
        let lecture = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'exo2-1-.mp4':\n  Duration: 00:18:39.55, start: 0.000000, bitrate: 5038 kb/s\n  Stream #0:0[0x1](und): Video: hevc (Main) (hvc1 / 0x31637668), yuv420p(tv, bt709), 1730x1080, 4834 kb/s, 30 fps\n  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 44100 Hz, stereo, fltp, 197 kb/s (default)\n";
        assert_eq!(parse_input_info(lecture), (1119.55, 2));
        assert_eq!(parse_input_info("  Duration: 00:00:02.00, start: 0\n  Stream #0:1: Audio: opus, 48000 Hz, mono, fltp\n"), (2.0, 1));
        assert_eq!(parse_input_info("  Duration: 00:01:00.00, bitrate: 1 kb/s\n  Stream #0:1: Audio: ac3, 48000 Hz, 5.1(side), fltp\n").1, 6);
        assert_eq!(parse_input_info("  Duration: N/A, bitrate: N/A\n  Stream #0:0: Video: h264\n"), (0.0, 0));
        assert_eq!(output_path(Path::new("/a/exo 2.mp4"), Codec::Hevc), PathBuf::from("/a/exo 2.hevc.mp4"));
    }
}
