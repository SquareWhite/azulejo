import AppKit

/// Picks the Dock icon while the app runs: `[appearance] app_icon` in the
/// config, "light" (the default), "dark" or "auto" (docs/themes.md).
///
/// The bundle's `AppIcon.icns` is the light look: Finder, Launchpad,
/// Spotlight and the Dock before launch always show it, whatever the
/// setting. An appearance-aware icon needs an `Assets.car` compiled by
/// Xcode's `actool`, and macOS ignores a dark variant nested in an `.icns`
/// for an app (docs/design/icon/README.md). So for the dark icon the app
/// hands the Dock `AppIconDark.icns`, and for the light one it hands back the
/// bundle's own icon. "auto" follows the system's light or dark appearance
/// (`Theme.systemAppearance`), not the theme's: the Dock is the system's.
///
/// Both `.icns` files share one canvas and macOS 26's corner (radius 214 on
/// an 824 px body in 1024), so the Dock shows the same size and outline
/// either way.
@MainActor
enum DockIcon {
    enum Mode: String {
        case light, dark, auto
    }

    private static var observer: NSObjectProtocol?
    private static var log: (String) -> Void = { _ in }
    private static let darkImage: NSImage? = Bundle.main.url(forResource: "AppIconDark", withExtension: "icns")
        .flatMap(NSImage.init(contentsOf:))
    /// Whether the Dock has the dark icon now.
    private(set) static var showingDark = false
    /// The config's choice. Light until the config says otherwise, so the
    /// Dock never flashes the dark icon at launch.
    static var mode: Mode = .light {
        didSet {
            guard mode != oldValue else { return }
            log("dock icon: app_icon = \(mode.rawValue)")
            update()
        }
    }

    static func install(log: @escaping (String) -> Void) {
        self.log = log
        if darkImage == nil { log("dock icon: no AppIconDark.icns in the bundle; the Dock keeps the light icon") }
        // The system's light/dark switch. (`NSApp.effectiveAppearance` follows
        // the theme, which sets `NSApp.appearance`, so it can't tell us.)
        // Automation's `theme system light|dark|real` calls `update()` too.
        observer = DistributedNotificationCenter.default().addObserver(
            forName: Notification.Name("AppleInterfaceThemeChangedNotification"), object: nil, queue: .main
        ) { _ in
            MainActor.assumeIsolated { update() }
        }
        update()
    }

    static func update() {
        let dark: Bool
        switch mode {
        case .light: dark = false
        case .dark: dark = true
        case .auto: dark = Theme.systemAppearance == .dark
        }
        guard dark != showingDark, let darkImage else { return }
        showingDark = dark
        // nil restores the bundle's icon, exactly as the Dock drew it at launch.
        NSApp.applicationIconImage = dark ? darkImage : nil
        log("dock icon: \(dark ? "dark" : "light")")
    }
}
