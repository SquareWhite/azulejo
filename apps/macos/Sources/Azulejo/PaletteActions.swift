import AppKit

/// What the ⌘K quick-open menu lists, and what choosing a row does. Commands
/// come from the core's `commands.list` (the keymap's actions plus
/// user-facing methods, with current chords); jump targets are every
/// workspace, tab and tile in the session. Both sit in one list.
@MainActor
enum PaletteStore {
    /// `commands.list` entries by id, from the last time the menu opened.
    static var entries: [String: [String: Any]] = [:]

    /// The menu's own actions: it doesn't list itself.
    static let selfActions: Set<String> = ["quick_open", "command_palette", "goto"]

    /// Keymap contexts that are modes rather than tile kinds: their actions
    /// only make sense while the mode has the keyboard.
    static let modeContexts: Set<String> = ["palette", "find_bar", "resize"]

    /// Recently run rows (commands and places), newest first. A per-viewer
    /// convenience: lost with the app's settings, which is fine. Each dev
    /// instance keeps its own (`SettingsStore`).
    static func recents() -> [String] {
        let settings = SettingsStore.shared
        if let list = settings.stringArray(forKey: "quickOpen") { return list }
        // Before ⌘K there were two lists: start from both, places first.
        let goto = settings.stringArray(forKey: "paletteRecent.goto") ?? []
        let commands = settings.stringArray(forKey: "paletteRecent.commands") ?? []
        var merged: [String] = []
        for id in zip(goto, commands).flatMap({ [$0, $1] }) + goto + commands where !merged.contains(id) {
            merged.append(id)
        }
        return Array(merged.prefix(30))
    }

    static func remember(_ id: String) {
        var list = recents().filter { $0 != id }
        list.insert(id, at: 0)
        SettingsStore.shared.set(Array(list.prefix(30)), forKey: "quickOpen")
    }

    /// Clears this dev instance's recents (automation); other instances keep
    /// theirs. Refuses outside dev runs, so it never touches the installed
    /// app's settings.
    static func forgetDevRecents() -> Bool {
        let settings = SettingsStore.shared
        guard settings.isDevInstance else { return false }
        for name in ["quickOpen", "paletteRecent.goto", "paletteRecent.commands"] {
            settings.removeObject(forKey: name)
        }
        return true
    }
}

extension AppController {
    /// Opens the ⌘K menu with `query` in the field: `""` lists everything,
    /// `">"` commands only (the old ⌘⇧P), `"@"` places only (the old ⌘P).
    /// The same call while it's open in that scope closes it; from another
    /// scope it switches.
    func openQuickOpen(query: String = "", in controller: WindowController) {
        let palette = controller.palette
        if palette.isOpen {
            if palette.promptingFor == nil, palette.scope == PaletteView.Scope.of(query).scope {
                return palette.close()
            }
            palette.setQuery(query)
            controller.layoutChrome()
            return
        }
        let items = gotoItems(for: controller) + commandItems(for: controller)
        palette.onRun = { [weak self, weak controller] item, input in
            guard let self, let controller else { return }
            PaletteStore.remember(item.id)
            if item.kind.isCommand {
                self.runCommand(item, input: input, in: controller)
            } else {
                self.runGoto(item)
            }
        }
        palette.onClose = { [weak controller] in
            controller?.tiles.focusedView?.setFocused(true)
        }
        palette.chord = controller.quickOpen.chordText
        palette.open(items: items, recents: PaletteStore.recents(), query: query)
        controller.layoutChrome()
    }

    // MARK: Commands

