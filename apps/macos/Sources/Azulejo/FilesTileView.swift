import AppKit
@preconcurrency import Quartz
import UniformTypeIdentifiers

/// A file browser tile: the core lists one folder at a time
/// (`files.expand`, gitignore-aware, folders first in natural order) and
/// watches the folders that are open, so a 50k-file repository opens as fast
/// as a small one. Changes arrive as `files.changed` (list those folders
/// again), `files.revealed` (open folders down to a path and select it) and
/// `files.git` (decorations).
///
/// Listings never block the main thread: a home folder with thousands of
/// entries, a network volume or a folder behind a permission prompt can
/// take seconds. Each one is requested with `Core.executeAsync` and applied
/// when it lands: the root shows "Loading…" meanwhile, a folder opens once
/// its rows arrive, and a listing for a root that changed since, or older
/// than one already shown for that folder, is dropped.
///
/// Keys while the tree has focus come from the keymap's `files` context
/// (defaults: ↓↑ or j/k move, h/l close and open folders, Return opens a file
/// in the tab's preview tile or toggles a folder, ⌘↓ opens in the default
/// app, ⌘↑ goes up, Space toggles Quick Look, / filters, . shows hidden
/// files). ← → are AppKit's outline keys. While the filter has the keyboard
/// the context is `find_bar`: enter goes to the list, escape clears it.
@MainActor
final class FilesTileView: NSView, TileView, ThemeAware {
    let tileID: String
    let kind: TileKind = .files

    private var model: TileModel
    private let header = FilesHeader()
    private let filter = NSSearchField()
    /// "Loading…", "Can't list this folder." or a remote folder's error,
    /// in a strip above the tree.
    private let status = NSTextField(wrappingLabelWithString: "")
    private let scroll = NSScrollView()
    private let outline = FilesOutlineView()
    private var filterHeight: NSLayoutConstraint!
    private var statusHeight: NSLayoutConstraint!
    /// Shown as the sidebar's Files view (`files.panel`) rather than a tile:
    /// no card of its own, and Esc hands the keyboard back to the tiles.
    private(set) var isPanel = false
    var onEscape: (() -> Void)?
    /// The herdr machine whose files this panel shows; nil for
    /// Local. Remote paths aren't on this Mac: Finder, the default app and
    /// Quick Look don't apply, files open as a copy in the preview tile.
    private(set) var machine: String?
    var isRemote: Bool { machine != nil }

    /// The root's node; its children are the outline's top level.
    private var root = FileNode(path: "", name: "", dir: true)
    /// Every listed node by absolute path, for events and selection.
    private var nodes: [String: FileNode] = [:]
    private var git: [String: String] = [:]
    private var showHidden = false
    private var query = ""
    /// With a filter: each node's kept children (matches and folders holding
    /// one), by identity.
    private var filtered: [ObjectIdentifier: [FileNode]] = [:]

    // Listings in flight (see the type's docs).
    /// Bumped when the root loads (again): replies for an older root drop.
    private var generation = 0
    /// The root's listing hasn't landed yet.
    private var loadingRoot = false
    /// The core couldn't list the root at all.
    private var rootFailed = false
    /// Folders being listed for the first time.
    private var loading = Set<String>()
    /// Numbers listing requests, so an older listing never replaces a newer
    /// one of the same folder (replies can land out of order).
    private var requests = 0
    private var shownRequest: [String: Int] = [:]
    /// A `files.revealed` that came before the root's rows.
    private var pendingReveal: (path: String, dirs: [String])?

    // Timings and counters for automation and measurements.
    private let created = CACurrentMediaTime()
    private var firstRowsMs: Double?
    private var listMs: Double?
    private var changesApplied = 0
    private var lastChangeAt: Double?
    private var revealed = 0

    // MARK: Setup

    static func install(core: Core, log: @escaping (String) -> Void) {
        Self.core = core
        Self.log = log
        TileViews.register(.files) { tile in FilesTileView(tile) }
    }

    /// Hands `files.*` events to the tile they're for; a substring check keeps
    /// every other event cheap.
    static func route(_ json: String) {
        guard json.contains("\"files."),
              let data = json.data(using: .utf8),
              let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let name = object["event"] as? String,
              let body = object["data"] as? [String: Any],
              let tile = body["tile"] as? String,
              let view = live[tile]?.view
        else { return }
        switch name {
        case "files.changed": view.changed(body["dirs"] as? [String] ?? [])
        case "files.revealed":
            view.reveal(path: body["path"] as? String ?? "", dirs: body["dirs"] as? [String] ?? [])
        case "files.git": view.gitChanged(body["paths"] as? [String: String] ?? [:])
        default: break
        }
    }

    private static var core: Core?
    private static var log: (String) -> Void = { _ in }
    private static var live: [String: WeakFiles] = [:]
    private static var icons: [String: NSImage] = [:]

