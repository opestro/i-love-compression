import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct ContentView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        if let mode = model.mode {
            ModeView(model: model, mode: mode)
        } else {
            HomeView(model: model)
        }
    }
}

// MARK: - Home: choose video or image

struct HomeView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        VStack(spacing: 28) {
            VStack(spacing: 10) {
                Image(nsImage: NSApp.applicationIconImage)
                    .resizable()
                    .frame(width: 88, height: 88)
                Text("I Love Compression")
                    .font(.system(size: 34, weight: .bold))
                Text("What do you want to compress?")
                    .font(.title3)
                    .foregroundStyle(.secondary)
            }
            HStack(spacing: 20) {
                ModeCard(symbol: "film.stack", title: "Video", detail: "Lecture videos up to 20× smaller",
                         formats: "MP4 · MOV · MKV · AVI · WebM") { model.mode = .video }
                ModeCard(symbol: "photo.on.rectangle.angled", title: "Image", detail: "Photos and graphics 70–90% smaller",
                         formats: "JPEG · PNG · WebP · AVIF · ICO") { model.mode = .image }
            }
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct ModeCard: View {
    let symbol: String
    let title: String
    let detail: String
    let formats: String
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            VStack(spacing: 12) {
                Image(systemName: symbol)
                    .font(.system(size: 44, weight: .light))
                    .foregroundStyle(.tint)
                Text(title).font(.title.weight(.semibold))
                Text(detail).foregroundStyle(.secondary)
                Text(formats).font(.caption).foregroundStyle(.tertiary)
            }
            .frame(width: 260, height: 220)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .glassCard(cornerRadius: 24, interactive: true)
        .scaleEffect(hovering ? 1.03 : 1)
        .animation(.spring(duration: 0.25), value: hovering)
        .onHover { hovering = $0 }
        .accessibilityLabel("Compress \(title.lowercased())s")
    }
}

// MARK: - Mode: options + files

struct ModeView: View {
    @ObservedObject var model: AppModel
    let mode: Mode
    @State private var picking = false
    @State private var targeted = false

