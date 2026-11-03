import AppKit

/// Runs a keyboard walkthrough inside the app, with no OS-level input: each
/// `key` line becomes an NSEvent posted to our own event queue, so it goes
/// through the same local monitor and key router as a real keypress. Nothing
/// is sent to other apps and no window is raised (automation runs as an
/// accessory app with its windows ordered back).
///
/// Script (`.azs`), one command per line, `#` comments:
///
///     key alt+h              post a keyDown for the chord
///     action move_tile_left  run a keymap action by name (menu path), for
///                            actions with no default chord
///     wait 300               milliseconds
///     shot name              PNG of the window's content → <out>/name.png
///     expect tiles 3         assert on the focused window's state; keys:
///                            windows workspaces workspace tabs tab tiles
///                            focus zoomed sidebar axis ratio
///     log text               a note in the log
///     quit                   finish (implied at the end)
///
/// Writes `<out>/automation.log` (every step with the layout after it) and
/// `<out>/result` (`ok` or `fail: …`), then quits.
@MainActor
final class Automation {
    private weak var app: AppController?
    private let lines: [String]
    private let out: URL
    private var step = 0
    private var logText = ""
    private var failures: [String] = []
    private var notes: [String] = []

    init(app: AppController, script: String, out: String) {
        self.app = app
        self.out = URL(fileURLWithPath: out, isDirectory: true)
        let text = (try? String(contentsOfFile: script, encoding: .utf8)) ?? ""
        lines = text.split(separator: "\n", omittingEmptySubsequences: false).map {
            $0.split(separator: "#", maxSplits: 1, omittingEmptySubsequences: false).first
                .map { $0.trimmingCharacters(in: .whitespaces) } ?? ""
        }
        try? FileManager.default.createDirectory(at: self.out, withIntermediateDirectories: true)
        if text.isEmpty { failures.append("can't read script \(script)") }
    }

    func start() {
        record("automation start; core \(app?.core.info()["herdr_session"] ?? "?")")
        if let controller = app?.focusedController {
            try? "\(controller.window.windowNumber)\n".write(
                to: out.appendingPathComponent("window-id"), atomically: true, encoding: .utf8)
        }
        // Let the first layout settle (and herdr bind the first terminal).
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { self.next() }
    }

    /// Lines the app logs while a step runs (key outcomes, failures).
    func note(_ line: String) { notes.append(line) }

    private func next() {
        guard step < lines.count else { return finish() }
        let line = lines[step]
        step += 1
        guard !line.isEmpty else { return next() }
        let parts = line.split(separator: " ", maxSplits: 1).map(String.init)
        let arg = parts.count > 1 ? parts[1] : ""
        var delay = 0.15
        notes.removeAll()
        switch parts[0] {
        case "key": press(arg)
        case "action":
            let bits = arg.split(separator: " ").map(String.init)
            app?.perform(action: bits[0], index: bits.count > 1 ? Int(bits[1]) : nil)
        case "wait": delay = (Double(arg) ?? 0) / 1000
        case "shot": shot(arg); delay = 0
        case "expect": expect(arg); delay = 0
        case "log": record("# \(arg)"); delay = 0
        case "quit": return finish()
        default: failures.append("line \(step): unknown command \(parts[0])")
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
            if parts[0] == "key" || parts[0] == "action" { self.record("\(parts[0]) \(arg)") }
            self.next()
        }
    }

    // MARK: Commands