    private func commandItems(for controller: WindowController) -> [PaletteItem] {
        guard case .success(let reply) = core.execute(["method": "commands.list"]),
              let list = reply["commands"] as? [[String: Any]]
        else { return [] }
        let tab = session?.window(controller.windowID)?.workspace?.tab
        let focused = tab.flatMap { $0.tile($0.focused) }
        PaletteStore.entries = [:]
        var items: [PaletteItem] = []
        for entry in list {
            guard let id = entry["id"] as? String, let title = entry["title"] as? String else { continue }
            // The menu doesn't list itself, nor the keys of modes that are
            // over once it runs (its own, find bars', resize mode's).
            if PaletteStore.selfActions.contains(id) { continue }
            let context = entry["context"] as? String
            if let context, PaletteStore.modeContexts.contains(context) { continue }
            PaletteStore.entries[id] = entry
            let planned = entry["planned"] as? Bool ?? false
            let needsTile = entry["needs_tile"] as? Bool ?? false
            let prompt = entry["prompt"] as? String
            var item = PaletteItem(
                id: id, title: title, kind: .command,
                chords: entry["chords"] as? [String] ?? [],
                prompt: prompt, searchExtra: id)
            item.category = entry["category"] as? String
            if planned {
                item.enabled = false
                item.note = "planned"
            } else if needsTile, focused == nil {
                item.enabled = false
                item.note = "needs a tile"
            } else if let context, let kind = focused?.kind.rawValue, kind != context {
                // A context's own keys (files_down…).
                item.enabled = false
                item.note = context == "files" ? "Files view only" : "\(context) tiles"
            }
            if id == "toggle_unread" {
                // Says what it'll do to the focused tile; agent tiles only.
                if let title = Self.unreadTitle(focused, menu: false) {
                    item.title = title
                } else if item.enabled {
                    item.enabled = false
                    item.note = "agent tiles"
                }
            }
            item.initialInput = initialInput(for: id, prompt: prompt, controller: controller)
            items.append(item)
        }
        return items
    }

    /// Prefills for two-step commands: the current name.
    private func initialInput(for id: String, prompt: String?, controller: WindowController) -> String? {
        guard prompt != nil, let ws = session?.window(controller.windowID)?.workspace else { return nil }
        let tile = ws.tab.flatMap { $0.tile($0.focused) }
        switch id {
        case "rename_workspace": return ws.name
        case "rename_tab": return ws.tab?.name
        case "rename_tile": return tile?.title
        default: return nil
        }
    }

    private func runCommand(_ item: PaletteItem, input: String?, in controller: WindowController) {
        guard let entry = PaletteStore.entries[item.id] else { return }
        let ws = session?.window(controller.windowID)?.workspace
        let value = input?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""

        // Renames and other prompted actions run as their method, with the input.
        switch entry["action"] as? String {
        case "rename_workspace"?:
            return report(send(["method": "workspace.rename",
                                "params": ["workspace": ws?.id ?? "", "name": value]]), controller)
        case "rename_tab"?:
            return report(send(["method": "tab.rename",
                                "params": ["tab": ws?.tab?.id ?? "", "name": value]]), controller)
        case "rename_tile"?:
            return report(send(["method": "tile.rename",
                                "params": ["tile": ws?.tab?.focused ?? "", "title": value]]), controller)
        case let action?:
            return perform(action: action, index: entry["index"] as? Int)
        case nil:
            break
        }

        guard let method = entry["method"] as? String else { return }
        var params: [String: Any] = [:]
        if let prompt = entry["prompt"] as? String {
            guard !value.isEmpty else { return controller.flash("\(item.title): nothing entered") }
            switch prompt {
            case "index":
                guard let n = Int(value), n >= 1 else { return controller.flash("a position from 1") }
                params["index"] = n - 1
            case "path":
                params["path"] = absolutePath(value, controller: controller)
            case "kind":
                // "Three Column" and "three-column" both mean three_column.
                params["kind"] = value.lowercased()
                    .replacingOccurrences(of: " ", with: "_")
                    .replacingOccurrences(of: "-", with: "_")
            default:
                params[prompt] = value
            }
        }
        report(send(["method": method, "params": params]), controller)
    }

    /// `~/x` and relative paths, resolved against the focused terminal's cwd,
    /// else the workspace's, else home.
    private func absolutePath(_ path: String, controller: WindowController) -> String {
        let expanded = (path as NSString).expandingTildeInPath
        if expanded.hasPrefix("/") { return expanded }
        let ws = session?.window(controller.windowID)?.workspace
        let base = ws?.tab.flatMap { $0.tile($0.focused) }?.cwd ?? ws?.cwd ?? NSHomeDirectory()
        return ((base as NSString).appendingPathComponent(expanded) as NSString).standardizingPath
    }

    private func report(_ result: Result<[String: Any], CoreError>, _ controller: WindowController) {
        if case .failure(let error) = result { controller.report(error) }
    }

    // MARK: Go to

