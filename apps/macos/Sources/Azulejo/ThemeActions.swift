import AppKit
import AzulejoCore

// "Set theme…" in ⌘K, View → Theme, and the automation verbs for themes
// (docs/themes.md). Choosing a theme saves `[appearance] theme` in the
// config file through the core; the reload that follows applies it.

extension Core {
    /// The themes and the `[appearance]` choice (`azj_core_theme`).
    func theme() -> [String: Any] {
        callJSON([:]) { handle, _ in azj_core_theme(handle) }
    }

    /// Saves `theme` (an id or "auto") in the config file and reloads:
    /// `{"ok":true,…}` or `{"ok":false,"message":…}`.
    func setTheme(_ theme: String) -> [String: Any] {
        guard let data = try? JSONSerialization.data(withJSONObject: theme, options: .fragmentsAllowed),
              let json = String(data: data, encoding: .utf8)
        else { return ["ok": false, "message": "bad theme name"] }
        return callJSON([:]) { handle, _ in azj_core_set_theme(handle, json) }
    }
}

extension AppController {
    /// Reads the core's themes and applies the choice (launch, config reloads).
    func loadTheme() {
        let json = core.theme()
        Theme.load(json)
        // Not a colour, but `[appearance]` too: the agent badge in headers.
        AttentionStyle.tileBadge = json["tile_agent_badge"] as? Bool ?? false
    }

    /// Saves `id` (or "auto") as the theme. Shows the problem when the
    /// config can't take it, and goes back to the saved theme then.
    func chooseTheme(_ id: String, in controller: WindowController?) {
        let reply = core.setTheme(id)
        if reply["ok"] as? Bool == true {
            // The core has the new config already: apply it now, then drop
            // the preview (same theme, so nothing redraws twice).
            loadTheme()
            Theme.preview(nil)
            log("theme: chose \(id)")
        } else {
            Theme.preview(nil)
            let message = reply["message"] as? String ?? "couldn't save the theme"
            controller?.flash(message)
            log("theme: \(message)")
        }
    }

    /// The ⌘K list of themes ("Set theme…"): moving through it previews
    /// each one live; Enter saves it, Esc goes back to the one before.
    func openThemePicker(in controller: WindowController) {
        let palette = controller.palette
        if palette.isOpen { palette.close() }
        let choice = Theme.choice
        let current = choice.auto ? "auto" : choice.theme
        var items: [PaletteItem] = [
            PaletteItem(
                id: "theme:auto", title: "Auto", kind: .theme,
                detail: "follows the system: \(Theme.theme(choice.light)?.name ?? choice.light) or "
                    + "\(Theme.theme(choice.dark)?.name ?? choice.dark)",
                note: current == "auto" ? "current" : nil,
                searchExtra: "system light dark auto"),
        ]
        for spec in Theme.themes {
            let origin = spec.builtIn ? "built-in" : "yours"
            items.append(PaletteItem(
                id: "theme:" + spec.id, title: spec.name, kind: .theme,
                detail: "\(spec.appearance.rawValue) · \(origin)",
                note: spec.id == current ? "current" : nil,
                searchExtra: spec.id))
        }
        palette.onRun = { [weak self, weak controller] item, _ in
            guard let self, item.id.hasPrefix("theme:") else { return }
            self.chooseTheme(String(item.id.dropFirst("theme:".count)), in: controller)
        }
        palette.onClose = { [weak controller] in
            controller?.tiles.focusedView?.setFocused(true)
        }
        palette.openList(id: "themes", items: items, selectedID: "theme:" + current,
                         placeholder: "Set theme…   ↑↓ previews · ↩ keeps it · esc goes back")
        palette.onSelect = { item in
            guard item.id.hasPrefix("theme:") else { return }
            Theme.preview(String(item.id.dropFirst("theme:".count)))
        }
        palette.onCancel = { Theme.preview(nil) }
        controller.layoutChrome()
    }
}

// MARK: - View → Theme

@MainActor
enum ThemeMenu {
    /// View → Theme: Auto, then every theme, the current one ticked
    /// (`MenuTarget.validateMenuItem`). Rebuilt with the menus on each
    /// config reload, so new user themes show up. `picker` is the keymap's
    /// `set_theme` item, so it shows the user's chord when they bind one.
    static func item(target: MenuTarget, picker: NSMenuItem) -> NSMenuItem {
        let top = NSMenuItem(title: "Theme", action: nil, keyEquivalent: "")
        let menu = NSMenu(title: "Theme")
        let auto = NSMenuItem(title: "Auto (Follow System)", action: #selector(MenuTarget.chooseTheme(_:)),
                              keyEquivalent: "")
        auto.representedObject = "auto"
        auto.target = target
        menu.addItem(auto)
        menu.addItem(.separator())
        var builtIn = true
        for spec in Theme.themes {
            if builtIn, !spec.builtIn {
                builtIn = false
                menu.addItem(.separator())
            }
            let item = NSMenuItem(title: spec.name, action: #selector(MenuTarget.chooseTheme(_:)),
                                  keyEquivalent: "")
            item.representedObject = spec.id
            item.target = target
            menu.addItem(item)
        }
        menu.addItem(.separator())
        menu.addItem(picker)
        top.submenu = menu
        return top
    }

