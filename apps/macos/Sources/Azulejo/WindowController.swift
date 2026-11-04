import AppKit

/// One app window: sidebar, tab bar and the active tab's tiles. Owns no
/// state of its own beyond view state; everything comes from the core.
@MainActor
final class WindowController: NSObject, NSWindowDelegate {
    let windowID: String
    let window: NSWindow
    let tiles = TileContainerView()
    private let sidebar = SidebarView()
    /// The sidebar's rounded card: the traffic lights sit in its top band
    /// and the sidebar's rows below them (D.2).
    private let sidebarCard = SidebarCardView()
    private let tabBar = TabStripView()
    let search = ToolbarSearchView()
    private lazy var sidebarToggle = SidebarToggleButton { [weak self] in
        self?.app?.perform(action: "toggle_sidebar")
    }
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
            sidebarCard.isHidden = !sidebarVisible
            content.needsLayout = true
        }
    }

    init(windowID: String, app: AppController) {
        self.windowID = windowID
        self.app = app
        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1100, height: 700),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false)
        super.init()
        window.isReleasedWhenClosed = false
        window.title = "Azulejo"
        // D.2: no visible title bar; the toolbar row is ours and the traffic
        // lights sit in the sidebar card (`TrafficLights`).
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.titlebarSeparatorStyle = .none
        window.backgroundColor = Design.Color.window
        window.minSize = NSSize(width: 480, height: 320)
        window.tabbingMode = .disallowed
        content.sidebarCard = sidebarCard
        content.sidebar = sidebar
        content.sidebarToggle = sidebarToggle
        content.tabBar = tabBar
        content.search = search
        content.tiles = tiles
        content.hud = hud
        content.attention = attention
        content.banner = banner
        content.palette = palette
        // The card draws the sidebar's (rounded) background; the rows sit on it.
        sidebar.layer?.backgroundColor = nil
        [sidebarCard, sidebar, sidebarToggle, tabBar, search, tiles, attention, banner, hud, palette]
            .forEach(content.addSubview)
        content.wantsLayer = true
        window.contentView = content
        window.delegate = self
        search.apply(keymap: app.core.keymap())
        search.onActivate = { [weak self, weak app] in
            guard let self, let app else { return }
            app.openQuickOpen(in: self)
        }
        palette.onShownChange = { [weak self] shown in self?.search.active = shown }
        tabBar.onAdd = { [weak app] in app?.perform(action: "new_tab") }

        sidebar.onSelect = { [weak app] id in app?.send(["method": "workspace.focus", "params": ["workspace": id]]) }
        sidebar.onAdd = { [weak app, windowID] machine in app?.newWorkspace(on: machine, window: windowID) }
        sidebar.onReturnFocus = { [weak self] in
            guard let self else { return }
            window.makeFirstResponder(nil)
            tiles.focusedView?.setFocused(true)
        }
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
        Perf.measure("apply.sidebar") { sidebar.apply(model, machines: app?.machines ?? []) }
        Perf.measure("apply.tabbar") { tabBar.apply(model.workspace) }
        Perf.measure("apply.tiles") { tiles.apply(model) }
        Perf.measure("apply.attention") { attention.apply(model.workspace?.tab, frames: tiles.visibleTileFrames()) }
        let ws = model.workspace
        let title = [ws?.name, ws?.tab?.label].compactMap { $0 }.joined(separator: " — ")
        if window.title != title { window.title = title }
        // AppKit lays the hidden title bar out again on its own (a new title,
        // among others) and puts the traffic lights back at their default
        // spot, behind the sidebar card. Put them back after every refresh,
        // and once more after AppKit's own pass. `place` only moves what's off.
        TrafficLights.place(in: window)
        DispatchQueue.main.async { [weak window] in if let window { TrafficLights.place(in: window) } }
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
    /// Automation: each sidebar row's branch (F26).
    var sidebarBranches: [String] { sidebar.branches }
    /// Automation: the sidebar itself (Agents | Files, rows, footer).
    var sidebarView: SidebarView { sidebar }
    var tabMarkers: [String] { tabBar.markers }

    /// The keymap changed (config reload): the search field's chord badge.
    func refreshKeymap(_ keymap: [String: Any]) {
        search.apply(keymap: keymap)
    }

    /// Automation: each tab's state icon.
    var tabStatuses: [String] { tabBar.statuses }

    /// Re-lays out the chrome (palette or banner changed size).
    func layoutChrome() {
        content.needsLayout = true
        content.layoutSubtreeIfNeeded()
    }

    // MARK: HUD

    /// Shows `text` briefly, in sentence case: core messages start lower
    /// case because the CLI prints them after an error code.
    func flash(_ text: String) { hud.show(text.prefix(1).uppercased() + text.dropFirst(), sticky: false) }
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
        TrafficLights.place(in: window)
    }

    func windowDidResignKey(_ notification: Notification) {
        app?.windowResignedKey(self)
        TrafficLights.place(in: window)
    }

    func windowDidEnterFullScreen(_ notification: Notification) { content.needsLayout = true }

    func windowDidExitFullScreen(_ notification: Notification) {
        content.needsLayout = true
        TrafficLights.place(in: window)
    }

    func windowWillClose(_ notification: Notification) {
        sidebar.closePanel()
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        app?.closeWindow(self)
        return false
    }
}

