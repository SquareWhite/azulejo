import AppKit

/// The single place keyDowns are decided. A local event monitor hands every
/// keyDown here before AppKit dispatches it; chords the keymap binds are
/// consumed and run, everything else goes on to the focused tile (and to menu
/// key equivalents such as ⌘C or ⌘Q).
///
/// Resolution happens in the core (`azj_core_key`), so the user's
/// `config.toml` overrides, contexts (`cmd+a` toggles the sidebar only in
/// terminals) and prefix mode behave the same as they will on Linux. Nothing
/// here matches a chord itself: the context decides which keys mean what.
///
/// Contexts, most specific first: resize mode (`resize`, only its own keys),
/// quick open (`palette`), then the `KeyScope` around the first responder (a
/// find bar or the files filter: `find_bar`; the files list: `files`; a
/// tile: its kind), else the focused tile's kind. Actions that only work in
/// one context go to that scope, which may decline them (the chord then goes
/// on as typed); everything else runs through `AppController.perform`.
@MainActor
final class KeyRouter {
    private weak var app: AppController?
    private var monitor: Any?
    /// Sticky resize mode: `[keys.*.resize]` bindings until resize_mode_exit.
    private(set) var resizing = false
    /// The last keyDown the router looked at and let through (by timestamp).
    /// Fields that fall back on AppKit's enter/escape commands check it, so
    /// unbinding `find_close` really frees escape there.
    private static var lastPassed: TimeInterval?
    /// keyDowns the monitor has seen (automation checks it reached us).
    private(set) var seen = 0
    /// Automation keys handed to the router directly because the monitor
    /// hadn't seen them yet (by event timestamp). If the queued event turns
    /// up after all — the main thread was busy — it's dropped, not typed twice.
    private var routedDirectly: Set<TimeInterval> = []
    /// What the last keyDown came to (`action:split_right`, `passed`,
    /// `composing`, `palette`…); automation's `key.last`.
    private(set) var lastOutcome = "none"
    /// The keymap context the last keyDown resolved in (`key.context`).
    private(set) var lastContext = "-"

    init(app: AppController) {
        self.app = app
    }

