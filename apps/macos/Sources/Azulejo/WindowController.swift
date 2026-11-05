import AppKit

/// One app window: sidebar, tab bar and the active tab's tiles. Owns no
/// state of its own beyond view state; everything comes from the core.
@MainActor
final class WindowController: NSObject, NSWindowDelegate {
    let windowID: String
    let window: NSWindow
    let tiles = TileContainerView()
    private let sidebar = SidebarView()
    /// The collapsed sidebar's rail (`[sidebar] collapsed = "rail"`).
    private let rail = SidebarRailView()
    /// The sidebar's rounded card: the traffic lights sit in its top band
    /// and the sidebar's rows below them (D.2).
    private let sidebarCard = SidebarCardView()
    /// Automation: the sidebar card's vibrancy material, if the theme has one.
    var sidebarMaterial: String? { sidebarCard.materialName }
    private let tabBar = TabStripView()
    let search = ToolbarSearchView()
    /// The layout picker, left of the search field.
    let layoutPicker = LayoutPickerView()
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

    /// How the sidebar shows: full, a rail or hidden. It changes with layout
    /// motion (the card changes shape or slides and fades, the tiles grow or
    /// shrink to match); instantly on the first refresh, and when motion is
    /// off.
    var sidebarState: SidebarState = .full {
        didSet {
            guard sidebarState != oldValue else { return }
            // The keyboard doesn't stay in a sidebar that's going away (the
            // Files panel): back to the focused tile, as Esc would.
            if sidebarState != .full, let responder = window.firstResponder as? NSView,
               responder.isDescendant(of: sidebar) {
                sidebar.onReturnFocus?()
            }
            content.setSidebar(sidebarState, animated: model != nil && window.isVisible)
        }
    }

    /// The full sidebar is open (`toggle_sidebar` collapses it, else opens it).
    var sidebarVisible: Bool { sidebarState == .full }

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
        // The theme's appearance (darkAqua or aqua): system controls, menus
        // and scrollers match it. Theme.set keeps it current.
        window.appearance = Theme.current.appearance.nsAppearance
        window.minSize = NSSize(width: 480, height: 320)
        window.tabbingMode = .disallowed
        content.sidebarCard = sidebarCard
        content.sidebar = sidebar
        content.rail = rail
        content.sidebarToggle = sidebarToggle
        content.tabBar = tabBar
        content.search = search
        content.picker = layoutPicker
        content.tiles = tiles
        content.hud = hud
        content.attention = attention
        content.banner = banner
        content.palette = palette
        // The card draws the sidebar's (rounded) background; the rows, full
        // or the rail's, sit in it and it clips them as it changes shape.
        sidebar.layer?.backgroundColor = nil
        sidebarCard.addSubview(sidebar)
        sidebarCard.addSubview(rail)
        rail.isHidden = true
        [sidebarCard, sidebarToggle, tabBar, layoutPicker, search, tiles, attention, banner, hud, palette]
            .forEach(content.addSubview)
        content.wantsLayer = true
        window.contentView = content
        window.delegate = self
        let keymap = app.core.keymap()
        search.apply(keymap: keymap)
        layoutPicker.apply(keymap: keymap)
        // A kind's key does the same: `toggle_layout_*`, for a kind the tab
        // isn't in (where toggling is setting).
        layoutPicker.onPick = { [weak self, weak app] kind in
            guard let self, let app, let tab = self.model?.workspace?.tab, tab.kind != kind else { return }
            app.perform(action: kind.toggleAction, target: .tab(tab.id), in: self)
        }
        search.onActivate = { [weak self, weak app] in
            guard let self, let app else { return }
            app.openQuickOpen(in: self)
        }
        palette.onShownChange = { [weak self] shown in self?.search.active = shown }
        tabBar.onAdd = { [weak app] in app?.perform(action: "new_tab") }
        tabBar.onMenu = { [weak self] id in self.flatMap { ContextMenus.tabMenu(id, in: $0) } }
        // A tab dropped in a new place: the core's `tab.move`, as Move Left /
        // Right in its menu.
        tabBar.onMove = { [weak self, weak app] id, index in
            guard let self, let app else { return }
            app.perform(["command": ["method": "tab.move", "params": ["tab": id, "index": index]]], in: self)
        }