/// The window's chrome (D.2), from the mockup: the sidebar card down the
/// left with the traffic lights in its top band, a toolbar row along the top
/// holding the tab strip and the search field, and the tiles below. Manual
/// layout keeps it predictable while tiles are being moved around.
///
///     ╭─────────╮ ( tabs … + )                 [ Jump to …   ⌘K ]
///     │ ● ● ●  ▯│ ┌─────────────────── tiles ───────────────────┐
///     │ sidebar │ │                                              │
///     ╰─────────╯ └──────────────────────────────────────────────┘
@MainActor
final class ChromeLayoutView: NSView {
    weak var sidebarCard: NSView?
    weak var sidebar: NSView?
    weak var sidebarToggle: NSView?
    weak var tabBar: TabStripView?
    weak var search: ToolbarSearchView?
    weak var tiles: NSView?
    weak var hud: NSView?
    weak var attention: AttentionOverlayView?
    weak var banner: BannerView?
    weak var palette: PaletteView?

    // The content covers the whole window (full-size content view) and paints
    // the window colour itself: with a transparent title bar, AppKit doesn't
    // reliably draw the window's background under a layer-backed content
    // view, and the gaps around the cards came out see-through.
    override var wantsUpdateLayer: Bool { true }
    override var isOpaque: Bool { true }

    override func updateLayer() {
        layer?.backgroundColor = Design.Color.window.cgColor
    }

    /// The toolbar row along the top. Its empty space moves the window; the
    /// tabs, "+", the toggle and the search field take their own clicks.
    var toolbarRect: NSRect {
        NSRect(x: 0, y: bounds.height - Design.Space.toolbarHeight,
               width: bounds.width, height: Design.Space.toolbarHeight)
    }

