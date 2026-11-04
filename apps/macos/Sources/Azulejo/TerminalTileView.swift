import AppKit
import IOSurface
import AzulejoCore
import GhosttyKit

/// A terminal tile: a libghostty surface viewing one herdr pane (ADR 0001).
///
/// The surface's child is `herdr terminal attach <terminal-id>`. When it ends,
/// the tile reads why (ADR 0003) and moves to one of:
///
/// - **attached**: the live terminal.
/// - **read-only**: another client owns the pane (it took over, or had it
///   first). The tile shows herdr's frames through `terminal session observe`
///   at its own size, with a Take over / Take back button. Never retakes on
///   its own.
/// - **ended**: the pane is gone. Respawn starts a new pane in the tile's
///   place; Close closes the tile.
/// - **waiting**: no pane yet, herdr is unreachable (retries with backoff), or
///   the core runs without herdr. A tile also waits until the core says its
///   machine is connected: the core checks every bound pane first, so after
///   a herdr restart the tile attaches to the pane herdr has now, not to the
///   terminal the restart replaced (attaching to that one, then switching,
///   cost a failed attach and up to a second's back-off).
/// - **gone**: herdr says the terminal is gone; the core is about to close
///   the tile, end it or rebind it. Nothing restarts until it does.
///
/// Closing the tile only detaches: the pane keeps running in herdr. Killing it
/// is the core's `tile.close {kill: true}`.
@MainActor
final class TerminalTileView: NSView, TileView, ThemeAware {
    let tileID: String
    let kind = TileKind.terminal
    private var model: TileModel

    private enum Phase: Equatable {
        case waiting(String)
        case attached
        case observing(takenOver: Bool)
        case ended
        case retrying(attempt: Int)
        /// The attach ended with "terminal gone": wait for the core's verdict.
        case gone
    }

    private var phase: Phase = .waiting("starting…")
    private var surface: TerminalSurfaceView?
    /// The terminal the running surface views (so a rebind restarts it).
    private var surfaceTerminal: String?
    /// The terminal the last surface viewed, kept after it ends.
    private var lastTerminal: String?
    private var focused = false
    private var closed = false
    private var retryWork: DispatchWorkItem?
    /// Attaches anyway if the core never says the machine is connected.
    private var connectWait: DispatchWorkItem?
    private var resizeWork: DispatchWorkItem?
    private var observedSize: (cols: Int, rows: Int)?
    /// Recent detaches (ctrl+b q); a burst means something keeps detaching us.
    private var detaches: [Date] = []
    /// Recent unexplained attach failures; the first reattaches at once.
    private var failures: [Date] = []
    /// When our own attach (not observe) last ended, and to which terminal:
    /// herdr can still count that client for a moment, so an "already
    /// attached" right after it is usually ourselves.
    private var ownAttachEnded: (terminal: String, at: Date)?
    private var ghostRetries: [Date] = []
    private let overlay = TerminalOverlay()
    /// ⌘F: made on first use, then kept with its query.
    private var finder: TerminalFinder?
    private let runDir: URL
    /// Layout motion (ADR 0004): the last frame, covering the live surface
    /// while the tile moves and the PTY redraws at its new size. `freeze` is
    /// a plain background; `freezeImage` the frame itself, either filling it
    /// (scale style) or at 1:1 where the container places it (crop, outline).
    private var freeze: NSView?
    private var freezeImage: NSView?
    private var freezeFills = true
    private var freezeBefore: IOSurfaceRef?
    /// When the snapshot went up and when its motion ended, for the reveal log.
    private var frozenAt: CFTimeInterval = 0
    private var motionEndedAt: CFTimeInterval?
    /// Bumped by every motion that freezes (or keeps frozen) this tile.
    private var motionToken = 0
    /// How long after a motion the reveal waits for a fresh frame at most.
    static let revealDeadline: CFTimeInterval = 0.25
    /// PTY resizes are at most one per 50 ms while frozen (key repeat).
    private var lastSurfaceResize: CFTimeInterval = 0
    private var pendingSurfaceResize: DispatchWorkItem?

    private static var core: Core?
    private static var log: (String) -> Void = { _ in }
    /// Every open terminal tile, for herdr.status and wake health checks.
    private static let live = NSHashTable<TerminalTileView>.weakObjects()
    /// Automation: terminal surfaces started so far (each one an attach).
    private(set) static var attaches = 0
    /// Machines (`local` or herdr's ID) the core has said are connected, with
    /// their bound panes checked: `machines.list`, then `herdr.status` and
    /// `machine.status`.
    private static var connected: Set<String> = []
    /// When the current wait for an attach began (launch, wake, herdr back),
    /// and why; logged with the time it took (PLAN 1.12: ≤ 2 s).
    private var attachClock: (start: Date, reason: String)?

    /// Registers the tile kind. Called once at launch. Without libghostty the
    /// placeholder stays.
    static func install(core: Core, log: @escaping (String) -> Void) {
        Self.core = core
        Self.log = log
        TerminalSurfaceView.log = log
        guard TerminalRuntime.start(log: log) != nil else { return }
        TileViews.register(.terminal) { tile in TerminalTileView(tile) }
        installDebugHook()
    }

