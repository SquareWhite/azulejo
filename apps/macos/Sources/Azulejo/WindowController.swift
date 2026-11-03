import AppKit

/// One app window: sidebar, tab bar and the active tab's tiles. Owns no
/// state of its own beyond view state; everything comes from the core.
@MainActor
final class WindowController: NSObject, NSWindowDelegate {
    let windowID: String
    let window: NSWindow
    let tiles = TileContainerView()
    private let sidebar = SidebarView()
    private let tabBar = TabBarView()
    private let hud = HUDView()
    let palette = PaletteView()
    let banner = BannerView()
    let attention = AttentionOverlayView()
    private let content = ChromeLayoutView()
    private weak var app: AppController?
    private(set) var model: WindowModel?
    private var frameSave: DispatchWorkItem?
    /// Set while applying a frame from the core, so it isn't echoed back.
    private var applyingFrame = false

    var sidebarVisible = true {
        didSet {
            sidebar.isHidden = !sidebarVisible
            content.needsLayout = true
        }
    }

    init(windowID: String, app: AppController) {
        self.windowID = windowID
        self.app = app
        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1100, height: 700),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false)
        super.init()
        window.isReleasedWhenClosed = false
        window.title = "Azulejo"
        window.minSize = NSSize(width: 480, height: 320)
        window.tabbingMode = .disallowed
        content.sidebar = sidebar
        content.tabBar = tabBar
        content.tiles = tiles
        content.hud = hud
        content.attention = attention
        content.banner = banner
        content.palette = palette
        [sidebar, tabBar, tiles, attention, banner, hud, palette].forEach(content.addSubview)
        window.contentView = content
        window.delegate = self

        sidebar.onSelect = { [weak app] id in app?.send(["method": "workspace.focus", "params": ["workspace": id]]) }
        sidebar.onAdd = { [weak app, windowID] machine in app?.newWorkspace(on: machine, window: windowID) }
        tabBar.onSelect = { [weak app] id in app?.send(["method": "tab.focus", "params": ["tab": id]]) }
        tiles.onClickTile = { [weak app] id in app?.send(["method": "tile.focus", "params": ["tile": id]]) }
    }

    /// The keymap context for the focused tile (`"terminal"`, …).
    var keyContext: String? { tiles.focusedView?.keyContext }

    func apply(_ model: WindowModel) {
        let first = self.model == nil
        self.model = model
        if first { placeWindow(model.frame) }
        let showSidebar = model.sidebarHidden != true
        if sidebarVisible != showSidebar { sidebarVisible = showSidebar }
        sidebar.apply(model, machines: app?.machines ?? [])
        tabBar.apply(model.workspace)
        tiles.apply(model)
        attention.apply(model.workspace?.tab, frames: tiles.visibleTileFrames())
        let ws = model.workspace
        window.title = [ws?.name, ws?.tab?.label].compactMap { $0 }.joined(separator: " — ")
    }

    private func placeWindow(_ frame: FrameModel?) {
        applyingFrame = true
        defer { applyingFrame = false }
        if let f = frame, f.width >= 200, f.height >= 150 {
            let rect = NSRect(x: f.x, y: f.y, width: f.width, height: f.height)
            // Only restore a frame that's still at least partly on a screen.
            if NSScreen.screens.contains(where: { $0.visibleFrame.intersects(rect) }) {
                window.setFrame(rect, display: false)
                return
            }
        }
        window.center()
    }

    func show(activate: Bool) {
        if activate {
            window.makeKeyAndOrderFront(nil)
        } else {
            // Background runs (automation, `dev.sh --bg`): never raise above
            // other apps' windows.
            window.orderBack(nil)
        }
    }

    /// Automation: the sidebar's and tab bar's attention markers.
    var sidebarMarkers: [String] { sidebar.markers }
    /// Automation: the sidebar's machine group headers.
    var sidebarMachines: [String] { sidebar.machineHeaders }
    var tabMarkers: [String] { tabBar.markers }

    /// Re-lays out the chrome (palette or banner changed size).
    func layoutChrome() {
        content.needsLayout = true
        content.layoutSubtreeIfNeeded()
    }

    // MARK: HUD

    func flash(_ text: String) { hud.show(text, sticky: false) }
    func setStatus(_ text: String?) { text.map { hud.show($0, sticky: true) } ?? hud.hide() }

    // MARK: NSWindowDelegate

    func windowDidMove(_ notification: Notification) { scheduleFrameSave() }
    func windowDidResize(_ notification: Notification) { scheduleFrameSave() }

    private func scheduleFrameSave() {
        guard !applyingFrame, model != nil else { return }
        frameSave?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.saveFrame() }
        frameSave = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
    }

    func saveFrame() {
        let f = window.frame
        app?.send(["method": "window.set_frame", "params": [
            "window": windowID,
            "frame": ["x": f.minX, "y": f.minY, "width": f.width, "height": f.height],
        ]])
    }

    func windowDidBecomeKey(_ notification: Notification) {
        app?.windowBecameKey(self)
    }

    func windowDidResignKey(_ notification: Notification) {
        app?.windowResignedKey(self)
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        app?.closeWindow(self)
        return false
    }
}

