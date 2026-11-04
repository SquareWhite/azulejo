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
///     type some text         a keyDown per character (letters, digits, space,
///                            `- = , . / ; ' [ ]`)
///     delivery direct|post   how keys arrive: `post` (default) queues an
///                            NSEvent; `direct` routes it as a key window
///                            would, menu key equivalents included (posted
///                            events lose those in a background run)
///     action move_tile_left  run a keymap action by name (menu path), for
///                            actions with no default chord
///     wait 300               milliseconds
///     palette … / seen-visible … / sh …   see `ShellAutomation`
///     ime … / clip … / surface … / responder …   see `FocusAutomation`
///     shot name              PNG of the window's content → <out>/name.png
///     expect tiles 3         assert on the focused window's state; keys:
///                            windows workspaces workspace tabs tab tiles
///                            focus zoomed sidebar axis ratio kind masters
///                            orientation part visible resizes dimmed
///                            order layout
///                            (dimmed: unfocused tiles dimmed, by place in
///                            the tab, "2,3" or "none"; dim.strength: how
///                            much, "0.25")
///     remember key           note a state value now…
///     unchanged key          …and fail if it differs from the noted one
///     log text               a note in the log
///     theme set|preview|system|picker …   see `ThemeAutomation`
///     trace on|off           key and action lines also log the layout:
///                            kind, parameters, order, focus, weights, rects
///                            (`expect order t2,t1,t3` checks the order)
///     menu workspace N|rail N|tile [invoke TITLE]   build a right-click menu, log
///                            its items, run one (see `MenuAutomation`);
///                            `menu answer TEXT` answers the next prompt,
///                            `menu shot NAME` shoots it drawn open
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
    private var remembered: [String: String] = [:]
    /// `delivery direct`: keys skip the event queue (see `press`).
    private var directDelivery = false
    /// `trace on`: key and action lines also log the layout (`layoutTrace`).
    private var tracing = false

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
        case "type":
            // One keyDown per character, through the same path as `key`.
            for ch in arg {
                let s = String(ch)
                press(s == " " ? "space" : (s != s.lowercased() ? "shift+\(s.lowercased())" : s))
            }
            record("type \(arg)")
        case "palette", "seen-visible", "sh", "machine-add":
            if let problem = ShellAutomation.run(parts[0], arg, app: app) {
                failures.append("line \(step): \(parts[0]) \(arg): \(problem)")
            }
            record("\(parts[0]) \(arg)")
        case "ime", "clip", "surface", "responder", "terminal":
            if let problem = FocusAutomation.run(parts[0], arg, app: app) {
                failures.append("line \(step): \(parts[0]) \(arg): \(problem)")
            }
            record("\(parts[0]) \(arg)")
        case "action":
            let bits = arg.split(separator: " ").map(String.init)
            app?.perform(action: bits[0], index: bits.count > 1 ? Int(bits[1]) : nil)
        case "wait": delay = (Double(arg) ?? 0) / 1000
        case "shot": shot(arg); delay = 0
        case "expect": expect(arg); delay = 0
        case "remember":
            remembered[arg] = state()[arg] ?? "?"
            record("remember \(arg) = \(remembered[arg] ?? "?")"); delay = 0
        case "unchanged":
            let now = state()[arg] ?? "?"
            if now == remembered[arg] {
                record("unchanged \(arg) \(now) ✓")
            } else {
                failures.append("line \(step): \(arg) changed from \(remembered[arg] ?? "?") to \(now)")
                record("unchanged FAILED: \(arg) \(remembered[arg] ?? "?") → \(now)")
            }
            delay = 0
        case "log": record("# \(arg)"); delay = 0
        case "delivery": directDelivery = arg == "direct"; record("delivery \(arg)"); delay = 0
        case "trace":
            tracing = arg == "on"; record("trace \(arg)"); delay = 0
            if tracing { record("      \(state()["layout"] ?? "")") }
        case "core": coreCommand(arg)
        case "tile": tileCommand(arg)
        case "menu":
            if arg.hasPrefix("shot ") {
                menuShot(String(arg.dropFirst(5)))
            } else if let problem = MenuAutomation.run(arg, app: app) {
                failures.append("line \(step): menu \(arg): \(problem)")
            }
            record("menu \(arg)")
            for note in notes where note.hasPrefix("menu ") { record("      · \(note)") }
        case "theme":
            if let problem = ThemeAutomation.run(arg, app: app) {
                failures.append("line \(step): theme \(arg): \(problem)")
            }
            record("theme \(arg)")
        case "sidebar":
            if let problem = SidebarAutomation.run(arg, controller: app?.focusedController) {
                failures.append("line \(step): sidebar \(arg): \(problem)")
            }
            record("sidebar \(arg)")
        case "until": return until(arg)
        case "anim": animCommand(arg); delay = 0
        case "capture": capture(arg); delay = 0
        case "window-size":
            // `window-size <width> <height>`: the focused window's content size
            // in points (mockup comparisons capture at the mockup's size).
            let bits = arg.split(separator: " ").compactMap { Double($0) }
            if bits.count == 2, let window = app?.focusedController?.window {
                window.setContentSize(NSSize(width: bits[0], height: bits[1]))
                record("window-size \(Int(bits[0]))×\(Int(bits[1]))")
            } else {
                failures.append("line \(step): window-size \(arg): needs a width and a height")
            }
            delay = 0.3
        case "quit": return finish()
        default: failures.append("line \(step): unknown command \(parts[0])")
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
            Perf.measure("automation") {
                if parts[0] == "key" || parts[0] == "action" { self.record("\(parts[0]) \(arg)") }
                self.next()
            }
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
        // The characters a real keypress carries: shifted letters upper case,
        // ctrl+letter its control character, named keys their codes.
        var named = ["space": " ", "enter": "\r", "tab": "\t", "escape": "\u{1b}", "backspace": "\u{7f}"]
        for (name, key) in [("up", NSUpArrowFunctionKey), ("down", NSDownArrowFunctionKey),
                            ("left", NSLeftArrowFunctionKey), ("right", NSRightArrowFunctionKey),
                            ("pageup", NSPageUpFunctionKey), ("pagedown", NSPageDownFunctionKey),
                            ("home", NSHomeFunctionKey), ("end", NSEndFunctionKey),
                            ("delete", NSDeleteFunctionKey)] {
            named[name] = UnicodeScalar(key).map { String(Character($0)) }
        }
        var ignoring = raw.count == 1 ? raw : named[aliases[raw] ?? raw] ?? ""
        if flags.contains(.shift) { ignoring = ignoring.uppercased() }
        var chars = ignoring
        if flags.contains(.control), let scalar = ignoring.lowercased().unicodeScalars.first,
           ignoring.count == 1, ("a"..."z").contains(Character(scalar)) {
            chars = String(UnicodeScalar(UInt8(scalar.value & 0x1f)))
        }
        guard let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: flags,
            timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: controller.window.windowNumber, context: nil,
            characters: chars, charactersIgnoringModifiers: ignoring,
            isARepeat: false, keyCode: code)
        else { return failures.append("line \(step): couldn't make an event for \(chord)") }
        if directDelivery {
            // As a key window would: the router, then key equivalents, the
            // menus and the first responder (KeyDelivery). Posted events in a
            // background run lose menu key equivalents (no key window).
            app.routeDirectly(event)
            return
        }
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

    /// `menu shot NAME`: the window with the last menu built (`menu …`)
    /// drawn open where a right-click would open it.
    private func menuShot(_ name: String) {
        guard let (menu, anchor) = MenuAutomation.last,
              let window = app?.focusedController?.window, let view = window.contentView
        else { return failures.append("line \(step): menu shot: build a menu first") }
        let image = MenuRender.image(of: menu, appearance: view.effectiveAppearance)
        // Below the click, a little in: as a right-click near the anchor's
        // top left shows it (window coordinates are bottom-up).
        let origin = NSPoint(x: min(anchor.minX + 24, view.bounds.width - image.size.width - 8),
                             y: max(8, anchor.maxY - 14 - image.size.height))
        shot(name) { ctx in
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(cgContext: ctx, flipped: false)
            let shadow = NSShadow()
            shadow.shadowColor = NSColor.black.withAlphaComponent(0.3)
            shadow.shadowBlurRadius = 12
            shadow.shadowOffset = NSSize(width: 0, height: -4)
            shadow.set()
            image.draw(in: NSRect(origin: origin, size: image.size))
            NSGraphicsContext.restoreGraphicsState()
        }
    }

    private func shot(_ name: String, overlay: ((CGContext) -> Void)? = nil) {
        guard let view = app?.focusedController?.window.contentView else { return }
        view.layoutSubtreeIfNeeded()
        guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return }
        view.cacheDisplay(in: view.bounds, to: rep)
        // Onto a plain bitmap with the traffic lights, which live outside
        // the content view.
        var output = rep
        if let window = view.window, let image = rep.cgImage, let ctx = CGContext(
            data: nil, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) {
            ctx.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
            ctx.scaleBy(x: CGFloat(image.width) / view.bounds.width, y: CGFloat(image.height) / view.bounds.height)
            Self.drawTitlebarButtons(of: window, in: ctx)
            overlay?(ctx)
            if let composed = ctx.makeImage() { output = NSBitmapImageRep(cgImage: composed) }
        }
        let url = out.appendingPathComponent("\(name).png")
        do {
            try output.representation(using: .png, properties: [:])?.write(to: url)
            record("shot \(name) → \(url.lastPathComponent)")
            checkFreshness(name)
        } catch {
            failures.append("line \(step): shot \(name): \(error)")
        }
    }

    /// Both renders read each terminal's layer contents: the IOSurface
    /// libghostty last presented, or the motion snapshot (a copy of an older
    /// one) while it's up. Says so when that isn't the terminal's current
    /// frame (`STALE` lines in the log), and writes `<name>.fresh.json`, so
    /// a capture never passes off an old frame as the app's state. A locked
    /// screen is noted too: the display link doesn't tick then.
    private func checkFreshness(_ name: String) {
        guard let tiles = app?.focusedController?.tiles else { return }
        let report = tiles.captureFreshness()
        let locked = Self.screenLocked
        let stale = report.tiles.flatMap { tile in
            (tile["stale"] as? [String] ?? []).map { "tile \(tile["tile"] ?? "?"): \($0)" }
        }
        let json: [String: Any] = [
            "capture": name, "screen_locked": locked, "stale": stale, "terminals": report.tiles,
            "display_link_ticks_since_last_motion": report.ticks,
        ]
        if let data = try? JSONSerialization.data(withJSONObject: json, options: [.prettyPrinted, .sortedKeys]) {
            try? data.write(to: out.appendingPathComponent("\(name).fresh.json"))
        }
        for line in stale { record("  STALE \(name): \(line)") }
        if locked { record("  note \(name): screen locked (no display-link ticks, no system captures)") }
    }

    /// `CGSSessionScreenIsLocked` from the console session.
    static var screenLocked: Bool {
        guard let session = CGSessionCopyCurrentDictionary() as? [String: Any] else { return false }
        return (session["CGSSessionScreenIsLocked"] as? NSNumber)?.boolValue ?? false
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
    /// `screencapture` gets nothing while the screen is locked, so the app
    /// also renders the layer presentation tree itself to `<name>-app.png`:
    /// tiles, headers and corner caps where the motion has them at that
    /// instant. Terminals show the frame libghostty last presented, or their
    /// motion snapshot while it's up; `checkFreshness` flags either when it
    /// isn't current.
    private func capture(_ arg: String) {
        let parts = arg.split(separator: " ").map(String.init)
        guard let name = parts.first else { return }
        let delay = parts.count > 1 ? (Double(parts[1]) ?? 0) / 1000 : 0
        let dir = out.appendingPathComponent(".capture", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
            try? Data().write(to: dir.appendingPathComponent(name), options: .atomic)
            self.renderPresentation("\(name)-app")
        }
        record("capture \(name) +\(Int(delay * 1000)) ms")
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard let self, let controller = self.app?.focusedController else { return }
            self.record("  frames \(name): \(controller.tiles.describeVisualFrames)")
            self.record("  chrome \(name): \(controller.describeChrome)")
        }
    }

    /// The window's content as Core Animation is showing it right now
    /// (presentation values, so mid-motion frames come out mid-motion).
    private func renderPresentation(_ name: String) {
        guard let window = app?.focusedController?.window,
              let view = window.contentView, let layer = view.layer else { return }
        let scale = window.backingScaleFactor
        let size = view.bounds.size
        guard size.width > 0, size.height > 0,
              let ctx = CGContext(
                data: nil, width: Int(size.width * scale), height: Int(size.height * scale),
                bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return }
        ctx.scaleBy(x: scale, y: scale)
        // theme: read per render.
        ctx.setFillColor(Design.Color.window.cgColor)
        ctx.fill(CGRect(origin: .zero, size: size))
        ctx.saveGState()
        if layer.isGeometryFlipped || view.isFlipped {
            ctx.translateBy(x: 0, y: size.height)
            ctx.scaleBy(x: 1, y: -1)
        }
        (layer.presentation() ?? layer).render(in: ctx)
        ctx.restoreGState()
        Self.drawTitlebarButtons(of: window, in: ctx)
        guard let image = ctx.makeImage() else { return }
        let rep = NSBitmapImageRep(cgImage: image)
        let url = out.appendingPathComponent("\(name).png")
        do {
            try rep.representation(using: .png, properties: [:])?.write(to: url)
            checkFreshness(name)
        } catch {
            notes.append("capture \(name): \(error)")
        }
    }

    /// The traffic lights live in the window's title bar, outside the
    /// content view, so neither render above has them: draw each where it
    /// sits. `ctx` is in window points, bottom-up (the content view fills the
    /// window). A background window's lights are grey, as on screen: AppKit
    /// draws those to add light to what's under them, so they go on with
    /// plus-lighter (that matches `screencapture` to a couple of levels).
    private static func drawTitlebarButtons(of window: NSWindow, in ctx: CGContext) {
        ctx.saveGState()
        if !window.isKeyWindow { ctx.setBlendMode(.plusLighter) }
        for type in [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton] {
            guard let button = window.standardWindowButton(type), !button.isHiddenOrHasHiddenAncestor,
                  let rep = button.bitmapImageRepForCachingDisplay(in: button.bounds)
            else { continue }
            button.cacheDisplay(in: button.bounds, to: rep)
            guard let image = rep.cgImage else { continue }
            ctx.draw(image, in: button.convert(button.bounds, to: nil))
        }
        ctx.restoreGState()
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
            "sidebar": controller.sidebarState == .full ? "visible" : controller.sidebarState.rawValue,
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
            s["kind"] = tab.kind.rawValue
            let params = tab.layout?.params
            switch tab.kind {
            case .tile:
                s["masters"] = params.map { "\($0.tile.masters)" } ?? "?"
                s["orientation"] = params?.tile.orientation ?? "?"
                s["part"] = params?.tile.rotatePart == true ? "yes" : "no"
                s["ratio"] = params.map { String(format: "%.2f", $0.tile.ratio) } ?? "?"
            case .threeColumn:
                s["masters"] = params.map { "\($0.threeColumn.masters)" } ?? "?"
                s["ratio"] = params.map { String(format: "%.2f", $0.threeColumn.ratio) } ?? "?"
            case .manual, .monocle:
                break
            }
            s["layout"] = Self.layoutTrace(tab)
            s["order"] = tab.order.map { Self.tileName($0, in: tab) }.joined(separator: ",")
            s["visible"] = "\(controller.tiles.visibleTileFrames().count)"
            s["dimmed"] = controller.tiles.dimmedTiles
            s["dim.strength"] = String(format: "%.2f", Motion.dimStrength)
            s["resizes"] = "\(controller.tiles.totalSurfaceResizes)"
        }
        if let tile = controller.tiles.focusedView as? AutomatableTile {
            s.merge(tile.automationState()) { $1 }
        }
        s.merge(ShellAutomation.state(app: app, controller: controller)) { $1 }
        s.merge(FocusAutomation.state(app: app, controller: controller)) { $1 }
        s.merge(SidebarAutomation.state(controller)) { $1 }
        s.merge(ThemeAutomation.state(controller)) { $1 }
        s.merge(MenuAutomation.state(app: app, controller: controller)) { $1 }
        s.merge(controller.chromeState()) { $1 }
        return s
    }

    /// A tile's name in traces and `expect order`: `t1` is the first tile
    /// seen, `t2` the next, and a name never moves to another tile while the
    /// layout order changes and tiles close.
    static func tileName(_ id: String, in tab: TabModel) -> String {
        for tile in tab.tiles where traceNames[tile.id] == nil {
            traceNames[tile.id] = "t\(traceNames.count + 1)"
        }
        return traceNames[id] ?? "t?"
    }
    private static var traceNames: [String: String] = [:]

    /// `trace on`: the layout after each key, for checking chords against
    /// Bismuth: kind, parameters, order (masters first), focus, weights and
    /// each tile's rect in a 1000×1000 tab (top-left origin, from the tree
    /// the shell draws; Monocle marks the one tile shown).
    static func layoutTrace(_ tab: TabModel) -> String {
        let name = { (id: String) in tileName(id, in: tab) }
        var parts = ["kind=\(tab.kind.rawValue)"]
        let p = tab.layout?.params
        switch tab.kind {
        case .tile:
            parts.append("o=\(p?.tile.orientation ?? "?") m=\(p?.tile.masters ?? -1)"
                + " r=\(String(format: "%.3f", p?.tile.ratio ?? 0)) part=\(p?.tile.rotatePart == true ? "yes" : "no")")
        case .threeColumn:
            parts.append("m=\(p?.threeColumn.masters ?? -1) r=\(String(format: "%.3f", p?.threeColumn.ratio ?? 0))")
        case .manual, .monocle: break
        }
        parts.append("order=\(tab.order.map(name).joined(separator: ","))")
        parts.append("focus=\(name(tab.focused))")
        let weights = (tab.layout?.weights ?? [:]).sorted { name($0.key) < name($1.key) }
        if !weights.isEmpty {
            parts.append("w=" + weights.map { "\(name($0.key)):\(String(format: "%.2f", $0.value))" }.joined(separator: ","))
        }
        let size: CGFloat = 1000
        let frames = tab.root.frames(in: CGRect(x: 0, y: 0, width: size, height: size), gap: 0)
        let rects: [String] = tab.tiles.map { tile in
            if tab.kind == .monocle {
                return "\(name(tile.id))=\(tile.id == tab.focused ? "shown" : "hidden")"
            }
            guard let f = frames[tile.id] else { return "\(name(tile.id))=?" }
            return "\(name(tile.id))=\(Int(f.minX)),\(Int(size - f.maxY)),\(Int(f.width))x\(Int(f.height))"
        }
        parts.append("rects " + rects.joined(separator: " "))
        return parts.joined(separator: " ")
    }

    private func record(_ what: String) {
        let s = state()
        var line = "\(String(format: "%3d", step)) \(what)"
        if what.hasPrefix("key") || what.hasPrefix("action") || what.hasPrefix("automation") {
            line += "\n      ws \(s["workspace"] ?? "?")/\(s["workspaces"] ?? "?") tab \(s["tab"] ?? "?")/\(s["tabs"] ?? "?")"
            line += " tiles \(s["tiles"] ?? "?") focus \(s["focus"] ?? "?") zoom \(s["zoomed"] ?? "?")"
            line += " sidebar \(s["sidebar"] ?? "?")  \(s["tree"] ?? "")"
            if tracing {
                line += "\n      → \(s["key.last"] ?? "?")  \(s["layout"] ?? "")  hint \"\(s["tab.hint"] ?? "")\""
            }
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
