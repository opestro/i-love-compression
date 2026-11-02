// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "ILoveCompression",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(name: "ILoveCompression", path: "Sources")
    ],
    swiftLanguageModes: [.v5]
)
