// swift-tools-version: 6.0
// SwiftPM build of the macOS shell, used when full Xcode isn't installed.
// The canonical project is project.yml (XcodeGen); both compile Sources/Azulejo.
import PackageDescription

let package = Package(
    name: "Azulejo",
    platforms: [.macOS(.v14)],
    targets: [
        .systemLibrary(name: "AzulejoCore", path: "AzulejoCore"),
        .executableTarget(
            name: "Azulejo",
            dependencies: ["AzulejoCore"],
            path: "Sources/Azulejo",
            linkerSettings: [.unsafeFlags(["-L../../target/debug"])]
        ),
    ]
)