        sidebar.onSelect = { [weak app] id in app?.send(["method": "workspace.focus", "params": ["workspace": id]]) }
        sidebar.onAdd = { [weak app, windowID] machine in app?.newWorkspace(on: machine, window: windowID) }
        sidebar.onMenu = { [weak self] id in self.flatMap { ContextMenus.workspaceMenu(id, in: $0) } }
        sidebar.onReturnFocus = { [weak self] in
            guard let self else { return }
            window.makeFirstResponder(nil)
            tiles.focusedView?.setFocused(true)
        }
        rail.onSelect = sidebar.onSelect
        rail.onMenu = sidebar.onMenu
        // A dropped drag (full sidebar or rail): the core moves the
        // workspace, and the rows are rebuilt in the new order at once, so
        // the dragged copy lands on its new row.
        sidebar.onMove = { [weak app] id, index in
            app?.send(["method": "workspace.move", "params": ["workspace": id, "index": index]])
            app?.refresh()
        }
        rail.onMove = sidebar.onMove
        rail.onExpand = { [weak app, windowID] in
            app?.send(["method": "window.set_sidebar", "params": ["window": windowID, "visible": true]])
        }
        tabBar.onSelect = { [weak app] id in app?.send(["method": "tab.focus", "params": ["tab": id]]) }
        tiles.onClickTile = { [weak app] id in app?.send(["method": "tile.focus", "params": ["tile": id]]) }
        tiles.onSetRatio = { [weak app] tab, path, ratio in
            var params: [String: Any] = ["tab": tab, "path": path]
            if let ratio { params["ratio"] = ratio }
            app?.send(["method": "layout.set_ratio", "params": params])
        }
        attention.header = { [weak tiles = self.tiles] id in tiles?.header(of: id) }
    }

    /// The keymap context for the focused tile (`"terminal"`, …).
    var keyContext: String? { tiles.focusedView?.keyContext }

    func apply(_ model: WindowModel) {
        let first = self.model == nil
        if first { placeWindow(model.frame) }
        // Before `self.model` is set: the first refresh shows it as it was.
        if sidebarState != model.sidebarState { sidebarState = model.sidebarState }
        self.model = model
        Perf.measure("apply.sidebar") { sidebar.apply(model, machines: app?.machines ?? []) }
        Perf.measure("apply.rail") { rail.apply(model, machines: app?.machines ?? []) }
        Perf.measure("apply.tabbar") { tabBar.apply(model.workspace) }
        layoutPicker.apply(model.workspace?.tab?.kind)
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

    /// Automation, for captures: the tile area in window points from the
    /// top left, and the sidebar card as it shows now in the tile area's
    /// coordinates ("container=246,60,846,632 card=-238,0,230,684"; "-"
    /// while it's hidden).
    var describeChrome: String {
        let t = tiles.frame
        let container = "\(Int(t.minX)),\(Int(content.bounds.height - t.maxY)),\(Int(t.width)),\(Int(t.height))"
        return "container=\(container) card=\(content.describeSidebarCard ?? "-") "
            + "palette=\(palette.motion.rawValue):\(palette.describeLook)"
    }

    /// Automation: the sidebar's and tab bar's attention markers.
    var sidebarMarkers: [String] { sidebar.markers }
    /// Automation: the sidebar's machine group headers.
    var sidebarMachines: [String] { sidebar.machineHeaders }
    /// Automation: each sidebar row's branch (F26).
    var sidebarBranches: [String] { sidebar.branches }
    /// Automation: the sidebar itself (Agents | Files, rows, footer).
    var sidebarView: SidebarView { sidebar }
    /// Automation: the collapsed sidebar's rail.
    var railView: SidebarRailView { rail }
    var tabMarkers: [String] { tabBar.markers }
    /// Automation: the tab strip (its tabs' right-click menus).
    var tabStrip: TabStripView { tabBar }

    /// The keymap changed (config reload): the search field's chord badge.
    func refreshKeymap(_ keymap: [String: Any]) {
        search.apply(keymap: keymap)
        layoutPicker.apply(keymap: keymap)
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
    /// case because the CLI prints them after an error code. Every flash is
    /// logged and counted (automation: `hud.flashes`, `hud.last`).
    func flash(_ text: String) {
        let shown = text.prefix(1).uppercased() + text.dropFirst()
        flashes += 1
        lastFlash = shown
        app?.log("hud: flash \"\(shown)\"")
        hud.show(shown, sticky: false)
    }

    /// A failed command a key, menu or quick open asked for: flashed,
    /// unless there was simply nothing there (focus or swap past the edge,
    /// a resize with no divider), which does nothing, silently.
    func report(_ error: CoreError) {
        if error.nothingThere {
            app?.log("hud: quiet, \(error)")
            return
        }
        flash(error.message)
    }

    private(set) var flashes = 0
    private(set) var lastFlash = ""
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
/// holding the tab strip, the layout picker and the search field, and the
/// tiles below. Manual
/// layout keeps it predictable while tiles are being moved around.
///
///     ╭─────────╮ ( tabs … + )        [▤▥▣■] [ Jump to …   ⌘K ]
///     │ ● ● ●  ▯│ ┌─────────────────── tiles ───────────────────┐
///     │ sidebar │ │                                              │
///     ╰─────────╯ └──────────────────────────────────────────────┘
@MainActor
final class ChromeLayoutView: NSView {
    weak var sidebarCard: NSView?
    weak var sidebar: NSView?
    weak var rail: NSView?
    weak var sidebarToggle: NSView?
    weak var tabBar: TabStripView?
    weak var search: ToolbarSearchView?
    weak var picker: LayoutPickerView?
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
        // Redrawn on every theme change (Theme.set marks every view).
        layer?.backgroundColor = Design.Color.window.cgColor
    }

    // MARK: Sidebar motion

    /// How the sidebar shows. Full, the card runs down the window's left
    /// with the traffic lights in its top band; as a rail it's a narrow card
    /// under the toolbar row, the lights on the window colour above it;
    /// hidden, it waits off the window's left edge.
    private(set) var sidebarState: SidebarState = .full
    /// Set for the one layout pass that changes the sidebar: the tiles move
    /// as a layout motion instead of jumping.
    private var animatingSidebar = false
    /// Bumped by each change; a finished one only tidies up (hides what
    /// faded out) if it's still the latest.
    private var sidebarGeneration = 0
    private static let chromeKey = "azj.chrome"

    /// Opens, collapses (to a rail) or hides the sidebar. Animated (and
    /// motion on), the card changes shape (full ↔ rail: its top and right
    /// edges move, and it clips its rows) or slides and fades (hidden),
    /// while the tile area grows or shrinks to match with the tiles' own
    /// motion (same length, curve and style; crop rules for terminals). The
    /// full rows and the rail's cross-fade; the toggle and the tab strip
    /// slide along. The card's right edge and the tiles' left edge move in
    /// step, a gutter apart. A change mid-motion starts from where
    /// everything shows now. The traffic lights don't move.
    func setSidebar(_ state: SidebarState, animated: Bool) {
        guard state != sidebarState else { return }
        sidebarState = state
        sidebarGeneration += 1
        let generation = sidebarGeneration
        let animate = animated && Motion.active && window != nil && bounds.width > 1 && bounds.height > 1
        let others: [NSView?] = [sidebarCard, sidebarToggle, tabBar, picker, search, banner, hud]
        let sliding = others.compactMap { $0 }
        // The rows inside the card: shown in their own form only.
        let rows: [(view: NSView, shown: Bool)] = [(sidebar, state == .full), (rail, state == .rail)]
            .compactMap { view, shown in view.map { ($0, shown) } }
        var before: [ObjectIdentifier: (frame: CGRect, opacity: Float)] = [:]
        for view in sliding + rows.map(\.view) {
            if animate { before[ObjectIdentifier(view)] = Self.visual(view) }
            view.layer?.removeAnimation(forKey: Self.chromeKey)
        }
        // What shows, shows now; what goes stays up while it fades out.
        if state != .hidden || !animate { sidebarCard?.isHidden = state == .hidden }
        for (view, shown) in rows where shown || !animate { view.isHidden = !shown }

        animatingSidebar = animate
        needsLayout = true
        layoutSubtreeIfNeeded()
        animatingSidebar = false
        guard animate else { return }

        let duration = Motion.duration
        CATransaction.begin()
        CATransaction.setCompletionBlock { [weak self] in
            guard let self, generation == self.sidebarGeneration else { return }
            if let card = self.sidebarCard {
                card.isHidden = self.sidebarState == .hidden
                card.layer?.removeAnimation(forKey: Self.chromeKey)
            }
            for (view, shown) in rows {
                view.isHidden = !shown
                view.layer?.removeAnimation(forKey: Self.chromeKey)
            }
        }
        for view in sliding {
            guard let layer = view.layer, let was = before[ObjectIdentifier(view)] else { continue }
            let isCard = view === sidebarCard
            let group = CAAnimationGroup()
            var parts: [CAAnimation] = []
            if !Self.close(was.frame, view.frame) {
                let position = CABasicAnimation(keyPath: "position")
                position.fromValue = NSValue(point: Motion.position(of: was.frame, anchorPoint: layer.anchorPoint))
                position.toValue = NSValue(point: Motion.position(of: view.frame, anchorPoint: layer.anchorPoint))
                parts.append(position)
                if was.frame.size != view.frame.size {
                    // Full ↔ rail: the card changes shape round its rows.
                    let bounds = CABasicAnimation(keyPath: "bounds")
                    bounds.fromValue = NSValue(rect: CGRect(origin: layer.bounds.origin, size: was.frame.size))
                    bounds.toValue = NSValue(rect: layer.bounds)
                    parts.append(bounds)
                }
            }
            if isCard {
                let fade = CABasicAnimation(keyPath: "opacity")
                fade.fromValue = was.opacity
                fade.toValue = state == .hidden ? 0 : 1
                parts.append(fade)
                // Held at 0 until the completion hides it.
                group.fillMode = .forwards
                group.isRemovedOnCompletion = false
            }
            guard !parts.isEmpty else { continue }
            group.animations = parts
            Motion.configure(group, duration: duration)
            layer.add(group, forKey: Self.chromeKey)
        }
        // The full rows and the rail's cross-fade in the card.
        for (view, shown) in rows {
            guard let layer = view.layer, let was = before[ObjectIdentifier(view)] else { continue }
            let target: Float = shown ? 1 : 0
            guard was.opacity != target else { continue }
            let fade = CABasicAnimation(keyPath: "opacity")
            fade.fromValue = was.opacity
            fade.toValue = target
            fade.fillMode = .forwards
            fade.isRemovedOnCompletion = false
            Motion.configure(fade, duration: duration)
            layer.add(fade, forKey: Self.chromeKey)
        }
        CATransaction.commit()
    }

    /// Where a view shows right now and how opaque (mid-motion: the
    /// presentation).
    private static func visual(_ view: NSView) -> (frame: CGRect, opacity: Float) {
        guard let layer = view.layer else { return (view.frame, view.isHidden ? 0 : 1) }
        if view.isHidden { return (view.frame, 0) }
        guard layer.animation(forKey: chromeKey) != nil, let p = layer.presentation() else {
            return (view.frame, layer.opacity)
        }
        return (p.frame, p.opacity)
    }

    private static func close(_ a: CGRect, _ b: CGRect) -> Bool {
        abs(a.minX - b.minX) < 0.5 && abs(a.minY - b.minY) < 0.5
            && abs(a.width - b.width) < 0.5 && abs(a.height - b.height) < 0.5
    }

    /// Automation: the sidebar card's frame as it shows now, in the tile
    /// container's coordinates (bottom-up), or nil while it's hidden.
    var describeSidebarCard: String? {
        guard let card = sidebarCard, !card.isHidden, let tiles else { return nil }
        let r = tiles.convert(Self.visual(card).frame, from: self)
        return "\(Int(r.minX)),\(Int(r.minY)),\(Int(r.width)),\(Int(r.height))"
    }

    /// The toolbar row along the top. Its empty space moves the window; the
    /// tabs, "+", the toggle, the layout picker and the search field take
    /// their own clicks.
    var toolbarRect: NSRect {
        NSRect(x: 0, y: bounds.height - Design.Space.toolbarHeight,
               width: bounds.width, height: Design.Space.toolbarHeight)
    }

    override func layout() {
        super.layout()
        let inset = Design.Space.windowInset
        // The toolbar row's centre line: level with the traffic lights.
        let rowMidY = bounds.height - Design.Space.titlebarLightsY - 7
        let fullScreen = window?.styleMask.contains(.fullScreen) == true
        var tilesLeft = inset
        var toolbarLeft: CGFloat
        let full = NSRect(x: inset, y: inset, width: SidebarView.width, height: max(0, bounds.height - 2 * inset))
        // The rail: under the toolbar row, level with the tiles' top.
        let railHeight = max(0, bounds.height - Design.Space.toolbarHeight - inset)
        var card = full
        switch sidebarState {
        case .full: break
        case .rail: card = NSRect(x: inset, y: inset, width: SidebarRailView.width, height: railHeight)
        // Hidden, the card waits just off the left edge, where it slides
        // out to and in from: its right edge a gutter left of the tiles'.
        case .hidden: card.origin.x = -card.width
        }
        sidebarCard?.frame = card
        // The rows sit in the card from its bottom left at their own size,
        // so it clips them, without a relayout, while it changes shape.
        sidebar?.frame = NSRect(x: 0, y: 0, width: full.width, height: max(0, full.height - Design.Space.titlebarZone))
        rail?.frame = NSRect(x: 0, y: 0, width: SidebarRailView.width, height: railHeight)
        if sidebarState == .full {
            sidebarToggle?.frame = NSRect(
                x: card.maxX - Design.Space.titlebarToggleInset - 11, y: rowMidY - 11, width: 22, height: 22)
            tilesLeft = card.maxX + inset
            toolbarLeft = card.maxX + Design.Space.toolbarGap
        } else {
            // The toggle after the traffic lights (or first, in full screen).
            let lightsEnd = fullScreen ? inset - 6 : Design.Space.titlebarLightsX + 2 * 20 + 14
            sidebarToggle?.frame = NSRect(x: lightsEnd + 12, y: rowMidY - 11, width: 22, height: 22)
            toolbarLeft = lightsEnd + 12 + 22 + Design.Space.toolbarGap
            if sidebarState == .rail { tilesLeft = card.maxX + inset }
        }

        // The search field at the row's right end, the layout picker left of
        // it; the tabs get what's left.
        var searchLeft = bounds.width - inset
        let room = max(0, bounds.width - inset - toolbarLeft)
        // The tab strip keeps room for the selected tab and "+" (N4c).
        let strip = tabBar.map { $0.minimumWidth + Design.Space.toolbarGap } ?? 0
        // The picker shows when it leaves the tabs and the search field their
        // least; in a narrower window it goes first (its keys still work).
        let pickerSpace = LayoutPickerView.width + Design.Space.layoutPickerGap
        let showPicker = picker?.kind != nil && room >= strip + pickerSpace + Design.Space.searchMinWidth
        picker?.isHidden = !showPicker
        if let search {
            // In a narrow window the search shrinks below its minimum before the tabs do.
            let rest = room - (showPicker ? pickerSpace : 0)
            let wanted = max(min(Design.Space.searchWidth, rest * 0.4), min(Design.Space.searchMinWidth, rest))
            let width = max(min(wanted, rest - strip), min(Design.Space.searchHeight, rest))
            let height = Design.Space.searchHeight
            search.frame = NSRect(x: bounds.width - inset - width, y: rowMidY - height / 2, width: width, height: height)
            searchLeft = search.frame.minX
        }
        if let picker, showPicker {
            let height = Design.Space.searchHeight
            picker.frame = NSRect(x: searchLeft - Design.Space.layoutPickerGap - LayoutPickerView.width,
                                  y: rowMidY - height / 2, width: LayoutPickerView.width, height: height)
            searchLeft = picker.frame.minX
        }
        if let tabBar {
            let height = TabStripView.height
            let room = max(0, searchLeft - Design.Space.toolbarGap - toolbarLeft)
            tabBar.frame = NSRect(x: toolbarLeft, y: rowMidY - height / 2,
                                  width: min(tabBar.naturalWidth, room), height: height)
        }

        var rest = NSRect(x: tilesLeft, y: inset,
                          width: max(0, bounds.width - inset - tilesLeft),
                          height: max(0, bounds.height - Design.Space.toolbarHeight - inset))
        // The config banner gets its own row under the toolbar, as wide as
        // the tile area, and the tiles move down for it: it covers no header
        // and no terminal text. Terminals resize once when it shows and once
        // when it goes, which is rare (a broken config).
        if let banner, !banner.isHidden {
            // At most a fifth of the tile area (two lines at the minimum
            // window size); a capped banner cuts its text and has the rest in
            // its tooltip.
            let height = banner.preferredHeight(width: rest.width, limit: max(32, rest.height / 5))
            banner.frame = NSRect(x: rest.minX, y: rest.maxY - height, width: rest.width, height: height)
            rest.size.height = max(0, rest.height - height - Design.Space.gutter)
        }
        if let container = tiles as? TileContainerView {
            container.setChromeFrame(rest, animated: animatingSidebar)
        } else {
            tiles?.frame = rest
        }
        if let attention, let container = tiles as? TileContainerView {
            attention.frame = rest
            attention.relayout(frames: container.visibleTileFrames())
        }
        if let palette, palette.isOpen {
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
/// Themes with a sidebar material (the macOS ones) show it through a
/// vibrancy view instead of the flat `sidebar` colour.
@MainActor
final class SidebarCardView: NSView, ThemeAware {
    private var vibrancy: NSVisualEffectView?

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        // The rows (full or the rail's) are subviews, clipped to the card as
        // it changes shape between the two.
        clipsToBounds = true
        layer?.masksToBounds = true
        layer?.cornerRadius = Design.Radius.card
        layer?.borderWidth = Design.Border.hairline
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func applyTheme() {
        let material = Theme.current.sidebarMaterial.flatMap(Self.material)
        if let material {
            let effect = vibrancy ?? NSVisualEffectView()
            if vibrancy == nil {
                effect.blendingMode = .behindWindow
                effect.state = .followsWindowActiveState
                effect.wantsLayer = true
                effect.layer?.cornerRadius = Design.Radius.card
                effect.layer?.masksToBounds = true
                effect.frame = bounds
                effect.autoresizingMask = [.width, .height]
                addSubview(effect, positioned: .below, relativeTo: nil)
                vibrancy = effect
            }
            effect.material = material
            layer?.backgroundColor = nil
        } else {
            vibrancy?.removeFromSuperview()
            vibrancy = nil
            layer?.backgroundColor = Design.Color.sidebar.cgColor
        }
        layer?.borderColor = Design.Color.border.cgColor
    }

    /// Automation: whether the card shows a vibrancy material.
    var materialName: String? { vibrancy.map { _ in Theme.current.sidebarMaterial ?? "?" } }

    private static func material(_ name: String) -> NSVisualEffectView.Material? {
        switch name {
        case "sidebar": .sidebar
        case "menu": .menu
        case "popover": .popover
        case "headerView": .headerView
        case "windowBackground": .windowBackground
        case "underWindowBackground": .underWindowBackground
        case "contentBackground": .contentBackground
        case "titlebar": .titlebar
        case "hudWindow": .hudWindow
        default: nil
        }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
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
    /// ("29,26"), or "hidden".
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
final class HUDView: NSView, ThemeAware {
    private let label = NSTextField(labelWithString: "")
    private var hideWork: DispatchWorkItem?

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.hud
        Theme.track(self)
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

    func applyTheme() {
        layer?.backgroundColor = Design.Color.hud.cgColor
    }

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
    /// `chrome.lights` (close button centre from the top left, "29,26"),
    /// `chrome.hit.<part>` (the view class a click there reaches: `tab1`,
    /// `add`, `search`, `picker` (its Monocle segment), `toggle`, `toolbar` (an empty spot in the row),
    /// `close`), `chrome.tiles` (the tile area, top-left coordinates),
    /// `chrome.fits`, `tab.statuses`, `search.chord`, `search.active` and
    /// `palette.anchor` (`search` while the ⌘K dropdown hangs from the field).
    func chromeState() -> [String: String] {
        var s: [String: String] = ["chrome.lights": TrafficLights.describe(window)]
        s["hud.flashes"] = "\(flashes)"
        s["hud.last"] = lastFlash.isEmpty ? "-" : lastFlash
        guard let theme = window.contentView?.superview else { return s }
        func hit(_ view: NSView?, at point: NSPoint? = nil) -> String {
            guard let view, !view.isHidden else { return "hidden" }
            let p = point ?? view.convert(NSPoint(x: view.bounds.midX, y: view.bounds.midY), to: theme)
            return theme.hitTest(p).map { "\(type(of: $0))" } ?? "nil"
        }
        s["chrome.hit.tab1"] = hit(tabBar.firstTab)
        s["chrome.hit.add"] = hit(tabBar.subviews.first { $0 is NSButton })
        s["chrome.hit.search"] = hit(search)
        s["chrome.hit.picker"] = hit(layoutPicker.isHidden ? nil : layoutPicker.segment(.monocle))
        s["chrome.hit.toggle"] = hit(sidebarToggle)
        let right = layoutPicker.isHidden ? search.frame.minX : layoutPicker.frame.minX
        // An empty spot in the row: between the tab strip and the picker (or the search field).
        let gap = NSPoint(x: (tabBar.frame.maxX + right) / 2, y: search.frame.midY)
        s["chrome.hit.toolbar"] = hit(content, at: content.convert(gap, to: theme))
        s["chrome.hit.close"] = hit(window.standardWindowButton(.closeButton))
        let t = tiles.frame
        s["chrome.tiles"] = "\(Int(t.minX)),\(Int(content.bounds.height - t.maxY)),\(Int(t.width)),\(Int(t.height))"
        // The tabs end before the picker, the picker before the search field.
        s["chrome.fits"] = tabBar.frame.maxX <= right && (layoutPicker.isHidden
            || layoutPicker.frame.maxX <= search.frame.minX) ? "yes" : "no"
        // As a rail, the card sits clear of the traffic lights, the toggle,
        // the tab strip and the tiles (a gutter from them): what it
        // overlaps, or "none".
        if sidebarState == .rail {
            let card = sidebarCard.frame
            var hits: [String] = []
            let lights = [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton].compactMap {
                window.standardWindowButton($0).map { $0.convert($0.bounds, to: content) }
            }
            if lights.contains(where: { $0.intersects(card) }) { hits.append("lights") }
            if sidebarToggle.frame.intersects(card) { hits.append("toggle") }
            if tabBar.frame.intersects(card) { hits.append("tabs") }
            if tiles.frame.insetBy(dx: -Design.Space.gutter + 1, dy: 0).intersects(card) { hits.append("tiles") }
            s["chrome.rail.overlaps"] = hits.isEmpty ? "none" : hits.joined(separator: ",")
            s["chrome.rail"] = "\(Int(card.minX)),\(Int(content.bounds.height - card.maxY)),\(Int(card.width)),\(Int(card.height))"
        }
        s["tab.statuses"] = tabBar.statuses.joined(separator: ",")
        s["tab.overflow"] = tabBar.overflow
        s["tab.names"] = tabBar.tabButtons.map(\.title).joined(separator: "|")
        s["tab.hint"] = tabBar.selectedHint
        s["search.chord"] = search.chordText.isEmpty ? "-" : search.chordText
        s["search.active"] = search.active ? "yes" : "no"
        if !palette.isOpen {
            s["palette.anchor"] = "-"
        } else {
            let p = palette.frame, f = search.frame
            s["palette.anchor"] = abs(p.maxX - f.maxX) < 1 && p.maxY >= f.maxY ? "search" : "elsewhere"
        }
        return s
    }
}