    var body: some View {
        HStack(spacing: 0) {
            Group {
                if mode == .video { VideoOptions(model: model) } else { ImageOptions(model: model) }
            }
            .frame(width: 320)
            .disabled(model.running)
            Divider()
            VStack(spacing: 0) {
                if model.jobs.isEmpty {
                    dropZone(large: true).padding(24)
                } else {
                    List(model.jobs) { JobRow(job: $0, mode: mode, model: model) }
                        .listStyle(.inset)
                    Divider()
                    bottomBar
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .overlay { if targeted { RoundedRectangle(cornerRadius: 12).stroke(.tint, lineWidth: 3).padding(6) } }
            .dropDestination(for: URL.self) { urls, _ in
                model.add(urls)
                return true
            } isTargeted: { targeted = $0 }
        }
        .navigationTitle(mode == .video ? "Video" : "Image")
        .toolbar {
            ToolbarItem(placement: .navigation) {
                Button { model.jobs.removeAll(); model.mode = nil } label: { Label("Home", systemImage: "chevron.left") }
                    .disabled(model.running)
                    .help("Back to choosing video or image")
            }
            ToolbarItem(placement: .primaryAction) {
                Button { picking = true } label: { Label("Add Files", systemImage: "plus") }
                    .help("Choose files to compress")
            }
        }
        .fileImporter(isPresented: $picking, allowedContentTypes: [mode == .video ? .movie : .image], allowsMultipleSelection: true) { result in
            if case let .success(urls) = result { model.add(urls) }
        }
    }

    private func dropZone(large: Bool) -> some View {
        VStack(spacing: 14) {
            Image(systemName: mode == .video ? "film.stack" : "photo.stack")
                .font(.system(size: 48, weight: .light))
                .foregroundStyle(.secondary)
            Text(mode == .video ? "Drop videos here" : "Drop images here").font(.title2.weight(.semibold))
            Text("Then pick your options on the left and press Compress.").foregroundStyle(.secondary)
            Button("Choose Files…") { picking = true }.controlSize(.large)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(
            RoundedRectangle(cornerRadius: 18)
                .strokeBorder(style: StrokeStyle(lineWidth: 1.5, dash: [7, 5]))
                .foregroundStyle(.tertiary)
        )
    }

    private var bottomBar: some View {
        HStack(spacing: 12) {
            if model.savedBytes > 0 {
                Label("Saved \(formatBytes(model.savedBytes))", systemImage: "heart.fill")
                    .foregroundStyle(.pink)
                    .font(.headline)
            }
            Spacer()
            Button("Clear Finished") { model.clearFinished() }
                .disabled(model.running)
            if model.running {
                Button(role: .cancel) { model.stop() } label: { Text("Stop").frame(minWidth: 90) }
                    .controlSize(.large)
            } else {
                Button { model.start() } label: {
                    Text(model.waitingCount == 1 ? "Compress 1 File" : "Compress \(model.waitingCount) Files").frame(minWidth: 130)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .keyboardShortcut(.defaultAction)
                .disabled(model.waitingCount == 0)
            }
        }
        .padding(14)
    }
}

struct VideoOptions: View {
    @ObservedObject var model: AppModel

    var body: some View {
        Form {
            Section("Format") {
                Picker("Format", selection: $model.codec) {
                    ForEach(VideoCodec.allCases) { Text($0.title).tag($0) }
                }
                .pickerStyle(.radioGroup)
                .labelsHidden()
                Text(model.codec.hint).font(.caption).foregroundStyle(.secondary)
            }
            Section("Tuned for lectures") {
                Label("15 fps: handwriting and the webcam look normal", systemImage: "speedometer")
                Label("Mono speech audio at the original loudness", systemImage: "waveform")
                Label("Hardware decoding on Apple Silicon", systemImage: "cpu")
            }
            .font(.callout)
            .foregroundStyle(.secondary)
            Section {
                Text("Compressed copies are saved next to the originals as name.hevc.mp4 or name.av1.mp4.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
    }
}

struct ImageOptions: View {
    @ObservedObject var model: AppModel

    var body: some View {
        Form {
            Section("Preset") {
                Picker("Preset", selection: Binding(get: { model.preset }, set: { model.apply($0) })) {
                    ForEach(ImagePreset.allCases) { Text($0.rawValue).tag($0) }
                }
            }
            Section("Output") {
                Picker("Format", selection: custom($model.format)) {
                    ForEach(ImageFormat.allCases) { Text($0.title).tag($0) }
                }
                if model.format != .ico {
                    LabeledContent("Quality") {
                        HStack {
                            Slider(value: custom($model.quality), in: 1...100, step: 1)
                            Text("\(Int(model.quality))").monospacedDigit().frame(width: 30, alignment: .trailing)
                        }
                    }
                }
                Text(model.format.hint).font(.caption).foregroundStyle(.secondary)
            }
            Section("Resize") {
                Toggle("Shrink to fit", isOn: custom($model.resize))
                if model.resize {
                    HStack {
                        TextField("Width", value: custom($model.width), format: .number.grouping(.never))
                        Text("×")
                        TextField("Height", value: custom($model.height), format: .number.grouping(.never))
                    }
                    HStack {
                        ForEach([("HD", 1920, 1080), ("4K", 3840, 2160), ("Social", 1200, 1200)], id: \.0) { name, w, h in
                            Button(name) { model.width = w; model.height = h; model.preset = .custom }
                                .controlSize(.small)
                        }
                    }
                    Toggle("Keep aspect ratio", isOn: custom($model.keepAspect))
                    Text("Images are only ever made smaller, never enlarged.").font(.caption).foregroundStyle(.secondary)
                }
            }
            Section("Details") {
                Toggle("Keep metadata (EXIF, colour profile)", isOn: $model.keepMetadata)
                Toggle("Rename files", isOn: $model.rename)
                if model.rename {
                    TextField("Pattern", text: $model.pattern)
                    Stepper("Start number: \(model.startNumber)", value: $model.startNumber, in: 1...99_999)
                    Text("{original_name} {number} {date} {width} {height}").font(.caption.monospaced()).foregroundStyle(.secondary)
                }
            }
            Section("Save To") {
                LabeledContent(model.outputFolder?.lastPathComponent ?? "Next to the originals") {
                    HStack {
                        if model.outputFolder != nil { Button("Reset") { model.outputFolder = nil } }
                        Button("Choose…") { chooseFolder() }
                    }
                }
            }
        }
        .formStyle(.grouped)
    }

    /// Editing a preset's setting switches the preset to Custom, like the old app.
    private func custom<T>(_ binding: Binding<T>) -> Binding<T> {
        Binding(get: { binding.wrappedValue }, set: { binding.wrappedValue = $0; model.preset = .custom })
    }

    private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.prompt = "Save Here"
        if panel.runModal() == .OK { model.outputFolder = panel.url }
    }
}

struct JobRow: View {
    @ObservedObject var job: Job
    let mode: Mode
    @ObservedObject var model: AppModel

    var body: some View {
        HStack(spacing: 12) {
            icon.font(.title2).frame(width: 28)
            VStack(alignment: .leading, spacing: 4) {
                Text(job.url.lastPathComponent).lineLimit(1).truncationMode(.middle)
                detail.font(.caption)
            }
            Spacer(minLength: 8)
            if job.status == .done, let output = job.output {
                Button { NSWorkspace.shared.activateFileViewerSelecting([output]) } label: { Image(systemName: "magnifyingglass") }
                    .buttonStyle(.borderless)
                    .help("Show in Finder")
            }
        }
        .padding(.vertical, 4)
    }

    @ViewBuilder private var icon: some View {
        switch job.status {
        case .waiting: Image(systemName: "clock").foregroundStyle(.secondary)
        case .running: ProgressView().controlSize(.small)
        case .done: Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
        case .failed: Image(systemName: "xmark.octagon.fill").foregroundStyle(.red)
        }
    }

    @ViewBuilder private var detail: some View {
        switch job.status {
        case .waiting:
            Text("Waiting").foregroundStyle(.secondary)
        case .running:
            if mode == .video {
                HStack {
                    ProgressView(value: job.progress)
                    Text("\(Int(job.progress * 100))%").monospacedDigit().foregroundStyle(.secondary)
                }
            } else {
                Text("Compressing…").foregroundStyle(.secondary)
            }
        case .done:
            HStack(spacing: 6) {
                Text("\(formatBytes(job.before)) → \(formatBytes(job.after))").foregroundStyle(.secondary)
                if job.before > 0 {
                    let change = 100 - Int(job.after * 100 / job.before)
                    Text(change >= 0 ? "−\(change)%" : "+\(-change)%")
                        .fontWeight(.semibold)
                        .foregroundStyle(change >= 0 ? .green : .orange)
                }
                if !job.message.isEmpty { Text("· \(job.message)").foregroundStyle(.secondary) }
            }
        case .failed:
            Text(job.message).foregroundStyle(.red).lineLimit(2)
        }
    }
}