    init(_ tile: TileModel) {
        tileID = tile.id
        model = tile
        super.init(frame: .zero)
        wantsLayer = true
        // Rectangular clip: the card (TileFrame) rounds the corners.
        layer?.masksToBounds = true
        Theme.track(self)

        // The root and the buttons live in the tile's header strip (TileFrame).
        header.onToggleHidden = { [weak self] in self?.toggleHidden() }
        header.onFilter = { [weak self] in self?.showFilter() }

        filter.translatesAutoresizingMaskIntoConstraints = false
        filter.controlSize = .small
        filter.font = Design.Font.field
        filter.placeholderString = "Filter loaded files"
        filter.sendsSearchStringImmediately = true
        filter.target = self
        filter.action = #selector(filterChanged)
        filter.delegate = self
        filter.isHidden = true

        let column = NSTableColumn(identifier: .init("name"))
        column.resizingMask = .autoresizingMask
        outline.addTableColumn(column)
        outline.outlineTableColumn = column
        outline.headerView = nil
        outline.style = .plain
        outline.rowHeight = 20
        outline.indentationPerLevel = 14
        outline.autoresizesOutlineColumn = false
        outline.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        outline.allowsMultipleSelection = false
        outline.backgroundColor = .clear
        outline.dataSource = self
        outline.delegate = self
        outline.target = self
        outline.doubleAction = #selector(doubleClicked)
        outline.owner = self

        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.documentView = outline
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false

        status.translatesAutoresizingMaskIntoConstraints = false
        status.font = Design.Font.caption
        status.textColor = Design.Color.textMuted
        status.maximumNumberOfLines = 2
        status.lineBreakMode = .byWordWrapping
        status.cell?.truncatesLastVisibleLine = true
        status.isHidden = true

        addSubview(filter)
        addSubview(status)
        addSubview(scroll)
        filterHeight = filter.heightAnchor.constraint(equalToConstant: 0)
        statusHeight = status.heightAnchor.constraint(equalToConstant: 0)
        NSLayoutConstraint.activate([
            filter.topAnchor.constraint(equalTo: topAnchor, constant: 2),
            filter.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            filter.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            filterHeight,
            status.topAnchor.constraint(equalTo: filter.bottomAnchor),
            status.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            status.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            statusHeight,
            scroll.topAnchor.constraint(equalTo: status.bottomAnchor, constant: 2),
            scroll.leadingAnchor.constraint(equalTo: leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        Self.live[tileID] = WeakFiles(self)
        loadRoot()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// A files panel: the same tree for `panel`, a `files.panel` the core
    /// shows outside the layout.
    convenience init(panel: String, root: String, machine: String? = nil) {
        self.init(TileModel(id: panel, kind: .files, root: root))
        isPanel = true
        self.machine = machine
        layer?.cornerRadius = 0
        layer?.backgroundColor = nil
    }

    /// Opens (or re-roots) a files panel, on herdr machine `machine` when
    /// set, without waiting (the core checks `root` on disk); `then` gets
    /// its id, or nil when the core can't show `root`.
    static func openPanel(
        root: String, machine: String? = nil, reuse panel: String?,
        then: @escaping @MainActor (String?) -> Void
    ) {
        guard let core else { return then(nil) }
        var params: [String: Any] = ["root": root]
        if let panel { params["panel"] = panel }
        if let machine { params["machine"] = machine }
        core.executeAsync(
            ["method": "files.panel", "params": params],
            decode: { result in result.map { $0["tile"] as? String } },
            then: { result in
                switch result {
                case .success(let id): then(id)
                case .failure(let error):
                    log("files: panel on \(root) failed: \(error.message)")
                    then(nil)
                }
            })
    }

    static func closePanel(_ panel: String) {
        _ = core?.execute(["method": "files.panel_close", "params": ["panel": panel]])
    }

    /// Takes the keyboard (the sidebar's Files view was clicked or chosen).
    func focusTree() {
        window?.makeFirstResponder(outline)
        if outline.selectedRow < 0, outline.numberOfRows > 0 {
            outline.selectRowIndexes([0], byExtendingSelection: false)
        }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    /// The tile's body; in the sidebar (a panel) it sits on the card. Rows
    /// draw their text with the tokens, so a redraw recolours them.
    func applyTheme() {
        layer?.backgroundColor = isPanel ? nil : Design.Color.tile.cgColor
    }

    // MARK: TileView

    func update(_ tile: TileModel) {
        guard tile != model else { return }
        let rootChanged = tile.root != model.root
        model = tile
        if rootChanged { loadRoot() }
    }

    func setFocused(_ focused: Bool) {
        guard focused, let window, TileFocus.canTake(in: window) else { return }
        if window.firstResponder !== outline, window.firstResponder !== filter.currentEditor() {
            window.makeFirstResponder(outline)
        }
        if outline.selectedRow < 0, outline.numberOfRows > 0 {
            outline.selectRowIndexes([0], byExtendingSelection: false)
        }
    }

    func tileWillClose() {
        Self.live[tileID] = nil
        if QLPreviewPanel.sharedPreviewPanelExists(), QLPreviewPanel.shared().isVisible,
           QLPreviewPanel.shared().dataSource === outline {
            QLPreviewPanel.shared().orderOut(nil)
        }
    }

    override var acceptsFirstResponder: Bool { true }

    // MARK: Loading

    /// Lists the root again (a new tile, or a new root): the old tree goes
    /// at once and "Loading…" shows until the listing lands.
    private func loadRoot() {
        generation += 1
        loadingRoot = true
        rootFailed = false
        loading = []
        shownRequest = [:]
        pendingReveal = nil
        root = FileNode(path: "", name: "", dir: true)
        nodes = [:]
        git = [:]
        rebuildFilter()
        outline.reloadData()
        header.setRoot(model.root, detail: "loading…")
        showStatus()
        let started = CACurrentMediaTime()
        list(dir: nil, method: "files.expand") { [weak self] listing in
            guard let self else { return }
            self.loadingRoot = false
            guard let listing else {
                self.rootFailed = true
                self.header.setRoot(self.model.root, detail: "can't list")
                self.showStatus()
                return
            }
            self.listMs = (CACurrentMediaTime() - started) * 1000
            self.showRoot(listing)
        }
    }

    private func showRoot(_ listing: Listing) {
        root = FileNode(path: listing.dir, name: (listing.dir as NSString).lastPathComponent, dir: true)
        nodes = [listing.dir: root]
        showHidden = listing.showHidden
        apply(listing, to: root)
        rebuildFilter()
        outline.reloadData()
        header.setRoot(listing.root, detail: listing.truncated ? "truncated" : nil)
        header.setHidden(showHidden)
        if window?.firstResponder === outline, outline.selectedRow < 0, outline.numberOfRows > 0 {
            outline.selectRowIndexes([0], byExtendingSelection: false)
        }
        showStatus()
        if firstRowsMs == nil {
            outline.layoutSubtreeIfNeeded()
            outline.displayIfNeeded()
            firstRowsMs = (CACurrentMediaTime() - created) * 1000
            Self.log(String(
                format: "files: %@ first rows %.1f ms (list %.1f ms, %d rows)",
                Short.id(tileID), firstRowsMs ?? 0, listMs ?? 0, outline.numberOfRows))
        }
        if let (path, dirs) = pendingReveal {
            pendingReveal = nil
            reveal(path: path, dirs: dirs)
        }
    }

    /// The root's state in the status line: loading, can't be listed, or a
    /// remote folder's error (with the last listing still shown), hidden
    /// otherwise.
    private func showStatus() {
        let text: String? = if loadingRoot {
            "Loading…"
        } else if rootFailed {
            "Can't list this folder."
        } else if let error = root.error {
            root.children?.isEmpty == false ? "\(error) (showing the last listing)" : error
        } else if root.loading {
            "Loading…"
        } else {
            nil
        }
        status.stringValue = text ?? ""
        status.toolTip = text
        status.isHidden = text == nil
        statusHeight.constant = text == nil ? 0 : (root.error != nil ? 32 : 16)
    }

    /// Lists a folder through the core without waiting; `files.expand` also
    /// watches it. `then` runs on the main actor with the listing, or nil
    /// when the core couldn't list it; never when the root changed since.
    private func list(dir: String?, method: String, then: @escaping @MainActor (Listing?) -> Void) {
        guard let core = Self.core else { return then(nil) }
        var params: [String: Any] = ["tile": tileID]
        if let dir { params["dir"] = dir }
        requests += 1
        let request = requests
        let generation = generation
        core.executeAsync(
            ["method": method, "params": params],
            decode: { result in result.map { Listing($0, request: request) } },
            then: { [weak self] result in
                guard let self, generation == self.generation else { return }
                switch result {
                case .success(let listing):
                    if listing == nil { Self.log("files: \(method) \(dir ?? "root"): unexpected reply") }
                    then(listing)
                case .failure(let error):
                    Self.log("files: \(method) \(dir ?? "root") failed: \(error.message)")
                    then(nil)
                }
            })
    }

    /// Merges a listing into `node`: entries that stay keep their node (and
    /// so their open children); gone ones drop out of the index. False (and
    /// nothing changes) when a newer listing of the folder is already shown.
    @discardableResult
    private func apply(_ listing: Listing, to node: FileNode) -> Bool {
        guard listing.request >= shownRequest[node.path, default: 0] else { return false }
        shownRequest[node.path] = listing.request
        node.loading = listing.loading
        node.error = listing.error
        let old = Dictionary((node.children ?? []).map { ($0.path, $0) }, uniquingKeysWith: { a, _ in a })
        var kept = Set<String>()
        node.children = listing.entries.map { e in
            let child = old[e.path] ?? FileNode(path: e.path, name: e.name, dir: e.dir)
            child.name = e.name
            if child.dir != e.dir { child.children = nil }
            child.dir = e.dir
            child.symlink = e.symlink
            child.hidden = e.hidden
            child.ignored = e.ignored
            child.parent = node
            if let state = e.git { git[e.path] = state }
            kept.insert(e.path)
            nodes[e.path] = child
            return child
        }
        for (path, gone) in old where !kept.contains(path) {
            forget(gone)
        }
        return true
    }

    private func forget(_ node: FileNode) {
        nodes[node.path] = nil
        shownRequest[node.path] = nil
        for child in node.children ?? [] { forget(child) }
    }

    /// Lists a folder the first time it opens; it shows open and empty
    /// until its rows land.
    private func load(_ node: FileNode) {
        guard node.dir, node.children == nil, !loading.contains(node.path) else { return }
        loading.insert(node.path)
        list(dir: node.path, method: "files.expand") { [weak self] listing in
            guard let self else { return }
            self.loading.remove(node.path)
            guard let listing, self.nodes[node.path] === node, self.apply(listing, to: node) else { return }
            let selected = self.selectedNode
            self.rebuildFilter()
            self.outline.reloadItem(node, reloadChildren: true)
            if node.open, self.query.isEmpty {
                self.outline.expandItem(node)
                self.restoreExpansion(node)
            }
            self.keepSelection(selected)
        }
    }

    /// Selects `node` again after rows moved, without scrolling.
    private func keepSelection(_ node: FileNode?) {
        guard let node else { return }
        let row = outline.row(forItem: node)
        if row >= 0, row != outline.selectedRow {
            outline.selectRowIndexes([row], byExtendingSelection: false)
        }
    }

    // MARK: Events

    /// `files.changed`: list the open folders that changed again, then show
    /// them all at once.
    private func changed(_ dirs: [String]) {
        let open = dirs.filter { dir in
            guard let node = nodes[dir], node.children != nil else { return false }
            return node === root || outline.isItemExpanded(node)
        }
        guard !open.isEmpty else { return }
        let batch = ListingBatch(open.count)
        for dir in open {
            list(dir: dir, method: "files.list") { [weak self] listing in
                guard batch.add(listing, for: dir) else { return }
                self?.showChanges(batch.listings, dirs: open)
            }
        }
    }

    private func showChanges(_ listings: [String: Listing], dirs: [String]) {
        var reloaded = false
        let selected = selectedNode?.path
        for dir in dirs {
            guard let listing = listings[dir], let node = nodes[dir], node.children != nil,
                  node === root || outline.isItemExpanded(node),
                  apply(listing, to: node)
            else { continue }
            showHidden = listing.showHidden
            reloaded = true
        }
        guard reloaded else { return }
        rebuildFilter()
        outline.reloadData()
        restoreExpansion(root)
        if let selected, let node = nodes[selected] { select(node) }
        header.setHidden(showHidden)
        showStatus()
        changesApplied += 1
        lastChangeAt = Date().timeIntervalSince1970 * 1000
        Self.log(String(format: "files: %@ change applied at %.0f (%@)",
                        Short.id(tileID), lastChangeAt ?? 0, dirs.joined(separator: ", ")))
    }

    /// After `reloadData`, open again the folders that were open.
    private func restoreExpansion(_ node: FileNode) {
        for child in children(of: node) where child.dir && child.open {
            outline.expandItem(child)
            restoreExpansion(child)
        }
    }

    /// `files.revealed`: open each folder down to `path`, then select it.
    /// The folders not listed yet are listed together first.
    private func reveal(path: String, dirs: [String]) {
        if loadingRoot {
            pendingReveal = (path, dirs)
            return
        }
        let unlisted = dirs.filter { nodes[$0]?.children == nil }
        guard !unlisted.isEmpty else { return showReveal(path: path, dirs: dirs, listings: [:]) }
        let batch = ListingBatch(unlisted.count)
        for dir in unlisted {
            list(dir: dir, method: "files.expand") { [weak self] listing in
                guard batch.add(listing, for: dir) else { return }
                self?.showReveal(path: path, dirs: dirs, listings: batch.listings)
            }
        }
    }

    private func showReveal(path: String, dirs: [String], listings: [String: Listing]) {
        // Root first, so each folder's node exists by the time it's opened.
        for dir in dirs {
            guard let node = nodes[dir] else { continue }
            if node.children == nil, let listing = listings[dir], apply(listing, to: node) {
                outline.reloadItem(node, reloadChildren: true)
            }
            if node !== root {
                node.open = true
                outline.expandItem(node)
            }
        }
        if !query.isEmpty { clearFilter() }
        if let node = nodes[path] {
            select(node)
            revealed += 1
        }
    }

    private func gitChanged(_ paths: [String: String]) {
        git = paths
        let visible = outline.rows(in: outline.visibleRect)
        guard visible.length > 0 else { return }
        outline.reloadData(
            forRowIndexes: IndexSet(integersIn: visible.location..<(visible.location + visible.length)),
            columnIndexes: [0])
    }

    // MARK: Actions

    var selectedNode: FileNode? {
        let row = outline.selectedRow
        return row >= 0 ? outline.item(atRow: row) as? FileNode : nil
    }

    private func select(_ node: FileNode) {
        let row = outline.row(forItem: node)
        guard row >= 0 else { return }
        outline.selectRowIndexes([row], byExtendingSelection: false)
        outline.scrollRowToVisible(row)
    }

    /// Return: folders open or close; files show in the tab's preview tile.
    func activate(_ node: FileNode) {
        if node.dir {
            if outline.isItemExpanded(node) { outline.collapseItem(node) } else { outline.expandItem(node) }
        } else {
            open(node, target: "preview")
        }
    }

    func open(_ node: FileNode, target: String) {
        guard let core = Self.core else { return }
        let result = core.execute([
            "method": "files.open",
            "params": ["tile": tileID, "path": node.path, "target": target],
        ])
        if case .failure(let error) = result { Self.log("files: open failed: \(error.message)") }
    }

    /// A remote file has no local app to open it: it goes to the preview
    /// tile instead, as a copy.
    func openInDefaultApp(_ node: FileNode) {
        if isRemote {
            if !node.dir { open(node, target: "preview") }
            return
        }
        NSWorkspace.shared.open(URL(fileURLWithPath: node.path))
    }

    /// Local files only: Finder can't show a remote path.
    func revealInFinder(_ node: FileNode) {
        guard !isRemote else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: node.path)])
    }

    func copyPath(_ node: FileNode) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(node.path, forType: .string)
    }

    func toggleHidden() {
        _ = Self.core?.execute(["method": "files.toggle_hidden", "params": ["tile": tileID]])
    }

    func goToParent(of node: FileNode) {
        if let parent = node.parent, parent !== root { select(parent) }
    }

    @objc private func doubleClicked() {
        let row = outline.clickedRow
        guard row >= 0, let node = outline.item(atRow: row) as? FileNode else { return }
        activate(node)
    }

    // MARK: Filter

    func showFilter() {
        filter.isHidden = false
        filterHeight.constant = 22
        window?.makeFirstResponder(filter)
    }

    private func clearFilter() {
        filter.stringValue = ""
        query = ""
        filter.isHidden = true
        filterHeight.constant = 0
        rebuildFilter()
        outline.reloadData()
        restoreExpansion(root)
        window?.makeFirstResponder(outline)
    }

    @objc private func filterChanged() {
        query = filter.stringValue.lowercased()
        rebuildFilter()
        outline.reloadData()
        if query.isEmpty {
            restoreExpansion(root)
        } else {
            // Show every kept folder open, so matches deep down are visible.
            outline.expandItem(nil, expandChildren: true)
        }
        if outline.numberOfRows > 0 { outline.selectRowIndexes([0], byExtendingSelection: false) }
    }

    private func rebuildFilter() {
        filtered = [:]
        guard !query.isEmpty else { return }
        _ = keep(root)
    }

    /// Whether `node` matches or holds a loaded match; fills `filtered`.
    private func keep(_ node: FileNode) -> Bool {
        var kept: [FileNode] = []
        for child in node.children ?? [] where keep(child) {
            kept.append(child)
        }
        filtered[ObjectIdentifier(node)] = kept
        return node !== root && (node.name.lowercased().contains(query) || !kept.isEmpty)
    }

    fileprivate func children(of node: FileNode) -> [FileNode] {
        if !query.isEmpty { return filtered[ObjectIdentifier(node)] ?? [] }
        return node.children ?? []
    }

    // MARK: Cells

    fileprivate func icon(for node: FileNode) -> NSImage {
        let key = node.dir ? "/" : (node.name as NSString).pathExtension.lowercased()
        if let cached = Self.icons[key] { return cached }
        let type: UTType = node.dir ? .folder : (UTType(filenameExtension: key) ?? .data)
        let image = NSWorkspace.shared.icon(for: type)
        image.size = NSSize(width: 16, height: 16)
        Self.icons[key] = image
        return image
    }

    fileprivate func gitState(for node: FileNode) -> String? { git[node.path] }
}

// MARK: - Outline data source and delegate

extension FilesTileView: NSOutlineViewDataSource, NSOutlineViewDelegate {
    func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
        children(of: (item as? FileNode) ?? root).count
    }