    /// The tick: the saved choice (Auto when following the system).
    static func isCurrent(_ id: String) -> Bool {
        let choice = Theme.choice
        return id == (choice.auto ? "auto" : choice.theme)
    }
}

extension MenuTarget: NSMenuItemValidation {
    @objc func chooseTheme(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String else { return }
        app?.chooseTheme(id, in: app?.focusedController)
    }

    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        if item.action == #selector(chooseTheme(_:)), let id = item.representedObject as? String {
            item.state = ThemeMenu.isCurrent(id) ? .on : .off
        }
        return true
    }
}

// MARK: - Automation

/// `theme set ID` (save, as View → Theme does), `theme preview ID|none`,
/// `theme system light|dark|real` (what `auto` takes the system's appearance
/// to be: the app's own idea, never the Mac's setting), `theme picker`
/// (⌘K's "Set theme…"). Keys: `theme` (the theme showing), `theme.choice`
/// (`auto` or an id), `theme.appearance`, `theme.window` (the window's
/// appearance: dark / light), `theme.terminals` (theme / ghostty),
/// `theme.terminal.updates` (config swaps so far), `theme.sidebar` (the
/// card's material or `flat`), `theme.tracked` (views that follow it),
/// `terminal.attaches` (terminal surfaces started: attaches and reattaches).
@MainActor
enum ThemeAutomation {
    static func run(_ arg: String, app: AppController?) -> String? {
        guard let app else { return "no app" }
        let parts = arg.split(separator: " ", maxSplits: 1).map(String.init)
        let rest = parts.count > 1 ? parts[1] : ""
        switch parts.first ?? "" {
        case "set":
            let reply = app.core.setTheme(rest)
            guard reply["ok"] as? Bool == true else { return "theme set: \(reply["message"] ?? "failed")" }
            app.loadTheme()
        case "preview":
            Theme.preview(rest == "none" ? nil : rest)
        case "system":
            switch rest {
            case "light": Theme.systemOverride = .light
            case "dark": Theme.systemOverride = .dark
            case "real": Theme.systemOverride = nil
            default: return "theme system light|dark|real"
            }
        case "picker":
            guard let controller = app.focusedController else { return "no window" }
            app.openThemePicker(in: controller)
        default:
            return "theme set|preview|system|picker"
        }
        return nil
    }

    static func state(_ controller: WindowController?) -> [String: String] {
        var s = [
            "theme": Theme.current.id,
            "theme.choice": Theme.choice.auto ? "auto" : Theme.choice.theme,
            "theme.appearance": Theme.current.appearance.rawValue,
            "theme.terminals": Theme.terminal == nil ? "ghostty" : "theme",
            "theme.terminal.updates": "\(TerminalRuntime.shared?.configUpdates ?? 0)",
            "theme.tracked": "\(Theme.trackedCount)",
            // Attaches and PTY resizes, so a switch can show it caused neither.
            "terminal.attaches": "\(TerminalTileView.attaches)",
        ]
        if let window = controller?.window {
            let name = window.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua])
            s["theme.window"] = name == .darkAqua ? "dark" : "light"
        }
        s["theme.sidebar"] = controller?.sidebarMaterial ?? "flat"
        if let controller { s["theme.regions"] = regions(controller) }
        return s
    }

    /// Where the ⌘K menu and a visible find bar are, in points from the
    /// window's top left ("palette=x,y,w,h;find=x,y,w,h"), so a script can
    /// sample their pixels in a capture.
    private static func regions(_ controller: WindowController) -> String {
        guard let content = controller.window.contentView else { return "" }
        func frame(_ view: NSView) -> String {
            let r = view.convert(view.bounds, to: content)
            let top = content.isFlipped ? r.minY : content.bounds.height - r.maxY
            return "\(Int(r.minX)),\(Int(top)),\(Int(r.width)),\(Int(r.height))"
        }
        var parts: [String] = []
        if controller.palette.isOpen { parts.append("palette=" + frame(controller.palette)) }
        func findBar(in view: NSView) -> FindBar? {
            if let bar = view as? FindBar, !bar.isHiddenOrHasHiddenAncestor, bar.frame.height > 1 { return bar }
            for sub in view.subviews { if let bar = findBar(in: sub) { return bar } }
            return nil
        }
        if let bar = findBar(in: content) { parts.append("find=" + frame(bar)) }
        return parts.isEmpty ? "-" : parts.joined(separator: ";")
    }
}