    init(_ tile: TileModel) {
        tileID = tile.id
        model = tile
        runDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("azulejo-term", isDirectory: true)
            .appendingPathComponent(tile.id, isDirectory: true)
        super.init(frame: .zero)
        wantsLayer = true
        // Rectangular clip (cheap): the card (TileFrame) rounds the corners.
        layer?.masksToBounds = true
        Theme.track(self)
        try? FileManager.default.createDirectory(at: runDir, withIntermediateDirectories: true)
        overlay.onButton = { [weak self] action in self?.overlayAction(action) }
        addSubview(overlay)
        Self.live.add(self)
        // Tiles restored at launch time from process start; later ones from now.
        attachClock = Date().timeIntervalSince(Self.processStart) < 20
            ? (Self.processStart, "launch") : (Date(), "new tile")
        settle()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var acceptsFirstResponder: Bool { surface == nil }

    // MARK: TileView

    func update(_ tile: TileModel) {
        guard tile != model else { return }
        model = tile
        settle()
    }

    func setFocused(_ focused: Bool) {
        self.focused = focused
        guard focused, let window, TileFocus.canTake(in: window) else { return }
        // Typing in this tile's find bar: refreshes leave it there.
        if finder?.bar.holdsKeyboard == true { return }
        let target: NSView = surface ?? self
        if window.firstResponder !== target { window.makeFirstResponder(target) }
    }

    func tileWillClose() {
        closed = true
        pendingSurfaceResize?.cancel()
        Self.live.remove(self)
        retryWork?.cancel()
        connectWait?.cancel()
        resizeWork?.cancel()
        stopSurface()
        try? FileManager.default.removeItem(at: runDir)
    }

    // MARK: Layout

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        overlay.frame = bounds
        layoutFindBar()
        if freezeFills {
            freeze?.frame = bounds
            freezeImage?.frame = bounds
        }
        if freeze != nil { resizeSurfaceCoalesced() } else { applySurfaceSize() }
        if surface == nil {
            settle()
        } else if case .observing = phase {
            scheduleObserveResize()
        }
    }

    /// The surface takes the tile's size (the PTY resize).
    private func applySurfaceSize() {
        pendingSurfaceResize = nil
        guard let surface, surface.frame.size != bounds.size || surface.frame.origin != .zero else { return }
        surface.frame = bounds
        lastSurfaceResize = CACurrentMediaTime()
    }

    /// While frozen, resize at once unless the last resize was under 50 ms
    /// ago; then once, 50 ms after it, to the latest size.
    private func resizeSurfaceCoalesced() {
        pendingSurfaceResize?.cancel()
        let since = CACurrentMediaTime() - lastSurfaceResize
        if since >= 0.05 { return applySurfaceSize() }
        let work = DispatchWorkItem { [weak self] in self?.applySurfaceSize() }
        pendingSurfaceResize = work
        DispatchQueue.main.asyncAfter(deadline: .now() + (0.05 - since), execute: work)
    }

    fileprivate func flushSurfaceResize() {
        if let work = pendingSurfaceResize {
            work.cancel()
            applySurfaceSize()
        }
    }

    fileprivate var surfaceResizeCount: Int { surface?.resizeCount ?? 0 }

    /// The backdrop round the surface: the terminal's own background (the
    /// theme's palette, or the user's Ghostty colours).
    func applyTheme() {
        layer?.backgroundColor = (TerminalRuntime.shared?.background ?? Design.Color.tile).cgColor
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        settle()
    }

    // MARK: State

    /// Brings the surface in line with the model: start, restart on rebind,
    /// or show why there's nothing to show.
    private func settle() {
        guard !closed else { return }
        if model.ended == true {
            if phase != .ended {
                stopSurface()
                phase = .ended
            }
            return render()
        }
        guard let herdr = model.herdr else {
            stopSurface()
            phase = Self.core?.info()["herdr_session"] is String
                ? .waiting("starting the herdr pane…")
                : .waiting("no herdr: this terminal has no session")
            return render()
        }
        if let last = lastTerminal, last != herdr.terminalId {
            // A new terminal: the old one's failures say nothing about it.
            failures.removeAll()
            detaches.removeAll()
            ghostRetries.removeAll()
        }
        if phase == .ended {
            phase = .waiting("reattaching…")
            if attachClock == nil { attachClock = (Date(), "rebind") }
        }
        if surface != nil, surfaceTerminal != herdr.terminalId {
            // Rebound (the pane moved, or herdr restarted): view the new one.
            stopSurface()
            phase = .waiting("reattaching…")
            if attachClock == nil { attachClock = (Date(), "rebind") }
        }
        if surface == nil {
            switch phase {
            case .observing(let takenOver): startObserve(takenOver: takenOver)
            case .retrying where lastTerminal == herdr.terminalId:
                break // the timer restarts it
            case .gone where lastTerminal == herdr.terminalId:
                break // the core closes, ends or rebinds it
            case .waiting where connectWait != nil && !Self.isConnected(herdr.machine):
                break // the core says when the machine is connected
            case .retrying:
                // Rebound while backing off (herdr respawned the pane): no
                // reason to wait for the timer.
                retryWork?.cancel()
                if attachClock == nil { attachClock = (Date(), "rebind") }
                startAttach(takeover: false)
            default: startAttach(takeover: false)
            }
        }
        render()
    }

