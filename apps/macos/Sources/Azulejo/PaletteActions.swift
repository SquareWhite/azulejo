import AppKit

/// What the palette (⌘⇧P) and picker (⌘P) list, and what choosing a row
/// does. Commands come from the core's `commands.list` (the keymap's actions
/// plus user-facing methods, with current chords); the picker lists every
/// workspace, tab and tile in the session.
@MainActor
enum PaletteStore {
    /// `commands.list` entries by id, from the last time the palette opened.
    static var entries: [String: [String: Any]] = [:]

    /// Recently run rows, newest first, per mode. A per-viewer convenience:
    /// lost with the app's defaults, which is fine.
    static func recents(_ mode: PaletteView.Mode) -> [String] {
        UserDefaults.standard.stringArray(forKey: key(mode)) ?? []
    }

    static func remember(_ id: String, _ mode: PaletteView.Mode) {
        var list = recents(mode).filter { $0 != id }
        list.insert(id, at: 0)
        UserDefaults.standard.set(Array(list.prefix(30)), forKey: key(mode))
    }

    /// Dev runs share the installed app's defaults domain: keep their
    /// recents apart so test runs never reorder the user's palette.
    private static func key(_ mode: PaletteView.Mode) -> String {
        let dev = ProcessInfo.processInfo.environment["AZULEJO_PROFILE"] == "dev" ? "dev." : ""
        return "\(dev)paletteRecent.\(mode.rawValue)"
    }
}

extension AppController {
    func openPalette(_ mode: PaletteView.Mode, in controller: WindowController) {
        let palette = controller.palette
        if palette.isOpen, palette.mode == mode { return palette.close() }
        let items = mode == .commands ? commandItems(for: controller) : gotoItems(for: controller)
        palette.onRun = { [weak self, weak controller] item, input in
            guard let self, let controller else { return }
            PaletteStore.remember(item.id, mode)
            if mode == .commands {
                self.runCommand(item, input: input, in: controller)
            } else {
                self.runGoto(item)
            }
        }
        palette.onClose = { [weak controller] in
            controller?.tiles.focusedView?.setFocused(true)
        }
        palette.open(mode, items: items, recents: PaletteStore.recents(mode))
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
            // The palette doesn't list itself.
            if id == "command_palette" { continue }
            PaletteStore.entries[id] = entry
            let planned = entry["planned"] as? Bool ?? false
            let needsTile = entry["needs_tile"] as? Bool ?? false
            let prompt = entry["prompt"] as? String
            var item = PaletteItem(
                id: id, title: title,
                detail: entry["category"] as? String ?? "",
                chords: entry["chords"] as? [String] ?? [],
                prompt: prompt, searchExtra: id)
            if planned {
                item.enabled = false
                item.note = "planned"
            } else if needsTile, focused == nil {
                item.enabled = false
                item.note = "needs a tile"
            }
            item.initialInput = initialInput(for: id, prompt: prompt, controller: controller)
            items.append(item)
        }
        return items
    }

    /// Prefills for two-step commands: the current name, URL or path.
    private func initialInput(for id: String, prompt: String?, controller: WindowController) -> String? {
        guard prompt != nil, let ws = session?.window(controller.windowID)?.workspace else { return nil }
        let tile = ws.tab.flatMap { $0.tile($0.focused) }
        switch id {
        case "rename_workspace": return ws.name
        case "rename_tab": return ws.tab?.name
        case "rename_tile": return tile?.title
        case "browser.open": return tile?.kind == .browser ? tile?.url : nil
        case "preview.open": return tile?.kind == .preview ? tile?.path : nil
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
            case "url":
                params["url"] = value
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
        if case .failure(let error) = result { controller.flash(error.message) }
    }

    // MARK: Go to

    private func gotoItems(for controller: WindowController) -> [PaletteItem] {
        guard let session else { return [] }
        var items: [PaletteItem] = []
        let multiWindow = session.windows.count > 1
        for (w, window) in session.windows.enumerated() {
            let where0 = multiWindow ? "window \(w + 1) · " : ""
            for ws in window.workspaces {
                let blocked = ws.tabs.flatMap(\.tiles).filter { $0.agent?.status == .blocked }.count
                items.append(PaletteItem(
                    id: "ws:" + ws.id, title: ws.name,
                    detail: where0 + "workspace · \(ws.tabs.count) tab\(ws.tabs.count == 1 ? "" : "s")",
                    searchExtra: ws.cwd ?? "",
                    dot: blocked > 0 ? AttentionStyle.blocked : nil))
                for tab in ws.tabs {
                    if ws.tabs.count > 1 || tab.tiles.count > 1 {
                        items.append(PaletteItem(
                            id: "tab:" + tab.id, title: tab.label,
                            detail: "\(where0)tab in \(ws.name)"))
                    }
                    for tile in tab.tiles {
                        items.append(PaletteItem(
                            id: "tile:" + tile.id, title: Self.tileTitle(tile),
                            detail: "\(where0)\(tile.kind.rawValue) in \(ws.name)",
                            searchExtra: [tile.cwd, tile.url, tile.path, tile.root, tile.agent?.name,
                                          tile.command?.joined(separator: " ")]
                                .compactMap { $0 }.joined(separator: " "),
                            dot: AttentionStyle.color(for: tile)))
                    }
                }
            }
        }
        return items
    }

    /// The best name for a tile: its title, page title, agent, cwd, URL or path.
    static func tileTitle(_ tile: TileModel) -> String {
        for candidate in [tile.title, tile.pageTitle] {
            if let c = candidate, !c.isEmpty { return c }
        }
        switch tile.kind {
        case .terminal:
            if let name = tile.agent?.name, !name.isEmpty { return name }
            if let cmd = tile.command, !cmd.isEmpty { return cmd.joined(separator: " ") }
            if let cwd = tile.cwd { return (cwd as NSString).abbreviatingWithTildeInPath }
            return "terminal"
        case .browser: return tile.url ?? "browser"
        case .preview: return tile.path.map { ($0 as NSString).lastPathComponent } ?? "preview"
        case .files: return tile.root.map { ($0 as NSString).abbreviatingWithTildeInPath } ?? "files"
        }
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
