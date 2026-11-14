import AppKit
import Quartz
import WebKit

/// How AppKit hands on a keyDown the key router didn't consume, for runs where
/// the window isn't key (automation in the background, where `NSApp` would
/// drop the event): the window's key equivalents (a browser tile's ⌘L), then
/// the main menu (⌘C / ⌘V reach the first responder through the Edit menu),
/// then the first responder's `keyDown`.
@MainActor
enum KeyDelivery {
    static func deliver(_ event: NSEvent, window: NSWindow?) {
        guard let window else { return }
        let flags = event.modifierFlags.intersection([.command, .control])
        if !flags.isEmpty {
            if window.performKeyEquivalent(with: event) { return }
            if menuKeyEquivalent(event, window: window) { return }
        }
        window.sendEvent(event)
    }

    /// Runs the main-menu item bound to `event`, sending its action down the
    /// window's responder chain (with no key window, a nil-targeted action
    /// would go nowhere).
    private static func menuKeyEquivalent(_ event: NSEvent, window: NSWindow) -> Bool {
        guard let chord = KeyRouter.chord(for: event),
              let item = NSApp.mainMenu.flatMap({ find(chord, in: $0) }), item.isEnabled,
              let action = item.action
        else { return false }
        if let target = item.target {
            return NSApp.sendAction(action, to: target, from: item)
        }
        var responder: NSResponder? = window.firstResponder
        while let r = responder {
            if r.responds(to: action) { return NSApp.sendAction(action, to: r, from: item) }
            responder = r.nextResponder
        }
        return false
    }

    private static func find(_ chord: String, in menu: NSMenu) -> NSMenuItem? {
        for item in menu.items {
            if let sub = item.submenu, let found = find(chord, in: sub) { return found }
            guard !item.keyEquivalent.isEmpty else { continue }
            var parts: [String] = []
            let mask = item.keyEquivalentModifierMask
            if mask.contains(.control) { parts.append("ctrl") }
            if mask.contains(.option) { parts.append("alt") }
            if mask.contains(.command) { parts.append("cmd") }
            let key = item.keyEquivalent
            if mask.contains(.shift) || key != key.lowercased() { parts.append("shift") }
            parts.append(key.lowercased())
            if parts.joined(separator: "+") == chord { return item }
        }
        return nil
    }
}

/// Automation verbs and `expect` keys for the key and focus suite (2.11).
///
///     ime mark TEXT       an input method starts (or updates) a composition
///                         in the first responder (`setMarkedText`)
///     ime commit TEXT     …and commits it (`insertText`)
///     ime cancel          …or drops it (`unmarkText`)
///     clip set TEXT       put text on the pasteboard terminal copy/paste uses
///                         (a private one in automation runs, never the user's)
///     clip clear
///     clip has TEXT       fail unless that pasteboard holds TEXT
///     surface select-all  select everything in the focused terminal
///     responder focus     make the focused tile take first responder again
///     terminal ACTION     press an overlay button on the focused terminal
///                         tile (`takeover`, `respawn`, `retry`, `close`)
///
/// Keys: `responder` (what holds first responder: `terminal`, `terminal-ro`,
/// `terminal-tile`, `browser-page`, `browser-address`, `preview-page`,
/// `preview-native`, `files-tree`, `files-filter`, `palette`, `window`, or
/// `<kind>:<class>`), `responder.tile` (short id of the tile holding it, `-`),
/// `responder.focused` (`yes` when that tile is the focused one),
/// `key.last` (the router's verdict on the last keyDown: `action:<name>`,
/// `passed`, `composing`, `palette`, `palette-field`…), `ime.marked`,
/// `clipboard`, `palette.query`, `keywindow`, `banner.shown`,
/// `terminal.phase`, `terminal.selection`.
@MainActor
enum FocusAutomation {
    static func run(_ verb: String, _ arg: String, app: AppController?) -> String? {
        guard let app, let controller = app.focusedController else { return "no window" }
        let parts = arg.split(separator: " ", maxSplits: 1).map(String.init)
        let sub = parts.first ?? "", rest = parts.count > 1 ? parts[1] : ""
        switch (verb, sub) {
        case ("ime", "mark"), ("ime", "commit"), ("ime", "cancel"):
            guard let client = controller.window.firstResponder as? NSTextInputClient else {
                return "the first responder takes no input method text"
            }
            let none = NSRange(location: NSNotFound, length: 0)
            switch sub {
            case "mark":
                client.setMarkedText(rest, selectedRange: NSRange(location: rest.utf16.count, length: 0),
                                     replacementRange: none)
            case "commit": client.insertText(rest, replacementRange: none)
            default: client.unmarkText()
            }
        case ("clip", "set"):
            TerminalRuntime.pasteboard.clearContents()
            TerminalRuntime.pasteboard.setString(rest, forType: .string)
        case ("clip", "clear"):
            TerminalRuntime.pasteboard.clearContents()
        case ("clip", "has"):
            let text = TerminalRuntime.pasteboard.string(forType: .string) ?? ""
            if !text.contains(rest) { return "the pasteboard holds \(text.prefix(80).debugDescription)" }
        case ("surface", "select-all"):
            guard let surface = controller.window.firstResponder as? TerminalSurfaceView else {
                return "the first responder isn't a terminal"
            }
            surface.selectAll(nil)
        case ("responder", "focus"):
            controller.tiles.focusedView?.setFocused(true)
        case ("terminal", _):
            // Presses an overlay button (`terminal takeover`, `respawn`…)
            // on the focused terminal tile, as a click would.
            guard let terminal = focusedTerminal(controller) else { return "the focused tile isn't a terminal" }
            terminal.debugPress(sub)
        default:
            return "ime mark|commit|cancel, clip set|clear, surface select-all, responder focus, terminal ACTION"
        }
        return nil
    }

