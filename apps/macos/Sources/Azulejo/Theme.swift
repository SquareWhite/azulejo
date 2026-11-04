import AppKit

// Themes (docs/themes.md). The tables come from shared/design/tokens.json
// (Design.Themes, generated) and from the core (`azj_core_theme`: the same
// built-ins plus the user's themes and the `[appearance]` choice).
//
// How a theme change reaches every view (one mechanism, everywhere):
// - Every `Design.Color` token is a dynamic NSColor whose provider reads
//   `Theme.current`, resolved in the appearance it's drawn in. Anything that
//   draws with an NSColor at display time (text fields, `draw(_:)`, symbol
//   images in image views, tint colours) is current as soon as it redraws.
// - A CGColor (layer backgrounds, borders) or an image baked from a colour is
//   a snapshot. Views that keep one adopt `ThemeAware`: they set those in
//   `applyTheme()`, call `Theme.track(self)` once, and route
//   `viewDidChangeEffectiveAppearance` to `Theme.reapply(self)`.
// - `Theme.set` swaps the tables, sets the app's and every window's
//   appearance (darkAqua or aqua, so system controls, menus and scrollers
//   match), calls every tracked `applyTheme()` and redraws every view, all
//   in one Core Animation transaction with actions off: one frame, no
//   flicker, no implicit fades. Then `Theme.didChange` tells the rest
//   (terminal palettes, webviews, menus).
// `scripts/check-theme-colours` (just lint) fails on a `.cgColor` outside
// an `applyTheme()` without a `// theme:` note saying how it stays current.

/// A theme colour as tokens.json writes it.
enum ThemeColor: Equatable, Sendable {
    /// 0xRRGGBBAA, sRGB.
    case rgba(UInt32)
    /// An NSColor by name (`Design.systemColor`), resolved live so it follows
    /// the user's accent colour; `alpha` replaces its own; `fallback` is how
    /// macOS draws it by default.
    case system(String, alpha: CGFloat?, fallback: UInt32)
}

enum ThemeAppearance: String, Sendable {
    case light, dark

    var nsAppearance: NSAppearance {
        NSAppearance(named: self == .dark ? .darkAqua : .aqua)!
    }
}

/// A terminal palette in Ghostty's terms (0xRRGGBBAA).
struct TerminalSpec: Equatable, Sendable {
    var background: UInt32
    var foreground: UInt32
    var cursor: UInt32?
    var cursorText: UInt32?
    var selectionBackground: UInt32?
    /// nil: selected cells keep their own colour.
    var selectionForeground: UInt32?
    var palette: [UInt32]

    /// Ghostty config lines for this palette (loaded after the user's own
    /// config when terminals use the theme's colours).
    var ghosttyConfig: String {
        func hex(_ v: UInt32) -> String { String(format: "#%06x", v >> 8) }
        var lines = ["background = \(hex(background))", "foreground = \(hex(foreground))"]
        if let cursor { lines.append("cursor-color = \(hex(cursor))") }
        if let cursorText { lines.append("cursor-text = \(hex(cursorText))") }
        if let selectionBackground { lines.append("selection-background = \(hex(selectionBackground))") }
        lines.append("selection-foreground = " + (selectionForeground.map(hex) ?? "cell-foreground"))
        for (i, c) in palette.enumerated() { lines.append("palette = \(i)=\(hex(c))") }
        return lines.joined(separator: "\n") + "\n"
    }
}

struct ThemeSpec: Equatable, Sendable {
    var id: String
    var name: String
    var appearance: ThemeAppearance
    /// Preview syntax colours (`html[data-azj-syntax]`).
    var syntax: String
    /// NSVisualEffectView material for the sidebar card, or nil: a flat card.
    var sidebarMaterial: String?
    var colors: [String: ThemeColor]
    var terminal: TerminalSpec
    var builtIn = true
    /// A user theme's file.
    var path: String?
}

/// Views (and helpers) holding colour snapshots: CGColors, baked images.
@MainActor
protocol ThemeAware: AnyObject {
    /// Sets every colour snapshot again from the tokens. Runs inside the
    /// view's appearance with Core Animation actions off.
    func applyTheme()
}