    func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
        children(of: (item as? FileNode) ?? root)[index]
    }

    func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
        (item as? FileNode)?.dir ?? false
    }

    func outlineView(_ outlineView: NSOutlineView, viewFor tableColumn: NSTableColumn?, item: Any) -> NSView? {
        guard let node = item as? FileNode else { return nil }
        let cell = outlineView.makeView(withIdentifier: FileCell.id, owner: self) as? FileCell ?? FileCell()
        cell.show(node, icon: icon(for: node), git: gitState(for: node))
        return cell
    }

    func outlineView(_ outlineView: NSOutlineView, rowViewForItem item: Any) -> NSTableRowView? {
        FilesRowView()
    }

    func outlineViewItemWillExpand(_ notification: Notification) {
        // While filtering, folders open to show matches; that isn't the
        // user's choice and loads nothing new.
        guard let node = notification.userInfo?["NSObject"] as? FileNode, query.isEmpty else { return }
        node.open = true
        load(node)
    }

    func outlineViewItemDidCollapse(_ notification: Notification) {
        guard let node = notification.userInfo?["NSObject"] as? FileNode, query.isEmpty else { return }
        node.open = false
        _ = Self.core?.execute(["method": "files.collapse", "params": ["tile": tileID, "dir": node.path]])
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        if QLPreviewPanel.sharedPreviewPanelExists(), QLPreviewPanel.shared().isVisible {
            QLPreviewPanel.shared().reloadData()
        }
    }
}

