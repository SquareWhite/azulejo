import AppKit

/// Automation verbs and `expect` keys for the ⌘K menu, the config banner and
/// attention badges (dev runs only; see `Automation`).
///
///     palette open [QUERY]         open the ⌘K menu (QUERY `>` or `@` scopes it)
///     palette forget               clear the dev profile's recents
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
///     machine-add LABEL            what "+" on that sidebar machine group does
///
/// Keys: `palette` (closed / all / commands / jump / prompt:<id>), `palette.top`,
/// `palette.topkind` (Command / Workspace / Tab / Terminal …), `palette.kinds`
/// (the distinct kinds in the results), `palette.count`, `palette.selected`,
/// `banner` (first line or `none`),
/// `badges` (`ABCD=blocked!,…` or `none`: agent state on tiles), `agents`
/// (each tile's agent in reading order: `blocked!,none`), `header.agents`
/// (what each tile's header strip carries, in reading order: `badge`, `dot`
/// (a narrow tile's badge), `blocked` (the blocked icon) or `none`),
/// `tile.badge.setting` (`[appearance] tile_agent_badge`: `on` / `off`), `sidebar.markers` / `tab.markers` (`●2|-`),
/// `tab.label`, `dock`, `sidebar.machines` (group headers: `Local ●|dev-server ●
/// Reconnecting…`), `machines` (`Local=connected,dev-server=reconnecting`),
/// `workspace.machine` (the active workspace's machine label).
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
                if !controller.palette.isOpen || controller.palette.scope != PaletteView.Scope.of(rest).scope {
                    app.openQuickOpen(query: rest, in: controller)
                }
            case "type":
                guard controller.palette.isOpen else { return "palette isn't open" }
                controller.palette.setQuery(rest)
                controller.layoutChrome()
            case "key":
                // The chord through the keymap's `palette` context, as the
                // key router resolves it.
                guard controller.palette.isOpen else { return "palette isn't open" }
                let outcome = app.core.key(chord: rest, context: "palette")
                guard outcome["context"] as? String == "palette", let name = outcome["action"] as? String else {
                    return "palette ignored \(rest)"
                }
                controller.palette.perform(keyAction: name)
                controller.layoutChrome()
            case "close":
                controller.palette.close()
            case "select":
                // A row by id (the theme picker's `theme:<id>`), as if
                // arrowed to.
                guard controller.palette.isOpen else { return "palette isn't open" }
                if !controller.palette.select(id: rest) { return "no row \(rest)" }
            case "forget":
                // Fresh recents for a deterministic run (dev keys only).
                guard PaletteStore.forgetDevRecents() else { return "palette forget is dev-only" }
            default:
                return "palette open|type|key|close|forget"
            }
        case "seen-visible":
            AttentionTracker.assumeVisible = arg != "off"
            app.scheduleRefresh()
        case "sh":
            return shell(arg)
        case "machine-add":
            guard let machine = app.machines.first(where: { $0.label == arg || $0.id == arg }) else {
                return "no machine \(arg)"
            }
            app.newWorkspace(on: machine.id, window: controller.windowID)
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
        } else if let id = palette.listID {
            s["palette"] = "list:\(id)"
        } else {
            s["palette"] = palette.isOpen ? palette.scope.rawValue : "closed"
        }
        s["palette.count"] = "\(palette.results.count)"
        s["palette.index"] = palette.isOpen ? "\(palette.selected)" : "-"
        s["palette.top"] = palette.results.first?.item.title ?? "-"
        s["palette.topkind"] = palette.results.first?.item.kind.label ?? "-"
        var kinds: [String] = []
        for row in palette.results where !kinds.contains(row.item.kind.label) { kinds.append(row.item.kind.label) }
        s["palette.kinds"] = kinds.sorted().joined(separator: ",")
        s["palette.selected"] = palette.results.indices.contains(palette.selected)
            ? palette.results[palette.selected].item.title : "-"
        s["banner"] = controller.banner.isHidden
            ? "none" : controller.banner.message.split(separator: "\n").first.map(String.init) ?? ""
        s["banner.full"] = controller.banner.isHidden
            ? "none" : controller.banner.message.replacingOccurrences(of: "\n", with: " | ")
        let badges = controller.attention.shown.sorted { $0.key < $1.key }.map { "\($0.key)=\($0.value)" }
        s["badges"] = badges.isEmpty ? "none" : badges.joined(separator: ",")
        s["tile.badge.setting"] = AttentionStyle.tileBadge ? "on" : "off"
        func compact(_ markers: [String]) -> String {
            markers.map { $0.isEmpty ? "-" : $0.replacingOccurrences(of: " ", with: "") }.joined(separator: "|")
        }
        s["sidebar.markers"] = compact(controller.sidebarMarkers)
        s["sidebar.machines"] = controller.sidebarMachines.joined(separator: "|")
        s["machines"] = app.machines.map { "\($0.label)=\($0.state.rawValue)" }.joined(separator: ",")
        if let ws = app.session?.window(controller.windowID)?.workspace {
            s["workspace.machine"] = app.machines.first { $0.id == ws.machineID }?.label ?? ws.machineID
        }
        s["sidebar.branches"] = controller.sidebarBranches.map { $0.isEmpty ? "-" : $0 }.joined(separator: "|")
        s["tab.markers"] = compact(controller.tabMarkers)
        s["dock"] = NSApp.dockTile.badgeLabel ?? "none"
        // Which tile the badges belong to: the focused tile's short id.
        if let tab = app.session?.window(controller.windowID)?.workspace?.tab {
            s["focused.id"] = Short.id(tab.focused)
            s["tab.label"] = tab.label
            s["agents"] = tab.order.map { id in
                tab.tile(id)?.agent.map { $0.status.rawValue + ($0.attention ? "!" : "") } ?? "none"
            }.joined(separator: ",")
            let carried = controller.attention.inHeader
            s["header.agents"] = tab.order.map { carried[Short.id($0)] ?? "none" }.joined(separator: ",")
            s["focused.agent"] = tab.tile(tab.focused)?.agent.map {
                $0.status.rawValue + ($0.attention ? "!" : "")
            } ?? "none"
        }
        return s
    }
}