    private func startAttach(takeover: Bool, force: Bool = false) {
        guard let herdr = model.herdr else { return }
        if !takeover, !force, !Self.isConnected(herdr.machine) {
            waitForConnection()
            return
        }
        connectWait?.cancel()
        connectWait = nil
        let started = startSurface(
            mode: takeover ? "takeover" : "attach", herdr: herdr, readOnly: false)
        if started {
            phase = .attached
            if let clock = attachClock {
                attachClock = nil
                let ms = Int(Date().timeIntervalSince(clock.start) * 1000)
                let at = String(format: "%.3f", Date().timeIntervalSince1970)
                Self.log("terminal \(Short.id(tileID)): attached \(ms) ms after \(clock.reason) at \(at)")
            }
        }
        render()
    }

    /// Holds the attach until the core says the tile's machine is connected
    /// (`herdrBack`), or 2 s at most.
    private func waitForConnection() {
        if case .waiting = phase {} else { phase = .waiting("connecting to herdr…") }
        render()
        guard connectWait == nil else { return }
        let work = DispatchWorkItem { [weak self] in
            guard let self, !self.closed, self.surface == nil, self.model.ended != true else { return }
            self.connectWait = nil
            Self.log("terminal \(Short.id(self.tileID)): no word from the core; attaching anyway")
            self.startAttach(takeover: false, force: true)
        }
        connectWait = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 2, execute: work)
    }

    private static func isConnected(_ machine: String?) -> Bool {
        connected.contains(machine ?? MachineModel.local)
    }

    /// The core's machine list (`machines.list`): which machines are connected.
    static func noteMachines(_ machines: [MachineModel]) {
        connected = Set(machines.filter { $0.state == .connected }.map(\.id))
    }

    private func startObserve(takenOver: Bool) {
        guard let herdr = model.herdr else { return }
        let started = startSurface(mode: "observe", herdr: herdr, readOnly: true)
        if started { phase = .observing(takenOver: takenOver) }
        render()
    }

    /// Starts a surface running the command the core builds. Waits (returns
    /// false) while the tile has no size, since the PTY needs one first.
    @discardableResult
    private func startSurface(mode: String, herdr: HerdrRefModel, readOnly: Bool) -> Bool {
        guard window != nil, bounds.width >= 20, bounds.height >= 20,
              let core = Self.core, let runtime = TerminalRuntime.shared else { return false }
        stopSurface()
        let err = runDir.appendingPathComponent("stderr")
        let status = runDir.appendingPathComponent("status")
        try? FileManager.default.removeItem(at: err)
        try? FileManager.default.removeItem(at: status)
        var request: [String: Any] = [
            "mode": mode,
            "terminal_id": herdr.terminalId,
            "pane_id": herdr.paneId,
            "stderr_file": err.path,
            "status_file": status.path,
        ]
        if let machine = herdr.machine { request["machine"] = machine }
        if readOnly {
            let size = observeSize()
            observedSize = size
            request["cols"] = size.cols
            request["rows"] = size.rows
            request["decoder"] = Bundle.main.bundleURL
                .appendingPathComponent("Contents/MacOS/azj-observe").path
        }
        let reply = core.terminalCommand(request)
        guard reply["ok"] as? Bool == true, let command = reply["command"] as? String else {
            let error = (reply["error"] as? [String: Any])?["message"] as? String ?? "no command"
            Self.log("terminal \(Short.id(tileID)): \(error)")
            phase = .waiting(error)
            return false
        }
        guard let view = TerminalSurfaceView(
            runtime: runtime, frame: bounds, command: command, readOnly: readOnly)
        else {
            phase = .waiting("the terminal surface didn't start")
            return false
        }
        Self.attaches += 1
        // Sized by setFrameSize, so motion controls when the PTY resizes.
        view.autoresizingMask = []
        view.logName = Short.id(tileID)
        view.onExit = { [weak self, weak view] in
            guard let self, let view, view === self.surface else { return }
            self.surfaceExited(readOnly: readOnly)
        }
        view.onTitle = { [weak self] title in self?.titleChanged(title) }
        view.onClickFocus = { [weak self] in self?.clickFocus() }
        addSubview(view, positioned: .below, relativeTo: overlay)
        surface = view
        surfaceTerminal = herdr.terminalId
        lastTerminal = herdr.terminalId
        Self.log("terminal \(Short.id(tileID)): \(mode) \(herdr.terminalId)")
        // A surface that comes up (attach finished, reattach) takes the
        // keyboard from its tile, but not from the ⌘K field (TileFocus) or
        // the tile's find bar.
        if focused, let window, TileFocus.canTake(in: window), finder?.bar.holdsKeyboard != true {
            window.makeFirstResponder(view)
        }
        return true
    }

    private func stopSurface() {
        guard let view = surface else { return }
        if !view.readOnly, let terminal = surfaceTerminal {
            ownAttachEnded = (terminal, Date())
        }
        surface = nil
        surfaceTerminal = nil
        let hadFocus = window?.firstResponder === view
        view.close()
        view.removeFromSuperview()
        if hadFocus { window?.makeFirstResponder(self) }
    }

    /// The child ended: work out why and move on (see the type's docs).
    private func surfaceExited(readOnly: Bool) {
        guard !closed, let herdr = model.herdr else { return }
        let status = (try? String(contentsOf: runDir.appendingPathComponent("status"), encoding: .utf8))
            .flatMap { Int32($0.trimmingCharacters(in: .whitespacesAndNewlines)) }
        let stderr = try? String(contentsOf: runDir.appendingPathComponent("stderr"), encoding: .utf8)
        var request: [String: Any] = ["terminal_id": herdr.terminalId]
        if let machine = herdr.machine { request["machine"] = machine }
        request["status"] = status.map { Int($0) } ?? NSNull()
        request["stderr"] = stderr.map { String($0.suffix(2000)) } ?? NSNull()
        if attachClock == nil { attachClock = (Date(), "attach ended") }
        if !readOnly { ownAttachEnded = (herdr.terminalId, Date()) }
        let exit = Self.core?.attachExit(request) ?? [:]
        let kind = exit["exit"] as? String ?? "failed"
        Self.log("terminal \(Short.id(tileID)): \(readOnly ? "observe" : "attach") ended: \(kind) \(exit["message"] as? String ?? "")")
        stopSurface()
        switch kind {
        case "detached" where !readOnly:
            // ctrl+b q got through somehow. Reattach, unless it keeps happening.
            let now = Date()
            detaches = detaches.filter { now.timeIntervalSince($0) < 10 } + [now]
            if detaches.count >= 3 { retry(attempt: 1) } else { startAttach(takeover: false) }
        case "taken_over":
            startObserve(takenOver: true)
        case "already_attached" where isOwnGhost(herdr.terminalId):
            // Our previous attach to this terminal just ended; herdr hasn't
            // let go of it yet. Try again shortly rather than going read-only.
            phase = .waiting("reattaching…")
            render()
            let delay = 0.1 * Double(max(1, ghostRetries.count))
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
                guard let self, !self.closed, self.surface == nil,
                      self.model.herdr?.terminalId == herdr.terminalId else { return }
                self.startAttach(takeover: false)
            }
        case "already_attached":
            startObserve(takenOver: false)
        case "terminal_gone" where model.ended == true:
            attachClock = nil
            phase = .ended
            render()
        case "terminal_gone":
            // Usually the shell exited and the core is about to close this
            // tile (`[terminal] on_exit = "close"`), so hold the overlay back
            // briefly rather than flash it. Otherwise herdr may be restarting
            // (it respawns panes under new terminals, and the core rebinds):
            // keep trying; the core's `ended` or a rebind settles it.
            if case .retrying(let n) = phase {
                retry(attempt: n + 1)
            } else {
                phase = .gone
                render()
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
                    guard let self, !self.closed, self.surface == nil,
                          self.model.ended != true, self.model.herdr?.terminalId == herdr.terminalId
                    else { return }
                    self.retry(attempt: 1)
                }
            }
        case "detached":
            // An observer ended cleanly (terminal.closed): try for the pane again.
            startAttach(takeover: false)
        default:
            // Killed or dropped while running (sleep, a signal): reattach at
            // once; only repeated failures back off.
            let now = Date()
            failures = failures.filter { now.timeIntervalSince($0) < 10 } + [now]
            if failures.count == 1, kind == "failed" {
                startAttach(takeover: false)
            } else {
                retry(attempt: { if case .retrying(let n) = phase { n + 1 } else { 1 } }())
            }
        }
    }

    /// Whether an "already attached" is probably our own ended attach: it ended
    /// under 2 s ago, and we haven't retried for it more than 4 times in 3 s.
    private func isOwnGhost(_ terminal: String) -> Bool {
        guard let own = ownAttachEnded, own.terminal == terminal,
              Date().timeIntervalSince(own.at) < 2 else { return false }
        let now = Date()
        ghostRetries = ghostRetries.filter { now.timeIntervalSince($0) < 3 }
        guard ghostRetries.count < 4 else { return false }
        ghostRetries.append(now)
        return true
    }

    /// herdr unreachable or the attach failed: try again after 1, 2, 4… 30 s.
    private func retry(attempt: Int) {
        phase = .retrying(attempt: attempt)
        render()
        retryWork?.cancel()
        let delay = min(30, pow(2, Double(attempt - 1)))
        let work = DispatchWorkItem { [weak self] in
            guard let self, !self.closed, case .retrying = self.phase else { return }
            self.startAttach(takeover: false)
        }
        retryWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: work)
    }

    /// Observe renders at a fixed size; restart it when the tile settles at a
    /// new one.
    private func scheduleObserveResize() {
        resizeWork?.cancel()
        let work = DispatchWorkItem { [weak self] in
            guard let self, case .observing(let takenOver) = self.phase,
                  let current = self.observedSize else { return }
            let wanted = self.observeSize()
            if wanted != current { self.startObserve(takenOver: takenOver) }
        }
        resizeWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
    }

    /// The grid the tile fits, from the surface's cell size when there is one.
    private func observeSize() -> (cols: Int, rows: Int) {
        if let surface, let grid = surface.gridSize,
           surface.bounds.width > 0, surface.bounds.height > 0 {
            let cellW = surface.bounds.width / CGFloat(grid.cols)
            let cellH = surface.bounds.height / CGFloat(grid.rows)
            return (max(2, Int(bounds.width / cellW)), max(1, Int(bounds.height / cellH)))
        }
        // No surface yet: estimate from a 13pt monospaced cell.
        return (max(2, Int(bounds.width / 7.8)), max(1, Int(bounds.height / 17)))
    }

    // MARK: Actions

    private func overlayAction(_ action: TerminalOverlay.Action) {
        switch action {
        case .takeOver:
            startAttach(takeover: true)
        case .respawn:
            respawn()
        case .close:
            Self.core?.send(["method": "tile.close", "params": ["tile": tileID]])
        case .retryNow:
            retryWork?.cancel()
            startAttach(takeover: false)
        }
    }

    /// A fresh pane for this tile, in place: the core's `terminal.respawn`
    /// unbinds the ended pane and creates a new one (own herdr tab, the tile's
    /// cwd); the rebind arrives as a model update and `settle` attaches.
    private func respawn() {
        guard let core = Self.core else { return }
        attachClock = (Date(), "respawn")
        let result = core.execute(["method": "terminal.respawn", "params": ["tile": tileID]])
        if case .failure(let error) = result {
            Self.log("terminal \(Short.id(tileID)): respawn failed: \(error)")
        }
    }

    // MARK: Reattach (herdr back, wake)

    /// The core's link to herdr came back (`herdr.status`): tiles that were
    /// waiting or backing off try now instead of at their next retry.
    private func herdrBack(reason: String) {
        guard !closed, surface == nil, model.herdr != nil, model.ended != true else { return }
        switch phase {
        case .retrying, .waiting:
            retryWork?.cancel()
            connectWait?.cancel()
            connectWait = nil
            if attachClock == nil { attachClock = (Date(), reason) }
            Self.log("terminal \(Short.id(tileID)): reattaching (\(reason))")
            startAttach(takeover: false)
        default: break
        }
    }

    /// After sleep: a child that died while we weren't looking is handled as
    /// an exit now; tiles that were waiting reattach at once.
    private func healthCheck(reason: String) {
        guard !closed else { return }
        if let view = surface, let handle = view.surface, ghostty_surface_process_exited(handle) {
            Self.log("terminal \(Short.id(tileID)): child gone (\(reason))")
            attachClock = (Date(), reason)
            surfaceExited(readOnly: view.readOnly)
            return
        }
        herdrBack(reason: reason)
    }

    /// Routes core events this kind cares about: `herdr.status` (Local) and
    /// `machine.status` (any machine, ADR 0011).
    static func route(_ json: String) {
        if json.contains("\"machine.status\""),
           let data = json.data(using: .utf8),
           let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           object["event"] as? String == "machine.status",
           let machine = (object["data"] as? [String: Any])?["machine"] as? [String: Any],
           let id = machine["id"] as? String
        {
            guard machine["state"] as? String == "connected" else {
                Self.connected.remove(id)
                return
            }
            Self.connected.insert(id)
            log("machine \(machine["label"] as? String ?? id) connected")
            for tile in live.allObjects where (tile.model.herdr?.machine ?? MachineModel.local) == id {
                tile.herdrBack(reason: "machine back")
            }
            return
        }
        guard json.contains("\"herdr.status\""),
              let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              object["event"] as? String == "herdr.status",
              let body = object["data"] as? [String: Any]
        else { return }
        let connected = body["connected"] as? Bool == true
        log("herdr \(connected ? "connected" : "disconnected: \(body["message"] as? String ?? "")")")
        if connected { Self.connected.insert(MachineModel.local) } else { Self.connected.remove(MachineModel.local) }
        guard connected else { return }
        for tile in live.allObjects where tile.model.herdr?.machine == nil {
            tile.herdrBack(reason: "herdr back")
        }
    }

    /// Wake (NSWorkspace) and the dev hook: check every terminal tile.
    static func healthCheckAll(reason: String) {
        let tiles = live.allObjects
        log("terminal health check (\(reason)): \(tiles.count) tiles")
        for tile in tiles { tile.healthCheck(reason: reason) }
    }

    /// This process's start time, so launch timings include the core's boot.
    static let processStart: Date = {
        var info = kinfo_proc()
        var size = MemoryLayout<kinfo_proc>.size
        var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, getpid()]
        guard sysctl(&mib, 4, &info, &size, nil, 0) == 0 else { return Date() }
        let tv = info.kp_proc.p_un.__p_starttime
        return Date(timeIntervalSince1970: Double(tv.tv_sec) + Double(tv.tv_usec) / 1e6)
    }()

    private func clickFocus() {
        Self.core?.send(["method": "tile.focus", "params": ["tile": tileID]])
    }

    private func titleChanged(_ title: String) {
        // herdr owns the pane's title (ADR 0005); the attach client's own
        // title isn't the program's. Kept for logs only.
        _ = title
    }

    private var renderedPhase = ""

    private func render() {
        if phaseName != renderedPhase {
            renderedPhase = phaseName
            Self.log("terminal \(Short.id(tileID)): phase \(phaseName)")
        }
        switch phase {
        case .attached, .gone:
            overlay.show(nil)
        case .waiting(let text):
            overlay.show(.init(style: .placeholder, title: text, detail: nil, buttons: []))
        case .observing(let takenOver):
            overlay.show(.init(
                style: .banner,
                title: takenOver ? "Read-only: another client took this pane over"
                    : "Read-only: another client has this pane",
                detail: nil,
                buttons: [takenOver ? .init(.takeOver, "Take back") : .init(.takeOver, "Take over")]))
        case .ended:
            overlay.show(.init(
                style: .full, title: "Session ended",
                detail: "The herdr pane is gone.",
                buttons: [.init(.respawn, "Respawn"), .init(.close, "Close")]))
        case .retrying(let attempt):
            overlay.show(.init(
                style: .full, title: "Reconnecting to herdr",
                detail: "Retrying (attempt \(attempt))…",
                buttons: [.init(.retryNow, "Retry now")]))
        }
    }

    // MARK: Debug hook (dev profile only)

    /// The dev profile can press overlay buttons without OS input:
    /// `notifyutil`-style distributed notification
    /// `dev.azulejo.debug.terminal` with object `"<action> <tile-id>"`.
    static func installDebugHook() {
        guard ProcessInfo.processInfo.environment["AZULEJO_PROFILE"] == "dev" else { return }
        DistributedNotificationCenter.default().addObserver(
            forName: Notification.Name("dev.azulejo.debug.terminal"), object: nil, queue: .main
        ) { note in
            let words = (note.object as? String ?? "").split(separator: " ").map(String.init)
            guard words.count == 2 else { return }
            if words[0] == "wake" {
                return MainActor.assumeIsolated { healthCheckAll(reason: "wake (debug)") }
            }
            MainActor.assumeIsolated {
                for window in NSApp.windows {
                    if let tile = window.contentView?.findTerminal(words[1]) {
                        tile.debugPress(words[0])
                    }
                }
            }
        }
    }

    func debugPress(_ name: String) {
        let action: TerminalOverlay.Action? = switch name {
        case "takeover": .takeOver
        case "respawn": .respawn
        case "close": .close
        case "retry": .retryNow
        default: nil
        }
        if let action { overlayAction(action) }
    }

    /// For automation and logs: what the tile is showing.
    var phaseName: String {
        switch phase {
        case .waiting: "waiting"
        case .attached: "attached"
        case .observing(let takenOver): takenOver ? "observing-taken-over" : "observing"
        case .ended: "ended"
        case .retrying: "retrying"
        case .gone: "gone"
        }
    }
}