    private func press(_ chord: String) {
        guard let app, let controller = app.focusedController else {
            return failures.append("line \(step): no window")
        }
        var parts = chord.split(separator: "+").map(String.init)
        let aliases = ["]": "bracketright", "[": "bracketleft", "-": "minus", "=": "equal",
                       ",": "comma", ".": "period", "/": "slash", ";": "semicolon",
                       "'": "quote", "`": "grave", "\\": "backslash", "esc": "escape", "return": "enter"]
        guard let raw = parts.popLast(), let code = KeyRouter.keyCodes[aliases[raw] ?? raw] else {
            return failures.append("line \(step): can't type \(chord)")
        }
        var flags: NSEvent.ModifierFlags = []
        for part in parts {
            switch part {
            case "ctrl": flags.insert(.control)
            case "alt": flags.insert(.option)
            case "cmd": flags.insert(.command)
            case "shift": flags.insert(.shift)
            default: return failures.append("line \(step): bad modifier \(part)")
            }
        }
        let chars = raw.count == 1 ? raw : ""
        guard let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: flags,
            timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: controller.window.windowNumber, context: nil,
            characters: chars, charactersIgnoringModifiers: chars,
            isARepeat: false, keyCode: code)
        else { return failures.append("line \(step): couldn't make an event for \(chord)") }
        let before = app.keysSeen
        NSApp.postEvent(event, atStart: false)
        // If the local monitor didn't see it (no key window in a background
        // run), hand it to the router directly: same code path from there.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
            if app.keysSeen == before {
                self.notes.append("(monitor skipped; routed directly)")
                app.routeDirectly(event)
            }
        }
    }

    private func shot(_ name: String) {
        guard let view = app?.focusedController?.window.contentView else { return }
        view.layoutSubtreeIfNeeded()
        guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return }
        view.cacheDisplay(in: view.bounds, to: rep)
        let url = out.appendingPathComponent("\(name).png")
        do {
            try rep.representation(using: .png, properties: [:])?.write(to: url)
            record("shot \(name) → \(url.lastPathComponent)")
        } catch {
            failures.append("line \(step): shot \(name): \(error)")
        }
    }

    private func expect(_ arg: String) {
        let parts = arg.split(separator: " ", maxSplits: 1).map(String.init)
        guard parts.count == 2 else { return failures.append("line \(step): expect needs a key and a value") }
        let actual = state()[parts[0]] ?? "?"
        if actual == parts[1] {
            record("expect \(parts[0]) \(parts[1]) ✓")
        } else {
            let msg = "line \(step): expected \(parts[0]) \(parts[1]), got \(actual)"
            failures.append(msg)
            record("expect FAILED: \(msg)")
        }
    }

    // MARK: State

    private func state() -> [String: String] {
        guard let app, let controller = app.focusedController,
              let session = app.session, let window = session.window(controller.windowID)
        else { return [:] }
        var s: [String: String] = [
            "windows": "\(session.windows.count)",
            "workspaces": "\(window.workspaces.count)",
            "workspace": "\((window.workspaces.firstIndex { $0.id == window.activeWorkspace } ?? -1) + 1)",
            "sidebar": controller.sidebarVisible ? "visible" : "hidden",
        ]
        if let ws = window.workspace {
            s["tabs"] = "\(ws.tabs.count)"
            s["tab"] = "\((ws.tabs.firstIndex { $0.id == ws.tab?.id } ?? -1) + 1)"
        }
        if let tab = window.workspace?.tab {
            s["tiles"] = "\(tab.tiles.count)"
            s["focus"] = "\((tab.order.firstIndex(of: tab.focused) ?? -1) + 1)"
            s["zoomed"] = tab.zoomed == nil ? "no" : "yes"
            switch tab.root {
            case .leaf: s["axis"] = "leaf"; s["ratio"] = "-"
            case .split(let axis, let ratio, _, _):
                s["axis"] = axis == .horizontal ? "h" : "v"
                s["ratio"] = String(format: "%.2f", ratio)
            }
            s["tree"] = tab.root.describe { id in
                let mark = id == tab.focused ? "*" : ""
                return "\(mark)\(Short.id(id))"
            }
        }
        return s
    }

    private func record(_ what: String) {
        let s = state()
        var line = "\(String(format: "%3d", step)) \(what)"
        if what.hasPrefix("key") || what.hasPrefix("action") || what.hasPrefix("automation") {
            line += "\n      ws \(s["workspace"] ?? "?")/\(s["workspaces"] ?? "?") tab \(s["tab"] ?? "?")/\(s["tabs"] ?? "?")"
            line += " tiles \(s["tiles"] ?? "?") focus \(s["focus"] ?? "?") zoom \(s["zoomed"] ?? "?")"
            line += " sidebar \(s["sidebar"] ?? "?")  \(s["tree"] ?? "")"
            for note in notes { line += "\n      · \(note)" }
        }
        logText += line + "\n"
        FileHandle.standardError.write(Data((line + "\n").utf8))
    }

    private func finish() {
        let result = failures.isEmpty ? "ok" : "fail: \(failures.joined(separator: "; "))"
        record("automation done: \(result)")
        try? logText.write(to: out.appendingPathComponent("automation.log"), atomically: true, encoding: .utf8)
        try? (result + "\n").write(to: out.appendingPathComponent("result"), atomically: true, encoding: .utf8)
        NSApp.terminate(nil)
    }
}