extension FilesTileView: NSSearchFieldDelegate {
    /// AppKit's fallbacks in the filter field. Enter and escape normally
    /// arrive as `find_bar` actions; when the router let them through, the
    /// user unbound them there, so they do nothing. Down always moves into
    /// the list (AppKit's move command on the field).
    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        if KeyRouter.letThroughCurrentKey(),
           selector == #selector(NSResponder.cancelOperation(_:))
            || selector == #selector(NSResponder.insertNewline(_:)) { return true }
        switch selector {
        case #selector(NSResponder.cancelOperation(_:)):
            clearFilter()
            return true
        case #selector(NSResponder.insertNewline(_:)), #selector(NSResponder.moveDown(_:)):
            window?.makeFirstResponder(outline)
            if outline.selectedRow < 0, outline.numberOfRows > 0 {
                outline.selectRowIndexes([0], byExtendingSelection: false)
            }
            return true
        default:
            return false
        }
    }
}

// MARK: - Automation

extension FilesTileView: AutomatableTile {
    /// `tile select <path>`, `tile activate`, `tile open-terminal`,
    /// `tile filter <text>`, `tile clear-filter`.
    func automate(_ verb: String, _ arg: String) -> String? {
        switch verb {
        case "select":
            let path = arg.hasPrefix("/") ? arg : (root.path as NSString).appendingPathComponent(arg)
            guard let node = nodes[path] else { return "\(arg) isn't listed" }
            select(node)
        case "activate":
            guard let node = selectedNode else { return "nothing selected" }
            activate(node)
        case "open-terminal":
            guard let node = selectedNode else { return "nothing selected" }
            open(node, target: "terminal")
        case "filter":
            showFilter()
            filter.stringValue = arg
            filterChanged()
        case "clear-filter":
            clearFilter()
        default:
            return "unknown files verb \(verb)"
        }
        return nil
    }