    /// Every place in the session, in its order: each workspace, its tabs
    /// (when it has more than one, or the tab more than one tile) and their
    /// tiles. A tile with an agent is an agent row: named for where it is,
    /// its task as the detail, its state's icon and how long it's been in it.
    private func gotoItems(for controller: WindowController) -> [PaletteItem] {
        guard let session else { return [] }
        var items: [PaletteItem] = []
        let multiWindow = session.windows.count > 1
        let now = UInt64(max(0, Date().timeIntervalSince1970 * 1000))
        for (w, window) in session.windows.enumerated() {
            let windowCrumb = multiWindow ? ["window \(w + 1)"] : []
            // This window's ⌘1…⌘8 (⌘9 the last), in the sidebar's order.
            let ordered = window.id == controller.windowID ? SidebarView.ordered(window, machines: machines) : []
            for ws in window.workspaces {
                // Remote workspaces carry their machine's name: a chip on
                // the row, and in what the query matches.
                let host = Self.machineLabel(ws.machineID, machines: machines)
                var row = PaletteItem(id: "ws:" + ws.id, title: ws.name, kind: .workspace)
                row.crumbs = windowCrumb
                row.machine = host
                if let branch = ws.git?.branch, !branch.isEmpty {
                    row.detail = branch
                    row.detailMono = true
                }
                if let n = ordered.firstIndex(where: { $0.id == ws.id }) {
                    row.keyHint = SidebarView.chord(n + 1, of: ordered.count)
                }
                row.searchExtra = [ws.cwd.map(SidebarView.tilde), host].compactMap { $0 }.joined(separator: " ")
                items.append(row)
                let tabCrumbs = windowCrumb + [ws.name]
                for tab in ws.tabs {
                    if ws.tabs.count > 1 || tab.tiles.count > 1 {
                        var row = PaletteItem(id: "tab:" + tab.id, title: tab.label, kind: .tab)
                        row.crumbs = tabCrumbs
                        row.machine = host
                        let n = tab.tiles.count
                        row.detail = "\(n) terminal\(n == 1 ? "" : "s")"
                        row.searchExtra = host ?? ""
                        items.append(row)
                    }
                    // Where a tile lives: its workspace, and its tab when there are more.
                    let where_ = ws.tabs.count > 1 ? tabCrumbs + [tab.label] : tabCrumbs
                    for tile in tab.tiles {
                        items.append(tileItem(tile, in: ws, tab: tab, crumbs: where_, windowCrumb: windowCrumb,
                                              host: host, now: now))
                    }
                }
            }
        }
        return items
    }

    private func tileItem(_ tile: TileModel, in ws: WorkspaceModel, tab: TabModel, crumbs: [String],
                          windowCrumb: [String], host: String?, now: UInt64) -> PaletteItem {
        var row = PaletteItem(id: "tile:" + tile.id, title: tile.header?.title ?? Self.tileTitle(tile),
                              kind: .tile(tile.kind))
        row.machine = host
        let doing = Self.terminalDetail(tile, in: ws)
        row.detail = doing?.text ?? ""
        row.detailMono = doing?.mono ?? false
        row.crumbs = crumbs
        if let agent = tile.agent {
            // Named for where it is: the workspace, and the tab when there are more.
            if ws.tabs.count > 1 {
                row.crumbs = windowCrumb + [ws.name]
                row.title = tab.label
            } else {
                row.crumbs = windowCrumb
                row.title = ws.name
            }
            if let task = Self.task(tile) {
                row.detail = task
                row.detailMono = false
            }
            switch agent.status {
            case .blocked:
                row.status = .blocked
                row.bucket = .blocked
            case _ where agent.attention:
                row.status = .done
                row.bucket = .attention
            case .working:
                row.status = .working
                row.bucket = .working
            default:
                row.status = .idle
            }
            row.since = agent.sinceMs ?? 0
            if row.bucket != nil, let since = agent.sinceMs, since > 0, since <= now {
                row.wait = Self.waitText(ms: now - since)
            }
        }
        row.searchExtra = [tile.agent?.name, tile.header?.title, Self.tileTitle(tile),
                           tile.command.map { Self.withoutEnv($0.joined(separator: " ")) },
                           tile.terminalCwd.map(SidebarView.tilde), host]
            .compactMap { $0 }.joined(separator: " ")
        return row
    }

