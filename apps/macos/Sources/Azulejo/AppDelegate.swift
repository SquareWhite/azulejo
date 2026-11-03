import AppKit

/// Owns the core, one `WindowController` per app window, the key router and
/// the menus. Every change goes through the core; the UI redraws from
/// `session.get` after events (coalesced to one fetch per run-loop turn).
@MainActor
final class AppController: NSObject, NSApplicationDelegate {
    let core: Core
    let env: LaunchEnvironment
    private(set) var session: SessionModel?
    private var controllers: [String: WindowController] = [:]
    private var keys: KeyRouter!
    private var refreshScheduled = false
    private var automation: Automation?
    private var signalSources: [DispatchSourceSignal] = []
    /// For last_workspace / last_tab: what was active before the current one.
    private var previousWorkspace: [String: String] = [:]
    private var previousTab: [String: String] = [:]
    /// Set once launch finishes; windows created after that show themselves.
    private var launched = false

    init(core: Core, env: LaunchEnvironment) {
        self.core = core
        self.env = env
        super.init()
        keys = KeyRouter(app: self)
    }

    // MARK: Lifecycle

    func applicationDidFinishLaunching(_ notification: Notification) {
        if let pidfile = env.pidfile {
            try? "\(getpid())\n".write(toFile: pidfile, atomically: true, encoding: .utf8)
        }
        log("core: \(core.info())")
        installSignalHandlers()
        NSApp.mainMenu = MainMenu.build(app: self, keymap: core.keymap())
        keys.install()
        BrowserTileView.register(app: self)
        PreviewTileView.install(core: core, log: { [weak self] in self?.log($0) })
        TerminalTileView.install(core: core, log: { [weak self] in self?.log($0) })
        core.onEvent = { [weak self] json in
            PreviewTileView.route(json)
            TerminalTileView.route(json)
            self?.scheduleRefresh()
        }
        installWakeObservers()
        refresh()
        if session?.windows.isEmpty ?? true {
            send(["method": "window.create"])
            refresh()
        }
        let focused = session?.focusedWindow.flatMap { controllers[$0] }
        for controller in controllers.values where controller !== focused {
            controller.show(activate: false)
        }
        focused?.show(activate: !env.background)
        if !env.background { NSApp.activate() }
        launched = true
        let tileCount = session?.windows.reduce(0) { n, w in
            n + w.workspaces.reduce(0) { m, ws in m + ws.tabs.reduce(0) { $0 + $1.tiles.count } }
        } ?? 0
        Startup.launched(window: focused?.window, tileCount: tileCount) { [weak self] in self?.log($0) }
        LatencyBench.startIfRequested(log: { [weak self] in self?.log($0) })

        if let script = env.automation {
            automation = Automation(app: self, script: script, out: env.automationOut)
            automation?.start()
        }
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        controllers.values.forEach { $0.saveFrame() }
        return .terminateNow
    }