    func automationState() -> [String: String] {
        var s: [String: String] = [
            "tile.kind": "files",
            "tile.root": root.path,
            "tile.machine": machine ?? "local",
            "tile.status": rootFailed ? "error: can't list" : root.error.map { "error: \($0)" }
                ?? (loadingRoot || root.loading ? "loading" : "ok"),
            "tile.rows": "\(outline.numberOfRows)",
            "tile.hidden": showHidden ? "shown" : "hidden",
            "tile.changes": "\(changesApplied)",
            "tile.revealed": "\(revealed)",
            "tile.git": "\(git.count)",
            "tile.first-rows-ms": firstRowsMs.map { String(format: "%.1f", $0) } ?? "-",
            "tile.list-ms": listMs.map { String(format: "%.1f", $0) } ?? "-",
            "tile.loading": loadingRoot ? "yes" : "no",
            "tile.last-change-at": lastChangeAt.map { String(format: "%.0f", $0) } ?? "-",
        ]
        if let node = selectedNode {
            s["tile.selected"] = node.path.hasPrefix(root.path + "/")
                ? String(node.path.dropFirst(root.path.count + 1)) : node.path
        } else {
            s["tile.selected"] = "-"
        }
        s["tile.names"] = (0..<min(outline.numberOfRows, 12))
            .compactMap { (outline.item(atRow: $0) as? FileNode)?.name }
            .joined(separator: ",")
        return s
    }
}

