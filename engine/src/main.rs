//! ilc-engine: the compression engine behind the I Love Compression apps (SwiftUI on macOS,
//! WPF on Windows). The apps only draw the UI; every setting lives here.
//!
//!   ilc-engine probe
//!   ilc-engine video <file> [--codec hevc|av1] [--engine cpu|gpu]
//!   ilc-engine image <file>... [--format jpeg|png|webp|avif|ico] [--quality 1-100]
//!                    [--width N] [--height N] [--stretch] [--keep-metadata]
//!                    [--out-dir DIR] [--name PATTERN] [--number N]
//!
//! Prints one JSON event per line on stdout:
//!   {"event":"progress","progress":0.42}
//!   {"event":"done","index":0,"output":"…","before":123,"after":45,"note":""}
//!   {"event":"failed","index":0,"message":"…"}
//! Closing stdin cancels (also when the app quits or crashes): ffmpeg is killed, the partial
//! file removed, and no further images are started. Keep stdin open while it runs.

mod image;
mod video;

use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

#[derive(Serialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum Event {
    Progress {
        progress: f64,
    },
    Done {
        #[serde(skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        output: String,
        before: u64,
        after: u64,
        note: String,
    },
    Failed {
        #[serde(skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        message: String,
    },
}

impl Event {
    pub fn done(output: &Path, before: u64, after: u64, note: &str) -> Event {
        Event::Done { index: None, output: output.to_string_lossy().into_owned(), before, after, note: note.into() }
    }
    pub fn failed(message: String) -> Event {
        Event::Failed { index: None, message }
    }
    pub fn with_index(mut self, i: usize) -> Event {
        if let Event::Done { index, .. } | Event::Failed { index, .. } = &mut self {
            *index = Some(i);
        }
        self
    }
    pub fn emit(&self) {
        println!("{}", serde_json::to_string(self).expect("events serialise"));
    }
}

/// Flips when stdin reaches EOF; runs registered hooks (e.g. kill ffmpeg) once.
pub struct Cancel {
    flag: AtomicBool,
    hooks: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
}

impl Cancel {
    fn watch_stdin() -> &'static Cancel {
        let cancel: &'static Cancel = Box::leak(Box::new(Cancel { flag: AtomicBool::new(false), hooks: Mutex::default() }));
        std::thread::spawn(move || {
            let _ = std::io::stdin().read_to_end(&mut Vec::new());
            cancel.cancel();
        });
        cancel
    }
    fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        for hook in self.hooks.lock().unwrap().drain(..) {
            hook();
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
    pub fn on_cancel(&self, hook: impl FnOnce() + Send + 'static) {
        let mut hooks = self.hooks.lock().unwrap();
        if self.is_cancelled() {
            drop(hooks);
            hook();
        } else {
            hooks.push(Box::new(hook));
        }
    }
}

/// A Command that never flashes a console window on Windows.
pub fn command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

pub fn size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// ffmpeg/ffprobe shipped next to this executable; falls back to PATH for development.
fn sidecar(name: &str) -> PathBuf {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&file)))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(file))
}

const SWITCHES: &[&str] = &["--stretch", "--keep-metadata"];

