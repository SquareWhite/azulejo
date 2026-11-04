import AppKit

/// What a right-click menu's item acts on: its workspace row or its tile,
/// whether or not that's the focused one.
enum ActionTarget: Equatable {
    case workspace(String)
    case tile(String)
}

/// Renames and confirmations. Automation queues answers (`menu answer …`)
/// so scripts run them without a sheet in a background run.
@MainActor
enum Prompts {
    /// An answer that cancels the prompt.
    static let cancel = "\u{0}cancel"
    static var answers: [String] = []

    static func takeAnswer() -> String? { answers.isEmpty ? nil : answers.removeFirst() }

    /// A destructive confirmation as a sheet on `controller`'s window.
    static func confirm(_ message: String, info: String, button: String, in controller: WindowController,
                        background: Bool, done: @escaping (Bool) -> Void) {
        if let answer = takeAnswer() { return done(answer != cancel && answer != "no") }
        guard !background else {
            controller.flash("\(button) needs the app in front")
            return done(false)
        }
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = message
        alert.informativeText = info
        alert.addButton(withTitle: button).hasDestructiveAction = true
        alert.addButton(withTitle: "Cancel")
        alert.beginSheetModal(for: controller.window) { done($0 == .alertFirstButtonReturn) }
    }
}

/// A right-click menu item. Items that stand for a keymap action run it
/// through `AppController.perform(action:target:)`, the path the keymap, the
/// menu bar and ⌘K take, and show its chord from the core's catalogue.
@MainActor
final class ContextMenuItem: NSMenuItem {
    /// The keymap action it runs, if any.
    var actionName: String?
    /// The bound chord as the keymap spells it (`"cmd+shift+w"`).
    var chord: String?
    var run: () -> Void = {}

    init(_ title: String, enabled: Bool = true, run: @escaping () -> Void) {
        super.init(title: title, action: #selector(fire(_:)), keyEquivalent: "")
        target = self
        isEnabled = enabled
        self.run = run
    }

    @available(*, unavailable)
    required init(coder: NSCoder) { fatalError() }

    @objc func fire(_ sender: Any?) { run() }
}

/// The right-click menus on workspace rows (sidebar, Agents view) and tile
/// header strips (the browser merges these into its chrome's menu). Standard
/// NSMenus: native, following the system appearance. They never handle a
/// right-click inside a terminal's body: libghostty and mouse-reporting apps
/// own that.
@MainActor
enum ContextMenus {
    static weak var app: AppController?

    /// Automation: the last string a Copy item wrote (to the private
    /// pasteboard in automation runs) and the last path Reveal showed.
    static var lastCopied: String?
    static var lastRevealed: String?

    /// Automation runs never open Finder and never touch the clipboard.
    static var automating: Bool {
        ProcessInfo.processInfo.environment["AZULEJO_AUTOMATION"].map { !$0.isEmpty } ?? false
    }

    // MARK: Workspace rows

    static func workspaceMenu(_ id: String, in controller: WindowController) -> NSMenu? {
        guard let app, let ws = app.session?.windows.flatMap(\.workspaces).first(where: { $0.id == id })
        else { return nil }
        let chords = catalogueChords(app)
        let local = ws.machineID == MachineModel.local
        let folder = folder(of: ws)
        let target = ActionTarget.workspace(id)
        let menu = NSMenu(title: ws.name)
        menu.autoenablesItems = false

        menu.addItem(action("Rename…", "rename_workspace", target, chords, controller))
        menu.addItem(item("New Tab", "new_tab", chords) {
            // Show the workspace the tab lands in.
            app.send(["method": "workspace.focus", "params": ["workspace": id]])
            app.perform(action: "new_tab", target: target, in: controller)
        })
        if let folder {
            menu.addItem(.separator())
            if local {
                menu.addItem(ContextMenuItem("Reveal Folder in Finder") { reveal(folder) })
                menu.addItem(ContextMenuItem("Open Folder in Files Tile") {
                    openFiles(folder, in: ws, app: app, controller: controller, chords: chords)
                })
            }
            menu.addItem(ContextMenuItem("Copy Path") { copy(folder) })
        }
        menu.addItem(.separator())
        let close = action("Close Workspace", "close_workspace", target, chords, controller)
        close.toolTip = "Detaches: herdr keeps its tabs and panes running"
        menu.addItem(close)
        return menu
    }

