import AppKit

/// In-app mouse input on the focused terminal, for selection and link
/// checks. No OS-level events: each step is an NSEvent built here and handed
/// to whatever view the window's hit-testing finds at that point (so an
/// overlay that swallows clicks fails the check), as AppKit would deliver it.
/// Cells are `column,row`, zero-based, in the visible grid.
///
///     mouse click C,R [N] [MODS]   N clicks in a row (2: a word, 3: a line)
///     mouse drag C,R C,R [MODS]    press at one cell, drag to the other, release
///     mouse hover C,R [MODS]       the modifiers change, then the pointer moves there
///     mouse mods [MODS]            only the modifiers change (none without MODS)
///
/// MODS: `cmd`, `shift`, `alt`, `ctrl`, joined with `+`.
///
/// Keys: `mouse.target` (what the last event reached: `terminal`, or the
/// view's class), `terminal.selection.text` (newlines as `\n`, trailing
/// spaces trimmed as ⌘C trims them, `-` for none), `terminal.cursor` (`text`, `pointer`, `arrow`), `terminal.link`
/// (the link under the pointer, `-`), `terminal.opened` (the last link a
/// click asked to open; automation records it instead of opening it),
/// `terminal.refused` (the last link refused), `terminal.mouse` (`local`
/// while a selection gesture has libghostty's mouse reporting off, else
/// `reported`), `terminal.mouse.owner` (`shell`: plain drags select;
/// `program`: plain clicks go to the program in the pane; `select`: a
/// program on `[terminal] select_on_drag`, plain drags select and clicks go
/// to it).
@MainActor
enum MouseAutomation {
    private static var target = "-"

    static func run(_ arg: String, app: AppController?) -> String? {
        guard let controller = app?.focusedController else { return "no window" }
        let window: NSWindow = controller.window
        guard let surface = surface(controller) else { return "the focused tile has no live terminal" }
        var tokens = arg.split(separator: " ").map(String.init)
        guard !tokens.isEmpty else { return usage }
        let verb = tokens.removeFirst()
        var cells: [(Int, Int)] = []
        var count = 1
        var flags: NSEvent.ModifierFlags = []
        for token in tokens {
            if token.contains(",") {
                let xy = token.split(separator: ",").compactMap { Int($0) }
                guard xy.count == 2 else { return "bad cell \(token)" }
                cells.append((xy[0], xy[1]))
            } else if let n = Int(token) {
                count = n
            } else {
                for part in token.split(separator: "+") {
                    switch part {
                    case "cmd": flags.insert(.command)
                    case "shift": flags.insert(.shift)
                    case "alt": flags.insert(.option)
                    case "ctrl": flags.insert(.control)
                    case "none": break
                    default: return "bad modifier \(part)"
                    }
                }
            }
        }
        var points: [CGPoint] = []
        for (column, row) in cells {
            guard let p = surface.automationPoint(column: column, row: row) else {
                return "cell \(column),\(row) is outside the grid"
            }
            points.append(surface.convert(p, to: nil))
        }
        switch (verb, points.count) {
        case ("click", 1):
            for n in 1...max(1, count) {
                send(.leftMouseDown, at: points[0], flags: flags, clicks: n, window: window)
                send(.leftMouseUp, at: points[0], flags: flags, clicks: n, window: window)
            }
        case ("drag", 2):
            let (a, b) = (points[0], points[1])
            send(.leftMouseDown, at: a, flags: flags, clicks: 1, window: window)
            for i in 1...6 {
                let t = CGFloat(i) / 6
                send(.leftMouseDragged, at: CGPoint(x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t),
                     flags: flags, clicks: 1, window: window)
            }
            send(.leftMouseUp, at: b, flags: flags, clicks: 1, window: window)
        case ("hover", 1):
            modifiers(flags, window: window)
            send(.mouseMoved, at: points[0], flags: flags, clicks: 0, window: window)
        case ("mods", 0):
            modifiers(flags, window: window)
        default:
            return usage
        }
        return nil
    }

    static func state(_ controller: WindowController) -> [String: String] {
        var s = ["mouse.target": target]
        let surface = surface(controller)
        // herdr paints blank cells as spaces; ⌘C trims them, so does this.
        s["terminal.selection.text"] = surface?.selectionText.map { text in
            text.split(separator: "\n", omittingEmptySubsequences: false)
                .map { String($0.reversed().drop { $0 == " " }.reversed()) }
                .joined(separator: "\\n")
        } ?? "-"
        s["terminal.cursor"] = surface?.cursorName ?? "-"
        s["terminal.mouse"] = surface.map { $0.reportingOff ? "local" : "reported" } ?? "-"
        s["terminal.mouse.owner"] = surface.map {
            $0.shellInForeground() ? "shell" : $0.selectsOnDrag() ? "select" : "program"
        } ?? "-"
        s["terminal.link"] = surface?.hoveredLink ?? "-"
        s["terminal.opened"] = TerminalLinks.lastOpened ?? "-"
        s["terminal.refused"] = TerminalLinks.lastRefused ?? "-"
        return s
    }

    private static let usage = "mouse click C,R [N] [MODS] | drag C,R C,R [MODS] | hover C,R [MODS] | mods [MODS]"

    private static func surface(_ controller: WindowController) -> TerminalSurfaceView? {
        ((controller.tiles.focusedView as? TileFrame)?.content as? TerminalTileView)?.hintSurface
    }

    /// A modifier change reaches the first responder, as a key window sends it.
    private static func modifiers(_ flags: NSEvent.ModifierFlags, window: NSWindow) {
        let code: UInt16 = flags.contains(.command) ? 55 : flags.contains(.shift) ? 56
            : flags.contains(.option) ? 58 : flags.contains(.control) ? 59 : 55
        guard let event = NSEvent.keyEvent(
            with: .flagsChanged, location: .zero, modifierFlags: flags,
            timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
            context: nil, characters: "", charactersIgnoringModifiers: "", isARepeat: false, keyCode: code)
        else { return }
        window.firstResponder?.flagsChanged(with: event)
    }

    /// One mouse event, delivered to the view under `point` (window
    /// coordinates); drags and releases go where the press went, as AppKit
    /// sends them.
    private static var pressed: NSView?

    private static func send(_ kind: NSEvent.EventType, at point: CGPoint, flags: NSEvent.ModifierFlags,
                             clicks: Int, window: NSWindow) {
        guard let event = NSEvent.mouseEvent(
            with: kind, location: point, modifierFlags: flags,
            timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
            context: nil, eventNumber: 0, clickCount: clicks, pressure: kind == .leftMouseUp ? 0 : 1)
        else { return }
        let frameView = window.contentView?.superview ?? window.contentView
        let hit = kind == .leftMouseDown || kind == .mouseMoved || pressed == nil
            ? frameView?.hitTest(point) : pressed
        target = hit is TerminalSurfaceView ? "terminal" : hit.map { "\(type(of: $0))" } ?? "none"
        guard let hit else { return }
        switch kind {
        case .leftMouseDown:
            pressed = hit
            hit.mouseDown(with: event)
        case .leftMouseDragged: hit.mouseDragged(with: event)
        case .leftMouseUp:
            pressed = nil
            hit.mouseUp(with: event)
        default: hit.mouseMoved(with: event)
        }
    }
}
