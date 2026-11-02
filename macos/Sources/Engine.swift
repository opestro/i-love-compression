import Foundation

/// One JSON line from ilc-engine (protocol in engine/src/main.rs).
struct EngineEvent: Decodable {
    let event: String
    var progress: Double?
    var index: Int?
    var output: String?
    var before: Int64?
    var after: Int64?
    var note: String?
    var message: String?
}

/// One run of ilc-engine, which ships next to the app's executable.
/// Closing its stdin cancels it; the OS does the same if this app quits or crashes.
final class EngineRun {
    private let process = Process()
    private let stdin = Pipe()
    private var buffer = Data()
    private var stdoutClosed = false
    private var exited = false
    private let onExit: () -> Void

    static var executable: URL {
        if let path = ProcessInfo.processInfo.environment["ILC_ENGINE"] { return URL(fileURLWithPath: path) }
        return Bundle.main.url(forAuxiliaryExecutable: "ilc-engine") ?? URL(fileURLWithPath: "ilc-engine")
    }

    init(arguments: [String], onEvent: @escaping (EngineEvent) -> Void, onExit: @escaping () -> Void) {
        self.onExit = onExit
        process.executableURL = Self.executable
        process.arguments = arguments
        process.standardInput = stdin
        process.standardError = FileHandle.nullDevice
        let stdout = Pipe()
        process.standardOutput = stdout
        stdout.fileHandleForReading.readabilityHandler = { [weak self] handle in
            guard let self else { return }
            let data = handle.availableData
            if data.isEmpty {
                handle.readabilityHandler = nil
                DispatchQueue.main.async { self.stdoutClosed = true; self.finishIfDone() }
                return
            }
            buffer.append(data)
            while let newline = buffer.firstIndex(of: 0x0A) {
                let line = buffer[buffer.startIndex..<newline]
                buffer.removeSubrange(buffer.startIndex...newline)
                if let event = try? JSONDecoder().decode(EngineEvent.self, from: line) {
                    DispatchQueue.main.async { onEvent(event) }
                }
            }
        }
        // Exit is reported only after every event line has been delivered.
        process.terminationHandler = { [weak self] _ in
            DispatchQueue.main.async { self?.exited = true; self?.finishIfDone() }
        }
    }

    func start() throws { try process.run() }

    func stop() { try? stdin.fileHandleForWriting.close() }

    private func finishIfDone() {
        if stdoutClosed && exited { onExit() }
    }
}