private extension NSView {
    func findTerminal(_ id: String) -> TerminalTileView? {
        if let tile = self as? TerminalTileView, tile.tileID == id { return tile }
        for sub in subviews { if let found = sub.findTerminal(id) { return found } }
        return nil
    }
}

// MARK: - Find (⌘F)

extension TerminalTileView: Findable {
    var findBar: FindBar? { finder?.bar }

    func beginFind() -> String? {
        guard model.herdr != nil, model.ended != true else { return "no herdr pane to search" }
        let finder = self.finder ?? makeFinder()
        if finder.bar.superview == nil {
            // Over the top rows, not above them: shrinking the surface would
            // resize the PTY and reflow herdr's pane mid-search.
            addSubview(finder.bar, positioned: .above, relativeTo: overlay)
        }
        layoutFindBar()
        finder.open()
        return nil
    }

    private func makeFinder() -> TerminalFinder {
        let finder = TerminalFinder(tileID: tileID, core: { Self.core }, surface: { [weak self] in self?.surface })
        finder.bar.onFocus = { [weak self] in
            // A click into the bar of a tile that isn't focused focuses it.
            if self?.focused == false { self?.clickFocus() }
        }
        finder.onClosed = { [weak self] in
            guard let self, let window = self.window else { return }
            window.makeFirstResponder(self.surface ?? self)
        }
        self.finder = finder
        return finder
    }