    static func state(app: AppController, controller: WindowController) -> [String: String] {
        let window = controller.window
        let (kind, tile) = classify(window.firstResponder, controller: controller)
        var s: [String: String] = [
            "responder": kind,
            "responder.tile": tile.map(Short.id) ?? "-",
            "key.last": app.lastKeyOutcome,
            "keywindow": NSApp.keyWindow === window ? "yes" : "no",
            "palette.query": controller.palette.isOpen ? controller.palette.field.stringValue : "-",
            "clipboard": TerminalRuntime.pasteboard.string(forType: .string) ?? "-",
            "banner.shown": controller.banner.isHidden ? "no" : "yes",
        ]
        let focused = app.session?.window(controller.windowID)?.workspace?.tab?.focused
        s["responder.focused"] = tile != nil && tile == focused ? "yes" : "no"
        s["terminal.phase"] = focusedTerminal(controller)?.phaseName ?? "-"
        if let surface = window.firstResponder as? TerminalSurfaceView {
            s["ime.marked"] = surface.hasMarkedText() ? surface.markedString : "-"
            s["terminal.selection"] = surface.hasSelection ? "yes" : "no"
        } else if let client = window.firstResponder as? NSTextInputClient, client.hasMarkedText(),
                  let marked = client.attributedSubstring(forProposedRange: client.markedRange(), actualRange: nil) {
            s["ime.marked"] = marked.string
        } else {
            s["ime.marked"] = "-"
        }
        return s
    }

    private static func focusedTerminal(_ controller: WindowController) -> TerminalTileView? {
        (controller.tiles.focusedView as? TileFrame)?.content as? TerminalTileView
    }

    /// What kind of view holds first responder, and in which tile.
    static func classify(_ responder: NSResponder?, controller: WindowController) -> (String, String?) {
        guard let responder else { return ("none", nil) }
        var view = responder as? NSView
        // A text field's field editor stands in for the field.
        if let editor = responder as? NSTextView, editor.isFieldEditor, let field = editor.delegate as? NSView {
            view = field
        }
        if responder === controller.window { return ("window", nil) }
        guard let start = view else { return ("other:\(type(of: responder))", nil) }
        if start === controller.palette.field || start.isDescendant(of: controller.palette) {
            return ("palette", nil)
        }
        var frame: TileFrame?
        var cursor: NSView? = start
        var inWeb = false, inQuickLook = false
        while let v = cursor {
            if v is WKWebView { inWeb = true }
            if v is QLPreviewView { inQuickLook = true }
            if let f = v as? TileFrame { frame = f; break }
            cursor = v.superview
        }
        guard let frame else {
            return start === controller.window.contentView ? ("window", nil) : ("chrome:\(type(of: start))", nil)
        }
        let kind = frame.kind.rawValue
        let label: String
        switch start {
        case let surface as TerminalSurfaceView: label = surface.readOnly ? "terminal-ro" : "terminal"
        case is TerminalTileView: label = "terminal-tile"
        case is NSOutlineView: label = "files-tree"
        case is NSSearchField where frame.kind == .files: label = "files-filter"
        case is NSTextField where frame.kind == .browser: label = "browser-address"
        default:
            if inWeb { label = "\(kind)-page" }
            else if inQuickLook { label = "preview-native" }
            else { label = "\(kind):\(type(of: start))" }
        }
        return (label, frame.tileID)
    }
}
