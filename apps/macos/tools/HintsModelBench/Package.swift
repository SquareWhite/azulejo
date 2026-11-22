// swift-tools-version: 6.0
// Smart select's on-device model spike: scores Apple's on-device language
// model (FoundationModels, macOS 26) against the rules on a fictional
// corpus. Dev-only; nothing in the app depends on it. See
// apps/macos/docs/smart-select.md, "On-device model spike".
import PackageDescription

let package = Package(
    name: "HintsModelBench",
    platforms: [.macOS("26.0")],
    targets: [
        .executableTarget(name: "HintsModelBench", path: "Sources"),
    ]
)