// MARK: - Model

/// One listed entry. `children` stays nil until the folder is listed.
@MainActor
final class FileNode {
    let path: String
    var name: String
    var dir: Bool
    var symlink = false
    var hidden = false
    var ignored = false
    var children: [FileNode]?
    weak var parent: FileNode?
    /// Open in the outline (kept across reloads).
    var open = false
    /// A remote folder whose listing hasn't come in yet.
    var loading = false
    /// Why a remote folder couldn't be listed.
    var error: String?

    init(path: String, name: String, dir: Bool) {
        self.path = path
        self.name = name
        self.dir = dir
    }
}

/// A `files` reply, parsed off the main thread.
private struct Listing: Sendable {
    struct Entry: Sendable {
        let name: String
        let path: String
        let dir: Bool
        let symlink: Bool
        let hidden: Bool
        let ignored: Bool
        let git: String?
    }

    let root: String
    let dir: String
    let showHidden: Bool
    let truncated: Bool
    let entries: [Entry]
    /// Which request it answers (`FilesTileView.list`).
    let request: Int
    /// Remote folders: not listed yet, or why they can't be.
    let loading: Bool
    let error: String?

    init?(_ reply: [String: Any], request: Int) {
        guard let root = reply["root"] as? String, let dir = reply["dir"] as? String else { return nil }
        self.request = request
        self.root = root
        self.dir = dir
        showHidden = reply["show_hidden"] as? Bool ?? false
        truncated = reply["truncated"] as? Bool ?? false
        loading = reply["loading"] as? Bool ?? false
        error = reply["error"] as? String
        entries = (reply["entries"] as? [[String: Any]] ?? []).compactMap { e in
            guard let name = e["name"] as? String, let path = e["path"] as? String else { return nil }
            return Entry(
                name: name, path: path, dir: e["dir"] as? Bool ?? false,
                symlink: e["symlink"] as? Bool ?? false, hidden: e["hidden"] as? Bool ?? false,
                ignored: e["ignored"] as? Bool ?? false, git: e["git"] as? String)
        }
    }
}

/// Listings requested together (a `files.changed` or `files.revealed`),
/// shown together once the last one lands.
@MainActor
private final class ListingBatch {
    private var waiting: Int
    private(set) var listings: [String: Listing] = [:]

    init(_ count: Int) { waiting = count }

    /// Records one reply (nil: that folder couldn't be listed); true once
    /// every folder has answered.
    func add(_ listing: Listing?, for dir: String) -> Bool {
        if let listing { listings[dir] = listing }
        waiting -= 1
        return waiting == 0
    }
}

private struct WeakFiles {
    weak var view: FilesTileView?
    init(_ view: FilesTileView) { self.view = view }
}

// MARK: - Views