    /// What a terminal is doing, for its row: the foreground command
    /// (leading `VAR=value` words dropped: they can hold anything), else
    /// where it is, relative to the workspace's folder (nothing when it's
    /// that folder; never a full path).
    static func terminalDetail(_ tile: TileModel, in ws: WorkspaceModel) -> (text: String, mono: Bool)? {
        if tile.ended == true { return ("session ended", false) }
        if let command = tile.pane?.command.map(withoutEnv), !command.isEmpty { return (command, true) }
        guard let cwd = tile.terminalCwd.map(trimSlash), !cwd.isEmpty else { return nil }
        if let base = ws.cwd.map(trimSlash), !base.isEmpty {
            if cwd == base { return nil }
            if cwd.hasPrefix(base + "/") { return (String(cwd.dropFirst(base.count + 1)), true) }
        }
        if cwd == NSHomeDirectory() { return ("~", true) }
        return ((cwd as NSString).lastPathComponent, true)
    }

    /// `command` without environment assignments: the `VAR=value` words
    /// before it (`TERM=x just dogfood` → `just dogfood`), and everything
    /// from the first one after it. A program that rewrites its own title
    /// can show its whole environment after its name (`claude TERM=…
    /// HOME=…`); an argument like `make CC=gcc` loses its tail too, which is
    /// the price of never showing an environment.
    static func withoutEnv(_ command: String) -> String {
        func isAssignment(_ word: Substring) -> Bool {
            guard let eq = word.firstIndex(of: "="), eq != word.startIndex,
                  let first = word.first, first == "_" || first.isLetter else { return false }
            return word[..<eq].allSatisfy { $0 == "_" || $0.isLetter || $0.isNumber }
        }
        var words = command.split(separator: " ", omittingEmptySubsequences: true)
        while let first = words.first, isAssignment(first) { words.removeFirst() }
        if let cut = words.firstIndex(where: isAssignment) { words.removeSubrange(cut...) }
        return words.joined(separator: " ")
    }

    /// An agent's task: the title its program set, less the spinner or
    /// state glyphs in front; nil when that's only the agent's own name or
    /// a path.
    static func task(_ tile: TileModel) -> String? {
        guard let raw = tile.pane?.title else { return nil }
        let title = String(raw.drop { !($0.isLetter || $0.isNumber) }).trimmingCharacters(in: .whitespaces)
        guard !title.isEmpty, !raw.hasPrefix("/"), !raw.hasPrefix("~") else { return nil }
        let lowered = title.lowercased()
        if let name = tile.agent?.name?.lowercased(), lowered == name || lowered == name + " code" { return nil }
        return title
    }

    /// Without a trailing `/` (a folder's).
    private static func trimSlash(_ path: String) -> String {
        path.count > 1 && path.hasSuffix("/") ? String(path.dropLast()) : path
    }

    /// "now", "4m", "1h 12m", "3d".
    static func waitText(ms: UInt64) -> String {
        let minutes = Int(ms / 60_000)
        if minutes < 1 { return "now" }
        if minutes < 60 { return "\(minutes)m" }
        let hours = minutes / 60
        if hours < 24 { return minutes % 60 == 0 ? "\(hours)h" : "\(hours)h \(minutes % 60)m" }
        return "\(hours / 24)d"
    }

    /// A remote machine's label for ⌘K rows; nil for Local. A machine herdr
    /// no longer lists shows its ID's start, as the sidebar does.
    static func machineLabel(_ id: String, machines: [MachineModel]) -> String? {
        if id == MachineModel.local { return nil }
        return machines.first(where: { $0.id == id })?.label ?? "machine \(id.prefix(8))"
    }

    /// The best name for a tile: its title, agent, command or cwd.
    static func tileTitle(_ tile: TileModel) -> String {
        if let title = tile.title, !title.isEmpty { return title }
        if let name = tile.agent?.name, !name.isEmpty { return name }
        if let cmd = tile.command, !cmd.isEmpty { return cmd.joined(separator: " ") }
        if let cwd = tile.cwd { return (cwd as NSString).abbreviatingWithTildeInPath }
        return "terminal"
    }

    private func runGoto(_ item: PaletteItem) {
        let parts = item.id.split(separator: ":", maxSplits: 1).map(String.init)
        guard parts.count == 2 else { return }
        switch parts[0] {
        case "ws": send(["method": "workspace.focus", "params": ["workspace": parts[1]]])
        case "tab": send(["method": "tab.focus", "params": ["tab": parts[1]]])
        default: send(["method": "tile.focus", "params": ["tile": parts[1]]])
        }
        // The target may live in another window: bring that one forward.
        refresh()
        if !env.background, let id = session?.focusedWindow, let c = controller(id: id),
           !c.window.isKeyWindow {
            c.window.makeKeyAndOrderFront(nil)
        }
    }
}
