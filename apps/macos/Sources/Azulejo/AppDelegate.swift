import AppKit

/// Owns the core, one `WindowController` per app window, the key router and
/// the menus. Every change goes through the core; the UI redraws from
/// `session.get` after events (coalesced to one fetch per run-loop turn).
@MainActor
final class AppController: NSObject, NSApplicationDelegate {
    let core: Core
    let env: LaunchEnvironment
    private(set) var session: SessionModel?
    /// herdr machines for the sidebar's groups (`machines.list`, ADR 0011).
    private(set) var machines: [MachineModel] = []
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
        DockIcon.install { [weak self] in self?.log($0) }
        NSApp.mainMenu = MainMenu.build(app: self, keymap: core.keymap())
        ContextMenus.app = self
        keys.install()
        BrowserTileView.register(app: self)
        // The theme before any view or terminal exists (docs/themes.md).
        Theme.log = { [weak self] in self?.log($0) }
        loadTheme()
        PreviewTileView.install(core: core, log: { [weak self] in self?.log($0) })
        FilesTileView.install(core: core, log: { [weak self] in self?.log($0) })
        TerminalTileView.install(core: core, log: { [weak self] in self?.log($0) })
        Motion.load(core.animation())
        Motion.log = { [weak self] in self?.log($0) }
        Perf.installRunLoopObservers()
        core.onEvent = { [weak self] json in
            Perf.noteEvent(json, moving: MotionStats.shared.moving)
            let start = CACurrentMediaTime()
            defer { Perf.record("event.route", start, CACurrentMediaTime()) }
            if json.contains("\"event\":\"machine") || json.contains("\"config.reloaded\"") {
                self?.machinesStale = true
            }
            // A machine connected: the core has rebound its tiles to the panes
            // herdr has now (a herdr restart replaces terminals). Take those
            // in before terminals reattach, so none attaches to a dead one.
            if json.contains("\"herdr.status\"") || json.contains("\"machine.status\"") {
                self?.refresh()
            }
            PreviewTileView.route(json)
            FilesTileView.route(json)
            TerminalTileView.route(json)
            if json.contains("\"config.reloaded\""), let core = self?.core {
                Motion.load(core.animation())
                // A saved theme, a theme file, or Ghostty's colours.
                self?.loadTheme()
            }
            self?.routeConfigReload(json)
            // Events trail the command that already refreshed the UI (a
            // layout change arrives as `layout.changed` just after); while
            // tiles move, they wait for the motion to end.
            self?.scheduleRefresh(event: true)
        }
        // Coming to the front (or leaving it) starts or stops a read (F11).
        for name in [NSApplication.didBecomeActiveNotification, NSApplication.didResignActiveNotification] {
            NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.scheduleRefresh() }
            }
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

    /// "+" on a sidebar machine group (ADR 0011): a workspace on that machine
    /// with a first terminal, ready to type in.
    func newWorkspace(on machine: String, window: String) {
        guard case .success(let reply) = send([
            "method": "workspace.create", "params": ["window": window, "machine": machine],
        ]), let workspace = reply["workspace"] as? String else { return }
        send(["method": "tab.create", "params": ["workspace": workspace]])
    }

    /// Brings the UI up to date on the next run-loop turn. `event`: the core
    /// reported a change. Mid-motion, that waits until the motion ends: a
    /// session fetch and apply on a frame the motion needs costs it a tick,
    /// and the change that started the motion is already on screen. A command
    /// from the user (`send`) always refreshes at once.
    func scheduleRefresh(event: Bool = false) {
        if event, MotionStats.shared.moving {
            guard !refreshDeferred else { return }
            refreshDeferred = true
            let delay = max(0, MotionStats.shared.movingUntil - CACurrentMediaTime()) + 0.03
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
                guard let self else { return }
                self.refreshDeferred = false
                self.scheduleRefresh(event: true)
            }
            return
        }
        guard !refreshScheduled else { return }
        refreshScheduled = true
        DispatchQueue.main.async { [weak self] in
            self?.refreshScheduled = false
            self?.refresh()
        }
    }

    /// The last `session.get` reply, so a refresh with nothing new skips the
    /// decode and every window's apply.
    private var sessionJSON: String?
    private var refreshDeferred = false
    /// `machines.list` changes only with `machine.*` events (and config reloads).
    private var machinesStale = true

    /// Fetches the session and brings every window up to date.
    func refresh() {
        let start = CACurrentMediaTime()
        defer { Perf.record("refresh", start, CACurrentMediaTime()) }
        Perf.noteRefresh(moving: MotionStats.shared.moving)
        var machinesChanged = false
        if machinesStale {
            machinesStale = false
            if let list = core.fetch("machines.list", field: "machines", as: [MachineModel].self) {
                TerminalTileView.noteMachines(list)
                if list != machines {
                    machines = list
                    machinesChanged = true
                }
            }
        }
        guard let fetched = core.fetchRaw(
            "session.get", field: "session", as: SessionModel.self,
            unlessEqualTo: machinesChanged ? nil : sessionJSON)
        else { return }
        guard let new = fetched.value else {
            // Nothing new: windows stay as they are; only "seen" can change
            // (the app came to the front).
            if fetched.raw == sessionJSON, let session { markSeen(session) }
            return
        }
        sessionJSON = fetched.raw
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
        Perf.measure("refresh.attention") { AttentionTracker.updateDock(new) }
        markSeen(new)
    }

    private func markSeen(_ session: SessionModel) {
        AttentionTracker.markSeen(
            session, controllers: Array(controllers.values), send: { send($0) },
            again: { [weak self] in
                guard let self, let session = self.session else { return }
                self.markSeen(session)
            })
    }

    /// `config.reloaded` (F25b): problems go in every window's banner; a clean
    /// reload clears it with a short note.
    private func routeConfigReload(_ json: String) {
        guard json.contains("\"config.reloaded\""),
              let data = json.data(using: .utf8),
              let event = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              event["event"] as? String == "config.reloaded",
              let payload = event["data"] as? [String: Any]
        else { return }
        log("config.reloaded applied=\(payload["applied"] ?? "?")")
        if payload["applied"] as? Bool == true {
            // New bindings: rebuild the menus' key equivalents.
            let keymap = core.keymap()
            NSApp.mainMenu = MainMenu.build(app: self, keymap: keymap)
            for controller in controllers.values { controller.refreshKeymap(keymap) }
        }
        for controller in controllers.values {
            if controller.banner.apply(reload: payload) { controller.flash("Config reloaded") }
            controller.layoutChrome()
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
    var lastKeyOutcome: String { keys.lastOutcome }
    var lastKeyContext: String { keys.lastContext }
    var keysResizing: Bool { keys.resizing }
    func routeDirectly(_ event: NSEvent) {
        keys.markRoutedDirectly(event)
        // Unbound chords go on as AppKit would deliver them: key equivalents,
        // the menus, then the first responder (KeyDelivery).
        if !keys.handle(event) { KeyDelivery.deliver(event, window: event.window ?? focusedController?.window) }
    }

    /// The controller for a session window, for automation.
    func controller(id: String) -> WindowController? { controllers[id] }

    func windowBecameKey(_ controller: WindowController) {
        if session?.focusedWindow != controller.windowID {
            send(["method": "window.focus", "params": ["window": controller.windowID]])
        }
        scheduleRefresh()
    }

    func windowResignedKey(_ controller: WindowController) {
        keys.reset()
        controller.palette.close()
        controller.setStatus(nil)
        // A read in progress there stops (F11).
        scheduleRefresh()
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
    /// `target` is the workspace or tile a right-click menu was opened on;
    /// without one, actions act on what's focused, as from the keyboard.
    /// `extra` adds params to the command (a files tile's content for "Open
    /// Folder in Files Tile").
    func perform(_ outcome: [String: Any], in controller: WindowController, target: ActionTarget? = nil,
                 extra: [String: Any] = [:]) {
        if var command = outcome["command"] as? [String: Any] {
            if target != nil || !extra.isEmpty {
                var params = command["params"] as? [String: Any] ?? [:]
                switch target {
                case .workspace(let id)?: params["workspace"] = id
                case .tile(let id)?: params["tile"] = id
                case .tab(let id)?: params["tab"] = id
                case nil: break
                }
                params.merge(extra) { $1 }
                command["params"] = params
            }
            if command["method"] as? String == "workspace.close" {
                let id = (command["params"] as? [String: Any])?["workspace"] as? String
                return closeWorkspace(id ?? session?.window(controller.windowID)?.activeWorkspace, in: controller)
            }
            if (command["params"] as? [String: Any])?["kill"] as? Bool == true {
                return confirmKill(command, in: controller)
            }
            switch send(command) {
            case .failure(let error):
                controller.report(error)
            case .success(let reply):
                // A new workspace from the keyboard gets a first terminal, as
                // "+" in the sidebar does: an empty one has nothing to type into.
                if command["method"] as? String == "workspace.create",
                   let workspace = reply["workspace"] as? String {
                    send(["method": "tab.create", "params": ["workspace": workspace]])
                }
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
        case "switch_workspace_last":
            // The last one in the sidebar, as ⌘9 in a browser.
            focusWorkspace(workspaces.last?.id)
        case "last_workspace":
            // The one that was active before this one.
            focusWorkspace(previousWorkspace[window.id].flatMap { id in workspaces.first { $0.id == id }?.id })
        case "next_workspace": focusWorkspace(step(workspaces, from: wsIndex, by: 1)?.id)
        case "previous_workspace": focusWorkspace(step(workspaces, from: wsIndex, by: -1)?.id)
        case "switch_tab":
            focusTab(index.flatMap { tabs.indices.contains($0 - 1) ? tabs[$0 - 1].id : nil })
        case "switch_tab_last":
            focusTab(tabs.last?.id)
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
        case "set_theme":
            openThemePicker(in: controller)
        case "toggle_sidebar":
            // Saved in the core so it comes back on relaunch.
            send(["method": "window.set_sidebar",
                  "params": ["window": controller.windowID, "visible": !controller.sidebarVisible]])
        case "resize_mode":
            keys.enterResizeMode(in: controller)
        case "rename_tile", "rename_tab", "rename_workspace":
            rename(name, in: controller, target: target)
        case "open_config":
            openConfig(in: controller)
        case "quick_open": openQuickOpen(in: controller)
        case "command_palette": openQuickOpen(query: ">", in: controller)
        case "goto": openQuickOpen(query: "@", in: controller)
        case "find":
            // The focused tile's find bar (terminal, browser, preview) or
            // filter (files).
            guard let findable = (controller.tiles.focusedView as? TileFrame)?.content as? Findable else {
                return controller.flash("nothing to find in here")
            }
            if let problem = findable.beginFind() { controller.flash(problem) }
        case "edit_scrollback": controller.flash("Edit scrollback isn't wired yet")
        case "reload_config":
            // The core reads the file and sends `config.reloaded` (banner).
            let reply = core.reloadConfig()
            if reply["path"] == nil { controller.flash("no config file (defaults only)") }
        case "quit": NSApp.terminate(nil)
        case "hide": NSApp.hide(nil)
        case "hide_others": NSApp.hideOtherApplications(nil)
        case "minimize": controller.window.performMiniaturize(nil)
        default:
            // Context-only actions (browser_back, files_toggle_hidden…) from a
            // menu or quick open: the focused tile runs them, if it can.
            if outcome["context"] is String {
                if KeyScopes.focusedTile(in: controller)?.perform(keyAction: name, event: nil) != true {
                    controller.flash("\(name) doesn't apply here")
                }
                return
            }
            controller.flash("\(name) isn't wired yet")
        }
    }

    /// Runs an action by config name (menus). `target`: see `perform(_:in:target:)`.
    func perform(action name: String, index: Int? = nil, target: ActionTarget? = nil,
                 extra: [String: Any] = [:], in controller: WindowController? = nil) {
        guard let controller = controller ?? focusedController else { return }
        var request: [String: Any] = ["action": name]
        if let index { request["index"] = index }
        let outcome = core.action(request)
        guard outcome["outcome"] as? String == "action" else {
            return controller.flash("unknown action \(name)")
        }
        perform(outcome, in: controller, target: target, extra: extra)
    }

    /// `close_workspace` (⌘W): closes the workspace and detaches its
    /// terminals, with no confirmation since nothing is killed (F14). The
    /// app's last workspace gets a fresh one in its place first, so the
    /// window stays; another window's last workspace closes that window.
    private func closeWorkspace(_ id: String?, in controller: WindowController) {
        guard let id, let session,
              let window = session.windows.first(where: { $0.workspaces.contains { $0.id == id } })
        else { return controller.flash("no such workspace") }
        if session.windows.count == 1, window.workspaces.count == 1,
           case .success(let reply) = send(["method": "workspace.create", "params": ["window": window.id]]),
           let fresh = reply["workspace"] as? String {
            send(["method": "tab.create", "params": ["workspace": fresh]])
        }
        if case .failure(let error) = send(["method": "workspace.close", "params": ["workspace": id]]) {
            controller.flash(error.message)
        }
    }

    /// `kill_tile`: ends the pane's process in herdr, so it asks first.
    private func confirmKill(_ command: [String: Any], in controller: WindowController) {
        let params = command["params"] as? [String: Any]
        let tileID = params?["tile"] as? String ?? session?.window(controller.windowID)?.workspace?.tab?.focused
        let tile = session?.windows.lazy.flatMap(\.workspaces).flatMap(\.tabs)
            .compactMap { $0.tile(tileID ?? "") }.first
        let name = tile.map(Self.tileTitle) ?? "this tile"
        let run = { [weak self, weak controller] in
            guard let self, case .failure(let error) = self.send(command) else { return }
            controller?.flash(error.message)
        }
        // Only terminals have a process to kill; anything else just closes.
        guard tile?.kind == .terminal, tile?.ended != true else { return run() }
        Prompts.confirm(
            "Kill “\(name)”?",
            info: "Its process ends and herdr closes the pane. Close Tile keeps it running instead.",
            button: "Kill", in: controller, background: env.background
        ) { if $0 { run() } }
    }

    private func rename(_ action: String, in controller: WindowController, target: ActionTarget?) {
        guard let window = session?.window(controller.windowID) else { return }
        // A right-click menu's workspace or tile, else what's focused.
        let all = session?.windows.flatMap(\.workspaces) ?? []
        let ws: WorkspaceModel?
        var tileID: String?
        var tabID: String?
        switch target {
        case .workspace(let id)?:
            ws = all.first { $0.id == id }
        case .tile(let id)?:
            ws = all.first { $0.tabs.contains { $0.tile(id) != nil } }
            tileID = id
        case .tab(let id)?:
            ws = all.first { $0.tabs.contains { $0.id == id } }
            tabID = id
        case nil:
            ws = window.workspace
        }
        guard let ws else { return }
        let tab = tabID.flatMap { id in ws.tabs.first { $0.id == id } }
            ?? tileID.flatMap { id in ws.tabs.first { $0.tile(id) != nil } } ?? ws.tab
        let tile = tileID ?? tab?.focused ?? ""
        let (title, current, build): (String, String?, (String) -> [String: Any]) = switch action {
        case "rename_workspace":
            ("Rename workspace", ws.name, { ["method": "workspace.rename", "params": ["workspace": ws.id, "name": $0]] })
        case "rename_tab":
            ("Rename tab", tab?.name, { ["method": "tab.rename", "params": ["tab": tab?.id ?? "", "name": $0]] })
        default:
            ("Rename tile", tab?.tile(tile)?.title,
             { ["method": "tile.rename", "params": ["tile": tile, "title": $0]] })
        }
        if let answer = Prompts.takeAnswer() {
            if answer != Prompts.cancel { send(build(answer)) }
            return
        }
        guard !env.background else { return controller.flash("rename needs the app in front") }
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

    /// `open_config` (⌘,): the config file, created from the core's
    /// commented `[keys]` template (`config.defaults`) when it's missing. A
    /// background run (automation, dev profiles) creates it but opens no
    /// editor.
    private func openConfig(in controller: WindowController) {
        guard let path = core.info()["config_path"] as? String else {
            return controller.flash("no config file (defaults only)")
        }
        let url = URL(fileURLWithPath: path)
        if !FileManager.default.fileExists(atPath: path) {
            var template = "# Azulejo config. See docs/keybindings.md.\n"
            if case .success(let reply) = send(["method": "config.defaults"]), let text = reply["text"] as? String {
                template = text
            }
            do {
                try FileManager.default.createDirectory(
                    at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
                try template.write(to: url, atomically: true, encoding: .utf8)
                log("open_config: wrote the template to \(path)")
            } catch {
                return controller.flash("can't create \(path): \(error.localizedDescription)")
            }
        }
        guard !env.background else { return controller.flash("config: \(path)") }
        NSWorkspace.shared.open(url)
    }

    // MARK: Logging

    func log(_ line: String) {
        if env.verbose { FileHandle.standardError.write(Data("azulejo: \(line)\n".utf8)) }
        automation?.note(line)
    }
}