/// A row that draws the theme's selection (`selected`), as the sidebar and
/// ⌘K do. The system's accent selection kept the names' own colours
/// (textPrimary, the git colours) on the accent: 2:1 in the light themes.
private final class FilesRowView: NSTableRowView {
    override func drawSelection(in dirtyRect: NSRect) {
        guard selectionHighlightStyle != .none else { return }
        Design.Color.selected.setFill()
        NSBezierPath(roundedRect: bounds.insetBy(dx: 2, dy: 0), xRadius: Design.Radius.row,
                     yRadius: Design.Radius.row).fill()
    }

    /// Never emphasized: the cells keep their colours on the theme's fill.
    override var isEmphasized: Bool {
        get { false }
        set {}
    }
}

/// The tree: vim-ish keys, a context menu and Quick Look.
@MainActor
final class FilesOutlineView: NSOutlineView, QLPreviewPanelDataSource, QLPreviewPanelDelegate {
    weak var owner: FilesTileView?

    // No keyDown override: the key router hands the `files` actions to the
    // tile (FilesTileView.perform(keyAction:)); what it lets through is
    // AppKit's (← → collapse and expand, type-select).

    /// Moves the selection by `delta` rows (NSTableView handles arrows in
    /// `keyDown`, not through `moveDown:`).
    fileprivate func step(_ delta: Int) {
        guard numberOfRows > 0 else { return }
        let row = max(0, min(numberOfRows - 1, (selectedRow < 0 ? -1 : selectedRow) + delta))
        selectRowIndexes([row], byExtendingSelection: false)
        scrollRowToVisible(row)
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        let row = self.row(at: convert(event.locationInWindow, from: nil))
        guard row >= 0, let node = item(atRow: row) as? FileNode, let owner else { return nil }
        selectRowIndexes([row], byExtendingSelection: false)
        let menu = NSMenu()
        func add(_ title: String, _ handler: @escaping () -> Void) {
            let item = NSMenuItem(title: title, action: #selector(MenuAction.run), keyEquivalent: "")
            let action = MenuAction(handler)
            item.target = action
            item.representedObject = action
            menu.addItem(item)
        }
        if !node.dir { add("Open in Preview") { owner.open(node, target: "preview") } }
        add("Open Terminal Here") { owner.open(node, target: "terminal") }
        // A remote path isn't on this Mac.
        if !owner.isRemote {
            add("Open in Default App") { owner.openInDefaultApp(node) }
            add("Reveal in Finder") { owner.revealInFinder(node) }
        }
        add("Copy Path") { owner.copyPath(node) }
        menu.addItem(.separator())
        add("Show or Hide Hidden Files") { owner.toggleHidden() }
        return menu
    }

    // Quick Look: the selected entry, toggled with Space.

    fileprivate func toggleQuickLook() {
        // Quick Look reads local files only.
        if owner?.isRemote == true { return }
        let panel = QLPreviewPanel.shared()!
        if panel.isVisible { panel.orderOut(nil) } else { panel.makeKeyAndOrderFront(nil) }
    }

    override func acceptsPreviewPanelControl(_ panel: QLPreviewPanel!) -> Bool { true }

    override func beginPreviewPanelControl(_ panel: QLPreviewPanel!) {
        MainActor.assumeIsolated {
            panel.dataSource = self
            panel.delegate = self
        }
    }

    override func endPreviewPanelControl(_ panel: QLPreviewPanel!) {
        MainActor.assumeIsolated {
            panel.dataSource = nil
            panel.delegate = nil
        }
    }

    nonisolated func numberOfPreviewItems(in panel: QLPreviewPanel!) -> Int {
        MainActor.assumeIsolated { owner?.selectedNode == nil || owner?.isRemote == true ? 0 : 1 }
    }

    nonisolated func previewPanel(_ panel: QLPreviewPanel!, previewItemAt index: Int) -> (any QLPreviewItem)! {
        let path: String? = MainActor.assumeIsolated { owner?.selectedNode?.path }
        guard let path else { return nil }
        return URL(fileURLWithPath: path) as NSURL
    }
}

/// A menu item's closure.
private final class MenuAction: NSObject {
    let handler: () -> Void
    init(_ handler: @escaping () -> Void) { self.handler = handler }
    @objc func run() { handler() }
}

/// One row: icon, name, git badge. Hidden and ignored entries are dimmed.
private final class FileCell: NSTableCellView {
    static let id = NSUserInterfaceItemIdentifier("file")
    private let icon = NSImageView()
    private let name = NSTextField(labelWithString: "")
    private let badge = NSTextField(labelWithString: "")

    init() {
        super.init(frame: .zero)
        identifier = Self.id
        imageView = icon
        textField = name
        name.font = Design.Font.row
        name.lineBreakMode = .byTruncatingMiddle
        name.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        badge.font = Design.Font.gitBadge
        badge.alignment = .right
        badge.setContentHuggingPriority(.required, for: .horizontal)
        for v in [icon, name, badge] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 2),
            icon.centerYAnchor.constraint(equalTo: centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 16),
            icon.heightAnchor.constraint(equalToConstant: 16),
            name.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 5),
            name.centerYAnchor.constraint(equalTo: centerYAnchor),
            badge.leadingAnchor.constraint(greaterThanOrEqualTo: name.trailingAnchor, constant: 4),
            badge.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            badge.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func show(_ node: FileNode, icon image: NSImage, git: String?) {
        icon.image = image
        icon.alphaValue = node.hidden || node.ignored ? Design.Alpha.dimmed : 1
        name.stringValue = node.name
        let (letter, color) = Self.badge(git)
        badge.stringValue = letter
        badge.textColor = color
        if let color, git != nil {
            name.textColor = color
        } else {
            name.textColor = node.hidden || node.ignored ? Design.Color.textMuted : Design.Color.textPrimary
        }
        toolTip = node.symlink ? "\(node.path) (symlink)" : node.path
        // An open remote folder still listing, or one that failed.
        if let error = node.error {
            badge.stringValue = "!"
            badge.textColor = Design.Color.textMuted
            toolTip = "\(node.path): \(error)"
        } else if node.loading {
            badge.stringValue = "…"
            badge.textColor = Design.Color.textMuted
        }
    }

