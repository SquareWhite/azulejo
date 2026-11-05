import AppKit

/// Keeps the Dock icon in step with dark mode while the app runs.
///
/// The bundle's `AppIcon.icns` is the light look: Finder, Launchpad,
/// Spotlight and the Dock before launch all show it. An appearance-aware
/// icon needs an `Assets.car` compiled by Xcode's `actool`, and macOS
/// ignores a dark variant nested in an `.icns` for an app
/// (docs/design/icon/README.md). So in dark mode the app hands the Dock
/// `AppIconDark.icns`, and in light mode it hands back the bundle's own icon.
///
/// Both `.icns` files share one canvas and macOS 26's corner (radius 214 on
/// an 824 px body in 1024), so the Dock shows the same size and outline
/// either way.
@MainActor
enum DockIcon {
    private static var observation: NSKeyValueObservation?
    private static var log: (String) -> Void = { _ in }
    private static let darkImage: NSImage? = Bundle.main.url(forResource: "AppIconDark", withExtension: "icns")
        .flatMap(NSImage.init(contentsOf:))
    /// Whether the Dock has the dark icon now.
    private(set) static var showingDark = false

    static func install(log: @escaping (String) -> Void) {
        self.log = log
        if darkImage == nil { log("dock icon: no AppIconDark.icns in the bundle; the Dock keeps the light icon") }
        // `effectiveAppearance` is key-value observable on NSApplication: it
        // changes with System Settings' appearance and with `NSApp.appearance`.
        observation = NSApp.observe(\.effectiveAppearance, options: [.initial, .new]) { _, _ in
            MainActor.assumeIsolated { update() }
        }
    }

    static func update() {
        let dark = NSApp.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
        guard dark != showingDark, let darkImage else { return }
        showingDark = dark
        // nil restores the bundle's icon, exactly as the Dock drew it at launch.
        NSApp.applicationIconImage = dark ? darkImage : nil
        log("dock icon: \(dark ? "dark" : "light")")
    }
}