    func install() {
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self else { return event }
            if self.routedDirectly.remove(event.timestamp) != nil { return nil }
            self.seen += 1
            return Perf.measure("key") { self.handle(event) } ? nil : event
        }
    }

    /// Automation: `event` is being routed by hand; drop it if it arrives later.
    func markRoutedDirectly(_ event: NSEvent) {
        routedDirectly.insert(event.timestamp)
    }

    /// Returns true when the event was consumed.
    @discardableResult
    func handle(_ event: NSEvent) -> Bool {
        guard let app else { return false }
        // Sheets and panels (rename, alerts) get their keys untouched.
        if let window = event.window, !app.owns(window) { return false }
        guard let controller = app.controller(for: event.window) else { return false }
        // Escape mid-way through dragging a tab puts it back; nothing else
        // sees that key.
        if event.keyCode == 53, controller.tabStrip.cancelDrag() { // escape
            lastOutcome = "tab-drag-cancel"
            return true
        }
        guard let chord = Self.chord(for: event) else {
            lastOutcome = "unnamed"
            return false
        }

        // An input method is composing (marked text in a terminal, the ⌘K
        // field or an address bar): every key belongs to it until it commits,
        // so it goes straight to the text view, past app chords and menu key
        // equivalents (⌘K, ctrl+cmd+t…).
        if let responder = (event.window ?? controller.window).firstResponder,
           let client = responder as? NSTextInputClient, client.hasMarkedText() {
            lastOutcome = "composing"
            responder.keyDown(with: event)
            return true
        }

        if resizing {
            lastOutcome = "resize"
            lastContext = "resize"
            return resizeMode(chord, in: controller)
        }

        // The ⌘K menu: its own keys (`palette`), and its toggles (⌘K, and ⌘⇧P
        // / ⌘P if bound). Everything else is typing in its search field.
        if controller.palette.isOpen {
            lastContext = "palette"
            let outcome = app.core.key(chord: chord, context: "palette")
            if outcome["outcome"] as? String == "action", let name = outcome["action"] as? String {
                if outcome["context"] as? String == "palette" {
                    lastOutcome = "palette"
                    controller.palette.perform(keyAction: name)
                    return true
                }
                if PaletteStore.selfActions.contains(name) {
                    lastOutcome = "action:\(name)"
                    app.perform(outcome, in: controller)
                    return true
                }
            }
            app.core.resetKeys()
            lastOutcome = "palette-field"
            return passOn(event)
        }

        // A find bar's field, the files list or filter, or a tile: the
        // nearest scope picks the context. While typing in a find bar other
        // chords resolve without the tile's context, so ⌘A selects the query.
        let scope = KeyScopes.around(event.window ?? controller.window)
        let context = scope?.keyScope ?? controller.keyContext
        lastContext = context ?? "global"
        let outcome = app.core.key(chord: chord, context: context)
        app.log("key \(chord) [\(context ?? "-")] → \(Self.describe(outcome))")
        switch outcome["outcome"] as? String {
        case "action":
            let name = outcome["action"] as? String ?? ""
            if outcome["context"] is String {
                // Only works in its context: the scope runs it, or declines
                // and the chord goes on as typed.
                let handler = scope ?? KeyScopes.focusedTile(in: controller)
                // `send_text` carries its text; a terminal types it.
                let done = if let text = outcome["text"] as? String {
                    (handler as? TerminalTileView)?.type(text: text) == true
                } else {
                    handler?.perform(keyAction: name, event: event) == true
                }
                guard done else {
                    lastOutcome = "declined:\(name)"
                    return passOn(event)
                }
                lastOutcome = context == "find_bar" ? "find" : "action:\(Self.describe(outcome))"
                controller.setStatus(nil)
                return true
            }
            lastOutcome = "action:\(Self.describe(outcome))"
            controller.setStatus(nil)
            app.perform(outcome, in: controller)
            return true
        case "prefix_armed":
            lastOutcome = "prefix_armed"
            controller.setStatus("prefix")
            return true
        case "prefix_cancelled":
            lastOutcome = "prefix_cancelled"
            controller.setStatus(nil)
            return true
        default:
            lastOutcome = "passed"
            return passOn(event)
        }
    }

    /// Lets `event` through, noting that the keymap had nothing for it.
    private func passOn(_ event: NSEvent) -> Bool {
        Self.lastPassed = event.timestamp
        return false
    }

    /// Whether the router saw the keyDown being handled now and let it
    /// through: a field's AppKit fallback for enter or escape must then leave
    /// it alone (the user unbound that key there).
    static func letThroughCurrentKey() -> Bool {
        guard let event = KeyDelivery.delivering ?? NSApp.currentEvent, event.type == .keyDown else { return false }
        return lastPassed == event.timestamp
    }

    func enterResizeMode(in controller: WindowController) {
        resizing = true
        controller.setStatus(Self.resizeHint(app?.core.keymap() ?? [:]))
    }

    /// "resize: h j k l, escape to finish", from the `resize` bindings.
    static func resizeHint(_ keymap: [String: Any]) -> String {
        let list = (keymap["contexts"] as? [String: Any])?["resize"] as? [[String: Any]] ?? []
        func chords(_ action: String) -> [String] {
            list.filter { $0["action"] as? String == action }.compactMap { $0["chord"] as? String }
        }
        let moves = ["left", "down", "up", "right"].compactMap { chords("resize_tile_\($0)").first }
        let exits = chords("resize_mode_exit")
        let done = exits.contains("escape") ? "escape" : exits.first
        var hint = "resize"
        if !moves.isEmpty { hint += ": " + moves.joined(separator: " ") }
        if let done { hint += ", \(done) to finish" }
        return hint
    }

    /// Resize mode: only `[keys.*.resize]` bindings run; everything else is
    /// swallowed while the mode is on, like herdr's.
    private func resizeMode(_ chord: String, in controller: WindowController) -> Bool {
        guard let app else { return true }
        let outcome = app.core.key(chord: chord, context: "resize")
        guard outcome["outcome"] as? String == "action", let name = outcome["action"] as? String else {
            app.core.resetKeys()
            return true
        }
        if name == "resize_mode_exit" {
            resizing = false
            controller.setStatus(nil)
        } else {
            app.perform(outcome, in: controller)
        }
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
