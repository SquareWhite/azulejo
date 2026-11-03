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
///     palette … / seen-visible … / sh …   see `ShellAutomation`
///     shot name              PNG of the window's content → <out>/name.png
///     expect tiles 3         assert on the focused window's state; keys:
///                            windows workspaces workspace tabs tab tiles
///                            focus zoomed sidebar axis ratio
///     log text               a note in the log
///     core {json}            run a core command (`{"method": …, "params": …}`)
///     tile verb [arg]        hand a verb to the focused tile (AutomatableTile)
///     until key value [ms]   wait until `expect key value` holds (default 5000)
///                            tile kinds add `tile.*` keys to expect/until
///     anim duration MS       slow (or speed up) layout motion; `default` resets
///     anim enabled BOOL      force motion on or off; `default` follows config
///     anim reset             clear motion stats
///     anim stats [name]      write motion stats → <out>/name.json (anim-stats)
///     anim expect-clean [N]  fail if more than N (default 0) frames dropped, or
///                            PTY resizes ≠ one per terminal per motion
///     capture name [ms]      after ms, ask the harness to `screencapture` the
///                            window → <out>/name.png (Metal and in-flight
///                            motion included, unlike `shot`)
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
        case "palette", "seen-visible", "sh":
            if let problem = ShellAutomation.run(parts[0], arg, app: app) {
                failures.append("line \(step): \(parts[0]) \(arg): \(problem)")
            }
            record("\(parts[0]) \(arg)")
        case "action":
            let bits = arg.split(separator: " ").map(String.init)
            app?.perform(action: bits[0], index: bits.count > 1 ? Int(bits[1]) : nil)
        case "wait": delay = (Double(arg) ?? 0) / 1000
        case "shot": shot(arg); delay = 0
        case "expect": expect(arg); delay = 0
        case "log": record("# \(arg)"); delay = 0
        case "core": coreCommand(arg)
        case "tile": tileCommand(arg)
        case "until": return until(arg)
        case "anim": animCommand(arg); delay = 0
        case "capture": capture(arg); delay = 0
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

    private func animCommand(_ arg: String) {
        let parts = arg.split(separator: " ").map(String.init)
        let value = parts.count > 1 ? parts[1] : ""
        switch parts.first ?? "" {
        case "duration":
            Motion.durationOverride = value == "default" ? nil : (Double(value) ?? 200) / 1000
        case "enabled":
            Motion.enabledOverride = value == "default" ? nil : value == "true"
        case "style":
            Motion.styleOverride = value == "default" ? nil : value
        case "reset":
            MotionStats.shared.reset()
        case "stats":
            let summary = MotionStats.shared.summary()
            let name = value.isEmpty ? "anim-stats" : value
            if let data = try? JSONSerialization.data(withJSONObject: summary, options: [.prettyPrinted, .sortedKeys]) {
                try? data.write(to: out.appendingPathComponent("\(name).json"))
            }
            record("anim stats: \(summary.filter { $0.key != "resize_mismatches" })")
        case "expect-clean":
            let summary = MotionStats.shared.summary()
            let dropped = summary["dropped_frames"] as? Int ?? -1
            let mismatches = summary["resize_mismatches"] as? [String] ?? []
            let allowed = Int(value) ?? 0
            if dropped > allowed { failures.append("line \(step): \(dropped) dropped frame(s), allowed \(allowed)") }
            if !mismatches.isEmpty { failures.append("line \(step): \(mismatches.joined(separator: ", "))") }
        default:
            failures.append("line \(step): anim needs duration|enabled|reset|stats|expect-clean")
        }
        record("anim \(arg)")
    }

    /// A window capture by the system (Metal layers and running animations
    /// included). The app can't run `screencapture` itself (no Screen
    /// Recording permission), so it drops a request in `<out>/.capture/` for
    /// the harness to take (apps/macos/automation/anim.sh watches it).
    private func capture(_ arg: String) {
        let parts = arg.split(separator: " ").map(String.init)
        guard let name = parts.first else { return }
        let delay = parts.count > 1 ? (Double(parts[1]) ?? 0) / 1000 : 0
        let dir = out.appendingPathComponent(".capture", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
            try? Data().write(to: dir.appendingPathComponent(name), options: .atomic)
        }
        record("capture \(name) +\(Int(delay * 1000)) ms")
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

    private func coreCommand(_ json: String) {
        guard let data = json.data(using: .utf8),
              let command = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        else { return failures.append("line \(step): core needs a JSON command") }
        if case .failure(let error)? = app?.send(command) {
            failures.append("line \(step): core \(command["method"] ?? "?"): \(error.message)")
        }
        record("core \(command["method"] ?? "?")")
    }

    private func tileCommand(_ arg: String) {
        let parts = arg.split(separator: " ", maxSplits: 1).map(String.init)
        guard let tile = app?.focusedController?.tiles.focusedView as? AutomatableTile else {
            return failures.append("line \(step): the focused tile takes no automation verbs")
        }
        var verb = parts.first ?? "", rest = parts.count > 1 ? parts[1] : ""
        // `snapshot-into name.png` writes into the automation's out folder.
        if verb == "snapshot-into" { verb = "snapshot"; rest = out.appendingPathComponent(rest).path }
        if verb == "downloads-into" { verb = "downloads-dir"; rest = out.appendingPathComponent(rest).path }
        if let problem = tile.automate(verb, rest) {
            failures.append("line \(step): tile \(arg): \(problem)")
        }
        record("tile \(arg)")
    }

    /// Polls `expect key value` every 50 ms until it holds or times out.
    private func until(_ arg: String) {
        let parts = arg.split(separator: " ").map(String.init)
        guard parts.count >= 2 else {
            failures.append("line \(step): until needs a key and a value")
            return next()
        }
        let timeout = parts.count > 2 ? (Double(parts.last!) ?? 5000) / 1000 : 5
        let value = parts.count > 2 ? parts[1..<(parts.count - 1)].joined(separator: " ") : parts[1]
        let deadline = Date().addingTimeInterval(timeout)
        func poll() {
            let actual = state()[parts[0]] ?? "?"
            if actual == value {
                record("until \(parts[0]) \(value) ✓")
                return next()
            }
            if Date() > deadline {
                let msg = "line \(step): timed out waiting for \(parts[0]) \(value), got \(actual)"
                failures.append(msg)
                record("until FAILED: \(msg)")
                return next()
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05, execute: poll)
        }
        poll()
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
        if let tile = controller.tiles.focusedView as? AutomatableTile {
            s.merge(tile.automationState()) { $1 }
        }
        s.merge(ShellAutomation.state(app: app, controller: controller)) { $1 }
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

/// Tile views that automation scripts can drive (`tile …`, `tile.*` keys).
@MainActor
protocol AutomatableTile {
    /// Runs a verb; returns a problem description, or nil when it went fine.
    func automate(_ verb: String, _ arg: String) -> String?
    /// Extra `expect` / `until` keys, prefixed `tile.`.
    func automationState() -> [String: String]
}
