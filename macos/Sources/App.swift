import AppKit
import SwiftUI

@main
struct ILoveCompressionApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @StateObject private var model = AppModel.shared

    var body: some Scene {
        Window("I Love Compression", id: "main") {
            ContentView(model: model)
                .frame(minWidth: 860, minHeight: 580)
        }
        .windowToolbarStyle(.unified)
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    /// Command line (runs at launch, independent of the window being shown):
    ///   ILoveCompression [--compress] file…   opens the files' mode and optionally starts
    /// Dev aids: `--mode video|image` opens a mode; `--snapshot out.png` renders the window to a
    /// PNG once everything has finished (works with the screen locked), then quits.
    @MainActor func applicationDidFinishLaunching(_ notification: Notification) {
        let args = CommandLine.arguments
        func value(after flag: String) -> String? {
            guard let i = args.firstIndex(of: flag), i + 1 < args.count else { return nil }
            return args[i + 1]
        }
        let model = AppModel.shared
        let snapshot = value(after: "--snapshot")
        let modeArg = value(after: "--mode")
        if let mode = modeArg.flatMap(Mode.init(rawValue:)) { model.mode = mode }
        let files = args.dropFirst()
            .filter { !$0.hasPrefix("--") && $0 != snapshot && $0 != modeArg }
            .map { URL(fileURLWithPath: $0) }
            .filter { FileManager.default.fileExists(atPath: $0.path) }
        model.open(files, compress: args.contains("--compress"))
        if let snapshot { renderWhenIdle(to: URL(fileURLWithPath: snapshot)) }
    }

    @MainActor private func renderWhenIdle(to url: URL) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
            if AppModel.shared.running { return self.renderWhenIdle(to: url) }
            // The frame view (titlebar + window background), not just the content: otherwise
            // dark-mode text is drawn on a transparent bitmap.
            // Rendered off-screen, so it needn't be visible (it isn't while the screen is locked).
            let window = NSApp.windows.filter { $0.contentView != nil }.max { $0.frame.width < $1.frame.width }
            if let view = window?.contentView?.superview,
               let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
                view.cacheDisplay(in: view.bounds, to: rep)
                try? rep.representation(using: .png, properties: [:])?.write(to: url)
            }
            NSApp.terminate(nil)
        }
    }
}

extension View {
    /// Liquid Glass on macOS 26, the regular material before that.
    @ViewBuilder func glassCard(cornerRadius: CGFloat = 18, interactive: Bool = false) -> some View {
        if #available(macOS 26.0, *) {
            glassEffect(interactive ? .regular.interactive() : .regular, in: .rect(cornerRadius: cornerRadius))
        } else {
            background(.regularMaterial, in: RoundedRectangle(cornerRadius: cornerRadius))
        }
    }
}

func formatBytes(_ n: Int64) -> String {
    ByteCountFormatter.string(fromByteCount: n, countStyle: .file)
}