/// `--key value` flags, bare `--switch`es, and everything else as inputs.
fn parse(args: &[String]) -> (HashMap<String, String>, Vec<PathBuf>) {
    let (mut flags, mut inputs) = (HashMap::new(), Vec::new());
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if SWITCHES.contains(&arg.as_str()) {
            flags.insert(arg.clone(), String::new());
        } else if let Some(key) = arg.strip_prefix("--").filter(|k| !k.is_empty()) {
            flags.insert(format!("--{key}"), it.next().cloned().unwrap_or_default());
        } else {
            inputs.push(PathBuf::from(arg));
        }
    }
    (flags, inputs)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command_name, rest) = args.split_first().map(|(c, r)| (c.as_str(), r)).unwrap_or(("", &[]));
    let (flags, inputs) = parse(rest);
    let flag = |k: &str| flags.get(k).map(String::as_str);

    let ok = match command_name {
        "probe" => {
            println!("{}", serde_json::to_string(&video::probe(&sidecar("ffmpeg"))).unwrap());
            true
        }
        "video" => {
            let Some(input) = inputs.first() else { usage() };
            let codec = if flag("--codec") == Some("av1") { video::Codec::Av1 } else { video::Codec::Hevc };
            let engine = if flag("--engine") == Some("gpu") { video::Engine::Gpu } else { video::Engine::Cpu };
            let event = video::run(input, codec, engine, &sidecar("ffmpeg"), &sidecar("ffprobe"), Cancel::watch_stdin());
            let ok = matches!(event, Event::Done { .. });
            event.emit();
            ok
        }
        "image" => {
            if inputs.is_empty() {
                usage();
            }
            let opts = image::Options {
                format: flag("--format").and_then(image::Format::parse).unwrap_or(image::Format::Webp),
                quality: flag("--quality").and_then(|q| q.parse().ok()).unwrap_or(80),
                width: flag("--width").and_then(|v| v.parse().ok()).filter(|&v| v > 0),
                height: flag("--height").and_then(|v| v.parse().ok()).filter(|&v| v > 0),
                stretch: flags.contains_key("--stretch"),
                keep_metadata: flags.contains_key("--keep-metadata"),
                out_dir: flag("--out-dir").filter(|d| !d.is_empty()).map(PathBuf::from),
                name: flag("--name").unwrap_or("{original_name}-compressed").to_string(),
                number: flag("--number").and_then(|v| v.parse().ok()).unwrap_or(1),
            };
            compress_images(&inputs, &opts, Cancel::watch_stdin())
        }
        _ => usage(),
    };
    std::process::exit(if ok { 0 } else { 1 });
}

/// A few images at once: each encoder is mostly single-threaded.
fn compress_images(inputs: &[PathBuf], opts: &image::Options, cancel: &Cancel) -> bool {
    let next = AtomicUsize::new(0);
    let failures = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 4).min(inputs.len());
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= inputs.len() || cancel.is_cancelled() {
                    break;
                }
                let event = image::compress(&inputs[i], i, opts);
                if matches!(event, Event::Failed { .. }) {
                    failures.fetch_add(1, Ordering::SeqCst);
                }
                event.emit();
            });
        }
    });
    failures.load(Ordering::SeqCst) == 0 && !cancel.is_cancelled()
}

fn usage() -> ! {
    eprintln!("usage: ilc-engine probe | video <file> [--codec hevc|av1] [--engine cpu|gpu] | image <file>... [--format jpeg|png|webp|avif|ico] [--quality N] [--width N] [--height N] [--stretch] [--keep-metadata] [--out-dir DIR] [--name PATTERN] [--number N]");
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags() {
        let args: Vec<String> = ["a.jpg", "--quality", "70", "--keep-metadata", "b c.png", "--format", "avif"].map(String::from).to_vec();
        let (flags, inputs) = parse(&args);
        assert_eq!(flags.get("--quality").map(String::as_str), Some("70"));
        assert_eq!(flags.get("--format").map(String::as_str), Some("avif"));
        assert!(flags.contains_key("--keep-metadata"));
        assert_eq!(inputs, vec![PathBuf::from("a.jpg"), PathBuf::from("b c.png")]);
    }

    #[test]
    fn events_serialise() {
        let done = Event::done(Path::new("/x/a.webp"), 10, 4, "").with_index(3);
        assert_eq!(serde_json::to_string(&done).unwrap(), r#"{"event":"done","index":3,"output":"/x/a.webp","before":10,"after":4,"note":""}"#);
        assert_eq!(serde_json::to_string(&Event::Progress { progress: 0.5 }).unwrap(), r#"{"event":"progress","progress":0.5}"#);
    }
}