@MainActor
enum Theme {
    /// Posted after a theme change has reached every tracked view.
    static let didChange = Notification.Name("azulejo.theme.didChange")

    /// The theme in force, read by every token's provider.
    nonisolated(unsafe) private(set) static var current: ThemeSpec = Design.Themes.builtIn[0]
    /// Opaque NSColors for the current theme's fixed colours, by token.
    nonisolated(unsafe) private static var fixed: [String: NSColor] = [:]

    // MARK: Tokens

    /// A colour token: dynamic, resolved against `current` per appearance.
    nonisolated static func token(_ name: String) -> NSColor {
        NSColor(name: NSColor.Name("azulejo." + name)) { appearance in
            Theme.resolve(name, in: appearance)
        }
    }

    nonisolated static func resolve(_ name: String, in appearance: NSAppearance) -> NSColor {
        switch current.colors[name] {
        case .rgba(let v)?:
            if let cached = fixed[name] { return cached }
            let color = rgba(v)
            fixed[name] = color
            return color
        case .system(let system, let alpha, let fallback)?:
            guard let base = Design.systemColor(system) else { return rgba(fallback) }
            var resolved = base
            appearance.performAsCurrentDrawingAppearance {
                resolved = base.usingColorSpace(.sRGB) ?? base
            }
            return alpha.map { resolved.withAlphaComponent($0) } ?? resolved
        case nil:
            return .magenta
        }
    }

    nonisolated static func rgba(_ v: UInt32) -> NSColor {
        NSColor(srgbRed: CGFloat((v >> 24) & 0xff) / 255, green: CGFloat((v >> 16) & 0xff) / 255,
                blue: CGFloat((v >> 8) & 0xff) / 255, alpha: CGFloat(v & 0xff) / 255)
    }

    /// A token resolved now, in the current theme's appearance, as sRGB.
    nonisolated static func resolved(_ color: NSColor) -> NSColor {
        var out = color
        current.appearance.nsAppearance.performAsCurrentDrawingAppearance {
            out = color.usingColorSpace(.sRGB) ?? color
        }
        return out
    }

    /// `#rrggbb` or `rgba(…)` for CSS.
    nonisolated static func css(_ color: NSColor) -> String {
        let c = resolved(color)
        let r = Int((c.redComponent * 255).rounded()), g = Int((c.greenComponent * 255).rounded())
        let b = Int((c.blueComponent * 255).rounded())
        if c.alphaComponent >= 0.999 { return String(format: "#%02x%02x%02x", r, g, b) }
        return String(format: "rgba(%d, %d, %d, %.3f)", r, g, b, c.alphaComponent)
    }

    // MARK: Views

    private static let tracked = NSHashTable<AnyObject>.weakObjects()

    /// Registers a view (or helper) for theme changes and applies the theme
    /// to it now.
    static func track(_ object: ThemeAware) {
        tracked.add(object)
        reapply(object)
    }

    /// `applyTheme()` in the object's appearance, without implicit animations.
    static func reapply(_ object: ThemeAware) {
        let appearance = (object as? NSView)?.effectiveAppearance ?? current.appearance.nsAppearance
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        appearance.performAsCurrentDrawingAppearance { object.applyTheme() }
        CATransaction.commit()
    }

    /// Automation: how many objects are tracked.
    static var trackedCount: Int { tracked.allObjects.count }

    // MARK: Choosing the theme

    /// What `azj_core_theme` said: the themes and the `[appearance]` choice.
    struct Choice: Equatable {
        var theme = Design.Themes.defaultID
        var light = "atom-one-light"
        var dark = "atom-one-dark"
        /// Terminals take the theme's palette (else the Ghostty config's).
        var terminalsUseTheme = true
        var auto: Bool { theme == "auto" }
    }