    func applicationWillTerminate(_ notification: Notification) {
        // Process exit skips deinit: stop the core here so state is flushed and
        // the API socket removed. herdr panes keep running.
        core.shutdown()
        if let pidfile = env.pidfile { try? FileManager.default.removeItem(atPath: pidfile) }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    /// After sleep, screen lock or fast user switching, attach processes may
    /// have died or stalled: check every terminal tile (PLAN 1.12).
    private func installWakeObservers() {
        let center = NSWorkspace.shared.notificationCenter
        let names: [Notification.Name] = [
            NSWorkspace.didWakeNotification,
            NSWorkspace.screensDidWakeNotification,
            NSWorkspace.sessionDidBecomeActiveNotification,
        ]
        for name in names {
            let reason = name.rawValue
            center.addObserver(forName: name, object: nil, queue: .main) { _ in
                MainActor.assumeIsolated { TerminalTileView.healthCheckAll(reason: reason) }
            }
        }
    }

    /// `dev.sh --stop` and `kill` send SIGTERM: quit properly so state is saved.
    private func installSignalHandlers() {
        for sig in [SIGTERM, SIGINT] {
            signal(sig, SIG_IGN)
            let source = DispatchSource.makeSignalSource(signal: sig, queue: .main)
            source.setEventHandler { NSApp.terminate(nil) }
            source.resume()
            signalSources.append(source)
        }
    }

    // MARK: Core

    @discardableResult
    func send(_ command: [String: Any]) -> Result<[String: Any], CoreError> {
        let result = core.execute(command)
        if case .failure(let error) = result {
            log("\(command["method"] ?? "?") failed: \(error)")
        }
        scheduleRefresh()
        return result
    }

    private func scheduleRefresh() {
        guard !refreshScheduled else { return }
        refreshScheduled = true
        DispatchQueue.main.async { [weak self] in
            self?.refreshScheduled = false
            self?.refresh()
        }
    }

    /// Fetches the session and brings every window up to date.
    func refresh() {
        guard let new = core.fetch("session.get", field: "session", as: SessionModel.self) else { return }
        recordHistory(old: session, new: new)
        session = new
        let ids = Set(new.windows.map(\.id))
        for (id, controller) in controllers where !ids.contains(id) {
            controller.window.orderOut(nil)
            controller.tiles.apply(WindowModel(id: id, workspaces: [], activeWorkspace: "", frame: nil))
            controllers[id] = nil
        }
        for window in new.windows {
            let controller: WindowController
            if let existing = controllers[window.id] {
                controller = existing
                controller.apply(window)
            } else {
                controller = WindowController(windowID: window.id, app: self)
                controllers[window.id] = controller
                controller.apply(window)
                // Windows created after launch (cmd+n, `azj window create`).
                if launched {
                    controller.show(activate: !env.background)
                }
            }
        }
    }

    private func recordHistory(old: SessionModel?, new: SessionModel) {
        guard let old else { return }
        for window in new.windows {
            guard let before = old.window(window.id) else { continue }
            if before.activeWorkspace != window.activeWorkspace {
                previousWorkspace[window.id] = before.activeWorkspace
            }
            for ws in window.workspaces {
                guard let beforeWs = before.workspaces.first(where: { $0.id == ws.id }),
                      let was = beforeWs.activeTab, was != ws.activeTab
                else { continue }
                previousTab[ws.id] = was
            }
        }
    }

    // MARK: Windows

    func controller(for window: NSWindow?) -> WindowController? {
        if let window, let found = controllers.values.first(where: { $0.window === window }) {
            return found
        }
        // Background runs have no key window: act on the core's focused one.
        return session?.focusedWindow.flatMap { controllers[$0] } ?? controllers.values.first
    }

    var focusedController: WindowController? { controller(for: NSApp.keyWindow) }

    func owns(_ window: NSWindow) -> Bool {
        controllers.values.contains { $0.window === window }
    }

    /// Automation: keyDowns the monitor has seen, and a way in when it can't.
    var keysSeen: Int { keys.seen }
    func routeDirectly(_ event: NSEvent) {
        // Unbound chords go on to the focused tile, as AppKit would deliver them.
        if !keys.handle(event) { event.window?.firstResponder?.keyDown(with: event) }
    }

    /// The controller for a session window, for automation.
    func controller(id: String) -> WindowController? { controllers[id] }

    func windowBecameKey(_ controller: WindowController) {
        if session?.focusedWindow != controller.windowID {
            send(["method": "window.focus", "params": ["window": controller.windowID]])
        }
    }

    func windowResignedKey(_ controller: WindowController) {
        keys.reset()
        controller.setStatus(nil)
    }

    /// The red button and close_window: closing the last window quits instead
    /// (the session is saved and comes back on relaunch); other windows close
    /// in the core, detaching their terminals.
    func closeWindow(_ controller: WindowController) {
        if controllers.count <= 1 {
            NSApp.terminate(nil)
        } else {
            send(["method": "window.close", "params": ["window": controller.windowID]])
        }
    }

    // MARK: Actions

    /// Runs a resolved keymap action (`azj_core_key` / `azj_core_action` JSON).
    func perform(_ outcome: [String: Any], in controller: WindowController) {
        if let command = outcome["command"] as? [String: Any] {
            if case .failure(let error) = send(command) {
                controller.flash(error.message)
            }
            return
        }
        let name = outcome["action"] as? String ?? ""
        let index = outcome["index"] as? Int
        guard let window = session?.window(controller.windowID) else { return }
        let workspaces = window.workspaces
        let ws = window.workspace
        let tabs = ws?.tabs ?? []

        func focusWorkspace(_ id: String?) {
            guard let id else { return controller.flash("no such workspace") }
            send(["method": "workspace.focus", "params": ["workspace": id]])
        }
        func focusTab(_ id: String?) {
            guard let id else { return controller.flash("no such tab") }
            send(["method": "tab.focus", "params": ["tab": id]])
        }
        func step<T>(_ items: [T], from current: Int?, by delta: Int) -> T? {
            guard !items.isEmpty else { return nil }
            let i = current ?? 0
            return items[((i + delta) % items.count + items.count) % items.count]
        }
        let wsIndex = workspaces.firstIndex { $0.id == window.activeWorkspace }
        let tabIndex = tabs.firstIndex { $0.id == ws?.tab?.id }

        switch name {
        case "switch_workspace":
            focusWorkspace(index.flatMap { workspaces.indices.contains($0 - 1) ? workspaces[$0 - 1].id : nil })
        case "last_workspace":
            focusWorkspace(previousWorkspace[window.id].flatMap { id in workspaces.first { $0.id == id }?.id })
        case "next_workspace": focusWorkspace(step(workspaces, from: wsIndex, by: 1)?.id)
        case "previous_workspace": focusWorkspace(step(workspaces, from: wsIndex, by: -1)?.id)
        case "switch_tab":
            focusTab(index.flatMap { tabs.indices.contains($0 - 1) ? tabs[$0 - 1].id : nil })
        case "last_tab":
            focusTab(ws.flatMap { w in previousTab[w.id] }.flatMap { id in tabs.first { $0.id == id }?.id })
        case "next_tab": focusTab(step(tabs, from: tabIndex, by: 1)?.id)
        case "previous_tab": focusTab(step(tabs, from: tabIndex, by: -1)?.id)
        case "cycle_tile_next", "cycle_tile_previous":
            guard let tab = ws?.tab else { return }
            let order = tab.order
            let next = step(order, from: order.firstIndex(of: tab.focused), by: name.hasSuffix("next") ? 1 : -1)
            if let next { send(["method": "tile.focus", "params": ["tile": next]]) }
        case "new_window":
            send(["method": "window.create"])
        case "close_window":
            closeWindow(controller)
        case "toggle_sidebar":
            // Saved in the core so it comes back on relaunch.
            send(["method": "window.set_sidebar",
                  "params": ["window": controller.windowID, "visible": !controller.sidebarVisible]])
        case "resize_mode":
            keys.enterResizeMode(in: controller)
        case "rename_tile", "rename_tab", "rename_workspace":
            rename(name, in: controller)
        case "open_config":
            openConfig(in: controller)
        case "command_palette": controller.flash("Command palette lands in step 2.2")
        case "goto": controller.flash("Go to lands with the palette (2.2)")
        case "find": controller.flash("Find lands with the terminal tile (1.7)")
        case "edit_scrollback": controller.flash("Edit scrollback isn't wired yet")
        case "reload_config": controller.flash("Live config reload lands in step 2.3")
        default:
            controller.flash("\(name) isn't wired yet")
        }
    }

    /// Runs an action by config name (menus).
    func perform(action name: String, index: Int? = nil) {
        guard let controller = focusedController else { return }
        var request: [String: Any] = ["action": name]
        if let index { request["index"] = index }
        let outcome = core.action(request)
        guard outcome["outcome"] as? String == "action" else {
            return controller.flash("unknown action \(name)")
        }
        perform(outcome, in: controller)
    }

    private func rename(_ action: String, in controller: WindowController) {
        guard !env.background else { return controller.flash("rename needs the app in front") }
        guard let window = session?.window(controller.windowID), let ws = window.workspace else { return }
        let (title, current, build): (String, String?, (String) -> [String: Any]) = switch action {
        case "rename_workspace":
            ("Rename workspace", ws.name, { ["method": "workspace.rename", "params": ["workspace": ws.id, "name": $0]] })
        case "rename_tab":
            ("Rename tab", ws.tab?.name, { ["method": "tab.rename", "params": ["tab": ws.tab?.id ?? "", "name": $0]] })
        default:
            ("Rename tile", ws.tab.flatMap { $0.tile($0.focused)?.title },
             { ["method": "tile.rename", "params": ["tile": ws.tab?.focused ?? "", "title": $0]] })
        }
        let alert = NSAlert()
        alert.messageText = title
        alert.addButton(withTitle: "Rename")
        alert.addButton(withTitle: "Cancel")
        let field = NSTextField(string: current ?? "")
        field.frame = NSRect(x: 0, y: 0, width: 260, height: 24)
        alert.accessoryView = field
        alert.window.initialFirstResponder = field
        alert.beginSheetModal(for: controller.window) { [weak self] response in
            guard response == .alertFirstButtonReturn else { return }
            self?.send(build(field.stringValue))
        }
    }

    private func openConfig(in controller: WindowController) {
        guard let path = core.info()["config_path"] as? String else {
            return controller.flash("no config file (defaults only)")
        }
        guard !env.background else { return controller.flash("config: \(path)") }
        let url = URL(fileURLWithPath: path)
        if !FileManager.default.fileExists(atPath: path) {
            try? FileManager.default.createDirectory(
                at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? "# Azulejo config. See docs/keybindings.md.\n".write(to: url, atomically: true, encoding: .utf8)
        }
        NSWorkspace.shared.open(url)
    }

    // MARK: Logging

    func log(_ line: String) {
        if env.verbose { FileHandle.standardError.write(Data("azulejo: \(line)\n".utf8)) }
        automation?.note(line)
    }
}