    fileprivate func layoutFindBar() {
        guard let bar = finder?.bar else { return }
        bar.frame = NSRect(x: 0, y: bounds.height - FindBar.height, width: bounds.width, height: FindBar.height)
    }
}

extension TerminalTileView: AutomatableTile {
    func automate(_ verb: String, _ arg: String) -> String? {
        "unknown terminal verb \(verb)"
    }

    func automationState() -> [String: String] {
        finder?.automationState() ?? ["tile.find": "closed"]
    }
}

// MARK: - Core calls

extension Core {
    /// `azj_core_terminal_command`: the command a terminal surface runs.
    func terminalCommand(_ request: [String: Any]) -> [String: Any] {
        callJSON(request) { azj_core_terminal_command($0, $1) }
    }

    /// `azj_core_attach_exit`: why an attach or observe child ended.
    func attachExit(_ request: [String: Any]) -> [String: Any] {
        callJSON(request) { azj_core_attach_exit($0, $1) }
    }
}

// MARK: - Overlay

/// What a terminal tile shows over (or instead of) its surface.
// MARK: - Layout motion

extension TerminalTileView: MotionAware {
    var surfaceResizes: Int { surfaceResizeCount }

    @discardableResult
    func motionWillStart(freeze: Bool, fill: Bool) -> Bool {
        motionToken += 1  // a reveal timer from an earlier motion no longer applies
        if self.freeze != nil { return true }  // mid-motion: keep the first snapshot
        guard freeze, let surface, let image = surface.snapshot() else { return false }
        let scale = window?.backingScaleFactor ?? 2
        // Exactly the snapshot's own size: 1:1, never stretched.
        let size = CGSize(width: CGFloat(IOSurfaceGetWidth(image)) / scale,
                          height: CGFloat(IOSurfaceGetHeight(image)) / scale)
        let cover = NSView(frame: bounds)
        cover.wantsLayer = true
        // theme: a copy for the length of one motion.
        cover.layer?.backgroundColor = layer?.backgroundColor
        cover.autoresizingMask = []
        let picture = NSView(frame: fill ? bounds : CGRect(origin: .zero, size: size))
        picture.wantsLayer = true
        picture.layer?.contents = image
        picture.layer?.contentsGravity = .resize
        picture.layer?.contentsScale = scale
        picture.autoresizingMask = []
        addSubview(cover, positioned: .above, relativeTo: surface)
        addSubview(picture, positioned: .above, relativeTo: cover)
        self.freeze = cover
        freezeImage = picture
        freezeFills = fill
        freezeBefore = surface.presentedSurface
        frozenAt = CACurrentMediaTime()
        motionEndedAt = nil
        return true
    }