    private(set) static var themes: [ThemeSpec] = Design.Themes.builtIn
    private(set) static var choice = Choice()
    /// ⌘K's live preview: this theme until it's chosen or cancelled.
    private(set) static var previewID: String?
    /// Automation: the system appearance `auto` follows (never the real one's
    /// setting; the app's own idea of it).
    static var systemOverride: ThemeAppearance? {
        didSet { if systemOverride != oldValue { refresh() } }
    }
    /// The terminal palette state last applied (nil: Ghostty's own colours).
    private(set) static var terminal: TerminalSpec?
    static var log: (String) -> Void = { _ in }
    private static var observingSystem = false

    /// The system's light or dark appearance (or the automation override).
    static var systemAppearance: ThemeAppearance {
        if let systemOverride { return systemOverride }
        return UserDefaults.standard.string(forKey: "AppleInterfaceStyle") == "Dark" ? .dark : .light
    }

    static func theme(_ id: String) -> ThemeSpec? { themes.first { $0.id == id } }

    /// The theme the choice (and any preview) asks for now.
    static var wanted: ThemeSpec {
        let auto = previewID == "auto" || (previewID == nil && choice.auto)
        let id = auto ? (systemAppearance == .dark ? choice.dark : choice.light) : (previewID ?? choice.theme)
        return theme(id) ?? theme(Design.Themes.defaultID) ?? Design.Themes.builtIn[0]
    }

    /// Reads `azj_core_theme`'s JSON (launch and every `config.reloaded`) and
    /// applies the result.
    static func load(_ json: [String: Any]) {
        var list: [ThemeSpec] = []
        for item in json["themes"] as? [[String: Any]] ?? [] {
            if let spec = spec(from: item) { list.append(spec) }
        }
        if !list.isEmpty { themes = list }
        var c = Choice()
        if let v = json["theme"] as? String { c.theme = v }
        if let v = json["theme_light"] as? String { c.light = v }
        if let v = json["theme_dark"] as? String { c.dark = v }
        c.terminalsUseTheme = (json["terminal_colors"] as? String ?? "theme") == "theme"
        choice = c
        if !observingSystem { observeSystemAppearance() }
        refresh()
    }

    /// ⌘K's live preview: shows `id` now; nil goes back to the choice.
    static func preview(_ id: String?) {
        previewID = id
        refresh()
    }

    /// Applies `wanted` if it differs from what's showing.
    static func refresh() {
        set(wanted, terminalsUseTheme: choice.terminalsUseTheme)
    }

    private static var applied = false

    /// Makes `spec` the current theme everywhere.
    /// `force`: apply even when nothing in the theme changed (the system's
    /// colours did: the accent colour, or the system appearance).
    static func set(_ spec: ThemeSpec, terminalsUseTheme: Bool, force: Bool = false) {
        let palette = terminalsUseTheme ? spec.terminal : nil
        if applied, !force, spec == current, palette == terminal { return }
        let start = CACurrentMediaTime()
        applied = true
        current = spec
        fixed = [:]
        terminal = palette
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let appearance = spec.appearance.nsAppearance
        NSApp.appearance = appearance
        for window in NSApp.windows {
            window.appearance = appearance
        }
        // Terminals first: views read their background.
        TerminalRuntime.shared?.applyTheme(palette: palette, dark: spec.appearance == .dark, log: log)
        for object in tracked.allObjects {
            if let object = object as? ThemeAware { reapply(object) }
        }
        for window in NSApp.windows {
            window.invalidateShadow()
            if let frame = window.contentView?.superview { redraw(frame) }
        }
        CATransaction.commit()
        NotificationCenter.default.post(name: didChange, object: nil)
        log(String(format: "theme: %@ (%@, terminals %@) in %.1f ms", spec.id, spec.appearance.rawValue,
                   palette == nil ? "ghostty" : "theme", (CACurrentMediaTime() - start) * 1000))
    }

    /// Marks a view and everything in it for redrawing.
    private static func redraw(_ view: NSView) {
        view.needsDisplay = true
        for sub in view.subviews { redraw(sub) }
    }