    private static func badge(_ git: String?) -> (String, NSColor?) {
        switch git {
        case "modified": ("M", Design.Color.gitModified)
        case "added": ("A", Design.Color.gitAdded)
        case "untracked": ("U", Design.Color.gitAdded)
        case "deleted": ("D", Design.Color.gitDeleted)
        case "renamed": ("R", Design.Color.gitRenamed)
        case "conflicted": ("!", Design.Color.gitConflicted)
        default: ("", nil)
        }
    }
}

/// Root path, the hidden-files toggle and the filter button.
private final class FilesHeader: NSView {
    var onToggleHidden: () -> Void = {}
    var onFilter: () -> Void = {}
    private let label = NSTextField(labelWithString: "")
    private let detail = NSTextField(labelWithString: "")
    private let hiddenButton = NSButton()
    private let find = NSButton()

    init() {
        super.init(frame: .zero)
        wantsLayer = true
        label.font = Design.Font.monoDetail
        label.textColor = Design.Color.textSecondary
        label.lineBreakMode = .byTruncatingHead
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        detail.font = Design.Font.caption
        detail.textColor = Design.Color.textMuted
        for (button, symbol, tip) in [
            (hiddenButton, "eye.slash", "Show hidden and ignored files (.)"),
            (find, "line.3.horizontal.decrease", "Filter (/)"),
        ] {
            button.bezelStyle = .accessoryBarAction
            button.isBordered = false
            button.controlSize = .small
            button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: tip)
            button.toolTip = tip
            button.target = self
        }
        hiddenButton.action = #selector(hiddenClicked)
        find.action = #selector(findClicked)
        // The root itself is the tile header's title and subtitle.
        stack = NSStackView(views: [detail, find, hiddenButton])
        stack.orientation = .horizontal
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private var stack = NSStackView()

    override var fittingSize: NSSize { stack.fittingSize }

    func setRoot(_ path: String?, detail text: String?) {
        label.stringValue = path.map { ($0 as NSString).abbreviatingWithTildeInPath } ?? "files"
        label.toolTip = path
        detail.stringValue = text ?? ""
    }

    func setHidden(_ shown: Bool) {
        hiddenButton.image = NSImage(
            systemSymbolName: shown ? "eye" : "eye.slash",
            accessibilityDescription: shown ? "Hide hidden files" : "Show hidden files")
    }

    @objc private func hiddenClicked() { onToggleHidden() }
    @objc private func findClicked() { onFilter() }
}

extension FilesTileView: TileHeaderAccessory {
    var headerAccessory: NSView? { header }
}

// MARK: - Find (⌘F)

/// The files tile has a filter already; ⌘F opens it, as / does.
extension FilesTileView: Findable {
    func beginFind() -> String? {
        showFilter()
        filter.currentEditor()?.selectAll(nil)
        return nil
    }
}

// MARK: - Keys

extension FilesTileView: KeyScope {
    /// `files` while the list has the keyboard, `find_bar` while the filter
    /// does.
    var keyScope: String { isFiltering ? "find_bar" : "files" }

    var keyContext: String { keyScope }

    private var isFiltering: Bool {
        guard let editor = filter.currentEditor() else { return false }
        return window?.firstResponder === editor
    }

    /// The `files` actions, and `find_bar`'s in the filter.
    func perform(keyAction name: String, event: NSEvent?) -> Bool {
        let node = selectedNode
        switch name {
        case "find_next":
            guard isFiltering else { return false }
            focusTree()
        case "find_close":
            guard isFiltering else { return false }
            clearFilter()
        case "files_down": outline.step(1)
        case "files_up": outline.step(-1)
        case "files_open":
            guard let node else { return false }
            activate(node)
        case "files_expand":
            guard let node else { return false }
            if node.dir { outline.expandItem(node) } else { activate(node) }
        case "files_collapse":
            guard let node else { return false }
            if node.dir, outline.isItemExpanded(node) { outline.collapseItem(node) } else { goToParent(of: node) }
        case "files_open_external":
            guard let node else { return false }
            if node.dir { outline.expandItem(node) } else { openInDefaultApp(node) }
        case "files_parent":
            guard let node else { return false }
            goToParent(of: node)
        case "files_quick_look": outline.toggleQuickLook()
        case "files_filter": showFilter()
        case "files_toggle_hidden": toggleHidden()
        case "files_leave":
            guard isPanel else { return false }
            onEscape?()
        default:
            return false
        }
        return true
    }
}
