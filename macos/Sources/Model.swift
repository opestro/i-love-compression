import Foundation
import SwiftUI

enum Mode: String {
    case video, image

    var extensions: Set<String> {
        switch self {
        case .video: ["mp4", "mov", "m4v", "mkv", "avi", "webm", "wmv", "flv", "mts", "m2ts", "ts"]
        case .image: ["jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff", "ico"]
        }
    }

    static func of(_ url: URL) -> Mode? {
        [Mode.video, .image].first { $0.extensions.contains(url.pathExtension.lowercased()) }
    }
}

enum JobStatus { case waiting, running, done, failed }

final class Job: ObservableObject, Identifiable {
    let id = UUID()
    let url: URL
    @Published var status = JobStatus.waiting
    @Published var progress = 0.0
    @Published var before: Int64 = 0
    @Published var after: Int64 = 0
    @Published var output: URL?
    @Published var message = ""

    init(url: URL) { self.url = url }

    func apply(_ e: EngineEvent) {
        switch e.event {
        case "progress":
            progress = e.progress ?? progress
        case "done":
            status = .done
            progress = 1
            before = e.before ?? 0
            after = e.after ?? 0
            output = e.output.map { URL(fileURLWithPath: $0) }
            message = e.note ?? ""
        default:
            status = .failed
            message = e.message ?? "Failed"
        }
    }
}

enum VideoCodec: String, CaseIterable, Identifiable {
    case hevc, av1
    var id: String { rawValue }
    var title: String { self == .hevc ? "HEVC · plays everywhere" : "AV1 · smallest" }
    var hint: String {
        self == .hevc
            ? "Plays in QuickTime, Safari, Windows and on every iPhone and iPad."
            : "About 1.6× smaller than HEVC, but Apple devices only play it on M3+ Macs and iPhone 15 Pro+. Chrome, Firefox and VLC play it anywhere."
    }
}

enum ImageFormat: String, CaseIterable, Identifiable {
    case jpeg, png, webp, avif, ico
    var id: String { rawValue }
    var title: String { self == .ico ? "ICO" : rawValue.uppercased().replacingOccurrences(of: "WEBP", with: "WebP") }
    var hint: String {
        switch self {
        case .jpeg: "Photos. Opens everywhere."
        case .png: "Graphics and screenshots. 100 = lossless; lower values reduce colours (like TinyPNG), usually 60–80% smaller."
        case .webp: "Best all-rounder for websites. Every modern browser."
        case .avif: "Smallest files. Modern browsers, iPhone, Android."
        case .ico: "Windows and website icons, 16–256 px."
        }
    }
}

enum ImagePreset: String, CaseIterable, Identifiable {
    case web = "Web Optimized", social = "Social Media", quality = "High Quality", smallest = "Smallest", custom = "Custom"
    var id: String { rawValue }

    /// format, quality, resize box (nil = keep size)
    var settings: (ImageFormat, Double, (Int, Int)?)? {
        switch self {
        case .web: (.webp, 75, (1920, 1080))
        case .social: (.jpeg, 85, (1200, 1200))
        case .quality: (.png, 100, nil)
        case .smallest: (.avif, 60, nil)
        case .custom: nil
        }
    }
}

@MainActor
final class AppModel: ObservableObject {
    static let shared = AppModel()

    @Published var mode: Mode?
    @Published var jobs: [Job] = []
    @Published private(set) var running = false

    @Published var codec: VideoCodec = VideoCodec(rawValue: UserDefaults.standard.string(forKey: "codec") ?? "") ?? .hevc {
        didSet { UserDefaults.standard.set(codec.rawValue, forKey: "codec") }
    }

    @Published var preset: ImagePreset = .web
    @Published var format: ImageFormat = .webp
    @Published var quality = 75.0
    @Published var resize = true
    @Published var width = 1920
    @Published var height = 1080
    @Published var keepAspect = true
    @Published var keepMetadata = true
    @Published var rename = false
    @Published var pattern = "{original_name}-compressed"
    @Published var startNumber = 1
    @Published var outputFolder: URL?

    private var current: EngineRun?
    private var stopping = false

    var waitingCount: Int { jobs.filter { $0.status == .waiting }.count }
    var savedBytes: Int64 { jobs.filter { $0.status == .done }.reduce(0) { $0 + max(0, $1.before - $1.after) } }

    func apply(_ preset: ImagePreset) {
        self.preset = preset
        guard let (format, quality, box) = preset.settings else { return }
        self.format = format
        self.quality = quality
        resize = box != nil
        if let box { (width, height) = box }
    }

    func add(_ urls: [URL]) {
        guard let mode else { return }
        let known = Set(jobs.map(\.url))
        let new = urls.filter { mode.extensions.contains($0.pathExtension.lowercased()) && !known.contains($0) }
        jobs += new.map(Job.init)
        objectWillChange.send()
    }

    /// Files from Finder, the Dock or `open -a`: switches to their mode when idle.
    func open(_ urls: [URL], compress: Bool) {
        guard let first = urls.first, let kind = Mode.of(first) else { return }
        if kind != mode && !running {
            jobs.removeAll()
            mode = kind
        }
        add(urls)
        if compress { start() }
    }

    func clearFinished() {
        jobs.removeAll { $0.status == .done || $0.status == .failed }
    }

    func start() {
        guard !running, waitingCount > 0 else { return }
        stopping = false
        if mode == .video { runNextVideo() } else { runImages() }
    }

    func stop() {
        stopping = true
        current?.stop()
    }

    // Videos one at a time: each encode already fills every core.
    private func runNextVideo() {
        guard !stopping, let job = jobs.first(where: { $0.status == .waiting }) else { return finish() }
        job.status = .running
        run(["video", job.url.path, "--codec", codec.rawValue], onEvent: { job.apply($0) }) { [weak self] in
            if job.status == .running { job.status = .failed; job.message = "Stopped" }
            self?.runNextVideo()
        }
    }

    // Images in one engine run, which compresses a few at once.
    private func runImages() {
        let batch = jobs.filter { $0.status == .waiting }
        batch.forEach { $0.status = .running }
        var args = ["image", "--format", format.rawValue, "--quality", String(Int(quality)),
                    "--name", rename ? pattern : "{original_name}-compressed", "--number", String(startNumber)]
        if resize {
            args += ["--width", String(width), "--height", String(height)]
            if !keepAspect { args.append("--stretch") }
        }
        if keepMetadata { args.append("--keep-metadata") }
        if let outputFolder { args += ["--out-dir", outputFolder.path] }
        run(args + batch.map(\.url.path), onEvent: { event in
            if let i = event.index, batch.indices.contains(i) { batch[i].apply(event) }
        }) { [weak self] in
            for job in batch where job.status == .running {
                job.status = .waiting // stopped before it started; can run again
            }
            self?.finish()
        }
    }

    private func run(_ args: [String], onEvent: @escaping (EngineEvent) -> Void, onExit: @escaping () -> Void) {
        running = true
        let engine = EngineRun(arguments: args, onEvent: { [weak self] event in
            onEvent(event)
            self?.objectWillChange.send()
        }, onExit: onExit)
        current = engine
        do {
            try engine.start()
        } catch {
            for job in jobs where job.status == .running {
                job.status = .failed
                job.message = "Couldn't start the engine: \(error.localizedDescription)"
            }
            finish()
        }
    }

    private func finish() {
        current = nil
        running = false
        objectWillChange.send()
    }
}