    /// The folder a workspace is about: its repository, else its cwd.
    static func folder(of ws: WorkspaceModel) -> String? {
        ws.repo ?? ws.git?.repo ?? ws.cwd
    }

    /// A files tile on `folder`, right of the workspace's focused tile (a
    /// new tab when it has none), through `split_right` / `new_tab`.
    private static func openFiles(_ folder: String, in ws: WorkspaceModel, app: AppController,
                                  controller: WindowController, chords: [String: String]) {
        app.send(["method": "workspace.focus", "params": ["workspace": ws.id]])
        let content: [String: Any] = ["kind": "files", "root": folder]
        if let tab = ws.tab {
            app.perform(action: "split_right", target: .tile(tab.focused), extra: ["content": content], in: controller)
        } else {
            app.perform(action: "new_tab", target: .workspace(ws.id), extra: ["content": content], in: controller)
        }
    }

    // MARK: Tiles

    /// The tile header's menu.
    static func tileMenu(_ id: String, in controller: WindowController) -> NSMenu? {
        let items = tileItems(id, in: controller)
        guard !items.isEmpty else { return nil }
        let menu = NSMenu(title: "Tile")
        menu.autoenablesItems = false
        items.forEach(menu.addItem)
        return menu
    }

    /// Items for any tile kind; the browser appends them to its chrome menu.
    static func tileItems(_ id: String, in controller: WindowController) -> [NSMenuItem] {
        guard let app, let session = app.session,
              let ws = session.windows.flatMap(\.workspaces).first(where: { $0.tabs.contains { $0.tile(id) != nil } }),
              let tab = ws.tabs.first(where: { $0.tile(id) != nil }), let tile = tab.tile(id)
        else { return [] }
        let chords = catalogueChords(app)
        let target = ActionTarget.tile(id)
        let local = ws.machineID == MachineModel.local
        var items: [NSMenuItem] = []

        items.append(action("Split Right", "split_right", target, chords, controller))
        items.append(action("Split Down", "split_down", target, chords, controller))
        let zoom = action("Zoom", "zoom_tile", target, chords, controller)
        zoom.state = tab.zoomed == id ? .on : .off
        items.append(zoom)
        items.append(action("Rename…", "rename_tile", target, chords, controller))

        items.append(.separator())
        let title = (tile.header?.title).flatMap { $0.isEmpty ? nil : $0 } ?? AppController.tileTitle(tile)
        items.append(ContextMenuItem("Copy Title") { copy(title) })
        if tile.kind == .terminal, let cwd = tile.cwd {
            items.append(ContextMenuItem("Copy Working Directory") { copy(cwd) })
        }
        if local, let path = revealPath(tile) {
            items.append(ContextMenuItem("Reveal in Finder") { reveal(path) })
        }

        items.append(.separator())
        let close = action("Close Tile", "close_tile", target, chords, controller)
        if tile.kind == .terminal { close.toolTip = "Detaches: herdr keeps the pane running" }
        items.append(close)
        if tile.kind == .terminal, tile.herdr != nil, tile.ended != true {
            items.append(action("Kill Pane…", "kill_tile", target, chords, controller))
        }
        return items
    }

    /// What Reveal in Finder shows for a tile: a terminal's cwd, a preview's
    /// file, a files tile's root, a browser's local file.
    static func revealPath(_ tile: TileModel) -> String? {
        switch tile.kind {
        case .terminal: return tile.cwd
        case .preview: return tile.path
        case .files: return tile.root
        case .browser:
            guard let url = tile.url.flatMap(URL.init(string:)), url.isFileURL else { return nil }
            return url.path
        }
    }

    // MARK: Pieces

    /// An item for keymap action `name` on `target`, with its chord.
    private static func action(_ title: String, _ name: String, _ target: ActionTarget,
                               _ chords: [String: String], _ controller: WindowController) -> ContextMenuItem {
        item(title, name, chords) { [weak controller] in
            app?.perform(action: name, target: target, in: controller)
        }
    }

    private static func item(_ title: String, _ name: String, _ chords: [String: String],
                             run: @escaping () -> Void) -> ContextMenuItem {
        let item = ContextMenuItem(title, run: run)
        item.actionName = name
        if let chord = chords[name] {
            item.chord = chord
            MainMenu.apply(chord, to: item)
        }
        return item
    }

