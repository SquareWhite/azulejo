import AppKit

/// The menu bar. Azulejo items run keymap actions by name, and show the
/// user's current chord for them (from the core's keymap, so config overrides
/// show up). The key router consumes bound chords before menus see them, so
/// the shortcuts shown here never fire twice; standard items (⌘Q, ⌘C, ⌘V, …)
/// only fire for chords the keymap leaves unbound.
@MainActor
enum MainMenu {
    static func build(app: AppController, keymap: [String: Any]) -> NSMenu {
        var chords: [String: String] = [:]
        for binding in keymap["bindings"] as? [[String: Any]] ?? [] {
            guard let action = binding["action"] as? String, binding["index"] is NSNull || binding["index"] == nil,
                  let chord = binding["chord"] as? String, chords[action] == nil
            else { continue }
            chords[action] = chord
        }
        let target = MenuTarget.shared
        target.app = app

        func item(_ title: String, _ action: String) -> NSMenuItem {
            let item = NSMenuItem(title: title, action: #selector(MenuTarget.run(_:)), keyEquivalent: "")
            item.target = target
            item.representedObject = action
            if let chord = chords[action] { apply(chord, to: item) }
            return item
        }
        func std(_ title: String, _ selector: Selector?, _ key: String, _ mods: NSEvent.ModifierFlags = .command) -> NSMenuItem {
            let item = NSMenuItem(title: title, action: selector, keyEquivalent: key)
            item.keyEquivalentModifierMask = mods
            return item
        }
        func menu(_ title: String, _ items: [NSMenuItem]) -> NSMenuItem {
            let top = NSMenuItem(title: title, action: nil, keyEquivalent: "")
            let sub = NSMenu(title: title)
            items.forEach(sub.addItem)
            top.submenu = sub
            return top
        }

        let main = NSMenu()
        main.addItem(menu("Azulejo", [
            std("About Azulejo", #selector(NSApplication.orderFrontStandardAboutPanel(_:)), ""),
            .separator(),
            item("Settings…", "open_config"),
            {
                let i = std("Install Command-Line Tool…", #selector(InstallCLIAction.install(_:)), "")
                i.target = InstallCLIAction.shared
                return i
            }(),
            .separator(),
            std("Hide Azulejo", #selector(NSApplication.hide(_:)), "h"),
            std("Hide Others", #selector(NSApplication.hideOtherApplications(_:)), "h", [.command, .option]),
            std("Show All", #selector(NSApplication.unhideAllApplications(_:)), ""),
            .separator(),
            std("Quit Azulejo", #selector(NSApplication.terminate(_:)), "q"),
        ]))
        // ⌘W closes the workspace and ⌘⇧W the tab (keymap), so Close Window
        // has no chord of its own unless `close_window` is bound.
        let closeWindow = chords["close_window"] == nil
            ? std("Close Window", #selector(MenuTarget.closeWindow(_:)), "")
            : item("Close Window", "close_window")
        closeWindow.target = target
        main.addItem(menu("File", [
            item("New Window", "new_window"),
            item("New Workspace", "new_workspace"),
            item("New Tab", "new_tab"),
            .separator(),
            item("Close Workspace", "close_workspace"),
            item("Close Tab", "close_tab"),
            item("Close Tile", "close_tile"),
            item("Kill Tile…", "kill_tile"),
            closeWindow,
        ]))
        main.addItem(menu("Edit", [
            std("Undo", Selector(("undo:")), "z"),
            std("Redo", Selector(("redo:")), "z", [.command, .shift]),
            .separator(),
            std("Cut", #selector(NSText.cut(_:)), "x"),
            std("Copy", #selector(NSText.copy(_:)), "c"),
            std("Paste", #selector(NSText.paste(_:)), "v"),
            std("Select All", #selector(NSText.selectAll(_:)), "a"),
            .separator(),
            item("Find…", "find"),
        ]))
        main.addItem(menu("View", [
            item("Toggle Sidebar", "toggle_sidebar"),
            item("Zoom Tile", "zoom_tile"),
            .separator(),
            item("Quick Open…", "quick_open"),
            item("Commands…", "command_palette"),
            item("Go To…", "goto"),
        ]))
        main.addItem(menu("Layout", [
            item("Split Right", "split_right"),
            item("Split Down", "split_down"),
            item("New Tile in Stack (Tall)", "split_auto"),
            .separator(),
            item("Promote to Master", "promote_tile"),
            item("Rotate Layout", "rotate_layout"),
            item("Equalize Stack", "equalize_layout"),
            .separator(),
            item("Resize Mode", "resize_mode"),
            item("Next Tile", "cycle_tile_next"),
            item("Previous Tile", "cycle_tile_previous"),
            .separator(),
            item("Rename Tile…", "rename_tile"),
            item("Rename Tab…", "rename_tab"),
            item("Rename Workspace…", "rename_workspace"),
        ]))
        let windowMenu = menu("Window", [
            std("Minimize", #selector(NSWindow.performMiniaturize(_:)), "m"),
            std("Zoom", #selector(NSWindow.performZoom(_:)), ""),
            .separator(),
            item("Next Workspace", "next_workspace"),
            item("Previous Workspace", "previous_workspace"),
            item("Last Workspace", "switch_workspace_last"),
            item("Previous Active Workspace", "last_workspace"),
            .separator(),
            item("Next Tab", "next_tab"),
            item("Previous Tab", "previous_tab"),
            item("Last Tab", "switch_tab_last"),
            item("Previous Active Tab", "last_tab"),
            .separator(),
            std("Bring All to Front", #selector(NSApplication.arrangeInFront(_:)), ""),
        ])
        main.addItem(windowMenu)
        NSApp.windowsMenu = windowMenu.submenu
        return main
    }

    /// Shows a keymap chord (`"ctrl+cmd+v"`) as the item's key equivalent.
    static func apply(_ chord: String, to item: NSMenuItem) {
        var parts = chord.split(separator: "+").map(String.init)
        guard let key = parts.popLast(), let equivalent = keyEquivalent(key) else { return }
        var mods: NSEvent.ModifierFlags = []
        for part in parts {
            switch part {
            case "ctrl": mods.insert(.control)
            case "alt": mods.insert(.option)
            case "cmd", "super": mods.insert(.command)
            case "shift": mods.insert(.shift)
            default: return
            }
        }
        item.keyEquivalent = equivalent
        item.keyEquivalentModifierMask = mods
    }

    private static func keyEquivalent(_ key: String) -> String? {
        if key.count == 1 { return key }
        let named: [String: String] = [
            "enter": "\r", "tab": "\t", "space": " ", "escape": "\u{1b}",
            "minus": "-", "equal": "=", "bracketleft": "[", "bracketright": "]",
            "backslash": "\\", "semicolon": ";", "quote": "'", "grave": "`",
            "comma": ",", "period": ".", "slash": "/", "backspace": "\u{8}",
        ]
        if let n = named[key] { return n }
        let function: [String: Int] = [
            "up": NSUpArrowFunctionKey, "down": NSDownArrowFunctionKey,
            "left": NSLeftArrowFunctionKey, "right": NSRightArrowFunctionKey,
        ]
        return function[key].flatMap { UnicodeScalar($0).map { String(Character($0)) } }
    }
}

/// Menu items' target: forwards to the app controller.
@MainActor
final class MenuTarget: NSObject {
    static let shared = MenuTarget()
    weak var app: AppController?

    @objc func run(_ sender: NSMenuItem) {
        guard let action = sender.representedObject as? String else { return }
        app?.perform(action: action)
    }

    @objc func closeWindow(_ sender: NSMenuItem) {
        if let controller = app?.focusedController { app?.closeWindow(controller) }
    }
}
