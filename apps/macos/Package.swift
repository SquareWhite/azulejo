// swift-tools-version: 6.0
// SwiftPM build of the macOS shell, used when full Xcode isn't installed.
// The canonical project is project.yml (XcodeGen); both compile Sources/Azulejo.
import PackageDescription

let ghosttyLib = "../../vendor/ghostty/macos/GhosttyKit.xcframework/macos-arm64"
// Where libazulejo_ffi.a lives: the debug build by default; the release
// script (scripts/release/build-mac.sh) points it at target/release.
let coreLib = Context.environment["AZJ_CORE_LIB_DIR"] ?? "../../target/debug"

let package = Package(
    name: "Azulejo",
    platforms: [.macOS(.v14)],
    targets: [
        .systemLibrary(name: "AzulejoCore", path: "AzulejoCore"),
        .systemLibrary(name: "GhosttyKit", path: "GhosttyKit"),
        .executableTarget(
            name: "Azulejo",
            dependencies: ["AzulejoCore", "GhosttyKit"],
            path: "Sources/Azulejo",
            linkerSettings: [
                .unsafeFlags(["-L\(coreLib)", "-L\(ghosttyLib)"]),
                .linkedFramework("AppKit"),
                .linkedFramework("Metal"),
                .linkedFramework("MetalKit"),
                .linkedFramework("QuartzCore"),
                .linkedFramework("CoreText"),
                .linkedFramework("CoreGraphics"),
                .linkedFramework("CoreVideo"),
                .linkedFramework("IOSurface"),
                .linkedFramework("Carbon"),
                .linkedFramework("UniformTypeIdentifiers"),
                .linkedFramework("UserNotifications"),
            ]
        ),
        // Decodes `herdr terminal session observe` frames for read-only tiles;
        // bundled as Contents/MacOS/azj-observe.
        .executableTarget(name: "AzjObserve", path: "Sources/AzjObserve"),
    ]
)