    /// Each catalogue entry's first chord, by id (`commands.list`, the list
    /// ⌘K shows), so user overrides show here too.
    private static func catalogueChords(_ app: AppController) -> [String: String] {
        guard case .success(let reply) = app.core.execute(["method": "commands.list"]),
              let list = reply["commands"] as? [[String: Any]]
        else { return [:] }
        var out: [String: String] = [:]
        for entry in list {
            if let id = entry["id"] as? String, let chord = (entry["chords"] as? [String])?.first {
                out[id] = chord
            }
        }
        return out
    }

    static func copy(_ text: String) {
        lastCopied = text
        TerminalRuntime.pasteboard.clearContents()
        TerminalRuntime.pasteboard.setString(text, forType: .string)
    }

    static func reveal(_ path: String) {
        lastRevealed = path
        app?.log("reveal \(path)")
        guard !automating else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }
}

/// Views that offer a right-click menu (automation builds it without a click).
@MainActor
protocol ContextMenuProviding: AnyObject {
    func contextMenu() -> NSMenu?
}

// MARK: - Automation

/// `menu workspace N` (the Nth sidebar row) and `menu tile` (the focused
/// tile's header, or a browser's chrome) build the menu a right-click
/// shows and log its items; `… invoke TITLE` then runs one through the
/// menu itself. `menu answer TEXT` (or `cancel`, `yes`, `no`) answers the
/// next rename or confirmation. State: `menu.items` (titles, `[chord]`,
/// `-` for separators, `(off)` for disabled), `menu.copied`,
/// `menu.revealed`, `workspace.names`. `menu shot NAME` (Automation)
/// captures the window with the last menu drawn open.
@MainActor
enum MenuAutomation {
    /// The last menu built, and where in the window it would open.
    static var last: (menu: NSMenu, anchor: NSRect)?

    static func run(_ arg: String, app: AppController?) -> String? {
        guard let app, let controller = app.focusedController else { return "no window" }
        var words = arg.split(separator: " ").map(String.init)
        guard let what = words.first else { return "menu needs workspace N, rail N, tile, answer or shot" }
        words.removeFirst()
        if what == "answer" {
            let text = words.joined(separator: " ")
            Prompts.answers.append(text == "cancel" ? Prompts.cancel : text)
            return nil
        }
        let built: (NSMenu, NSRect)?
        switch what {
        case "workspace":
            guard let n = words.first.flatMap(Int.init) else { return "menu workspace needs a row number" }
            words.removeFirst()
            let rows = controller.sidebarView.rows
            guard rows.indices.contains(n - 1) else { return "no row \(n)" }
            let row = rows[n - 1]
            built = row.contextMenu().map { ($0, row.convert(row.bounds, to: nil)) }
        case "rail":
            // A workspace row of the collapsed sidebar's rail.
            guard let n = words.first.flatMap(Int.init) else { return "menu rail needs a row number" }
            words.removeFirst()
            let rows = controller.railView.workspaceRows
            guard rows.indices.contains(n - 1) else { return "no rail row \(n)" }
            let row = rows[n - 1]
            built = row.contextMenu().map { ($0, row.convert(row.bounds, to: nil)) }
        case "tile":
            guard let view = controller.tiles.focusedView as? NSView & ContextMenuProviding else {
                return "the focused tile has no menu"
            }
            built = view.contextMenu().map { ($0, (view as? TileFrame)?.menuAnchor ?? view.convert(view.bounds, to: nil)) }
        default:
            return "menu needs workspace N, rail N, tile or answer"
        }
        guard let (menu, anchor) = built else { return "no menu" }
        last = (menu, anchor)
        app.log("menu \(what): \(describe(menu))")
        guard words.first == "invoke" else { return nil }
        let title = words.dropFirst().joined(separator: " ")
        guard let index = menu.items.firstIndex(where: { $0.title == title }) else {
            return "no item “\(title)” in \(describe(menu))"
        }
        guard menu.items[index].isEnabled else { return "“\(title)” is disabled" }
        menu.performActionForItem(at: index)
        return nil
    }

    static func describe(_ menu: NSMenu) -> String {
        menu.items.map { item in
            if item.isSeparatorItem { return "-" }
            var text = item.title
            if let chord = (item as? ContextMenuItem)?.chord { text += " [\(chord)]" }
            if item.state == .on { text += " (on)" }
            if !item.isEnabled { text += " (off)" }
            return text
        }.joined(separator: "|")
    }