    private static func observeSystemAppearance() {
        observingSystem = true
        DistributedNotificationCenter.default().addObserver(
            forName: Notification.Name("AppleInterfaceThemeChangedNotification"), object: nil, queue: .main
        ) { _ in
            MainActor.assumeIsolated { if Theme.choice.auto, Theme.systemOverride == nil { Theme.refresh() } }
        }
        // The accent colour (or another system colour) changed: system-colour
        // tokens resolve to new values, so every snapshot is taken again.
        NotificationCenter.default.addObserver(
            forName: NSColor.systemColorsDidChangeNotification, object: nil, queue: .main
        ) { _ in
            MainActor.assumeIsolated {
                let uses = Theme.current.colors.values.contains {
                    if case .system = $0 { return true } else { return false }
                }
                if uses { Theme.set(Theme.wanted, terminalsUseTheme: Theme.choice.terminalsUseTheme, force: true) }
            }
        }
    }

    // MARK: Parsing the core's JSON

    private static func spec(from json: [String: Any]) -> ThemeSpec? {
        guard let id = json["id"] as? String, let name = json["name"] as? String,
              let appearance = (json["appearance"] as? String).flatMap(ThemeAppearance.init(rawValue:)),
              let colorsJSON = json["colors"] as? [String: Any],
              let term = json["terminal"] as? [String: Any]
        else { return nil }
        var colors: [String: ThemeColor] = [:]
        for (token, value) in colorsJSON {
            if let hex = value as? String, let v = parseHex(hex) {
                colors[token] = .rgba(v)
            } else if let object = value as? [String: Any], let system = object["system"] as? String,
                      let fallback = (object["hex"] as? String).flatMap(parseHex) {
                colors[token] = .system(system, alpha: (object["alpha"] as? Double).map { CGFloat($0) },
                                        fallback: fallback)
            }
        }
        // A token the core didn't send keeps the built-in default's value.
        let base = Design.Themes.builtIn.first { $0.id == id }
            ?? Design.Themes.builtIn.first { $0.appearance == appearance }!
        for (token, value) in base.colors where colors[token] == nil { colors[token] = value }
        let color = { (key: String) in (term[key] as? String).flatMap(parseHex) }
        let palette = (term["palette"] as? [String] ?? []).compactMap(parseHex)
        return ThemeSpec(
            id: id, name: name, appearance: appearance,
            syntax: json["syntax"] as? String ?? base.syntax,
            sidebarMaterial: json["sidebar_material"] as? String,
            colors: colors,
            terminal: TerminalSpec(
                background: color("background") ?? base.terminal.background,
                foreground: color("foreground") ?? base.terminal.foreground,
                cursor: color("cursor"), cursorText: color("cursor_text"),
                selectionBackground: color("selection_background"),
                selectionForeground: color("selection_foreground"),
                palette: palette.count == 16 ? palette : base.terminal.palette),
            builtIn: json["builtin"] as? Bool ?? false,
            path: json["path"] as? String)
    }

    /// `#rrggbb` or `#rrggbbaa` → 0xRRGGBBAA.
    static func parseHex(_ text: String) -> UInt32? {
        guard text.hasPrefix("#") else { return nil }
        let digits = text.dropFirst()
        guard let v = UInt32(digits, radix: 16) else { return nil }
        switch digits.count {
        case 6: return v << 8 | 0xff
        case 8: return v
        default: return nil
        }
    }
}

// MARK: - Webviews

extension Theme {
    /// A script that sets the theme's CSS variables and syntax on a page
    /// (`shared/webview/preview.css` reads `--azj-*`; the core's syntax CSS
    /// keys on `data-azj-syntax`).
    static var webviewScript: String {
        var vars: [String] = []
        for (name, token) in Design.Webview.variables {
            let color = Theme.token(token)
            vars.append("\"--azj-\(name)\":\"\(css(color))\"")
        }
        // Inline code sits on no fill, as in the mockup.
        vars.append("\"--azj-code-bg\":\"transparent\"")
        let scheme = current.appearance.rawValue
        return """
        (function(){var r=document.documentElement;if(!r)return;\
        r.setAttribute("data-azj-syntax","\(current.syntax)");\
        r.setAttribute("data-azj-theme","\(current.id)");\
        r.style.colorScheme="\(scheme)";\
        var v={\(vars.joined(separator: ","))};\
        for(var k in v){r.style.setProperty(k,v[k]);}})();
        """
    }
}
