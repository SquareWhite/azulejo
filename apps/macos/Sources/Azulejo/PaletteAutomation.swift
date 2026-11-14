import AppKit

/// Automation verbs and `expect` keys for the palette, the config banner and
/// attention badges (dev runs only; see `Automation`).
///
///     palette open commands|goto   open the palette or the picker
///     palette type TEXT            set the search text (as if typed)
///     palette key CHORD            hand a navigation key to the palette
///     palette close
///     seen-visible on|off          treat background windows as on screen
///                                  (so focused tiles get `tile.seen`)
///     sh COMMAND                   run a shell command and wait; `{herdr}`,
///                                  `{session}`, `{config}`, `{pane}` (focused
///                                  tile's herdr pane), `{pane:N}` (Nth tile in
///                                  reading order), `{window}` (window number)
///                                  and `{out}` (automation output) are filled in
///
/// Keys: `palette` (closed / commands / goto / prompt:<id>), `palette.top`,
/// `palette.count`, `palette.selected`, `banner` (first line or `none`),
/// `badges` (`ABCD=blocked!,…` or `none`), `agents` (each tile's agent in
/// reading order: `blocked!,none`), `sidebar.markers` / `tab.markers` (`●2|-`),
/// `tab.label`, `dock`.
@MainActor
enum ShellAutomation {
    static func run(_ verb: String, _ arg: String, app: AppController?) -> String? {
        guard let app, let controller = app.focusedController else { return "no window" }
        let arg = fill(arg, app: app, controller: controller)
        switch verb {
        case "palette":
            let parts = arg.split(separator: " ", maxSplits: 1).map(String.init)
            let rest = parts.count > 1 ? parts[1] : ""
            switch parts.first ?? "" {
            case "open":
                guard let mode = PaletteView.Mode(rawValue: rest) else { return "palette open commands|goto" }
                if !controller.palette.isOpen || controller.palette.mode != mode {
                    app.openPalette(mode, in: controller)
                }
            case "type":
                guard controller.palette.isOpen else { return "palette isn't open" }
                controller.palette.setQuery(rest)
                controller.layoutChrome()
            case "key":
                guard controller.palette.isOpen else { return "palette isn't open" }
                if !controller.palette.handle(chord: rest) { return "palette ignored \(rest)" }
                controller.layoutChrome()
            case "close":
                controller.palette.close()
            default:
                return "palette open|type|key|close"
            }
        case "seen-visible":
            AttentionTracker.assumeVisible = arg != "off"
            app.scheduleRefresh()
        case "sh":
            return shell(arg)
        default:
            return "unknown verb \(verb)"
        }
        return nil
    }

    private static func fill(_ command: String, app: AppController, controller: WindowController) -> String {
        let info = app.core.info()
        var out = command
        let tab = app.session?.window(controller.windowID)?.workspace?.tab
        func pane(_ id: String?) -> String { id.flatMap { tab?.tile($0)?.herdr?.paneId } ?? "?" }
        out = out.replacingOccurrences(of: "{herdr}", with: info["herdr_bin"] as? String ?? "herdr")
        out = out.replacingOccurrences(of: "{session}", with: info["herdr_session"] as? String ?? "?")
        out = out.replacingOccurrences(of: "{config}", with: info["config_path"] as? String ?? "?")
        out = out.replacingOccurrences(of: "{pane}", with: pane(tab?.focused))
        out = out.replacingOccurrences(of: "{window}", with: "\(controller.window.windowNumber)")
        out = out.replacingOccurrences(
            of: "{out}", with: ProcessInfo.processInfo.environment["AZULEJO_AUTOMATION_OUT"] ?? "/tmp")
        for (i, id) in (tab?.order ?? []).enumerated() {
            out = out.replacingOccurrences(of: "{pane:\(i + 1)}", with: pane(id))
        }
        return out
    }

    private static func shell(_ command: String) -> String? {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/bin/sh")
        task.arguments = ["-c", command]
        var env = ProcessInfo.processInfo.environment
        for key in env.keys where key.hasPrefix("HERDR_") { env[key] = nil }
        task.environment = env
        let err = Pipe()
        task.standardError = err
        task.standardOutput = FileHandle.nullDevice
        do { try task.run() } catch { return "sh: \(error)" }
        task.waitUntilExit()
        guard task.terminationStatus == 0 else {
            let text = String(data: err.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
            return "sh exited \(task.terminationStatus): \(text.trimmingCharacters(in: .whitespacesAndNewlines))"
        }
        return nil
    }

    static func state(app: AppController, controller: WindowController) -> [String: String] {
        let palette = controller.palette
        var s: [String: String] = [:]
        if let id = palette.promptingFor {
            s["palette"] = "prompt:\(id)"
        } else {
            s["palette"] = palette.mode?.rawValue ?? "closed"
        }
        s["palette.count"] = "\(palette.results.count)"
        s["palette.top"] = palette.results.first?.item.title ?? "-"
        s["palette.selected"] = palette.results.indices.contains(palette.selected)
            ? palette.results[palette.selected].item.title : "-"
        s["banner"] = controller.banner.isHidden
            ? "none" : controller.banner.message.split(separator: "\n").first.map(String.init) ?? ""
        let badges = controller.attention.shown.sorted { $0.key < $1.key }.map { "\($0.key)=\($0.value)" }
        s["badges"] = badges.isEmpty ? "none" : badges.joined(separator: ",")
        func compact(_ markers: [String]) -> String {
            markers.map { $0.isEmpty ? "-" : $0.replacingOccurrences(of: " ", with: "") }.joined(separator: "|")
        }
        s["sidebar.markers"] = compact(controller.sidebarMarkers)
        s["tab.markers"] = compact(controller.tabMarkers)
        s["dock"] = NSApp.dockTile.badgeLabel ?? "none"
        // Which tile the badges belong to: the focused tile's short id.
        if let tab = app.session?.window(controller.windowID)?.workspace?.tab {
            s["focused.id"] = Short.id(tab.focused)
            s["tab.label"] = tab.label
            s["agents"] = tab.order.map { id in
                tab.tile(id)?.agent.map { $0.status.rawValue + ($0.attention ? "!" : "") } ?? "none"
            }.joined(separator: ",")
            s["focused.agent"] = tab.tile(tab.focused)?.agent.map {
                $0.status.rawValue + ($0.attention ? "!" : "")
            } ?? "none"
        }
        return s
    }
}