    static func state(app: AppController, controller: WindowController) -> [String: String] {
        [
            "menu.items": last.map { describe($0.menu) } ?? "-",
            "menu.copied": ContextMenus.lastCopied ?? "-",
            "menu.revealed": ContextMenus.lastRevealed ?? "-",
            "workspace.names": app.session?.window(controller.windowID)?.workspaces
                .map(\.name).joined(separator: "|") ?? "-",
        ]
    }
}

/// Draws a menu as macOS shows it, for automation shots: popping a real one
/// up would put a window in front of the user's apps and grab their mouse.
/// Built from the same NSMenu (titles, chords, separators, states, enabled),
/// in the system menu font and colours of the window's appearance.
@MainActor
enum MenuRender {
    static func image(of menu: NSMenu, appearance: NSAppearance) -> NSImage {
        let font = NSFont.menuFont(ofSize: 0)
        let rowHeight: CGFloat = 22, separatorHeight: CGFloat = 11, inset: CGFloat = 5
        let leading: CGFloat = 22, trailing: CGFloat = 16, gap: CGFloat = 28
        let attrs: [NSAttributedString.Key: Any] = [.font: font]
        var titleWidth: CGFloat = 0, chordWidth: CGFloat = 0, height = inset * 2
        for item in menu.items {
            height += item.isSeparatorItem ? separatorHeight : rowHeight
            titleWidth = max(titleWidth, (item.title as NSString).size(withAttributes: attrs).width)
            chordWidth = max(chordWidth, (glyphs(item) as NSString).size(withAttributes: attrs).width)
        }
        let width = max(180, ceil(leading + titleWidth + (chordWidth > 0 ? gap + chordWidth : 0) + trailing))
        let size = NSSize(width: width, height: height)
        let image = NSImage(size: size, flipped: true) { rect in
            appearance.performAsCurrentDrawingAppearance {
                let shape = NSBezierPath(roundedRect: rect.insetBy(dx: 0.5, dy: 0.5), xRadius: 8, yRadius: 8)
                NSColor.windowBackgroundColor.blended(withFraction: 0.04, of: .textColor)?.setFill()
                shape.fill()
                NSColor.separatorColor.setStroke()
                shape.lineWidth = 1
                shape.stroke()
                var y = inset
                for item in menu.items {
                    if item.isSeparatorItem {
                        NSColor.separatorColor.setFill()
                        NSRect(x: 10, y: y + separatorHeight / 2, width: width - 20, height: 1).fill()
                        y += separatorHeight
                        continue
                    }
                    let colour: NSColor = item.isEnabled ? .labelColor : .tertiaryLabelColor
                    let text: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: colour]
                    let baseline = y + (rowHeight - font.boundingRectForFont.height) / 2
                    if item.state == .on {
                        ("✓" as NSString).draw(at: NSPoint(x: 7, y: baseline), withAttributes: text)
                    }
                    (item.title as NSString).draw(at: NSPoint(x: leading, y: baseline), withAttributes: text)
                    let chord = glyphs(item)
                    if !chord.isEmpty {
                        let muted: [NSAttributedString.Key: Any] = [
                            .font: font, .foregroundColor: item.isEnabled ? NSColor.secondaryLabelColor : .tertiaryLabelColor,
                        ]
                        let w = (chord as NSString).size(withAttributes: muted).width
                        (chord as NSString).draw(at: NSPoint(x: width - trailing - w, y: baseline), withAttributes: muted)
                    }
                    y += rowHeight
                }
            }
            return true
        }
        return image
    }

    /// The key equivalent as a menu shows it: ⌃⌥⇧⌘ then the key.
    static func glyphs(_ item: NSMenuItem) -> String {
        guard !item.keyEquivalent.isEmpty else { return "" }
        let mods = item.keyEquivalentModifierMask
        var out = ""
        if mods.contains(.control) { out += "⌃" }
        if mods.contains(.option) { out += "⌥" }
        if mods.contains(.shift) { out += "⇧" }
        if mods.contains(.command) { out += "⌘" }
        let named: [String: String] = ["\r": "↩", "\t": "⇥", " ": "Space", "\u{1b}": "⎋", "\u{8}": "⌫"]
        let key = item.keyEquivalent
        if let n = named[key] { return out + n }
        if let scalar = key.unicodeScalars.first, scalar.value >= 0xF700 {
            let arrows: [UInt32: String] = [0xF700: "↑", 0xF701: "↓", 0xF702: "←", 0xF703: "→"]
            return out + (arrows[scalar.value] ?? "")
        }
        return out + key.uppercased()
    }
}
