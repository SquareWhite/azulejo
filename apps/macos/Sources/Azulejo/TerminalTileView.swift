import AppKit
import AzulejoCore

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
///   the core runs without herdr.
///
/// Closing the tile only detaches: the pane keeps running in herdr. Killing it
/// is the core's `tile.close {kill: true}`.
@MainActor
final class TerminalTileView: NSView, TileView {
    let tileID: String
    let kind = TileKind.terminal
    private var model: TileModel

    private enum Phase: Equatable {
        case waiting(String)
        case attached
        case observing(takenOver: Bool)
        case ended
        case retrying(attempt: Int)
    }

    private var phase: Phase = .waiting("starting…")
    private var surface: TerminalSurfaceView?
    /// The terminal the running surface views (so a rebind restarts it).
    private var surfaceTerminal: String?
    private var focused = false
    private var closed = false
    private var retryWork: DispatchWorkItem?
    private var resizeWork: DispatchWorkItem?
    private var observedSize: (cols: Int, rows: Int)?
    /// Recent detaches (ctrl+b q); a burst means something keeps detaching us.
    private var detaches: [Date] = []
    private let overlay = TerminalOverlay()
    private let runDir: URL

    private static var core: Core?
    private static var log: (String) -> Void = { _ in }

    /// Registers the tile kind. Called once at launch. Without libghostty the
    /// placeholder stays.
    static func install(core: Core, log: @escaping (String) -> Void) {
        Self.core = core
        Self.log = log
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
        layer?.backgroundColor = NSColor(calibratedWhite: 0.08, alpha: 1).cgColor
        layer?.cornerRadius = 4
        layer?.masksToBounds = true
        try? FileManager.default.createDirectory(at: runDir, withIntermediateDirectories: true)
        overlay.onButton = { [weak self] action in self?.overlayAction(action) }
        addSubview(overlay)
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
        guard focused, let window else { return }
        let target: NSView = surface ?? self
        if window.firstResponder !== target { window.makeFirstResponder(target) }
    }

    func tileWillClose() {
        closed = true
        retryWork?.cancel()
        resizeWork?.cancel()
        stopSurface()
        try? FileManager.default.removeItem(at: runDir)
    }

    // MARK: Layout

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        surface?.frame = bounds
        overlay.frame = bounds
        if surface == nil {
            settle()
        } else if case .observing = phase {
            scheduleObserveResize()
        }
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
        if phase == .ended { phase = .waiting("reattaching…") }
        if surface != nil, surfaceTerminal != herdr.terminalId {
            // Rebound (the pane moved, or herdr restarted): view the new one.
            stopSurface()
            phase = .waiting("reattaching…")
        }
        if surface == nil {
            switch phase {
            case .observing(let takenOver): startObserve(takenOver: takenOver)
            case .retrying: break // the timer restarts it
            default: startAttach(takeover: false)
            }
        }
        render()
    }

    private func startAttach(takeover: Bool) {
        guard let herdr = model.herdr else { return }
        let started = startSurface(
            mode: takeover ? "takeover" : "attach", herdr: herdr, readOnly: false)
        if started { phase = .attached }
        render()
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
        view.autoresizingMask = [.width, .height]
        view.onExit = { [weak self, weak view] in
            guard let self, let view, view === self.surface else { return }
            self.surfaceExited(readOnly: readOnly)
        }
        view.onTitle = { [weak self] title in self?.titleChanged(title) }
        view.onClickFocus = { [weak self] in self?.clickFocus() }
        addSubview(view, positioned: .below, relativeTo: overlay)
        surface = view
        surfaceTerminal = herdr.terminalId
        Self.log("terminal \(Short.id(tileID)): \(mode) \(herdr.terminalId)")
        if focused { window?.makeFirstResponder(view) }
        return true
    }

    private func stopSurface() {
        guard let view = surface else { return }
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
        request["status"] = status.map { Int($0) } ?? NSNull()
        request["stderr"] = stderr.map { String($0.suffix(2000)) } ?? NSNull()
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
        case "already_attached":
            startObserve(takenOver: false)
        case "terminal_gone":
            phase = .ended
            render()
        case "detached":
            // An observer ended cleanly (terminal.closed): try for the pane again.
            startAttach(takeover: false)
        default:
            retry(attempt: { if case .retrying(let n) = phase { n + 1 } else { 1 } }())
        }
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

    /// A fresh pane in this tile's place: split the ended tile, then close it,
    /// so the new terminal takes exactly its slot. (A core `terminal.respawn`
    /// that keeps the tile ID would be neater; this needs none.)
    private func respawn() {
        guard let core = Self.core else { return }
        var content: [String: Any] = ["kind": "terminal"]
        if let cwd = model.cwd { content["cwd"] = cwd }
        if let command = model.command { content["command"] = command }
        let split = core.execute([
            "method": "layout.split",
            "params": ["tile": tileID, "direction": "right", "content": content],
        ])
        if case .failure(let error) = split {
            Self.log("terminal \(Short.id(tileID)): respawn failed: \(error)")
            return
        }
        core.send(["method": "tile.close", "params": ["tile": tileID]])
    }

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
        case .attached:
            overlay.show(nil)
        case .waiting(let text):
            overlay.show(.init(style: .full, title: text, detail: nil, buttons: []))
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
                style: .full, title: "herdr isn't answering",
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
            MainActor.assumeIsolated {
                for window in NSApp.windows {
                    if let tile = window.contentView?.findTerminal(words[1]) {
                        tile.debugPress(words[0])
                    }
                }
            }
        }
    }

    fileprivate func debugPress(_ name: String) {
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
@MainActor
final class TerminalOverlay: NSView {
    enum Action { case takeOver, respawn, close, retryNow }
    enum Style { case full, banner }

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
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

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
        title.font = .systemFont(ofSize: content.style == .full ? 14 : 12, weight: .semibold)
        title.textColor = .white
        actions = content.buttons.map(\.action)
        let buttons = content.buttons.enumerated().map { index, button in
            let b = NSButton(title: button.title, target: self, action: #selector(pressed(_:)))
            b.tag = index
            b.bezelStyle = .rounded
            b.controlSize = content.style == .full ? .regular : .small
            return b
        }
        switch content.style {
        case .full:
            layer?.backgroundColor = NSColor(calibratedWhite: 0.08, alpha: 0.92).cgColor
            stack.orientation = .vertical
            stack.addArrangedSubview(title)
            if let detail = content.detail {
                let label = NSTextField(labelWithString: detail)
                label.font = .systemFont(ofSize: 12)
                label.textColor = NSColor(calibratedWhite: 0.75, alpha: 1)
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
            layer?.backgroundColor = NSColor.clear.cgColor
            stack.orientation = .horizontal
            stack.wantsLayer = true
            stack.layer?.backgroundColor = NSColor(calibratedRed: 0.55, green: 0.38, blue: 0.05, alpha: 0.95).cgColor
            stack.layer?.cornerRadius = 4
            stack.edgeInsets = NSEdgeInsets(top: 4, left: 10, bottom: 4, right: 6)
            stack.addArrangedSubview(title)
            buttons.forEach { stack.addArrangedSubview($0) }
            NSLayoutConstraint.activate([
                stack.topAnchor.constraint(equalTo: topAnchor, constant: 6),
                stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            ])
        }
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