    var snapshotFrame: CGRect? { freezeImage?.frame }

    var snapshotContents: Any? { freezeImage?.layer?.contents }

    func placeSnapshot(_ frame: CGRect, cover: CGRect) {
        guard let freeze, let freezeImage, !freezeFills else { return }
        freezeImage.frame = frame
        freeze.frame = cover
    }

    func revealWhenFresh(clock: MotionClock) {
        guard freeze != nil else { return }
        clock.add { [weak self] _ in
            guard let self, self.freeze != nil else { return false }
            if self.liveFrameReady() {
                self.unfreeze(reason: "fresh")
                return false
            }
            return true
        }
    }

    func motionDidEnd(clock: MotionClock) {
        guard freeze != nil else { return }
        flushSurfaceResize()
        motionEndedAt = CACurrentMediaTime()
        let deadline = CACurrentMediaTime() + Self.revealDeadline
        clock.add { [weak self] now in
            guard let self, self.freeze != nil else { return false }
            if self.liveFrameReady() || now > deadline {
                self.unfreeze(reason: now > deadline ? "deadline" : "fresh")
                return false
            }
            return true
        }
        // The clock runs on the display link, which doesn't tick while the
        // screen is locked or the display sleeps (and may not while the
        // window is hidden). libghostty keeps drawing then, so the snapshot
        // must still go: a plain timer takes it down shortly after the
        // deadline if the clock hasn't.
        let token = motionToken
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.revealDeadline + 0.05) { [weak self] in
            guard let self, self.freeze != nil, self.motionToken == token else { return }
            self.unfreeze(reason: "timer, no display-link tick")
        }
    }

    func motionCancelled() {
        flushSurfaceResize()
        unfreeze(reason: "cancelled")
    }

    /// The live surface has drawn a new frame at its current pixel size.
    private func liveFrameReady() -> Bool {
        guard let surface, let current = surface.presentedSurface else { return surface == nil }
        let px = surface.lastPixelSize
        let sized = IOSurfaceGetWidth(current) == Int(px.width) && IOSurfaceGetHeight(current) == Int(px.height)
        let fresh: Bool = {
            guard let before = freezeBefore else { return true }
            return !CFEqual(current, before) || IOSurfaceGetSeed(current) != IOSurfaceGetSeed(before)
        }()
        return sized && fresh
    }

    private func unfreeze(reason: String) {
        if freeze != nil { logReveal(reason) }
        freezeImage?.removeFromSuperview()
        freeze?.removeFromSuperview()
        freeze = nil
        freezeImage = nil
        freezeFills = true
        freezeBefore = nil
    }

    /// What a capture of this tile shows right now (automation): the live
    /// surface or its motion snapshot, and whether that's current. `stale`
    /// lists what makes it not current; empty when it is. `moving`: a motion
    /// is in flight, so a snapshot is expected.
    func captureFreshness(moving: Bool) -> [String: Any] {
        let now = CACurrentMediaTime()
        var info: [String: Any] = ["tile": Short.id(tileID), "phase": phaseName]
        var stale: [String] = []
        if freeze != nil {
            info["shows"] = "snapshot"
            let since = now - (motionEndedAt ?? frozenAt)
            if !moving, since > Self.revealDeadline + 0.1 {
                stale.append("its motion snapshot is still up \(Int(since * 1000)) ms after "
                    + (motionEndedAt == nil ? "freezing" : "the motion ended"))
            }
        } else {
            info["shows"] = surface == nil ? "overlay" : "live"
        }
        if let surface {
            let px = surface.lastPixelSize
            info["surface_px"] = "\(Int(px.width))x\(Int(px.height))"
            info["presented_px"] = surface.presentedSizeText
            if surface.resizedAt > 0 { info["ms_since_resize"] = Int((now - surface.resizedAt) * 1000) }
            if surface.lastFrameSeenAt > 0 { info["ms_since_frame_seen"] = Int((now - surface.lastFrameSeenAt) * 1000) }
            if !surface.presentedAtSize {
                stale.append("the live surface shows a \(surface.presentedSizeText) px frame but is "
                    + "\(Int(px.width))x\(Int(px.height)) px: libghostty hasn't drawn since the resize")
            }
        }
        info["stale"] = stale
        return info
    }

    /// One line per reveal: why, when, and whether the live surface it
    /// uncovers has drawn at its size (`STALE` when it hasn't).
    private func logReveal(_ reason: String) {
        let now = CACurrentMediaTime()
        var line = "terminal \(Short.id(tileID)): revealed (\(reason)) "
        if let end = motionEndedAt {
            line += "\(Int((now - end) * 1000)) ms after the motion ended"
        } else {
            line += "mid-motion, \(Int((now - frozenAt) * 1000)) ms after freezing"
        }
        if let surface {
            let px = surface.lastPixelSize
            line += "; live frame \(surface.presentedSizeText) px for \(Int(px.width))x\(Int(px.height))"
            if !surface.presentedAtSize { line += " STALE" }
        }
        Self.log(line)
    }
}

