import AppKit

/// The single place keyDowns are decided. A local event monitor hands every
/// keyDown here before AppKit dispatches it; chords the keymap binds are
/// consumed and run, everything else goes on to the focused tile (and to menu
/// key equivalents such as ⌘C or ⌘Q).
///
/// Resolution happens in the core (`azj_core_key`), so the user's
/// `config.toml` overrides, per-tile contexts (`cmd+a` toggles the sidebar
/// only in terminals) and prefix mode behave the same as they will on Linux.
@MainActor
final class KeyRouter {
    private weak var app: AppController?
    private var monitor: Any?
    /// Sticky resize mode: hjkl / arrows resize until escape or enter.
    private(set) var resizing = false
    /// keyDowns the monitor has seen (automation checks it reached us).
    private(set) var seen = 0

    init(app: AppController) {
        self.app = app
    }

    func install() {
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self else { return event }
            self.seen += 1
            return self.handle(event) ? nil : event
        }
    }

    /// Returns true when the event was consumed.
    @discardableResult
    func handle(_ event: NSEvent) -> Bool {
        guard let app else { return false }
        // Sheets and panels (rename, alerts) get their keys untouched.
        if let window = event.window, !app.owns(window) { return false }
        guard let controller = app.controller(for: event.window) else { return false }
        guard let chord = Self.chord(for: event) else { return false }

        if resizing {
            return resizeMode(chord, in: controller)
        }

        let outcome = app.core.key(chord: chord, tile: controller.keyContext)
        app.log("key \(chord) → \(Self.describe(outcome))")
        switch outcome["outcome"] as? String {
        case "action":
            controller.setStatus(nil)
            app.perform(outcome, in: controller)
            return true
        case "prefix_armed":
            controller.setStatus("prefix")
            return true
        case "prefix_cancelled":
            controller.setStatus(nil)
            return true
        default:
            return false
        }
    }

    func enterResizeMode(in controller: WindowController) {
        resizing = true
        controller.setStatus("resize: h j k l, esc to finish")
    }

    private func resizeMode(_ chord: String, in controller: WindowController) -> Bool {
        let direction: String? = switch chord {
        case "h", "left": "left"
        case "j", "down": "down"
        case "k", "up": "up"
        case "l", "right": "right"
        default: nil
        }
        if let direction {
            app?.send(["method": "layout.resize", "params": ["direction": direction]])
        } else if chord == "escape" || chord == "enter" {
            resizing = false
            controller.setStatus(nil)
        }
        // Everything else is swallowed while the mode is on, like herdr's.
        return true
    }

    func reset() {
        resizing = false
        app?.core.resetKeys()
    }

    private static func describe(_ outcome: [String: Any]) -> String {
        let kind = outcome["outcome"] as? String ?? "?"
        guard kind == "action" else { return kind }
        let name = outcome["action"] as? String ?? "?"
        if let index = outcome["index"] as? Int { return "\(name)(\(index))" }
        return name
    }

    // MARK: Chords

    /// The chord in the keymap's syntax (`"ctrl+alt+shift+h"`), or nil for keys
    /// the keymap can't name. Uses the physical key (US layout names), so
    /// Option-modified letters (`alt+h` types `˙`) still match.
    static func chord(for event: NSEvent) -> String? {
        guard let key = keyNames[event.keyCode] else { return nil }
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        var parts: [String] = []
        if flags.contains(.control) { parts.append("ctrl") }
        if flags.contains(.option) { parts.append("alt") }
        if flags.contains(.command) { parts.append("cmd") }
        if flags.contains(.shift) { parts.append("shift") }
        parts.append(key)
        return parts.joined(separator: "+")
    }

    /// macOS virtual key codes (ANSI layout) → keymap key names.
    static let keyNames: [UInt16: String] = {
        var names: [UInt16: String] = [
            0: "a", 1: "s", 2: "d", 3: "f", 4: "h", 5: "g", 6: "z", 7: "x", 8: "c", 9: "v",
            11: "b", 12: "q", 13: "w", 14: "e", 15: "r", 16: "y", 17: "t",
            18: "1", 19: "2", 20: "3", 21: "4", 22: "6", 23: "5", 24: "equal", 25: "9",
            26: "7", 27: "minus", 28: "8", 29: "0", 30: "bracketright", 31: "o", 32: "u",
            33: "bracketleft", 34: "i", 35: "p", 36: "enter", 37: "l", 38: "j", 39: "quote",
            40: "k", 41: "semicolon", 42: "backslash", 43: "comma", 44: "slash", 45: "n",
            46: "m", 47: "period", 48: "tab", 49: "space", 50: "grave", 51: "backspace",
            53: "escape", 76: "enter", 115: "home", 116: "pageup", 117: "delete",
            119: "end", 121: "pagedown", 123: "left", 124: "right", 125: "down", 126: "up",
        ]
        let fKeys: [UInt16] = [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111]
        for (i, code) in fKeys.enumerated() { names[code] = "f\(i + 1)" }
        return names
    }()

    /// The reverse, for building synthetic events (automation) and menu key
    /// equivalents.
    static let keyCodes: [String: UInt16] = {
        var codes: [String: UInt16] = [:]
        for (code, name) in keyNames where codes[name] == nil || code != 76 { codes[name] = code }
        return codes
    }()
}