    override func layout() {
        super.layout()
        let inset = Design.Space.windowInset
        // The toolbar row's centre line: level with the traffic lights.
        let rowMidY = bounds.height - Design.Space.titlebarLightsY - 7
        let sidebarShown = sidebar.map { !$0.isHidden } ?? false
        let fullScreen = window?.styleMask.contains(.fullScreen) == true
        var tilesLeft = inset
        var toolbarLeft: CGFloat
        if sidebarShown {
            let card = NSRect(x: inset, y: inset, width: SidebarView.width, height: max(0, bounds.height - 2 * inset))
            sidebarCard?.frame = card
            sidebar?.frame = NSRect(x: card.minX, y: card.minY, width: card.width,
                                    height: max(0, card.height - Design.Space.titlebarZone))
            sidebarToggle?.frame = NSRect(
                x: card.maxX - Design.Space.titlebarToggleInset - 11, y: rowMidY - 11, width: 22, height: 22)
            tilesLeft = card.maxX + inset
            toolbarLeft = card.maxX + Design.Space.toolbarGap
        } else {
            // The toggle after the traffic lights (or first, in full screen).
            let lightsEnd = fullScreen ? inset - 6 : Design.Space.titlebarLightsX + 2 * 20 + 14
            sidebarToggle?.frame = NSRect(x: lightsEnd + 12, y: rowMidY - 11, width: 22, height: 22)
            toolbarLeft = lightsEnd + 12 + 22 + Design.Space.toolbarGap
        }

        // The search field at the row's right end; the tabs get what's left.
        var searchLeft = bounds.width - inset
        if let search {
            let room = max(0, bounds.width - inset - toolbarLeft)
            let width = max(min(Design.Space.searchWidth, room * 0.4), min(Design.Space.searchMinWidth, room))
            let height = Design.Space.searchHeight
            search.frame = NSRect(x: bounds.width - inset - width, y: rowMidY - height / 2, width: width, height: height)
            searchLeft = search.frame.minX
        }
        if let tabBar {
            let height = TabStripView.height
            let room = max(0, searchLeft - Design.Space.toolbarGap - toolbarLeft)
            tabBar.frame = NSRect(x: toolbarLeft, y: rowMidY - height / 2,
                                  width: min(tabBar.naturalWidth, room), height: height)
        }

        let rest = NSRect(x: tilesLeft, y: inset,
                          width: max(0, bounds.width - inset - tilesLeft),
                          height: max(0, bounds.height - Design.Space.toolbarHeight - inset))
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
            // D.8: a dropdown from the search field: right edges aligned, its
            // own field over the toolbar field.
            let anchor = search?.frame ?? NSRect(x: rest.maxX, y: rest.maxY, width: 0, height: 0)
            let width = min(max(anchor.width, 560), max(0, bounds.width - 2 * inset))
            let top = anchor.maxY + 4
            let height = max(0, min(palette.preferredHeight, top - inset))
            palette.frame = NSRect(x: max(inset, anchor.maxX - width), y: top - height, width: width, height: height)
        }
        if let hud {
            let size = hud.fittingSize
            hud.frame = NSRect(
                x: rest.midX - size.width / 2, y: rest.minY + 16, width: size.width, height: size.height)
        }
        if let window { TrafficLights.place(in: window) }
    }

    /// Empty toolbar space moves the window; a double click there does what
    /// the user's "double-click a window's title bar" setting says.
    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        guard toolbarRect.contains(point), let window else { return super.mouseDown(with: event) }
        if event.clickCount == 2 {
            switch UserDefaults.standard.string(forKey: "AppleActionOnDoubleClick") {
            case "Minimize": window.performMiniaturize(nil)
            case "None": break
            default: window.performZoom(nil)
            }
            return
        }
        window.performDrag(with: event)
    }
}

/// The sidebar's rounded card (D.2); D.3 styles the rows inside it. Its top
/// band holds the traffic lights and moves the window like a title bar.
@MainActor
final class SidebarCardView: NSView {
    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.card
        layer?.borderWidth = Design.Border.hairline
        applyColours()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private func applyColours() {
        layer?.backgroundColor = Design.Color.sidebar.cgColor
        layer?.borderColor = Design.Color.border.cgColor
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        applyColours()
    }

    override func mouseDown(with event: NSEvent) { window?.performDrag(with: event) }
}

/// Puts the traffic lights in the sidebar card's top band (D.2). AppKit lays
/// the title bar out again on resizes, key changes and full screen, so the
/// chrome calls this after each of those (and on every chrome layout).
@MainActor
enum TrafficLights {
    static func place(in window: NSWindow) {
        guard !window.styleMask.contains(.fullScreen),
              let close = window.standardWindowButton(.closeButton),
              let titlebar = close.superview, let container = titlebar.superview,
              let theme = window.contentView?.superview
        else { return }
        // A title bar as tall as the toolbar row, so the lights sit level
        // with the row's middle and still take their own clicks. Everything
        // else in it falls through to the content view (it's transparent).
        let height = Design.Space.toolbarHeight
        let want = NSRect(x: 0, y: theme.bounds.height - height, width: theme.bounds.width, height: height)
        if container.frame != want { container.frame = want }
        if titlebar.frame != container.bounds { titlebar.frame = container.bounds }
        let types: [NSWindow.ButtonType] = [.closeButton, .miniaturizeButton, .zoomButton]
        for (i, type) in types.enumerated() {
            guard let button = window.standardWindowButton(type), let parent = button.superview else { continue }
            let size = button.frame.size
            let inTheme = NSRect(
                x: Design.Space.titlebarLightsX + CGFloat(i) * 20,
                y: theme.bounds.height - Design.Space.titlebarLightsY - size.height,
                width: size.width, height: size.height)
            let origin = parent.convert(inTheme, from: theme).origin
            if button.frame.origin != origin { button.setFrameOrigin(origin) }
        }
    }