@MainActor
final class TerminalOverlay: NSView, ThemeAware {
    enum Action { case takeOver, respawn, close, retryNow }
    /// `placeholder`: a new pane on its way, drawn on the terminal's own
    /// background so a split doesn't flash a dark hole before the shell.
    enum Style { case full, placeholder, banner }

    struct Button {
        let action: Action
        let title: String
        init(_ action: Action, _ title: String) {
            self.action = action
            self.title = title
        }
    }

    struct Content {
        let style: Style
        let title: String
        let detail: String?
        let buttons: [Button]
    }

    var onButton: ((Action) -> Void)?
    private let stack = NSStackView()
    private var actions: [Action] = []
    private var style: Style = .full
    private var titleLabel: NSTextField?

    init() {
        super.init(frame: .zero)
        wantsLayer = true
        isHidden = true
        autoresizingMask = [.width, .height]
        stack.orientation = .vertical
        stack.alignment = .centerX
        stack.spacing = 8
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// Placeholders sit on the terminal's own background; the full overlay
    /// (Session ended, reconnecting) on the overlay colour; the take-over
    /// banner on its amber strip.
    func applyTheme() {
        switch style {
        case .full, .placeholder:
            layer?.backgroundColor = (style == .placeholder
                ? TerminalRuntime.shared?.background ?? Design.Color.tile
                : Design.Color.overlay).cgColor
            stack.layer?.backgroundColor = nil
        case .banner:
            layer?.backgroundColor = NSColor.clear.cgColor
            stack.layer?.backgroundColor = Design.Color.takeover.cgColor
        }
        titleLabel?.textColor = switch style {
        case .placeholder: Design.Color.textMuted
        case .full: Design.Color.overlayTitle
        case .banner: Design.Color.textOnAccent
        }
    }

    func show(_ content: Content?) {
        guard let content else {
            isHidden = true
            return
        }
        isHidden = false
        style = content.style
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        NSLayoutConstraint.deactivate(constraints.filter { $0.firstItem === stack || $0.secondItem === stack })
        let title = NSTextField(labelWithString: content.title)
        title.font = content.style == .banner ? Design.Font.overlayTitleSmall : Design.Font.overlayTitle
        titleLabel = title
        actions = content.buttons.map(\.action)
        let buttons = content.buttons.enumerated().map { index, button in
            let b = NSButton(title: button.title, target: self, action: #selector(pressed(_:)))
            b.tag = index
            b.bezelStyle = .rounded
            b.controlSize = content.style == .banner ? .small : .regular
            return b
        }
        switch content.style {
        case .full, .placeholder:
            stack.orientation = .vertical
            stack.addArrangedSubview(title)
            if let detail = content.detail {
                let label = NSTextField(labelWithString: detail)
                label.font = Design.Font.overlayBody
                label.textColor = Design.Color.overlayText
                stack.addArrangedSubview(label)
            }
            if !buttons.isEmpty {
                let row = NSStackView(views: buttons)
                row.orientation = .horizontal
                row.spacing = 8
                stack.addArrangedSubview(row)
            }
            NSLayoutConstraint.activate([
                stack.centerXAnchor.constraint(equalTo: centerXAnchor),
                stack.centerYAnchor.constraint(equalTo: centerYAnchor),
                stack.widthAnchor.constraint(lessThanOrEqualTo: widthAnchor, constant: -24),
            ])
        case .banner:
            stack.orientation = .horizontal
            stack.wantsLayer = true
            stack.layer?.cornerRadius = Design.Radius.overlay
            stack.edgeInsets = NSEdgeInsets(top: 4, left: 10, bottom: 4, right: 6)
            stack.addArrangedSubview(title)
            buttons.forEach { stack.addArrangedSubview($0) }
            NSLayoutConstraint.activate([
                stack.topAnchor.constraint(equalTo: topAnchor, constant: 6),
                stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            ])
        }
        Theme.reapply(self)
    }

    /// The banner only covers its own strip; clicks elsewhere reach the
    /// surface underneath.
    override func hitTest(_ point: NSPoint) -> NSView? {
        let hit = super.hitTest(point)
        if style == .banner, hit === self { return nil }
        return hit
    }

    @objc private func pressed(_ sender: NSButton) {
        guard actions.indices.contains(sender.tag) else { return }
        onButton?(actions[sender.tag])
    }
}