/// Sidebar on the left, tab bar over the tiles. Manual layout keeps it
/// predictable while tiles are being moved around.
@MainActor
final class ChromeLayoutView: NSView {
    weak var sidebar: NSView?
    weak var tabBar: NSView?
    weak var tiles: NSView?
    weak var hud: NSView?
    weak var attention: AttentionOverlayView?
    weak var banner: BannerView?
    weak var palette: PaletteView?

    override func layout() {
        super.layout()
        var rest = bounds
        if let sidebar, !sidebar.isHidden {
            sidebar.frame = NSRect(x: 0, y: 0, width: SidebarView.width, height: bounds.height)
            rest.origin.x += SidebarView.width
            rest.size.width -= SidebarView.width
        }
        tabBar?.frame = NSRect(
            x: rest.minX, y: rest.maxY - TabBarView.height, width: rest.width, height: TabBarView.height)
        rest.size.height -= TabBarView.height
        tiles?.frame = rest
        if let attention, let container = tiles as? TileContainerView {
            attention.frame = rest
            attention.relayout(frames: container.visibleTileFrames())
        }
        if let banner, !banner.isHidden {
            let width = min(rest.width - 24, 760)
            let height = banner.preferredHeight(width: width)
            banner.frame = NSRect(x: rest.midX - width / 2, y: rest.maxY - height - 10, width: width, height: height)
        }
        if let palette, !palette.isHidden {
            let width = max(280, min(rest.width - 32, 640))
            let height = min(palette.preferredHeight, rest.height - 24)
            palette.frame = NSRect(x: rest.midX - width / 2, y: rest.maxY - height - 12, width: width, height: height)
        }
        if let hud {
            let size = hud.fittingSize
            hud.frame = NSRect(
                x: rest.midX - size.width / 2, y: rest.minY + 16, width: size.width, height: size.height)
        }
    }
}

/// A small status pill over the tiles: "prefix", "resize mode", and short
/// notes for shell actions that haven't landed yet.
@MainActor
final class HUDView: NSView {
    private let label = NSTextField(labelWithString: "")
    private var hideWork: DispatchWorkItem?

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = 8
        layer?.backgroundColor = NSColor.black.withAlphaComponent(0.75).cgColor
        label.textColor = .white
        label.font = .systemFont(ofSize: 12, weight: .medium)
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            label.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            label.topAnchor.constraint(equalTo: topAnchor, constant: 6),
            label.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -6),
        ])
        isHidden = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func show(_ text: String, sticky: Bool) {
        hideWork?.cancel()
        label.stringValue = text
        isHidden = false
        superview?.needsLayout = true
        guard !sticky else { return }
        let work = DispatchWorkItem { [weak self] in self?.isHidden = true }
        hideWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5, execute: work)
    }

    func hide() {
        hideWork?.cancel()
        isHidden = true
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}