    /// Automation: the close button's centre from the window's top left
    /// ("29,31"), or "hidden".
    static func describe(_ window: NSWindow) -> String {
        guard let close = window.standardWindowButton(.closeButton), let theme = window.contentView?.superview
        else { return "-" }
        if close.isHiddenOrHasHiddenAncestor || close.alphaValue == 0 || close.superview?.alphaValue == 0 {
            return "hidden"
        }
        let r = close.convert(close.bounds, to: theme)
        return "\(Int(r.midX.rounded())),\(Int((theme.bounds.height - r.midY).rounded()))"
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
        layer?.cornerRadius = Design.Radius.hud
        layer?.backgroundColor = Design.Color.hud.cgColor
        label.textColor = Design.Color.textOnAccent
        label.font = Design.Font.hud
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

extension WindowController {
    /// Automation keys for the window chrome (D.2/D.4/D.8):
    /// `chrome.lights` (close button centre from the top left, "29,31"),
    /// `chrome.hit.<part>` (the view class a click there reaches: `tab1`,
    /// `add`, `search`, `toggle`, `toolbar` (an empty spot in the row),
    /// `close`), `chrome.tiles` (the tile area, top-left coordinates),
    /// `chrome.fits`, `tab.statuses`, `search.chord`, `search.active` and
    /// `palette.anchor` (`search` while the ⌘K dropdown hangs from the field).
    func chromeState() -> [String: String] {
        var s: [String: String] = ["chrome.lights": TrafficLights.describe(window)]
        guard let theme = window.contentView?.superview else { return s }
        func hit(_ view: NSView?, at point: NSPoint? = nil) -> String {
            guard let view, !view.isHidden else { return "hidden" }
            let p = point ?? view.convert(NSPoint(x: view.bounds.midX, y: view.bounds.midY), to: theme)
            return theme.hitTest(p).map { "\(type(of: $0))" } ?? "nil"
        }
        s["chrome.hit.tab1"] = hit(tabBar.subviews.first { $0 is TabButton })
        s["chrome.hit.add"] = hit(tabBar.subviews.first { $0 is NSButton })
        s["chrome.hit.search"] = hit(search)
        s["chrome.hit.toggle"] = hit(sidebarToggle)
        // An empty spot in the row: between the tab strip and the search field.
        let gap = NSPoint(x: (tabBar.frame.maxX + search.frame.minX) / 2, y: search.frame.midY)
        s["chrome.hit.toolbar"] = hit(content, at: content.convert(gap, to: theme))
        s["chrome.hit.close"] = hit(window.standardWindowButton(.closeButton))
        let t = tiles.frame
        s["chrome.tiles"] = "\(Int(t.minX)),\(Int(content.bounds.height - t.maxY)),\(Int(t.width)),\(Int(t.height))"
        s["chrome.fits"] = tabBar.frame.maxX <= search.frame.minX ? "yes" : "no"
        s["tab.statuses"] = tabBar.statuses.joined(separator: ",")
        s["search.chord"] = search.chordText.isEmpty ? "-" : search.chordText
        s["search.active"] = search.active ? "yes" : "no"
        if palette.isHidden {
            s["palette.anchor"] = "-"
        } else {
            let p = palette.frame, f = search.frame
            s["palette.anchor"] = abs(p.maxX - f.maxX) < 1 && p.maxY >= f.maxY ? "search" : "elsewhere"
        }
        return s
    }
}
