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
///     mouse click|drag|hover|mods …   in-app mouse events on the focused
///                            terminal: selection and links (`MouseAutomation`)
///     drop [@N] files|text|url|image|hover|leave …   a drop on a terminal
///                            tile from a private pasteboard (`DropAutomation`)
///     shot name              PNG of the window's content → <out>/name.png
///     expect tiles 3         assert on the focused window's state; keys:
///                            windows workspaces workspace tabs tab tiles
///                            focus zoomed sidebar axis ratio kind masters
///                            orientation part visible resizes dimmed
///                            order layout tree rect.focused
///                            (rect.focused: "x,y,wxh" in a 1000×1000 tab;
///                            dimmed: unfocused tiles dimmed, by place in
///                            the tab, "2,3" or "none"; dim.strength: how
///                            much, "0.25")
///     remember key           note a state value now…
///     unchanged key          …and fail if it differs from the noted one
///     log text               a note in the log
///     theme set|preview|system|picker …   see `ThemeAutomation`
///     trace on|off           key and action lines also log the layout:
///                            kind, parameters, order, focus, weights, rects
///                            (`expect order t2,t1,t3` checks the order)
///     divider …              drag, double-click and probe the gutters between
///                            tiles with in-app mouse events (`DividerAutomation`)
///     picker KIND            pick a layout in the top bar's picker, as a
///                            click does (`picker.kind`, `picker.tips`)
///     tabdrag press N|move DX DY|over N|away|release|click N   drag a top
///                            tab with mouse events handed to it directly
///                            (see `TabDragAutomation`)
///     menu workspace N|rail N|tab N|tile [invoke TITLE]   build a right-click menu, log
///                            its items, run one (see `MenuAutomation`);
///                            `menu answer TEXT` answers the next prompt,
///                            `menu shot NAME` shoots it drawn open
///     core {json}            run a core command (`{"method": …, "params": …}`)
///     tile verb [arg]        hand a verb to the focused tile (AutomatableTile)
///     until key value [ms]   wait until `expect key value` holds (default 5000)
///     require key value [ms] `until`, and stop the script if it never holds
///                            tile kinds add `tile.*` keys to expect/until
///     anim duration MS       slow (or speed up) layout motion; `default` resets
///     anim enabled BOOL      force motion on or off; `default` follows config
///     anim style NAME        reflow|crop|scale|outline; `default` follows config
///     anim reflow-ms MS      reflow: shortest gap between a terminal's live resizes
///     anim reflow-lead MS    reflow: how far ahead of the frame a terminal's grid runs
///     anim reset             clear motion stats
///     anim stats [name]      write motion stats → <out>/name.json (anim-stats)
///     anim expect-clean [N]  fail if more than N (default 0) frames dropped by
///                            the app (its main thread busy through the gap;
///                            frames the system missed are reported, and fail
///                            past 5% of ticks: `frameVerdict`), if no frames
///                            were timed (display asleep), or if PTY resizes ≠
///                            one per terminal per motion (reflow: one to
///                            `Motion.maxLiveResizes`)
///     anim require-frames    stop the script at once if frames can't be timed
///                            (display asleep, screen locked)
///     anim no-frames BOOL    ignore display-link ticks, standing in for a
///                            sleeping display (motion ends on wall-clock time)
///     capture name [ms]      after ms, ask the harness to `screencapture` the
///                            window → <out>/name.png (Metal and in-flight
///                            motion included, unlike `shot`); the script
///                            carries on at once (mid-motion samples)
///     captured [ms]          wait until every capture asked for so far is on
///                            disk: the harness took the request and wrote
///                            the PNG (or gave up). A request nobody takes
///                            within 2 s means no watcher runs: go on.
///                            Default 5000 for the PNG; the end of a script
///                            waits the same way
///     until motion still [ms]   nothing moves or fades in the focused window
///                            and nothing has for a moment (`motion`: layout
///                            motion, tab or workspace switch, dim fade,
///                            divider drag, live content's reveal; "still"
///                            once 250 ms have passed since the last of them
///                            and 200 ms since the last step that acted)
///     appearance light|dark|system   set NSApp.appearance (the system's
///                            setting is untouched)
///     expect keys for unread marks and the Dock badge: `tile.unread`,
///                            `unread.count`, `dock.badge` (`UnreadAutomation`)
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
    /// When the last step that acts on the app ran (anything but a wait, a
    /// check or a note): `motion still` waits a moment past it, so a motion
    /// the step sets off has begun before it counts as still.
    private var actedAt: CFTimeInterval = 0
    /// Captures asked for (`capture`) and not yet known to be on disk, and
    /// when each one's request went out (after its delay).
    private var pendingCaptures: [String] = []
    private var requestedCaptures: [String: Date] = [:]
    /// A harness took a capture request: one watches (`awaitCaptures`).
    private var watcherSeen = false

    /// Steps that only wait, check or note: everything else acts.
    private static let passive: Set<String> = [
        "wait", "until", "require", "expect", "remember", "unchanged", "log", "delivery", "trace",
        "anim", "capture", "captured", "shot", "quit",
    ]
    /// The pause after a step that sets none of its own.
    private static let implicitDelay = 0.15
    /// Steps that only wait: the 0.15 s pause after the step before goes.
    private static let waits: Set<String> = ["wait", "until", "require", "captured"]
    /// `motion still`: quiet this long after the last motion ends (live
    /// content's reveal, a fade the last frame started)…
    private static let settleAfterMotion: CFTimeInterval = 0.25
    /// …and this long after the last step that acted (a posted key reaches
    /// the router within 0.05 s; the motion it sets off starts then).
    private static let settleAfterStep: CFTimeInterval = 0.2

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
        // Let herdr bind the first terminal (most scripts open with `until
        // terminal.phase attached`). No terminal in focus (yet): give the
        // first layout half a second, as before; one stuck on its way
        // (waiting, retrying): 10 s at most, then the script decides.
        let started = Date()
        func ready() {
            let phase = state()["terminal.phase"] ?? "-"
            let waited = Date().timeIntervalSince(started)
            switch phase {
            case "attached", "observing", "observing-taken-over", "ended":
                record("automation ready: terminal \(phase) after \(Int(waited * 1000)) ms")
                return next()
            case "-" where waited >= 0.5, _ where waited >= 10:
                record("automation ready: terminal \(phase) after \(Int(waited * 1000)) ms")
                return next()
            default:
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.05, execute: ready)
            }
        }
        ready()
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
        var delay = Self.implicitDelay
        notes.removeAll()
        if !Self.passive.contains(parts[0]) { actedAt = CACurrentMediaTime() }
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
        case "ime", "clip", "surface", "responder", "terminal", "hints":
            if let problem = FocusAutomation.run(parts[0], arg, app: app) {
                failures.append("line \(step): \(parts[0]) \(arg): \(problem)")
            }
            record("\(parts[0]) \(arg)")
        case "mouse":
            if let problem = MouseAutomation.run(arg, app: app) {
                failures.append("line \(step): mouse \(arg): \(problem)")
            }
            record("mouse \(arg)")
        case "drop":
            if let controller = app?.focusedController,
               let problem = DropAutomation.run(arg, controller: controller, tab: focusedTab(controller)) {
                failures.append("line \(step): drop \(arg): \(problem)")
            }
            record("drop \(arg)")
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
        case "picker":
            if let problem = LayoutPickerAutomation.run(arg, controller: app?.focusedController) {
                failures.append("line \(step): picker \(arg): \(problem)")
            }
            record("picker \(arg)")
        case "tabdrag":
            if let problem = TabDragAutomation.run(arg, app: app) {
                failures.append("line \(step): tabdrag \(arg): \(problem)")
            }
            record("tabdrag \(arg)")
        case "divider":
            var lines: [String] = []
            let (problem, wait) = DividerAutomation.run(arg, app: app, out: out) { lines.append($0) }
            if let problem { failures.append("line \(step): divider \(arg): \(problem)") }
            record("divider \(arg)\(problem.map { " FAILED: \($0)" } ?? "")")
            for line in lines { record("      · \(line)") }
            delay = wait
        case "sidebar" where arg.hasPrefix("width"):
            let (problem, wait) = SidebarEdgeAutomation.run(String(arg.dropFirst(5)).trimmingCharacters(in: .whitespaces),
                                                            controller: app?.focusedController)
            if let problem { failures.append("line \(step): sidebar \(arg): \(problem)") }
            record("sidebar \(arg)")
            delay = wait
        case "sidebar":
            if let problem = SidebarAutomation.run(arg, controller: app?.focusedController, app: app) {
                failures.append("line \(step): sidebar \(arg): \(problem)")
            }
            record("sidebar \(arg)")
            // Modifier changes time the ⌘N badges' wait: no extra pause.
            if arg.hasPrefix("hints") { delay = 0 }
        case "until": return until(arg)
        case "require": return until(arg, abort: true)
        case "anim" where arg == "require-frames":
            if let why = framesUnavailable {
                record("anim require-frames FAILED: \(why)")
                failures.append("line \(step): \(why); the script stops here")
                return finish()
            }
            record("anim require-frames ✓")
            delay = 0
        case "anim": animCommand(arg); delay = 0
        case "capture": capture(arg); delay = 0
        case "captured":
            record("captured \(pendingCaptures.joined(separator: " "))")
            return awaitCaptures(timeout: (Double(arg) ?? 5000) / 1000) { self.next() }
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
        case "appearance":
            // `appearance light|dark|system`: NSApp.appearance, so the app
            // sees dark mode without touching the system's setting.
            switch arg {
            case "light": NSApp.appearance = NSAppearance(named: .aqua)
            case "dark": NSApp.appearance = NSAppearance(named: .darkAqua)
            case "system": NSApp.appearance = nil
            default: failures.append("line \(step): appearance takes light, dark or system")
            }
            record("appearance \(arg)")
            delay = 0.3
        case "quit": return finish()
        default: failures.append("line \(step): unknown command \(parts[0])")
        }
        // A wait or a poll next paces the script itself: no pause before it,
        // but a key still gets to the router first (a posted key that the
        // monitor doesn't see is routed directly after 0.05 s).
        // (`wait`, `divider` and `sidebar width` set their own pause.)
        let ownPause = ["wait", "divider"].contains(parts[0]) || (parts[0] == "sidebar" && arg.hasPrefix("width"))
        if delay == Self.implicitDelay, !ownPause, let following = nextCommand(), Self.waits.contains(following) {
            delay = ["key", "type", "action"].contains(parts[0]) ? 0.06 : 0
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
            Perf.measure("automation") {
                if parts[0] == "key" || parts[0] == "action" { self.record("\(parts[0]) \(arg)") }
                self.next()
            }
        }
    }

    /// The command of the next line that has one.
    private func nextCommand() -> String? {
        lines[min(step, lines.count)...].first { !$0.isEmpty }
            .map { String($0.split(separator: " ", maxSplits: 1)[0]) }
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
        case "reduce-motion":
            // Stands in for macOS Reduce Motion; the user's setting stays.
            Motion.reduceMotionOverride = value == "default" ? nil : value == "true"
        case "reflow-ms":
            if let ms = Double(value), ms >= 0 { Motion.reflowInterval = ms / 1000 }
        case "reflow-lead":
            if let ms = Double(value), ms >= 0 { Motion.reflowLead = ms / 1000 }
        case "no-frames":
            // Stands in for a sleeping display: the display link's ticks are
            // ignored, and the motion clock runs on wall-clock time.
            TileContainerView.ignoreLinkTicks = value == "true"
        case "reset":
            MotionStats.shared.reset()
        case "stats":
            var summary = MotionStats.shared.summary()
            // A covered window may get fewer frames; a sleeping display none.
            if let controller = app?.focusedController {
                summary["window_visible"] = controller.window.occlusionState.contains(.visible)
                summary["fallback_ticks"] = controller.tiles.fallbackTicks
            }
            summary["frames_unavailable"] = framesUnavailable ?? "no"
            let name = value.isEmpty ? "anim-stats" : value
            if let data = try? JSONSerialization.data(withJSONObject: summary, options: [.prettyPrinted, .sortedKeys]) {
                try? data.write(to: out.appendingPathComponent("\(name).json"))
            }
            let quiet: Set = ["resize_mismatches", "reflow_resizes", "reflow_ends"]
            record("anim stats: \(summary.filter { !quiet.contains($0.key) })")
        case "expect-clean":
            let summary = MotionStats.shared.summary()
            let mismatches = summary["resize_mismatches"] as? [String] ?? []
            if !mismatches.isEmpty { failures.append("line \(step): \(mismatches.joined(separator: ", "))") }
            let verdict = frameVerdict(summary, allowed: Int(value) ?? 0)
            if let problem = verdict.problem { failures.append("line \(step): \(problem)") }
            record("anim \(arg): \(verdict.report)")
            return
        default:
            failures.append(
                "line \(step): anim needs duration|enabled|style|reduce-motion|reflow-ms|reflow-lead|reset|stats|"
                    + "expect-clean|require-frames|no-frames")
        }
        record("anim \(arg)")
    }

    /// Why frames can't be timed right now (nil: they can): the display-link
    /// stops while the window's display sleeps or the screen is locked, and
    /// frame checks then time nothing.
    var framesUnavailable: String? {
        if Self.screenLocked { return "screen is locked: frame-timing checks need an unlocked, awake display" }
        let screen = app?.focusedController?.window.screen ?? NSScreen.main
        let key = NSDeviceDescriptionKey("NSScreenNumber")
        let display = (screen?.deviceDescription[key] as? NSNumber)?.uint32Value ?? CGMainDisplayID()
        if CGDisplayIsAsleep(display) != 0 { return "display is asleep: frame-timing checks need an awake display" }
        return nil
    }

    /// `anim expect-clean N`: at most N dropped frames that were the app's
    /// doing (`MotionStats.DropCause`): its main thread busy and on a CPU
    /// through the gap, or busy and off it on a quiet machine (blocked).
    /// Frames the system missed with the main thread waiting (the window
    /// server, the GPU, the display link paced down), and the main thread
    /// preempted on a busy machine, aren't the app's: they're reported, and
    /// only fail past 5% of all ticks (something far off, whoever's).
    /// No ticks while motions ran (the display asleep, the screen locked)
    /// fails: there was nothing to time.
    private func frameVerdict(_ s: [String: Any], allowed: Int) -> (problem: String?, report: String) {
        let ticks = s["ticks"] as? Int ?? 0
        let animations = s["animation_count"] as? Int ?? 0
        let dropped = s["dropped_frames"] as? Int ?? 0
        let appDrops = s["dropped_app"] as? Int ?? 0
        let offCPU = s["dropped_off_cpu"] as? Int ?? 0
        let system = s["dropped_system"] as? Int ?? 0
        let load = MotionStats.loadAverage.first ?? 0
        let cores = ProcessInfo.processInfo.activeProcessorCount
        let busyMachine = load >= Double(cores) / 2
        let counted = appDrops + (busyMachine ? 0 : offCPU)
        let loadText = String(format: "load %.1f on %d cores%@", load, cores, busyMachine ? " (machine busy)" : "")
        let byKind = (s["dropped_app_by_kind"] as? [String: Int] ?? [:]).sorted { $0.key < $1.key }
            .map { "\($0.key) \($0.value)" }.joined(separator: ", ")
        let report = "\(dropped) of \(ticks) tick(s) dropped: \(appDrops) app (\(byKind.isEmpty ? "-" : byKind)), "
            + "\(offCPU) main thread off-CPU, \(system) system; allowed \(allowed); \(loadText)"
        if ticks == 0, animations > 0 {
            let why = framesUnavailable
                ?? "display asleep or screen locked? frame-timing checks need an awake display"
            return ("no display-link ticks while \(animations) motion(s) ran: \(why)", report)
        }
        if counted > allowed {
            return ("\(counted) dropped frame(s) the app caused (main thread busy), allowed \(allowed): \(report)", report)
        }
        let outside = dropped - counted
        if ticks > 0, Double(outside) > 0.05 * Double(ticks) {
            let pct = String(format: "%.1f", 100 * Double(outside) / Double(ticks))
            return ("\(outside) of \(ticks) frames (\(pct)%) dropped outside the app's main thread: \(report). "
                + (busyMachine ? "The machine is busy: rerun when it's quiet" : "A window server or GPU regression?"),
                report)
        }
        return (nil, report)
    }

    /// A window capture by the system (Metal layers and running animations
    /// included). The app can't run `screencapture` itself (no Screen
    /// Recording permission), so it drops a request in `<out>/.capture/` for
    /// the harness to take (`watch_captures` in apps/macos/automation/
    /// capture.sh): it removes the request when it takes it, then writes
    /// `<name>.png`, or `.capture/.failed-<name>` if `screencapture` got
    /// nothing (`captured` waits for one or the other).
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
        // A capture taken again replaces the last one.
        try? FileManager.default.removeItem(at: dir.appendingPathComponent(".failed-\(name)"))
        try? FileManager.default.removeItem(at: out.appendingPathComponent("\(name).png"))
        requestedCaptures[name] = nil
        if !pendingCaptures.contains(name) { pendingCaptures.append(name) }
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
            try? Data().write(to: dir.appendingPathComponent(name), options: .atomic)
            self.requestedCaptures[name] = Date()
            self.renderPresentation("\(name)-app")
        }
        record("capture \(name) +\(Int(delay * 1000)) ms")
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard let self, let controller = self.app?.focusedController else { return }
            self.record("  frames \(name): \(controller.tiles.describeVisualFrames)")
            self.record("  chrome \(name): \(controller.describeChrome)")
        }
    }

    /// Calls `then` once every pending capture is on disk: its request went
    /// out and the harness took it (removed it) and wrote the PNG or a
    /// failure mark. A request nobody takes means no watcher runs (a suite
    /// that only wants the `-app.png` renders): it counts as done after 2 s,
    /// or half a second while no request has ever been taken. A PNG still
    /// missing after `timeout` seconds is noted, not failed.
    private func awaitCaptures(timeout: TimeInterval, then: @escaping () -> Void) {
        let dir = out.appendingPathComponent(".capture", isDirectory: true)
        let fm = FileManager.default
        let deadline = Date().addingTimeInterval(timeout)
        func poll() {
            pendingCaptures.removeAll { name in
                guard let asked = requestedCaptures[name] else { return false }
                if fm.fileExists(atPath: out.appendingPathComponent("\(name).png").path) { return true }
                if fm.fileExists(atPath: dir.appendingPathComponent(".failed-\(name)").path) {
                    record("  capture \(name): the harness got no image")
                    return true
                }
                guard fm.fileExists(atPath: dir.appendingPathComponent(name).path) else {
                    watcherSeen = true  // taken: the PNG is on its way
                    return false
                }
                return Date().timeIntervalSince(asked) > (watcherSeen ? 2 : 0.5)
            }
            if pendingCaptures.isEmpty { return then() }
            if Date() > deadline {
                record("  captured: still no \(pendingCaptures.joined(separator: ", ")) after \(Int(timeout * 1000)) ms")
                pendingCaptures.removeAll()
                return then()
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.02, execute: poll)
        }
        poll()
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
    /// `require`: `until` that ends the script when it times out, for a
    /// precondition later steps act on (a suite that deletes files must be
    /// in its own folder, not wherever the view happens to be).
    private func until(_ arg: String, abort: Bool = false) {
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
                if abort {
                    failures.append("line \(step): required, so the script stops here")
                    return finish()
                }
                return next()
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05, execute: poll)
        }
        poll()
    }

    private func focusedTab(_ controller: WindowController) -> TabModel? {
        app?.session?.window(controller.windowID)?.workspace?.tab
    }

    // MARK: State

    /// The focused window's state, worked out key by key as a step asks for
    /// it: `expect`, `until` and `record` read a few keys, and building every
    /// key on every step cost the frame budget mid-motion.
    private func state() -> AutomationState {
        guard let app, let controller = app.focusedController,
              let session = app.session, let window = session.window(controller.windowID)
        else { return AutomationState([]) }
        // Trace names go to tiles as they're first seen, on every read, as
        // when each read built the whole state (`order` and `layout` use them).
        if let tab = window.workspace?.tab { Self.nameNewTiles(in: tab) }
        let motion = AutomationState.Source(["motion"]) { [actedAt] in
            ["motion": Self.motionState(controller, actedAt: actedAt)]
        }
        return AutomationState(Self.stateSources(app: app, controller: controller, session: session, window: window)
            + [motion])
    }

    /// `motion`: "still", or "moving: WHAT" while the focused window's tiles
    /// move or fade (`TileContainerView.motionInFlight`), until a moment
    /// after the last motion anywhere ends (live content's reveal) and after
    /// the last step that acted (a motion it sets off has begun by then).
    private static func motionState(_ controller: WindowController, actedAt: CFTimeInterval) -> String {
        if let what = controller.tiles.motionInFlight { return "moving: \(what)" }
        let now = CACurrentMediaTime()
        if now < MotionStats.shared.movingUntil + settleAfterMotion { return "moving: settling" }
        if now < actedAt + settleAfterStep { return "moving: step" }
        return "still"
    }

    /// Every state key, from the sources below in order: a later source's
    /// key wins, as when they were merged into one dictionary. Each source
    /// names the keys it can set (exact names and prefixes); a key no
    /// source names is still found in the full merge.
    private static func stateSources(app: AppController, controller: WindowController,
                                     session: SessionModel, window: WindowModel) -> [AutomationState.Source] {
        typealias Source = AutomationState.Source
        let tab = window.workspace?.tab
        return [
            Source(["windows", "workspaces", "workspace", "sidebar", "tabs", "tab"]) {
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
                return s
            },
            Source(["tiles", "focus", "zoomed", "axis", "ratio", "tree", "kind", "masters", "orientation", "part"]) {
                guard let tab else { return [:] }
                var s: [String: String] = [
                    "tiles": "\(tab.tiles.count)",
                    "focus": "\((tab.order.firstIndex(of: tab.focused) ?? -1) + 1)",
                    "zoomed": tab.zoomed == nil ? "no" : "yes",
                ]
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
                    s["orientation"] = params.map { $0.threeColumn.orientation ?? "columns" } ?? "?"
                    s["ratio"] = params.map { String(format: "%.2f", $0.threeColumn.ratio) } ?? "?"
                case .manual, .monocle:
                    break
                }
                return s
            },
            Source(["layout", "rect.focused", "order"]) {
                guard let tab else { return [:] }
                var s = ["layout": layoutTrace(tab)]
                // The focused tile's rect in a 1000×1000 tab, top-left origin
                // (`x,y,wxh`): follows one tile through a rotate.
                let size: CGFloat = 1000
                if let f = tab.root.frames(in: CGRect(x: 0, y: 0, width: size, height: size), gap: 0)[tab.focused] {
                    s["rect.focused"] = "\(Int(f.minX)),\(Int(size - f.maxY)),\(Int(f.width))x\(Int(f.height))"
                }
                s["order"] = tab.order.map { tileName($0, in: tab) }.joined(separator: ",")
                return s
            },
            Source(["slide", "slide.mode", "slide.ms", "slide.config.ms", "slide.count", "slide.retargets", "slide.last", "slide.in", "slide.dy",
                    "slide.out", "slide.out.dy", "slide.gap", "slide.ca"]) {
                controller.tiles.slideState
            },
            Source(["visible", "dimmed", "dim.strength", "resizes"]) {
                guard tab != nil else { return [:] }
                return [
                    "visible": "\(controller.tiles.visibleTileFrames().count)",
                    "dimmed": controller.tiles.dimmedTiles,
                    "dim.strength": String(format: "%.2f", Motion.dimStrength),
                    "resizes": "\(controller.tiles.totalSurfaceResizes)",
                ]
            },
            Source(prefixes: ["tile."]) {
                (controller.tiles.focusedView as? AutomatableTile)?.automationState() ?? [:]
            },
            Source([
                "palette", "palette.count", "palette.motion", "palette.visible", "palette.look", "palette.anims",
                "palette.typing", "palette.open.ms", "palette.close.ms", "palette.index", "palette.top",
                "palette.topkind", "palette.kinds", "palette.selected", "palette.sections", "palette.topstatus",
                "palette.topdetail", "palette.scrim", "palette.overdraw", "palette.selrow", "palette.footer",
                "palette.scroll", "banner", "banner.full", "badges",
                "tile.badge.setting", "sidebar.markers", "sidebar.machines", "machines", "workspace.machine",
                "sidebar.branches", "tab.markers", "dock", "focused.id", "tab.label", "agents", "header.agents",
                "focused.agent",
            ]) { ShellAutomation.state(app: app, controller: controller) },
            Source([
                "responder", "responder.tile", "responder.focused", "key.last", "key.context", "keys.resizing",
                "keywindow", "palette.query", "clipboard", "banner.shown", "banner.capped", "terminal.phase",
                "terminal.cwd", "ime.marked", "terminal.selection",
            ], prefixes: ["hints."]) { FocusAutomation.state(app: app, controller: controller) },
            Source(["drop.op", "drop.highlight", "drop.last", "drop.dir"]) {
                DropAutomation.state(controller: controller, tab: tab)
            },
            Source([
                "sidebar.mode", "sidebar.statuses", "sidebar.footer", "sidebar.numbers", "workspace.active",
                "sidebar.footer.words", "sidebar.rows", "sidebar.heights", "sidebar.overflow", "sidebar.pills",
                "sidebar.fades", "sidebar.active.visible", "sidebar.empty", "sidebar.card", "sidebar.tips",
                "sidebar.icons", "sidebar.icons.centred",
                "drag.last", "drag.landed", "drag.active", "drag.slot", "drag.rows", "drag.indicator",
            ], prefixes: ["rail.", "sidebar.files.", "sidebar.card."]) { SidebarAutomation.state(controller) },
            Source(["sidebar.hints", "sidebar.hints.pending", "sidebar.hints.fade"]) {
                SidebarAutomation.hintState(app: app, controller: controller)
            },
            Source(["sidebar.width", "sidebar.width.saved", "sidebar.edge", "sidebar.edge.steps", "sidebar.gap"]) {
                SidebarEdgeAutomation.state(controller)
            },
            Source(["theme", "terminal.attaches"], prefixes: ["theme."]) { ThemeAutomation.state(controller) },
            // After the `theme.` source, so asking for it computes only it.
            Source(["theme.colors"]) { ThemeAutomation.colors() },
            Source(["workspace.names"], prefixes: ["menu."]) { MenuAutomation.state(app: app, controller: controller) },
            Source(["tile.unread", "unread.count", "dock.badge", "dock.badge.setting", "menu.unread"]) {
                UnreadAutomation.state(app: app, controller: controller)
            },
            Source([
                "hud.flashes", "hud.last", "prompt.asks", "prompt.last", "tab.statuses", "tab.overflow",
                "tab.names", "tab.hint", "quickopen.chord", "quickopen.active", "palette.anchor",
            ], prefixes: ["chrome."]) { controller.chromeState() },
            Source(prefixes: ["picker."]) { LayoutPickerAutomation.state(controller) },
            Source(["tab.visual", "tab.selected", "tab.leftmost", "tab.session"], prefixes: ["tabdrag."]) {
                TabDragAutomation.state(app: app, controller: controller)
            },
            Source([
                "dividers", "hover", "cursor", "hint", "shape", "columns",
                "drag.commands", "drag.terminals", "drag.mid",
            ], prefixes: ["divider.", "rect."]) { DividerAutomation.state(controller) },
            Source([
                "mouse.target", "terminal.selection.text", "terminal.cursor", "terminal.mouse",
                "terminal.mouse.owner", "terminal.link", "terminal.opened", "terminal.refused",
            ]) { MouseAutomation.state(controller) },
        ]
    }

    /// Gives every tile of `tab` without a trace name the next one.
    private static func nameNewTiles(in tab: TabModel) {
        for tile in tab.tiles where traceNames[tile.id] == nil {
            traceNames[tile.id] = "t\(traceNames.count + 1)"
        }
    }

    /// A tile's name in traces and `expect order`: `t1` is the first tile
    /// seen, `t2` the next, and a name never moves to another tile while the
    /// layout order changes and tiles close.
    static func tileName(_ id: String, in tab: TabModel) -> String {
        nameNewTiles(in: tab)
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
            parts.append("o=\(p.map { $0.threeColumn.orientation ?? "columns" } ?? "?") m=\(p?.threeColumn.masters ?? -1)"
                + " r=\(String(format: "%.3f", p?.threeColumn.ratio ?? 0))")
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
        // The harness's watcher stops at the result: let it take the
        // captures still on their way first.
        guard pendingCaptures.isEmpty else {
            return awaitCaptures(timeout: 5) { self.finish() }
        }
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

/// One read of the automation state (`Automation.state()`): a source
/// runs the first time a key it names is asked for, then keeps its answer
/// for the rest of the read.
@MainActor
final class AutomationState {
    struct Source {
        let keys: Set<String>
        let prefixes: [String]
        let compute: () -> [String: String]

        init(_ keys: Set<String> = [], prefixes: [String] = [], _ compute: @escaping () -> [String: String]) {
            self.keys = keys
            self.prefixes = prefixes
            self.compute = compute
        }

        func names(_ key: String) -> Bool {
            keys.contains(key) || prefixes.contains { key.hasPrefix($0) }
        }
    }

    private let sources: [Source]
    private var computed: [Int: [String: String]] = [:]

    init(_ sources: [Source]) { self.sources = sources }

    private func values(_ i: Int) -> [String: String] {
        if let done = computed[i] { return done }
        let values = sources[i].compute()
        computed[i] = values
        return values
    }

    /// The last source to set `key` has it, as in one merged dictionary.
    subscript(key: String) -> String? {
        for i in sources.indices.reversed() where sources[i].names(key) {
            if let value = values(i)[key] { return value }
        }
        // Not where it's named (a key that's only sometimes set) or named
        // nowhere: every source has its say.
        for i in sources.indices.reversed() {
            if let value = values(i)[key] { return value }
        }
        return nil
    }
}
