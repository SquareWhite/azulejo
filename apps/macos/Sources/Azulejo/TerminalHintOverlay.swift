import AppKit
import CoreImage
import IOSurface

/// ⌘S smart select: one immutable, window-local selection session. Live
/// output continues behind captured terminal frames; labels and action
/// payloads never retarget.
///
/// Two modes. Choosing starts with no labels at all: a key panel at the
/// bottom of the window lists the categories (one per item kind: u URLs,
/// p paths, c commands, l lines…). A category's key shows its items alone,
/// each labelled with the shortest letters unique in that category (the
/// core's labels: `a`, `s`, `d`…; `TerminalSelection.shown`); typing
/// filters them, Backspace takes a letter back (or, with none typed, goes
/// back to the categories), a complete label selects its item. Arrow keys
/// move a focus ring across the items still shown (starting at the
/// terminal cursor), Return selects the focused one and ⇧ + arrow starts a
/// range from it. Selected: c / o / r act on the selection, arrows move it
/// (⇧ extends it from the anchor), ⇧ + another label of the category
/// extends it to that item, ⌥↑ / ⌥↓ grow it to the containing item, line,
/// paragraph, block or screen and shrink it back. The key panel always
/// shows the keys that apply; Esc leaves from anywhere.
@MainActor
final class TerminalHintOverlay: NSView, KeepsKeyboard, ThemeAware {
    struct Entry {
        var label = ""
        let item: TerminalHintItem
        let tileID: String
        let cwd: String?
        let rects: [CGRect]
        /// The item's terminal columns on each row it covers, cell size and
        /// grid width, for placing its label.
        let cells: [(row: Int, start: Int, end: Int)]
        let cell: CGSize
        let columns: Int
        /// The label: a callout hanging under the item's first cell.
        var callout = TerminalSelection.Callout(frame: .zero, tip: .zero, side: .below)
        /// The item's first cell, where the callout's tail points.
        var anchor: CGRect { CGRect(x: rects[0].minX, y: rects[0].minY, width: cell.width, height: rects[0].height) }
    }
    private struct Capture {
        let tile: TileModel
        let surface: TerminalSurfaceView
        let frame: CGRect
        let rows: [String]
        /// The pane's items in the core, for the selection keys.
        let hints: TerminalHints.Pane
        /// Its first entry's index in `entries` (a pane's entries are contiguous).
        let firstEntry: Int
        /// Column 0's cell on each row, in overlay coordinates.
        let rowCells: [Int: CGRect]
        /// Terminal columns at the UTF-16 offsets a selection can start or
        /// end at, on rows that aren't plain ASCII (elsewhere offset = column).
        var offsetColumns: [Int: [Int: Int]]
        let cols: Int
        /// Where callouts may go: the tile's body, the padding round the
        /// text included.
        let margin: CGRect
        /// Every run of non-blank cells, for callouts to keep off.
        let text: [CGRect]
        /// The terminal cursor, where the first arrow key starts.
        let cursor: TerminalTextPos?
        /// The frozen frame on show under the overlay (`frame`), which the
        /// item highlights redraw with the selection colour under the text.
        let picture: IOSurfaceRef
    }
    /// The chosen range: from the anchor to the focus, whichever comes
    /// first. Entry indices are the overlay's.
    private struct Selection {
        let pane: Int
        var anchor: TerminalSelection.Unit
        var focus: TerminalSelection.Unit
        /// What ⌥↓ goes back to.
        var history: [(anchor: TerminalSelection.Unit, focus: TerminalSelection.Unit)] = []
        var range: TerminalTextRange { anchor.range.union(focus.range) }
        /// The one entry selected, if the selection is exactly one.
        var single: Int? { anchor == focus ? anchor.entry : nil }
    }
    private weak var controller: WindowController?
    private weak var app: AppController?
    private weak var previousResponder: NSResponder?
    private var captures: [Capture] = []
    /// Each capture's frozen frame as an image, made on first use.
    private var pictures: [Int: NSImage] = [:]
    private var entries: [Entry] = []
    private var interactionMonitor: Any?
    private let canvas = HintCanvas()
    /// Labels a keystroke filtered out, fading away.
    private let ghost = HintCanvas()
    private var ghostEntries: Set<Int> = []
    /// With an item selected, the labels ⇧ + label extends to: hidden until
    /// ⇧ has been held a moment (`motion.hintExtendDelay`), then faded in
    /// (`motion.hintExtend`), and out again when ⇧ goes up.
    private let reach = HintCanvas()
    private var reachShown = false
    private var reachWait: DispatchWorkItem?
    /// The keys that apply right now, floating at the bottom of the window.
    private let keys = HintKeyPanel()
    /// Bumped each time the key panel shows, so an earlier fade-out's
    /// completion can't take down a newer panel.
    private var keysGeneration = 0
    /// The action popup beside a selection (Copy, Open, Run).
    private let panel = NSView()
    /// The category whose labels show while choosing; nil: the category list.
    private var category: TerminalHintItem.Kind?
    private let copyButton = HintActionButton("C")
    private let openButton = HintActionButton("O")
    private let runButton = HintActionButton("R")
    private var selection: Selection? {
        didSet {
            // The category follows the anchor (arrows and ⌥↑↓ can reach
            // another kind): ⇧ + label and Backspace work in its labels.
            if let entry = selection?.anchor.entry, TerminalSelection.letter(entries[entry].item.kind) != nil {
                category = entries[entry].item.kind
            }
        }
    }
    /// While choosing, the entry the arrow keys have focused (Return
    /// selects it), or nil before the first arrow key.
    private var focused: Int?
    private var prefix = ""
    /// The selection a category key made on its own, its category holding
    /// one item: Backspace from it goes back to the category list.
    private var autoSelected: Selection?
    /// Letters typed with ⇧ while selected: another item's label.
    private var extendPrefix = ""
    private var tabID: String?
    private var focusedID: String?
    private var machineID: String?
    private var workspaceCwd: String?
    private(set) var lastOpened = ""
    /// Automation: where the last Run went (`current`, `reuse:<N>`, `new`).
    private(set) var lastRunTarget = ""
    /// Automation: the last filter fade's length (0 with motion off).
    private var lastFade: TimeInterval = 0
    var isOpen: Bool { superview != nil }
    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        panel.wantsLayer = true
        panel.layer?.cornerRadius = Design.Radius.sidebarSegment
        panel.layer?.borderWidth = Design.Border.hairline
        panel.layer?.shadowOpacity = Float(Design.Alpha.paletteShadow)
        panel.layer?.shadowRadius = 6
        panel.layer?.shadowOffset = CGSize(width: 0, height: -2)
        for (button, symbol, label, tip) in [
            (copyButton, "doc.on.doc", "Copy", "Copy (C)"),
            (openButton, "arrow.up.forward.app", "Open", "Open in Default App (O)"),
            (runButton, "play.fill", "Run", "Run (R)"),
        ] {
            button.bezelStyle = .accessoryBarAction
            button.isBordered = false
            button.imagePosition = .imageOnly
            button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)
            button.symbolConfiguration = .init(pointSize: Design.Symbol.toolbarButton, weight: .medium)
            button.setAccessibilityLabel(label)
            button.toolTip = tip
            button.refusesFirstResponder = true
            button.target = self
            panel.addSubview(button)
        }
        copyButton.action = #selector(copySelection)
        openButton.action = #selector(openSelection)
        runButton.action = #selector(runSelection)
        ghost.wantsLayer = true
        reach.wantsLayer = true
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func open(in controller: WindowController, app: AppController) {
        if isOpen { close(); return }
        guard let content = controller.window.contentView, let workspace = controller.model?.workspace,
              let tab = workspace.tab else { return }
        self.controller = controller
        self.app = app
        previousResponder = controller.window.firstResponder
        tabID = tab.id
        focusedID = tab.focused
        machineID = workspace.machineID
        workspaceCwd = workspace.cwd
        frame = content.bounds
        autoresizingMask = [.width, .height]
        content.addSubview(self, positioned: .above, relativeTo: nil)
        var found: [Entry] = []
        let local = workspace.machineID == MachineModel.local
        for id in tab.order where controller.tiles.isVisible(id) {
            guard let tile = tab.tile(id), tile.kind == .terminal,
                  let terminal = (controller.tiles.tileView(id) as? TileFrame)?.content as? TerminalTileView,
                  let surface = terminal.hintSurface, let grid = surface.gridSize, let cell = surface.cellSize else { continue }
            let read = (0..<grid.rows).map { surface.hintRow($0, through: grid.cols - 1) }
            let rows = read.map { $0?.text ?? "" }
            let cwd = tile.terminalCwd ?? workspace.cwd
            // Soft wraps. herdr redraws the pane row by row, so Ghostty's
            // own wrap flag is never set here: a row whose last cell holds
            // text and whose next row starts with text counts as wrapped.
            // The core joins rows an application wrapped itself.
            var joins: Set<Int> = []
            if grid.rows > 1 {
                for row in 0..<(grid.rows - 1) where rows[row].last?.isWhitespace == false
                    && rows[row + 1].first?.isWhitespace == false {
                    if let last = surface.hintRow(row, from: grid.cols - 1, through: grid.cols - 1)?.text,
                       !last.isEmpty, !last.allSatisfy(\.isWhitespace) { joins.insert(row) }
                }
            }
            // Items and paragraphs in reading order, a paragraph before the
            // items inside it (the Rust core: core/crates/hints).
            guard let hints = TerminalHints.Pane(rows: rows, joins: joins, columns: grid.cols, cwd: cwd, local: local,
                                                 branch: workspace.git?.branch,
                                                 vt: surface.hintSnapshot(rows: rows.count, columns: grid.cols))
            else { continue }
            let items = hints.items
            let surfaceFrame = surface.convert(surface.bounds, to: self)
            func column(_ offset: Int, _ row: Int) -> Int {
                surface.hintColumn(at: offset, row: row, text: rows[row], columns: grid.cols)
            }
            var rowCells: [Int: CGRect] = [:]
            var offsetColumns: [Int: [Int: Int]] = [:]
            var text: [CGRect] = []
            for (row, line) in read.enumerated() {
                guard let line else { continue }
                let base = surface.convert(CGRect(x: line.x, y: surface.bounds.height - line.baseline - cell.height * 0.22,
                                                  width: cell.width, height: cell.height), to: self)
                rowCells[row] = base
                let ascii = line.text.utf8.allSatisfy { $0 < 128 }
                if !ascii, let extent = hints.extents[row] {
                    offsetColumns[row] = [extent.start: column(extent.start, row), extent.end: column(extent.end, row)]
                }
                for run in hints.runs[row] {
                    let start = ascii ? run.location : column(run.location, row)
                    let end = ascii ? NSMaxRange(run) : column(NSMaxRange(run), row)
                    text.append(CGRect(x: base.minX + CGFloat(start) * base.width, y: base.minY,
                                       width: CGFloat(end - start) * base.width, height: base.height))
                }
            }
            // The tile's body under its header strip, a hair inside the
            // card's rounded corners: callouts may use the padding.
            var margin = surfaceFrame
            if let card = controller.tiles.tileView(id) as? TileFrame {
                let body = card.convert(card.bounds, to: self)
                let strip = card.header.isHidden ? 0 : card.header.frame.height
                margin = CGRect(x: body.minX + 2, y: body.minY + strip + 1,
                                width: body.width - 4, height: body.height - strip - 3).union(surfaceFrame)
            }
            var positioned: [Entry] = []
            var kept: [Int] = []
            for (index, item) in items.enumerated() {
                let rects = item.spans.compactMap { span -> CGRect? in
                    guard let row = surface.hintRow(span.row, through: grid.cols - 1), row.text == rows[span.row] else { return nil }
                    let start = column(span.range.location, span.row)
                    let end = column(NSMaxRange(span.range), span.row)
                    let rect = CGRect(x: row.x + CGFloat(start) * cell.width,
                                      y: surface.bounds.height - row.baseline - cell.height * 0.22,
                                      width: CGFloat(end - start) * cell.width, height: cell.height)
                    return surface.convert(rect, to: self).intersection(surfaceFrame)
                }
                guard rects.count == item.spans.count, rects.allSatisfy({ !$0.isEmpty }) else { continue }
                let cells = item.spans.map { span in
                    (row: span.row, start: column(span.range.location, span.row), end: column(NSMaxRange(span.range), span.row))
                }
                for (span, cell) in zip(item.spans, cells) where offsetColumns[span.row] != nil {
                    offsetColumns[span.row]?[span.range.location] = cell.start
                    offsetColumns[span.row]?[NSMaxRange(span.range)] = cell.end
                }
                kept.append(index)
                positioned.append(Entry(item: item, tileID: tile.id, cwd: cwd, rects: rects,
                                        cells: cells, cell: cell, columns: grid.cols))
            }
            // The frozen picture must show the rows the labels sit on, not
            // a frame libghostty presented before the screen last changed
            // (its text then sits off the boxes): draw a frame of what the
            // terminal holds now, take it if it has the surface's size (a
            // stretched one would drift too), then read the rows again.
            // Output arriving meanwhile drops the pane; ⌘S again picks it up.
            surface.drawNow()
            guard surface.presentedAtSize, let image = surface.snapshot(),
                  (0..<grid.rows).allSatisfy({ surface.hintRow($0, through: grid.cols - 1)?.text == rows[$0] })
            else { continue }
            let frozen = NSView(frame: surfaceFrame)
            frozen.wantsLayer = true
            frozen.layer?.contents = image
            frozen.layer?.contentsGravity = .resize
            frozen.layer?.contentsScale = controller.window.backingScaleFactor
            addSubview(frozen)
            hints.keep(kept)
            captures.append(Capture(tile: tile, surface: surface, frame: surfaceFrame, rows: rows, hints: hints,
                                    firstEntry: found.count, rowCells: rowCells, offsetColumns: offsetColumns, cols: grid.cols,
                                    margin: margin, text: text, cursor: surface.hintCursor(), picture: image))
            found += positioned
        }
        // An item repeated in a pane (a prompt's cwd on every prompt line)
        // gets one label, on its newest copy; the older ones stay
        // unlabelled, still reachable with the arrows.
        let labels = TerminalHints.Pane.labels(captures.map(\.hints))
        entries = found
        for (index, label) in zip(entries.indices, labels) { entries[index].label = label }
        placeCallouts()
        for view in [canvas, ghost, reach] {
            view.frame = bounds
            view.autoresizingMask = [.width, .height]
            view.overlay = self
            addSubview(view)
        }
        ghost.kind = .ghost
        ghost.alphaValue = 0
        reach.kind = .reach
        reach.alphaValue = 0
        reachShown = false
        ghostEntries = []
        lastFade = 0
        category = nil
        addSubview(panel)
        panel.isHidden = true
        showKeys()
        applyTheme()
        refresh()
        app.core.resetKeys()
        controller.window.makeFirstResponder(self)
        interactionMonitor = NSEvent.addLocalMonitorForEvents(
            matching: [.scrollWheel, .leftMouseDown, .rightMouseDown, .otherMouseDown]
        ) { [weak self] event in
            guard let self, event.window === self.window else { return event }
            if event.type == .scrollWheel {
                self.close()
            } else if self.selection != nil {
                let point = self.convert(event.locationInWindow, from: nil)
                if !self.panel.frame.contains(point) {
                    let overTerminal = self.captures.contains { $0.frame.contains(point) }
                    self.close()
                    return overTerminal ? nil : event
                }
            }
            return event
        }
    }

    func close(restoreFocus: Bool = true) {
        guard isOpen else { return }
        if let interactionMonitor { NSEvent.removeMonitor(interactionMonitor) }
        interactionMonitor = nil
        let controller = controller
        let heldFocus = controller?.window.firstResponder === self
        hideKeys()
        cancelReachWait()
        reachShown = false
        removeFromSuperview()
        for view in subviews { view.removeFromSuperview() }
        captures.removeAll()
        pictures.removeAll()
        entries.removeAll()
        ghostEntries.removeAll()
        selection = nil
        autoSelected = nil
        focused = nil
        category = nil
        prefix = ""
        extendPrefix = ""
        app?.core.resetKeys()
        if restoreFocus, heldFocus, let controller {
            if let previousResponder, let view = previousResponder as? NSView,
               view.window === controller.window, !view.isHiddenOrHasHiddenAncestor {
                controller.window.makeFirstResponder(previousResponder)
            } else { controller.tiles.focusedView?.setFocused(true) }
        }
        previousResponder = nil
    }

    /// Headers/agent state can refresh freely. Layout, source binding, cwd,
    /// tab and focus changes invalidate the captured action context.
    func validate() {
        guard isOpen, let controller, let workspace = controller.model?.workspace,
              workspace.machineID == machineID, workspace.cwd == workspaceCwd,
              let tab = workspace.tab, tab.id == tabID, tab.focused == focusedID else {
            close(); return
        }
        for capture in captures {
            guard let tile = tab.tile(capture.tile.id), tile.herdr == capture.tile.herdr,
                  tile.terminalCwd == capture.tile.terminalCwd, tile.ended == capture.tile.ended,
                  controller.tiles.isVisible(tile.id), capture.surface.window === window,
                  capture.surface.convert(capture.surface.bounds, to: self) == capture.frame else {
                close(); return
            }
        }
    }

    // MARK: Keys

    /// ⇧ held with an item selected shows the labels it can extend to,
    /// after a moment; letting go hides them again (as ⌘ held shows the
    /// sidebar's workspace chords).
    override func flagsChanged(with event: NSEvent) {
        let held = event.modifierFlags.intersection([.command, .shift, .option, .control, .function])
        guard isOpen else { return }
        if held.contains(.shift), !held.contains(.command), selection != nil {
            guard !reachShown, reachWait == nil else { return }
            let work = DispatchWorkItem { [weak self] in
                guard let self else { return }
                self.reachWait = nil
                if self.isOpen, self.selection != nil { self.setReach(true, animated: true) }
            }
            reachWait = work
            DispatchQueue.main.asyncAfter(deadline: .now() + Design.Motion.hintExtendDelay, execute: work)
        } else {
            cancelReachWait()
            if reachShown { setReach(false, animated: true) }
        }
    }

    private func cancelReachWait() {
        reachWait?.cancel()
        reachWait = nil
    }

    private func setReach(_ shown: Bool, animated: Bool) {
        reachShown = shown
        reach.needsDisplay = true
        reach.layer?.removeAllAnimations()
        let duration = animated && Motion.active ? Design.Motion.hintExtend : 0
        guard duration > 0 else { reach.alphaValue = shown ? 1 : 0; return }
        NSAnimationContext.runAnimationGroup { context in
            context.duration = duration
            context.timingFunction = Motion.timing
            reach.animator().alphaValue = shown ? 1 : 0
        }
    }

    func handle(_ chord: String) {
        guard isOpen else { return }
        if chord == "escape" { close(); return }
        let before = visibleEntries
        if selection == nil { choose(chord) } else { act(chord) }
        guard isOpen else { return }
        fade(before.subtracting(visibleEntries))
        refresh()
    }

    /// A key name that types a letter, with or without ⇧.
    private static func letter(_ chord: String) -> (letter: String, shift: Bool)? {
        let shift = chord.hasPrefix("shift+")
        let key = shift ? String(chord.dropFirst(6)) : chord
        guard key.count == 1, key.first?.isLetter == true else { return nil }
        return (key, shift)
    }

    private static func direction(_ key: String) -> TerminalSelection.Direction {
        switch key {
        case "left": .left
        case "right": .right
        case "up": .up
        default: .down
        }
    }

    /// Kinds with at least one label: the categories a key can open.
    private var categories: Set<TerminalHintItem.Kind> {
        Set(entries.filter { !$0.label.isEmpty }.map(\.item.kind))
    }

    private func choose(_ chord: String) {
        guard let category else {
            // The category list: a category's key shows its labels.
            if let typed = Self.letter(chord), !typed.shift,
               let kind = TerminalSelection.kind(forLetter: typed.letter), categories.contains(kind) {
                self.category = kind
                prefix = ""
                focused = nil
                // One item in the category: selected at once, no label.
                let mine = entries.indices.filter { entries[$0].item.kind == kind && !entries[$0].label.isEmpty }
                if mine.count == 1 {
                    select(mine[0])
                    autoSelected = selection
                }
            }
            return
        }
        switch chord {
        case "backspace", "delete":
            if !prefix.isEmpty {
                prefix.removeLast()
            } else {
                // Back to the category list.
                self.category = nil
                focused = nil
            }
            return
        case "left", "right", "up", "down":
            moveFocus(Self.direction(chord))
            return
        case "shift+left", "shift+right", "shift+up", "shift+down":
            // A range from the focused item: select it, then extend.
            guard let focused else { return }
            select(focused)
            move(String(chord.dropFirst(6)), extend: true)
            return
        case "enter":
            if let focused { select(focused) }
            return
        default: break
        }
        guard let typed = Self.letter(chord) else { return }
        let next = prefix + typed.letter
        let mine = entries.indices.filter { entries[$0].item.kind == category }
        guard mine.contains(where: { entries[$0].label.hasPrefix(next) }) else { return }
        prefix = next
        // A complete label wins over the focus ring.
        if let index = mine.first(where: { entries[$0].label == next }) { select(index); return }
        refocus()
    }

    private func select(_ index: Int) {
        guard let pane = paneOf(index) else { return }
        setReach(false, animated: false)
        let unit = unit(index)
        selection = Selection(pane: pane, anchor: unit, focus: unit)
        focused = nil
        extendPrefix = ""
    }

    private func paneOf(_ entry: Int) -> Int? {
        captures.firstIndex { $0.tile.id == entries[entry].tileID }
    }

    /// A pane's entries whose labels show, as the pane's own indices.
    private func shownInPane(_ pane: Int, _ visible: Set<Int>) -> [Int] {
        paneEntries(pane).filter(visible.contains).map { $0 - captures[pane].firstEntry }
    }

    /// An arrow key while choosing: the focus ring moves among the items
    /// still shown (the core's neighbour logic, `azj_hints_pane_focus`).
    /// The first press starts at the focused tile's terminal cursor. ← / →
    /// past a pane's first or last item go on to the previous or next
    /// pane's; ↑ / ↓ stay in the pane.
    private func moveFocus(_ direction: TerminalSelection.Direction) {
        let visible = visibleEntries
        if let current = focused, let pane = paneOf(current) {
            let first = captures[pane].firstEntry
            if let next = captures[pane].hints.focus(among: shownInPane(pane, visible), from: current - first,
                                                       cursor: nil, direction) {
                focused = first + next
                return
            }
            guard direction == .left || direction == .right else { return }
            let step = direction == .right ? 1 : -1
            var other = pane + step
            while captures.indices.contains(other) {
                let shown = paneEntries(other).filter(visible.contains)
                if let pick = direction == .right ? shown.first : shown.last {
                    focused = pick
                    return
                }
                other += step
            }
            return
        }
        let home = captures.firstIndex { $0.tile.id == focusedID }
        let order = (home.map { [$0] } ?? []) + captures.indices.filter { $0 != home }
        for pane in order {
            let shown = shownInPane(pane, visible)
            guard !shown.isEmpty else { continue }
            if let next = captures[pane].hints.focus(among: shown, from: nil, cursor: captures[pane].cursor, direction) {
                focused = captures[pane].firstEntry + next
                return
            }
        }
    }

    /// Typing filtered out the focused item: the nearest one left in its
    /// pane takes the ring, or nothing when none is.
    private func refocus() {
        guard let current = focused, !visibleEntries.contains(current) else { return }
        guard let pane = paneOf(current) else { focused = nil; return }
        let first = captures[pane].firstEntry
        focused = captures[pane].hints.nearest(among: shownInPane(pane, visibleEntries), to: current - first)
            .map { first + $0 }
    }

    private func unit(_ index: Int) -> TerminalSelection.Unit {
        let level: TerminalSelection.Level = switch entries[index].item.kind {
        case .text: .text
        case .line: .line
        default: .item
        }
        return TerminalSelection.Unit(range: TerminalSelection.extent(entries[index].item), level: level, entry: index)
    }

    /// With a selection, the labels ⇧ reaches: the category's (labels are
    /// only unique within one).
    private func reachable(_ index: Int, from selection: Selection) -> Bool {
        entries[index].item.kind == category
    }

    private func act(_ chord: String) {
        guard var current = selection else { return }
        if let typed = Self.letter(chord), typed.shift {
            // ⇧ + a label: extend to that item (same pane only).
            let next = extendPrefix + typed.letter
            let pane = paneEntries(current.pane).filter { reachable($0, from: current) }
            guard pane.contains(where: { entries[$0].label.hasPrefix(next) }) else { return }
            extendPrefix = next
            if let index = pane.first(where: { entries[$0].label == next }) {
                current.focus = unit(index)
                current.history = []
                selection = current
                extendPrefix = ""
            }
            return
        }
        switch chord {
        case "c", "cmd+c": copySelection()
        case "o": openSelection()
        case "r": runSelection()
        case "left", "right", "up", "down": move(chord, extend: false)
        case "shift+left", "shift+right", "shift+up", "shift+down": move(String(chord.dropFirst(6)), extend: true)
        case "alt+up": grow()
        case "alt+down": shrink()
        case "backspace", "delete":
            if !extendPrefix.isEmpty { extendPrefix.removeLast(); return }
            // Selected by the category key alone: back to the category list.
            if let auto = autoSelected, auto.anchor == current.anchor, auto.focus == current.focus {
                autoSelected = nil
                selection = nil
                category = nil
                prefix = ""
                focused = nil
                return
            }
            // Back to choosing, one letter short of the anchor's label, the
            // focus ring on the anchor (Return picks it again).
            prefix = current.anchor.entry.map { String(entries[$0].label.dropLast()) } ?? ""
            selection = nil
            focused = current.anchor.entry.flatMap { visibleEntries.contains($0) ? $0 : nil }
        default: break
        }
    }

    private func paneEntries(_ pane: Int) -> [Int] {
        entries.indices.filter { entries[$0].tileID == captures[pane].tile.id }
    }

    /// A unit from the pane's core handle, its entry an overlay index.
    private func overlayUnit(_ unit: TerminalSelection.Unit?, pane: Int) -> TerminalSelection.Unit? {
        guard var unit else { return nil }
        unit.entry = unit.entry.map { captures[pane].firstEntry + $0 }
        return unit
    }

    /// A unit for the pane's core handle, its entry the pane's own index.
    private func paneUnit(_ unit: TerminalSelection.Unit, pane: Int) -> TerminalSelection.Unit {
        var unit = unit
        unit.entry = unit.entry.map { $0 - captures[pane].firstEntry }
        return unit
    }

    private func move(_ key: String, extend: Bool) {
        guard var current = selection else { return }
        let direction = Self.direction(key)
        let pane = current.pane
        guard let next = overlayUnit(captures[pane].hints.neighbour(paneUnit(current.focus, pane: pane), direction),
                                     pane: pane) else { return }
        current.focus = next
        if !extend { current.anchor = next }
        current.history = []
        selection = current
    }

    private func grownUnit(_ current: Selection) -> TerminalSelection.Unit? {
        overlayUnit(captures[current.pane].hints.grow(current.range), pane: current.pane)
    }

    private func shrunkUnit(_ current: Selection) -> TerminalSelection.Unit? {
        let level = current.anchor == current.focus ? current.focus.level : .line
        return overlayUnit(captures[current.pane].hints.shrink(current.range, from: level), pane: current.pane)
    }

    private func grow() {
        guard var current = selection, let unit = grownUnit(current) else { return }
        current.history.append((current.anchor, current.focus))
        current.anchor = unit
        current.focus = unit
        selection = current
    }

    private func shrink() {
        guard var current = selection else { return }
        if let last = current.history.popLast() {
            current.anchor = last.anchor
            current.focus = last.focus
        } else if let unit = shrunkUnit(current) {
            current.anchor = unit
            current.focus = unit
        } else { return }
        selection = current
    }

    // MARK: Selection payload

    /// What Copy puts on the pasteboard: the item's own value (a command
    /// without its continuation backslashes, a path without quotes) for a
    /// single item, else the range as a terminal selection would copy it.
    private var selectionText: String? {
        guard let selection else { return nil }
        if let single = selection.single { return entries[single].item.value }
        return captures[selection.pane].hints.text(selection.range)
    }

    private var singleEntry: Entry? { selection?.single.map { entries[$0] } }
    private var canOpen: Bool { singleEntry?.item.openURL != nil }
    private var isCommand: Bool { singleEntry?.item.kind == .command }
    private var canRun: Bool { isCommand && singleEntry?.cwd != nil }

    private func column(_ capture: Capture, row: Int, offset: Int) -> Int {
        capture.offsetColumns[row]?[offset] ?? min(offset, capture.cols)
    }

    /// The selection's highlight: an item's own rects, else one per row of
    /// the range (the first row from its start, the last to its end).
    private var selectionRects: [CGRect] {
        guard let selection else { return [] }
        if let single = selection.single { return entries[single].rects }
        let capture = captures[selection.pane], range = selection.range
        return (range.start.row...range.end.row).compactMap { row in
            guard let extent = capture.hints.extents[row], let base = capture.rowCells[row] else { return nil }
            let start = row == range.start.row ? range.start.offset : extent.start
            let end = row == range.end.row ? range.end.offset : extent.end
            guard end > start else { return nil }
            let from = column(capture, row: row, offset: start), to = column(capture, row: row, offset: end)
            return CGRect(x: base.minX + CGFloat(from) * base.width, y: base.minY,
                          width: CGFloat(to - from) * base.width, height: base.height).intersection(capture.frame)
        }.filter { !$0.isEmpty }
    }

    // MARK: Layout

    /// Entries whose labels show while choosing (the category's; none on
    /// the category list), or, selected, the labels ⇧ can still reach: the
    /// pane's items of the category outside the selection.
    private var visibleEntries: Set<Int> {
        if let selection {
            let range = selection.range
            return Set(paneEntries(selection.pane).filter {
                !entries[$0].label.isEmpty && !range.contains(TerminalSelection.extent(entries[$0].item))
                    && entries[$0].label.hasPrefix(extendPrefix) && reachable($0, from: selection)
            })
        }
        return Set(entries.indices.filter {
            TerminalSelection.shown(entries[$0].item.kind, label: entries[$0].label, category: category, typed: prefix)
        })
    }

    /// Labels a keystroke filtered out fade quickly instead of vanishing.
    private func fade(_ gone: Set<Int>) {
        guard !gone.isEmpty, selection == nil else { return }
        let duration = Motion.active ? Design.Motion.hintFilter : 0
        lastFade = duration
        guard duration > 0 else { return }
        ghostEntries = gone
        ghost.needsDisplay = true
        ghost.layer?.removeAllAnimations()
        ghost.alphaValue = 1
        NSAnimationContext.runAnimationGroup { context in
            context.duration = duration
            context.timingFunction = Motion.timing
            ghost.animator().alphaValue = 0
        }
    }

    /// Callout geometry for an entry's cell height: a pill a little shorter
    /// than a row, a tail that fills the rest of the row's height, the
    /// pill's corner radius and the tail's half width at its foot.
    private static func calloutMetrics(_ cell: CGSize)
        -> (font: NSFont, typed: NSFont, height: CGFloat, tail: CGFloat, padding: CGFloat, radius: CGFloat, half: CGFloat) {
        let height = (cell.height * 0.74).rounded(), tail = max(3, (cell.height * 0.24).rounded())
        let size = (cell.height * 0.5).rounded()
        return (NSFont.systemFont(ofSize: size, weight: .bold), NSFont.systemFont(ofSize: size, weight: .medium), height,
                tail, (height * 0.32).rounded(), (height * 0.3).rounded(), max(2.5, tail * 0.9))
    }

    /// Homerow-style labels, per pane in reading order
    /// (`TerminalSelection.placeCallouts`): a callout below, above or
    /// before each item's first cell, on blank space when there is any,
    /// with a short, nearly vertical tail; it may use the pane's padding.
    /// One pass per category in each pane, since a category shows on its
    /// own; all but lines keep off paragraphs' text before other items'.
    private func placeCallouts() {
        for capture in captures {
            // Unlabelled repeats get no callout: their text is text to
            // keep off like any other. Paragraphs have no label either.
            let pane = entries.indices.filter { entries[$0].tileID == capture.tile.id }
            let prose = pane.filter { entries[$0].item.kind == .text }.flatMap { entries[$0].rects }
            let labelled = pane.filter { !entries[$0].label.isEmpty }
            for kind in TerminalSelection.kindOrder.map(\.kind) {
                let lines = kind == .line
                let indices = labelled.filter { entries[$0].item.kind == kind }
                guard let cell = indices.first.map({ entries[$0].cell }) else { continue }
                let metrics = Self.calloutMetrics(cell)
                let items = indices.map { index in
                    let width = (entries[index].label.uppercased() as NSString)
                        .size(withAttributes: [.font: metrics.font]).width
                    return TerminalSelection.CalloutItem(
                        anchor: entries[index].anchor, rects: entries[index].rects, prose: false,
                        size: CGSize(width: max(metrics.height, ceil(width) + 2 * metrics.padding), height: metrics.height))
                }
                if lines {
                    // Every row is a line, so there's no blank space to
                    // hang labels in: each sits at its line's start, in the
                    // indent when it fits, else over the first letters.
                    for (index, item) in zip(indices, items) {
                        entries[index].callout = Self.lineCallout(item, margin: capture.margin, tail: metrics.tail)
                    }
                    continue
                }
                let placed = TerminalHints.placeCallouts(items, text: capture.text, prose: prose,
                                                         bounds: capture.margin, tail: metrics.tail, spacing: 2,
                                                         inset: metrics.radius + metrics.half, drift: cell.width / 2)
                for (index, callout) in zip(indices, placed) { entries[index].callout = callout }
            }
        }
    }

    /// A line's label: before its first cell, tail pointing at it, when the
    /// indent (and the pane's padding) has room; else over its first cells.
    private static func lineCallout(_ item: TerminalSelection.CalloutItem, margin: CGRect,
                                    tail: CGFloat) -> TerminalSelection.Callout {
        let anchor = item.anchor, size = item.size
        let y = (anchor.midY - size.height / 2).rounded()
        let left = anchor.minX - tail - size.width
        if left >= margin.minX {
            return .init(frame: CGRect(x: left, y: y, width: size.width, height: size.height),
                         tip: CGPoint(x: anchor.minX, y: anchor.midY),
                         base: CGPoint(x: anchor.minX - tail, y: anchor.midY), side: .left)
        }
        let frame = CGRect(x: max(margin.minX, anchor.minX), y: y, width: size.width, height: size.height)
        return .init(frame: frame, tip: CGPoint(x: anchor.midX, y: anchor.midY),
                     base: CGPoint(x: frame.midX, y: frame.midY), side: .inline)
    }

    /// Two entries can show at once: they're in the same category.
    private func together(_ a: Int, _ b: Int) -> Bool {
        entries[a].item.kind == entries[b].item.kind
    }

    private func refresh() {
        copyButton.isEnabled = selection != nil
        // Only the actions that apply: no greyed-out buttons.
        openButton.isHidden = !canOpen
        openButton.contentTintColor = Design.Color.textSecondary
        openButton.toolTip = "Open in Default App (O)"
        runButton.isHidden = !canRun
        layoutKeys()
        layoutPanel()
        canvas.needsDisplay = true
        reach.needsDisplay = true
    }

    /// What the key panel shows right now: a title, the main keys (the
    /// categories, or a selection's actions) and the keys that move about.
    private var keyContent: HintKeyPanel.Content {
        typealias Key = HintKeyPanel.Key
        guard let selection else {
            guard let category else {
                // The category list: every category, those with nothing on
                // screen dimmed, so the list keeps its shape.
                let present = categories
                let counts = Dictionary(grouping: entries.filter { !$0.label.isEmpty }, by: \.item.kind)
                    .mapValues(\.count)
                let main: [Key] = TerminalSelection.kindOrder.compactMap { kind, one, many in
                    guard let letter = TerminalSelection.letter(kind) else { return nil }
                    let count = counts[kind] ?? 0
                    return Key(key: String(letter), text: count == 1 ? one : many, kind: true,
                               count: count, enabled: present.contains(kind))
                }
                return .init(title: entries.isEmpty ? "Nothing to select" : "Select", main: main,
                             nav: [Key(key: "esc", text: "close")])
            }
            let shown = visibleEntries
            let total = entries.filter { $0.item.kind == category && !$0.label.isEmpty }.count
            var nav: [Key] = []
            if !shown.isEmpty { nav.append(Key(key: "a–z", text: "pick")) }
            if !shown.isEmpty { nav.append(Key(key: "↑↓←→", text: "move")) }
            if focused != nil { nav.append(Key(key: "⏎", text: "select")) }
            nav.append(Key(key: "⌫", text: prefix.isEmpty ? "categories" : "back"))
            nav.append(Key(key: "esc", text: "close"))
            return .init(title: Self.categoryName(category, count: total), titleKey: TerminalSelection.letter(category)
                .map(String.init), main: [], nav: nav)
        }
        let titleKey = TerminalSelection.letter(category ?? .text).map(String.init)
        let what = singleEntry.map { Self.categoryName($0.item.kind, count: 1) }
            ?? (selection.anchor == selection.focus ? selection.focus.level.name : "range")
        if !extendPrefix.isEmpty {
            return .init(title: "Extend to \(extendPrefix.uppercased())…", titleKey: titleKey, main: [],
                         nav: [Key(key: "⌫", text: "back"), Key(key: "esc", text: "close")])
        }
        var main: [Key] = [Key(key: "c", text: "copy")]
        if canOpen { main.append(Key(key: "o", text: "open")) }
        if canRun { main.append(Key(key: "r", text: "run")) }
        var nav: [Key] = []
        if captures[selection.pane].hints.hasTargets(selection.focus.level) { nav.append(Key(key: "←→↑↓", text: "move")) }
        // Holding ⇧ shows the labels it can extend to.
        nav.append(Key(key: "⇧", text: "hold to extend"))
        let grow = grownUnit(selection) != nil
        let shrink = !selection.history.isEmpty || shrunkUnit(selection) != nil
        if grow && shrink { nav.append(Key(key: "⌥↑↓", text: "grow/shrink")) }
        else if grow { nav.append(Key(key: "⌥↑", text: "grow")) }
        else if shrink { nav.append(Key(key: "⌥↓", text: "shrink")) }
        nav.append(Key(key: "⌫", text: "back"))
        nav.append(Key(key: "esc", text: "close"))
        return .init(title: "Selected \(what)", titleKey: titleKey, main: main, nav: nav)
    }

    /// A category's name for `count` items: "1 URL", "3 lines".
    private static func categoryName(_ kind: TerminalHintItem.Kind, count: Int) -> String {
        guard let names = TerminalSelection.kindOrder.first(where: { $0.kind == kind }) else { return kind.rawValue }
        return count == 1 ? names.one : "\(count) \(names.many)"
    }

    /// The key panel fades and rises in (`Motion.paletteOpenDuration`),
    /// centred above the window's bottom edge; motion off or Reduce Motion,
    /// it just shows.
    private func showKeys() {
        keysGeneration += 1
        keys.layer?.removeAllAnimations()
        keys.removeFromSuperview()
        keys.alphaValue = 1
        addSubview(keys)
        layoutKeys()
        keys.applyTheme()
        guard Motion.active, let layer = keys.layer else { return }
        let duration = Motion.paletteOpenDuration
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = 0
        fade.toValue = 1
        let rise = CABasicAnimation(keyPath: "transform.translation.y")
        rise.fromValue = -HintKeyPanel.rise
        rise.toValue = 0
        let group = CAAnimationGroup()
        group.animations = [fade, rise]
        group.duration = duration
        group.timingFunction = Motion.timing
        layer.add(group, forKey: "show")
    }

    /// The panel fades out after the overlay goes (`paletteCloseDuration`),
    /// over the window's content.
    private func hideKeys() {
        keys.layer?.removeAllAnimations()
        guard Motion.active, let host = superview, keys.superview === self, !keys.isHidden else {
            keys.removeFromSuperview()
            return
        }
        let frame = convert(keys.frame, to: host)
        keys.removeFromSuperview()
        keys.frame = frame
        host.addSubview(keys, positioned: .above, relativeTo: nil)
        let generation = keysGeneration
        NSAnimationContext.runAnimationGroup({ context in
            context.duration = Motion.paletteCloseDuration
            context.timingFunction = Motion.timing
            keys.animator().alphaValue = 0
        }, completionHandler: { [weak self] in
            MainActor.assumeIsolated {
                guard let self, self.keysGeneration == generation, self.keys.superview === host else { return }
                self.keys.removeFromSuperview()
            }
        })
    }

    /// Where the key panel may not go: what shows on screen (the
    /// category's labelled items, or the selection).
    private var keysAvoid: [CGRect] {
        if selection != nil { return selectionRects }
        guard let category else { return [] }
        return entries.filter { $0.item.kind == category && !$0.label.isEmpty }
            .flatMap { $0.rects + [$0.callout.frame] }
    }

    /// The key panel: centred above the window's bottom edge, unless it
    /// would hide what's on show there; then the first place that hides
    /// nothing (the bottom right, where terminal text rarely reaches, the
    /// top right, the bottom left, the top), else the one hiding least.
    private func layoutKeys() {
        guard keys.superview === self else { return }
        keys.content = keyContent
        let margin = HintKeyPanel.margin
        let size = keys.fitting(maxWidth: min(bounds.width - 2 * margin, HintKeyPanel.maxWidth))
        let bottom = bounds.maxY - size.height - margin, top = Design.Space.toolbarHeight + margin
        let centre = (bounds.midX - size.width / 2).rounded()
        let right = bounds.maxX - size.width - margin, left = bounds.minX + margin
        let candidates = [(centre, bottom), (right, bottom), (right, top), (left, bottom), (centre, top)]
            .map { CGRect(x: $0.0, y: $0.1, width: size.width, height: size.height) }
        let avoid = keysAvoid
        let hidden = { (frame: CGRect) in avoid.filter { $0.intersects(frame.insetBy(dx: -4, dy: -4)) }.count }
        let counts = candidates.map(hidden)
        let best = counts.firstIndex(of: 0) ?? counts.indices.min { counts[$0] < counts[$1] } ?? 0
        keys.frame = candidates[best]
        keys.needsDisplay = true
    }

    /// The key panel's frame (`.null` when it isn't on show).
    private var keysFrame: CGRect {
        keys.superview === self && !keys.isHidden ? keys.frame : .null
    }

    private func layoutPanel() {
        let rects = selectionRects
        guard let selection, let first = rects.first, let last = rects.last else {
            panel.isHidden = true
            return
        }
        let pane = captures[selection.pane].frame
        let buttonSize: CGFloat = 28, padding: CGFloat = 4, spacing: CGFloat = 2, gap: CGFloat = 6
        var x = padding
        for button in [copyButton, openButton, runButton] where !button.isHidden {
            button.frame = CGRect(x: x, y: padding, width: HintActionButton.width, height: buttonSize)
            x += HintActionButton.width + spacing
        }
        let size = CGSize(width: x - spacing + padding, height: buttonSize + padding * 2)
        let right = rects.map(\.maxX).max() ?? last.maxX
        let belowX = min(max(pane.minX, last.minX), pane.maxX - size.width)
        let aboveX = min(max(pane.minX, first.minX), pane.maxX - size.width)
        let positions = [
            CGPoint(x: (rects.count == 1 ? last.maxX : right) + gap, y: last.midY - size.height / 2),
            CGPoint(x: belowX, y: last.maxY + gap),
            CGPoint(x: aboveX, y: first.minY - size.height - gap),
        ]
        let candidates = positions.map { CGRect(origin: $0, size: size) }
        let keysFrame = keysFrame
        panel.frame = candidates.first { candidate in
            pane.contains(candidate) && !rects.contains(where: { $0.intersects(candidate) })
                && !candidate.intersects(keysFrame)
        } ?? CGRect(x: belowX, y: min(max(pane.minY, last.maxY + gap), pane.maxY - size.height), width: size.width, height: size.height)
        panel.isHidden = false
    }

    override func setFrameSize(_ newSize: NSSize) {
        let changed = newSize != frame.size
        super.setFrameSize(newSize)
        if isOpen, changed { close() }
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        guard isOpen else { return nil }
        if !panel.isHidden, panel.frame.contains(point) { return super.hitTest(point) }
        // Callouts in a pane's padding take clicks too.
        let onCallout = selection == nil && visibleEntries.contains { entries[$0].callout.frame.contains(point) }
        return onCallout || captures.contains(where: { $0.frame.contains(point) }) ? self : nil
    }

    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        if selection == nil, let index = entries.indices.first(where: {
            visibleEntries.contains($0) && entries[$0].callout.frame.contains(point)
        }) {
            select(index); refresh()
        } else { close() }
    }

    // MARK: Actions

    @objc private func copySelection() {
        guard let value = selectionText else { return }
        TerminalRuntime.pasteboard.clearContents()
        TerminalRuntime.pasteboard.setString(value, forType: .string)
        close()
    }

    @objc private func openSelection() {
        guard let url = singleEntry?.item.openURL else { return }
        // Automation records the dispatch target without launching another app.
        if ContextMenus.automating {
            lastOpened = url.absoluteString
            close()
        } else if NSWorkspace.shared.open(url) { close() }
        else {
            openButton.contentTintColor = Design.Color.error
            openButton.toolTip = "Could not open this item (O)"
        }
    }

    /// Runs the command in an idle shell (this pane, else the nearest) or
    /// a new pane: `TerminalRun`.
    @objc private func runSelection() {
        guard let entry = singleEntry, let app, let controller else { return }
        guard entry.item.kind == .command, let cwd = entry.cwd else { return }
        validate()
        guard isOpen, let workspace = controller.model?.workspace, let tab = workspace.tab else { return }
        close()
        lastRunTarget = TerminalRun.run(entry.item.value, cwd: cwd, source: entry.tileID, tab: tab,
                                        workspace: workspace, app: app, controller: controller)
    }

    // MARK: Drawing

    func applyTheme() {
        panel.layer?.backgroundColor = Design.Color.sidebar.cgColor
        panel.layer?.borderColor = Design.Color.border.cgColor
        panel.layer?.shadowColor = Design.Color.border.cgColor
        for button in [copyButton, openButton, runButton] {
            button.contentTintColor = Design.Color.textSecondary
        }
        keys.applyTheme()
        canvas.needsDisplay = true
    }

    fileprivate func drawHints(_ layer: HintCanvas.Kind) {
        switch layer {
        case .ghost:
            if selection == nil { drawChoosing(ghostEntries, typed: 0) }
            return
        case .reach:
            // The labels ⇧ can reach, while ⇧ is held (this view fades).
            guard selection != nil, extendPrefix.isEmpty else { return }
            for index in visibleEntries.sorted() { drawCallout(entries[index], alpha: 1, typed: 0) }
            return
        case .main: break
        }
        guard let selection else {
            // A category's labels on show: the rest of the text recedes.
            if category != nil { drawOthersFaded() }
            drawChoosing(visibleEntries, typed: prefix.count, focus: focused)
            return
        }
        for rect in selectionRects {
            let path = NSBezierPath(rect: rect)
            Design.Color.accent.withAlphaComponent(0.35).setFill()
            path.fill()
            Design.Color.accent.setStroke()
            path.lineWidth = 1
            path.stroke()
        }
        // An item's unlabelled repeats show while it's selected.
        if let single = selection.single { drawMatches(repeatsOf(single)) }
        // Once a ⇧ letter picks among the other labels, the matching ones
        // show, its letters greyed like typed ones (before that, `reach`).
        guard !extendPrefix.isEmpty else { return }
        for index in visibleEntries.sorted() {
            drawCallout(entries[index], alpha: 1, typed: extendPrefix.count)
        }
    }

    /// While choosing: every shown item's highlight, then the focus ring
    /// round the focused one (`focus`), then the callouts, the focused
    /// one's last and emphasised.
    private func drawChoosing(_ visible: Set<Int>, typed: Int, focus: Int? = nil) {
        let focus = focus.flatMap { visible.contains($0) ? $0 : nil }
        let order = visible.sorted().filter { $0 != focus } + (focus.map { [$0] } ?? [])
        let shown = order.map { entries[$0] }
        drawMatches(order)
        if let focus { drawFocusRing(entries[focus]) }
        // Callouts last: they float over every highlight and the text.
        for (index, entry) in zip(order, shown) {
            drawCallout(entry, alpha: 1, typed: typed, focused: index == focus)
        }
    }

    /// The focus ring the arrow keys move while choosing: a thicker
    /// outline (`hintFocus`) just outside the item's rects, distinct from
    /// the selected item's filled highlight.
    private func drawFocusRing(_ entry: Entry) {
        Design.Color.hintFocus.setStroke()
        for rect in entry.rects {
            let path = NSBezierPath(roundedRect: rect.insetBy(dx: -2, dy: -1.5), xRadius: 2, yRadius: 2)
            path.lineWidth = 2
            path.stroke()
        }
    }

    /// The terminal's background: what text fades towards.
    private static var terminalBackground: NSColor {
        Theme.terminal.map { Theme.rgba($0.background) } ?? Design.Color.tile
    }

    /// While a category's labels show, every pane's text but the items
    /// fades to `alpha.hintOthers` (the terminal background over it at the
    /// rest); `drawMatches` puts the items back on top at full strength.
    private func drawOthersFaded() {
        Self.terminalBackground.withAlphaComponent(1 - Design.Alpha.hintOthers).setFill()
        for capture in captures { NSBezierPath(rect: capture.frame).fill() }
    }

    /// Items' highlights, as the terminal shows a selection: its own
    /// selection colour under the text, no outline. The overlay can't draw
    /// under the terminal's text, so it redraws the frozen frame inside the
    /// items' cells and adds the step from the terminal background to the
    /// selection colour (plus-lighter, or plus-darker where the selection is
    /// darker): the background becomes the selection colour and each glyph
    /// shifts by the same step. Without the terminal's colours (a Ghostty
    /// config of the user's own), an accent wash.
    private func drawMatches(_ indices: [Int]) {
        guard !indices.isEmpty, let context = NSGraphicsContext.current?.cgContext else { return }
        let lift = Theme.terminal.flatMap { term in
            term.selectionBackground.map { Self.lift(from: Theme.rgba(term.background), to: Theme.rgba($0)) }
        }
        let panes = Dictionary(grouping: indices) { paneOf($0) ?? -1 }
        for (pane, members) in panes {
            let clip = NSBezierPath()
            for index in members { for rect in entries[index].rects { clip.appendRect(rect) } }
            NSGraphicsContext.saveGraphicsState()
            if let lift, pane >= 0, let picture = picture(pane) {
                clip.addClip()
                let frame = captures[pane].frame
                picture.draw(in: frame, from: .zero, operation: .copy, fraction: 1, respectFlipped: true,
                             hints: [.interpolation: NSNumber(value: NSImageInterpolation.none.rawValue)])
                context.setBlendMode(lift.lighter ? .plusLighter : .plusDarker)
                lift.color.setFill()
                NSBezierPath(rect: frame).fill()
            } else {
                Design.Color.accent.withAlphaComponent(0.18).setFill()
                clip.fill()
            }
            NSGraphicsContext.restoreGraphicsState()
        }
    }

    /// The colour that, added to the terminal background (plus-lighter) or
    /// taken from it (plus-darker: the colour is one minus the step), gives
    /// the selection colour.
    private static func lift(from background: NSColor, to selection: NSColor) -> (color: NSColor, lighter: Bool) {
        let b = Theme.resolved(background), s = Theme.resolved(selection)
        let lighter = s.redComponent + s.greenComponent + s.blueComponent
            >= b.redComponent + b.greenComponent + b.blueComponent
        func step(_ to: CGFloat, _ from: CGFloat) -> CGFloat { lighter ? max(0, to - from) : 1 - max(0, from - to) }
        return (NSColor(srgbRed: step(s.redComponent, b.redComponent), green: step(s.greenComponent, b.greenComponent),
                        blue: step(s.blueComponent, b.blueComponent), alpha: 1), lighter)
    }

    /// A capture's frozen frame as an image, at its pixel size.
    private func picture(_ pane: Int) -> NSImage? {
        if let picture = pictures[pane] { return picture }
        let surface = captures[pane].picture
        let space = IOSurfaceCopyValue(surface, kIOSurfaceColorSpace).flatMap { CGColorSpace(propertyListPlist: $0) }
            ?? CGColorSpace(name: CGColorSpace.sRGB)
        let image = CIImage(ioSurface: surface)
        guard let space, let cg = Self.pictureContext.createCGImage(image, from: image.extent, format: .BGRA8,
                                                                     colorSpace: space) else { return nil }
        let picture = NSImage(cgImage: cg, size: captures[pane].frame.size)
        pictures[pane] = picture
        return picture
    }

    private static let pictureContext = CIContext(options: [.cacheIntermediates: false])

    /// The unlabelled copies of a labelled item (same pane, kind and value).
    private func repeatsOf(_ index: Int) -> [Int] {
        let entry = entries[index]
        guard !entry.label.isEmpty else { return [] }
        return entries.indices.filter {
            $0 != index && entries[$0].label.isEmpty && entries[$0].tileID == entry.tileID
                && entries[$0].item.kind == entry.item.kind && entries[$0].item.value == entry.item.value
        }
    }

    /// A rounded pill with a short tail pointing at the item's first cell
    /// (up from below, down from above, right from the margin before it;
    /// none when it sits over the item itself) and a soft shadow: a neutral
    /// ink chip (`hintCallout*`, near-white in dark themes, near-black in
    /// light ones). The first `typed`
    /// letters, already typed, are greyed (`hintCalloutTyped`, lighter
    /// weight), the rest bold. Dimmed, the whole callout fades to `alpha`
    /// but the letters stay solid, picked to read on the faded fill
    /// (contrast.json's hints.callout.dimmed). The focused item's callout
    /// (arrow keys while choosing) is emphasised: a deeper fill
    /// (`hintCalloutFocus`) and a ring in the focus colour round it.
    private func drawCallout(_ entry: Entry, alpha: CGFloat, typed: Int, focused: Bool = false) {
        let callout = entry.callout, frame = callout.frame
        let metrics = Self.calloutMetrics(entry.cell)
        let radius = metrics.radius, half = metrics.half
        let pill = NSBezierPath(roundedRect: frame, xRadius: radius, yRadius: radius)
        let base = callout.base
        let tail = NSBezierPath()
        switch callout.side {
        case .below, .above:
            let nudge: CGFloat = callout.side == .above ? -0.5 : 0.5
            tail.move(to: CGPoint(x: base.x - half, y: base.y))
            tail.line(to: CGPoint(x: callout.tip.x, y: callout.tip.y + nudge))
            tail.line(to: CGPoint(x: base.x + half, y: base.y))
            tail.close()
        case .left:
            tail.move(to: CGPoint(x: base.x, y: base.y - half))
            tail.line(to: CGPoint(x: callout.tip.x - 0.5, y: callout.tip.y))
            tail.line(to: CGPoint(x: base.x, y: base.y + half))
            tail.close()
        case .inline: break
        }
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        context.saveGState()
        context.setAlpha(alpha)
        context.beginTransparencyLayer(auxiliaryInfo: nil)
        // Pill and tail as one shape (a layer of their own), one shadow
        // under both.
        NSGraphicsContext.saveGraphicsState()
        let shadow = NSShadow()
        shadow.shadowColor = Design.Color.hintCalloutShadow
        shadow.shadowBlurRadius = 3
        shadow.shadowOffset = CGSize(width: 0, height: -1)
        shadow.set()
        context.beginTransparencyLayer(auxiliaryInfo: nil)
        let body = focused ? Design.Color.hintCalloutFocus : Design.Color.hintCallout
        body.setFill()
        pill.fill()
        if !tail.isEmpty { tail.fill() }
        context.endTransparencyLayer()
        NSGraphicsContext.restoreGraphicsState()
        if focused {
            let ring = NSBezierPath(roundedRect: frame.insetBy(dx: -2.5, dy: -2.5), xRadius: radius + 2.5,
                                    yRadius: radius + 2.5)
            ring.lineWidth = 2
            Design.Color.hintFocus.setStroke()
            ring.stroke()
        }
        context.endTransparencyLayer()
        context.restoreGState()
        let terminal = Theme.terminal.map { Theme.rgba($0.background) } ?? Design.Color.window
        let fill = alpha < 1 ? Theme.solid(body.withAlphaComponent(alpha), over: terminal) : body
        let label = Self.calloutLabel(entry.label, typed: typed, metrics: metrics,
                                      text: Theme.readable(Design.Color.hintCalloutText, on: fill))
        let size = label.size()
        label.draw(at: CGPoint(x: frame.midX - size.width / 2, y: frame.midY - size.height / 2))
    }

    /// A callout's letters: the typed ones greyed and lighter, the rest bold.
    private static func calloutLabel(_ label: String, typed: Int, metrics: (font: NSFont, typed: NSFont, height: CGFloat,
                                     tail: CGFloat, padding: CGFloat, radius: CGFloat, half: CGFloat),
                                     text: NSColor) -> NSAttributedString {
        let letters = label.uppercased()
        let count = min(max(typed, 0), letters.count)
        let result = NSMutableAttributedString(string: String(letters.prefix(count)), attributes: [
            .font: metrics.typed, .foregroundColor: Design.Color.hintCalloutTyped, .kern: 0.3,
        ])
        result.append(NSAttributedString(string: String(letters.dropFirst(count)), attributes: [
            .font: metrics.font, .foregroundColor: text, .kern: 0.3,
        ]))
        return result
    }

    // MARK: Automation

    var automationState: [String: String] {
        let entry = singleEntry
        let rects = selectionRects
        let popupVisible = isOpen && !panel.isHidden
        let popupFits = selection.map { captures[$0.pane].frame.contains(panel.frame) } ?? false
        var popupPosition = "hidden"
        if popupVisible, let first = rects.first, let last = rects.last {
            if panel.frame.maxY <= first.minY { popupPosition = "above" }
            else if panel.frame.minY >= last.maxY { popupPosition = "below" }
            else if panel.frame.minX >= last.maxX { popupPosition = "right" }
            else { popupPosition = "overlap" }
        }
        let keysVisible = isOpen && !keysFrame.isNull
        let keysPosition = !keysVisible ? "hidden"
            : (keys.frame.midY > bounds.midY ? "bottom" : "top")
            + (abs(keys.frame.midX - bounds.midX) < 1 ? "" : keys.frame.midX > bounds.midX ? "-right" : "-left")
        let content = keysVisible ? keys.content : nil
        let calloutStats = calloutStatistics
        let actions = selection.map { _ in
            (["copy"] + (canOpen ? ["open"] : []) + (canRun ? ["run"] : [])).joined(separator: ",")
        } ?? "-"
        let kind = selection.map { s in
            entry?.item.kind.rawValue ?? (s.anchor == s.focus ? s.focus.level.name : "range")
        } ?? "-"
        // The items, lines aside: not the paragraphs (never labelled) nor
        // the lines (listed on their own).
        let items = entries.filter { $0.item.kind != .text && $0.item.kind != .line }
        let lines = entries.filter { $0.item.kind == .line }
        // What picks an item from the category list: its category's key,
        // then its label.
        func listed(_ list: [String]) -> String { list.isEmpty ? "-" : list.joined(separator: ",") }
        func keysOf(_ entry: Entry) -> String {
            entry.label.isEmpty ? "-" : (TerminalSelection.letter(entry.item.kind).map(String.init) ?? "") + entry.label
        }
        return [
            "hints.state": isOpen ? (selection == nil ? "choosing" : "selected") : "closed",
            "hints.category": category.map(\.rawValue) ?? "-",
            "hints.count": String(items.count),
            "hints.visible": String(isOpen ? visibleEntries.count : 0),
            "hints.types": items.map { $0.item.kind.rawValue }.joined(separator: ","),
            "hints.labels": items.isEmpty ? "-" : items.map { $0.label.isEmpty ? "-" : $0.label }.joined(separator: ","),
            "hints.keys": items.isEmpty ? "-" : items.map(keysOf).joined(separator: ","),
            // Each item's kind and first row.
            "hints.rows": listed(items.map { "\($0.item.kind.rawValue):\($0.item.spans[0].row)" }),
            // Every box sits on its own row: a row's box starts where row 0's
            // does, a whole number of cells further down.
            "hints.boxes.rows": entries.allSatisfy { entry in
                guard let capture = captures.first(where: { $0.tile.id == entry.tileID }),
                      let top = capture.rowCells[0]?.minY else { return false }
                return zip(entry.item.spans, entry.rects).allSatisfy { span, rect in
                    abs(rect.minY - (top + CGFloat(span.row) * entry.cell.height)) < 0.5
                        || rect.minY < capture.frame.minY + 0.5
                }
            } ? "yes" : "no",
            // The kinds of the labels on screen now, once each.
            "hints.shown.kinds": listed(Set(visibleEntries.map { entries[$0].item.kind.rawValue }).sorted()),
            "hints.shown.labels": listed(visibleEntries.sorted().map { entries[$0].label }),
            // Repeats left without a label (all but the newest copy).
            "hints.unlabelled": String(items.filter(\.label.isEmpty).count),
            // Every non-blank line, labelled but drawn only after `l`.
            "hints.lines": String(lines.count),
            "hints.lines.labels": lines.isEmpty ? "-" : lines.map { $0.label.isEmpty ? "-" : $0.label }
                .joined(separator: ","),
            "hints.lines.values": lines.isEmpty ? "-" : lines.map(\.item.value).joined(separator: "|"),
            // In the lines category: every non-blank row of every pane is in
            // a line whose label shows; no line sits on a blank row; the
            // lowest line has a single letter.
            "hints.lines.all": category == .line && selection == nil && captures.allSatisfy { capture in
                capture.hints.extents.indices.allSatisfy { row in
                    capture.hints.extents[row] == nil || visibleEntries.contains { index in
                        entries[index].tileID == capture.tile.id && entries[index].item.kind == .line
                            && entries[index].item.spans.contains { $0.row == row }
                    }
                }
            } ? "yes" : "no",
            "hints.lines.blank": String(lines.filter { line in
                guard let capture = captures.first(where: { $0.tile.id == line.tileID }) else { return false }
                return line.item.spans.contains { capture.hints.extents.indices.contains($0.row)
                    && capture.hints.extents[$0.row] == nil }
            }.count),
            "hints.lines.short": lines.last.map { $0.label.count == 1 ? "yes" : "no" } ?? "-",
            // Callouts on screen: while choosing, the matching ones; selected,
            // none until ⇧ is held (or a ⇧ letter typed).
            "hints.callouts": String(!isOpen ? 0 : selection == nil || !extendPrefix.isEmpty || reachShown
                ? visibleEntries.count : 0),
            "hints.reach": reachShown ? "shown" : reachWait != nil ? "waiting" : "hidden",
            "hints.selected": selectionText.map { $0.replacingOccurrences(of: "\n", with: "↵") } ?? "-",
            "hints.kind": kind,
            "hints.level": selection?.focus.level.name ?? "-",
            "hints.prefix": prefix.isEmpty ? "-" : prefix,
            // The focus ring while choosing: its item's value, kind and label.
            "hints.focus": focused.map { entries[$0].item.value.replacingOccurrences(of: "\n", with: "↵") } ?? "-",
            "hints.focus.kind": focused.map { entries[$0].item.kind.rawValue } ?? "-",
            "hints.focus.label": focused.map { entries[$0].label.isEmpty ? "-" : entries[$0].label } ?? "-",
            // The focused tile's terminal cursor, where the first arrow starts.
            "hints.cursor": (captures.first { $0.tile.id == focusedID }?.cursor).map { "\($0.row):\($0.offset)" } ?? "-",
            "hints.extend": extendPrefix.isEmpty ? "-" : extendPrefix,
            "hints.actions": actions,
            "hints.opened": lastOpened.isEmpty ? "-" : lastOpened,
            "hints.run.target": lastRunTarget.isEmpty ? "-" : lastRunTarget,
            "hints.panes": String(captures.count),
            "hints.label": entry.map { $0.label.isEmpty ? "-" : $0.label } ?? "-",
            // The selected item's boxes: one per row, on consecutive rows
            // ("rows:N"), or a single row ("row").
            "hints.spans": entry.map { e -> String in
                let rows = e.item.spans.map(\.row)
                guard rows.count > 1 else { return "row" }
                return zip(rows, rows.dropFirst()).allSatisfy { $1 == $0 + 1 } ? "rows:\(rows.count)" : "scattered"
            } ?? "-",
            "hints.spans.multi": entry.map { $0.item.spans.count > 1 ? "yes" : "no" } ?? "-",
            // The selected item's value: what Copy puts on the pasteboard.
            "hints.selected.value": entry.map { $0.item.value.replacingOccurrences(of: "\n", with: "↵") } ?? "-",
            "hints.fade": String(Int((lastFade * 1000).rounded())),
            // The key panel: its title, what each key does (main keys, then
            // the moving-about ones), the keys, the dimmed categories, where
            // it sits, and that it fits the window and hides nothing on show.
            "hints.panel.title": content?.title ?? "-",
            "hints.panel": listed(content.map { ($0.main + $0.nav).map(\.text) } ?? []),
            "hints.panel.keys": listed(content.map { ($0.main + $0.nav).map(\.key) } ?? []),
            "hints.panel.dimmed": listed(content.map { $0.main.filter { !$0.enabled }.map(\.key) } ?? []),
            "hints.panel.position": keysPosition,
            "hints.panel.fits": !keysVisible ? "-"
                : bounds.contains(keys.frame) && keys.fitting(maxWidth: keys.frame.width).width <= keys.frame.width + 0.5
                    && !keysAvoid.contains(where: { $0.intersects(keys.frame) }) ? "yes" : "no",
            "hints.panel.alpha": String(format: "%.2f", keys.alphaValue),
            "hints.panel.animating": keys.layer?.animation(forKey: "show") != nil ? "yes" : "no",
            "hints.popup": popupVisible ? "visible" : "hidden",
            "hints.popup.fits": popupVisible ? (popupFits ? "yes" : "no") : "-",
            "hints.popup.position": popupPosition,
            // Callouts overlapping each other, or covering another item's
            // first cell; and ones the placement couldn't clear. Only
            // callouts that show together count: lines' among lines, the
            // rest among the rest.
            "hints.callout.clash": String(entries.indices.reduce(0) { total, i in
                total + entries.indices[(i + 1)...].filter {
                    entries[$0].tileID == entries[i].tileID && together($0, i) && !entries[$0].label.isEmpty
                        && !entries[i].label.isEmpty
                        && entries[$0].callout.frame.insetBy(dx: 0.5, dy: 0.5).intersects(entries[i].callout.frame)
                }.count
            }),
            // (Unlabelled repeats' first cells are plain text to placement.)
            "hints.callout.hides": String(entries.indices.filter { i in
                !entries[i].label.isEmpty && entries.indices.contains { other in
                    entries[other].tileID == entries[i].tileID && together(other, i) && !entries[other].label.isEmpty
                        && entries[other].anchor != entries[i].anchor
                        && entries[other].callout.frame.insetBy(dx: 0.5, dy: 0.5).intersects(entries[i].anchor)
                }
            }.count),
            "hints.callout.flagged": String(entries.filter { !$0.label.isEmpty && $0.callout.clash }.count),
            "hints.callout.covers": String(calloutStats.covers),
            "hints.callout.slant": String(calloutStats.slant),
            "hints.callout.sides": calloutStats.sides,
            // Callouts drawn with typed letters greyed: the letters, and
            // how many callouts show them.
            "hints.callout.typed": typedPrefix.isEmpty ? "-" : "\(typedPrefix):\(isOpen ? visibleEntries.count : 0)",
        ]
    }

    /// The letters callouts show greyed: typed while choosing, or after ⇧.
    private var typedPrefix: String { selection == nil ? prefix : extendPrefix }

    /// Callouts over another item's text (paragraphs aside: an address in
    /// prose has nowhere else); tails slanting more than half a cell off
    /// their cell's middle; and how many sit on each side.
    private var calloutStatistics: (covers: Int, slant: Int, sides: String) {
        var covers = 0, slant = 0
        var sides: [String: Int] = [:]
        for (index, entry) in entries.enumerated() where !entry.label.isEmpty {
            let c = entry.callout, frame = c.frame.insetBy(dx: 0.5, dy: 0.5)
            sides[c.side.rawValue, default: 0] += 1
            let point = CGPoint(x: entry.anchor.midX, y: entry.anchor.midY)
            if entries.indices.contains(where: { other in
                other != index && entries[other].tileID == entry.tileID && entries[other].item.kind != .text
                    && together(other, index)
                    && !entries[other].rects.contains { $0.contains(point) }
                    && entries[other].rects.contains { $0.intersects(frame) }
            }) { covers += 1 }
            let base = c.base
            let off = c.side == .left ? abs(base.y - c.tip.y) : abs(base.x - c.tip.x)
            if c.side != .inline, off > entry.cell.width / 2 + 0.5 { slant += 1 }
        }
        let text = ["below", "above", "left", "inline"].map { "\($0):\(sides[$0] ?? 0)" }.joined(separator: ",")
        return (covers, slant, text)
    }

    /// The first labelled item with that value (repeats keep theirs on the
    /// newest copy); a line only when no other item has that value.
    private func labelledEntry(_ value: String) -> Entry? {
        let matches = entries.filter { $0.item.value == value && !$0.label.isEmpty }
        return matches.first { $0.item.kind != .line } ?? matches.first
    }

    /// `hints select <value>`: types the label of the first item with that value.
    func automationSelect(_ value: String) -> String? {
        guard isOpen, let entry = labelledEntry(value),
              let letter = TerminalSelection.letter(entry.item.kind) else { return "no such visible hint" }
        selection = nil; focused = nil; prefix = ""; extendPrefix = ""; category = nil
        handle(String(letter))
        // A category of one selects its item with its key alone.
        if selection == nil { for key in entry.label.map(String.init) { handle(key) } }
        return nil
    }

    /// `hints label <label> <value>`: the first item with that value has that label.
    func automationLabel(_ arg: String) -> String? {
        let parts = arg.split(separator: " ", maxSplits: 1).map(String.init)
        guard isOpen, parts.count == 2, let entry = labelledEntry(parts[1]) else { return "no such visible hint" }
        return entry.label == parts[0] ? nil : "label is \(entry.label), not \(parts[0])"
    }

    /// `hints callout <below|above|left|inline|nudged|centred> <value>`:
    /// where the item's callout sits (nudged: slid sideways off its first
    /// cell's middle), and that its tail points at that cell.
    func automationCallout(_ arg: String) -> String? {
        let parts = arg.split(separator: " ", maxSplits: 1).map(String.init)
        guard isOpen, parts.count == 2, let entry = labelledEntry(parts[1]) else { return "no such visible hint" }
        let c = entry.callout, cell = entry.anchor
        let facts = [c.side.rawValue, c.nudged ? "nudged" : "centred"]
        guard facts.contains(parts[0]) else { return "callout is \(facts.joined(separator: ", ")), not \(parts[0])" }
        let target = switch c.side {
        case .below: CGPoint(x: cell.midX, y: cell.maxY)
        case .above: CGPoint(x: cell.midX, y: cell.minY)
        case .left: CGPoint(x: cell.minX, y: cell.midY)
        case .inline: CGPoint(x: cell.midX, y: cell.midY)
        }
        let pointsAtCell = abs(c.tip.x - target.x) < 0.5 && abs(c.tip.y - target.y) < 0.5
        return pointsAtCell ? nil : "the callout's tail misses the item's first cell"
    }

    func automationClick(_ action: String) -> String? {
        let button: NSButton? = switch action {
        case "copy": copyButton
        case "open": openButton
        case "run": runButton
        default: nil
        }
        guard isOpen, let button, button.isEnabled, !button.isHidden else { return "hint action is unavailable" }
        button.performClick(nil)
        return nil
    }

    func automationClickAway(chrome: Bool) -> String? {
        guard isOpen, selection != nil, let window else { return "no selected hint popup" }
        let point = CGPoint(x: bounds.maxX - 2, y: chrome ? 2 : bounds.maxY - 2)
        guard !panel.frame.contains(point) else { return "click-away point is inside the popup" }
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            guard let event = NSEvent.mouseEvent(
                with: type, location: convert(point, to: nil), modifierFlags: [],
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
                context: nil, eventNumber: 0, clickCount: 1, pressure: type == .leftMouseDown ? 1 : 0
            ) else { return "could not create a click-away event" }
            NSApp.postEvent(event, atStart: false)
        }
        return nil
    }

    func automationOpenedFile(_ path: String) -> String? {
        guard let opened = URL(string: lastOpened), opened.isFileURL,
              opened.resolvingSymlinksInPath() == URL(fileURLWithPath: path).resolvingSymlinksInPath()
        else { return "open did not target the expected file" }
        return nil
    }
}

/// ⌘S keycaps, for the key panel and the action popup alike. Raised, like
/// the shortcut glyphs in menus (`hintBadge`, a thin edge
/// `hintBadgeBorder`, a lip `hintBadgeShadow` under it): the category and
/// title keys (`large`) and the popup's shortcuts. Flat (`hintBadgeFlat`):
/// the keys that move about. Letters `hintBadgeText`, as the labels show
/// them (capitals).
@MainActor
private enum HintKeycap {
    static let height: CGFloat = 16
    static let largeHeight: CGFloat = 18

    /// Letters as the callouts show them (capitals); other keys as named.
    static func display(_ key: String) -> String {
        key.count == 1 && key.first?.isLetter == true ? key.uppercased() : key
    }

    private static func font(large: Bool) -> NSFont {
        large ? Design.Font.hintCategoryKey : Design.Font.hintKey
    }

    static func size(for key: String, large: Bool = false) -> CGSize {
        let width = ceil((display(key) as NSString).size(withAttributes: [.font: font(large: large)]).width)
        let height = large ? largeHeight : height
        return CGSize(width: max(height, width + 8), height: height)
    }

    /// `color` at `alpha` of its own opacity (the tokens are translucent).
    private static func faded(_ color: NSColor, _ alpha: CGFloat) -> NSColor {
        alpha >= 1 ? color : color.withAlphaComponent(color.alphaComponent * alpha)
    }

    static func draw(_ key: String, in rect: CGRect, raised: Bool = true, large: Bool = false, enabled: Bool = true) {
        let alpha: CGFloat = enabled ? 1 : 0.4
        let radius = Design.Radius.hintBadge
        let path = NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius)
        if raised {
            // The lip: the key's shape a point lower, showing only below it
            // (the key's fill is translucent in dark themes).
            NSGraphicsContext.saveGraphicsState()
            let outside = NSBezierPath(rect: rect.insetBy(dx: -2, dy: -2))
            outside.append(path)
            outside.windingRule = .evenOdd
            outside.addClip()
            faded(Design.Color.hintBadgeShadow, alpha).setFill()
            NSBezierPath(roundedRect: rect.offsetBy(dx: 0, dy: 1), xRadius: radius, yRadius: radius).fill()
            NSGraphicsContext.restoreGraphicsState()
        }
        faded(raised ? Design.Color.hintBadge : Design.Color.hintBadgeFlat, alpha).setFill()
        path.fill()
        if raised {
            let edge = NSBezierPath(roundedRect: rect.insetBy(dx: 0.25, dy: 0.25), xRadius: radius - 0.25,
                                    yRadius: radius - 0.25)
            edge.lineWidth = Design.Border.hintPanel
            faded(Design.Color.hintBadgeBorder, alpha).setStroke()
            edge.stroke()
        }
        let text = display(key) as NSString
        let attributes: [NSAttributedString.Key: Any] = [
            .font: font(large: large), .foregroundColor: faded(Design.Color.hintBadgeText, enabled ? 1 : 0.5),
        ]
        let size = text.size(withAttributes: attributes)
        text.draw(at: CGPoint(x: rect.midX - size.width / 2, y: rect.midY - size.height / 2), withAttributes: attributes)
    }
}

/// ⌘S's key panel, which-key style: a HUD floating above the window's
/// bottom edge. A translucent system material (`material.hintPanel`: a HUD
/// in dark themes, a popover in light ones) under a tint (`hintPanel`, so it
/// stays legible where the material can't blur, in-app captures say), a
/// thin edge (`hintPanelBorder`), a highlight along the top
/// (`hintPanelHighlight`), radius `hintPanel` and the palette's shadow. A
/// title (with a category, its key) over the main keys in columns (the
/// categories and how many items each has, or a selection's actions), then
/// a row of the keys that move about. Keycaps are `HintKeycap`s: raised for
/// the main keys, flat for the rest.
@MainActor
private final class HintKeyPanel: NSView {
    struct Key {
        let key: String
        let text: String
        /// A category key: a large raised keycap.
        var kind = false
        /// How many items the category has (none: not shown).
        var count = 0
        /// A category with nothing on screen is listed, dimmed.
        var enabled = true
    }

    struct Content {
        var title: String
        var titleKey: String?
        var main: [Key]
        var nav: [Key]

        init(title: String, titleKey: String? = nil, main: [Key], nav: [Key]) {
            self.title = title
            self.titleKey = titleKey
            self.main = main
            self.nav = nav
        }
    }

    /// Room kept between the panel and the window's edges.
    static let margin: CGFloat = 16
    static let maxWidth: CGFloat = 600
    /// How far it rises as it fades in.
    static let rise: CGFloat = 8
    private static let padding: CGFloat = 12, row: CGFloat = 22, columnGap: CGFloat = 20, gap: CGFloat = 6
    private static let navSpacing: CGFloat = 14, title: CGFloat = 18, section: CGFloat = 8, maxColumns = 4

    var content = Content(title: "", main: [], nav: []) { didSet { needsDisplay = true } }
    /// The layout `fitting` settled on: columns of main keys, the nav keys
    /// that fit (Esc always does).
    private var columns = 1
    private var nav: [Key] = []
    /// The material, and the tint and keys drawn over it.
    private let material = NSVisualEffectView()
    private let face = Face()
    override var isFlipped: Bool { true }
    override var needsDisplay: Bool {
        didSet { if needsDisplay { face.needsDisplay = true } }
    }

    /// Draws the panel's tint, edge and keys over the material.
    private final class Face: NSView {
        weak var panel: HintKeyPanel?
        override var isFlipped: Bool { true }
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        override func draw(_ dirtyRect: NSRect) { panel?.drawFace() }
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.shadowOpacity = Float(Design.Alpha.paletteShadow)
        layer?.shadowRadius = 20
        layer?.shadowOffset = CGSize(width: 0, height: -10)
        material.blendingMode = .withinWindow
        material.state = .active
        material.wantsLayer = true
        material.layer?.cornerRadius = Design.Radius.hintPanel
        material.layer?.masksToBounds = true
        face.panel = self
        for view in [material, face] {
            view.frame = bounds
            view.autoresizingMask = [.width, .height]
            addSubview(view)
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        // The shadow from the card's outline, not the translucent content.
        layer?.shadowPath = CGPath(roundedRect: bounds, cornerWidth: Design.Radius.hintPanel,
                                   cornerHeight: Design.Radius.hintPanel, transform: nil)
    }

    func applyTheme() {
        material.material = Design.Material.hintPanel(Theme.current.appearance)
        material.appearance = Theme.current.appearance.nsAppearance
        layer?.shadowColor = Design.Color.shadow.cgColor
        needsDisplay = true
    }

    private static let mainFont = Design.Font.row
    private static let navFont = Design.Font.hintLegend
    private static let titleFont = Design.Font.hintPanelTitle

    private static func textWidth(_ text: String, _ font: NSFont) -> CGFloat {
        ceil((text as NSString).size(withAttributes: [.font: font]).width)
    }

    private static func width(_ key: Key, font: NSFont) -> CGFloat {
        var width = HintKeycap.size(for: key.key, large: key.kind).width + gap + textWidth(key.text, font)
        if key.count > 0 { width += gap + textWidth(String(key.count), font) }
        return width
    }

    private static func rowWidth(_ keys: [Key]) -> CGFloat {
        keys.map { width($0, font: navFont) }.reduce(0, +) + CGFloat(max(0, keys.count - 1)) * navSpacing
    }

    private var titleWidth: CGFloat {
        let key = content.titleKey.map { HintKeycap.size(for: $0, large: true).width + Self.gap } ?? 0
        return key + Self.textWidth(content.title, Self.titleFont)
    }

    private var columnWidth: CGFloat { content.main.map { Self.width($0, font: Self.mainFont) }.max() ?? 0 }

    /// The panel's size within `maxWidth`, settling its columns and which
    /// nav keys fit (dropped from the end, Esc kept).
    func fitting(maxWidth: CGFloat) -> CGSize {
        let room = max(0, maxWidth - 2 * Self.padding)
        let column = columnWidth
        let fit = Int((room + Self.columnGap) / max(1, column + Self.columnGap))
        columns = max(1, min(content.main.count, fit, Self.maxColumns))
        nav = content.nav
        while nav.count > 1, Self.rowWidth(nav) > room { nav.remove(at: nav.count - 2) }
        let grid = content.main.isEmpty ? 0 : CGFloat(columns) * column + CGFloat(columns - 1) * Self.columnGap
        let width = min(maxWidth, max(grid, Self.rowWidth(nav), titleWidth) + 2 * Self.padding)
        let rows = (content.main.count + columns - 1) / columns
        var height = Self.padding + Self.title
        if rows > 0 { height += Self.section + CGFloat(rows) * Self.row }
        height += Self.section + Self.row + Self.padding - 4
        return CGSize(width: ceil(width), height: ceil(height))
    }

    fileprivate func drawFace() {
        let radius = Design.Radius.hintPanel
        let card = NSBezierPath(roundedRect: bounds, xRadius: radius, yRadius: radius)
        Design.Color.hintPanel.setFill()
        card.fill()
        // The top highlight: the edge's upper band, a lighter line that
        // follows the corners and fades out down the sides.
        let line = Design.Border.hintPanel
        let inner = NSBezierPath(roundedRect: bounds.insetBy(dx: line * 1.5, dy: line * 1.5),
                                 xRadius: radius - line * 1.5, yRadius: radius - line * 1.5)
        inner.lineWidth = line
        NSGraphicsContext.saveGraphicsState()
        NSBezierPath(rect: CGRect(x: 0, y: 0, width: bounds.width, height: radius)).addClip()
        Design.Color.hintPanelHighlight.setStroke()
        inner.stroke()
        NSGraphicsContext.restoreGraphicsState()
        let edge = NSBezierPath(roundedRect: bounds.insetBy(dx: line / 2, dy: line / 2),
                                xRadius: radius - line / 2, yRadius: radius - line / 2)
        edge.lineWidth = line
        Design.Color.hintPanelBorder.setStroke()
        edge.stroke()

        var y = Self.padding
        var x = Self.padding
        if let key = content.titleKey {
            let size = HintKeycap.size(for: key, large: true)
            HintKeycap.draw(key, in: CGRect(x: x, y: y + (Self.title - size.height) / 2, width: size.width,
                                            height: size.height), large: true)
            x += size.width + Self.gap
        }
        draw(content.title, at: x, y: y, height: Self.title, font: Self.titleFont, color: Design.Color.hintPanelText)
        y += Self.title
        if !content.main.isEmpty {
            y += Self.section
            let column = columnWidth + Self.columnGap
            for (index, key) in content.main.enumerated() {
                let cell = CGPoint(x: Self.padding + CGFloat(index % columns) * column,
                                   y: y + CGFloat(index / columns) * Self.row)
                draw(key, at: cell, width: columnWidth, font: Self.mainFont, text: Design.Color.hintPanelText,
                     raised: true)
            }
            y += CGFloat((content.main.count + columns - 1) / columns) * Self.row
        }
        // A hairline over the moving-about keys.
        Design.Color.hintPanelBorder.setFill()
        let hairline = 1 / max(window?.backingScaleFactor ?? 2, 1)
        NSBezierPath(rect: CGRect(x: Self.padding, y: y + Self.section / 2, width: bounds.width - 2 * Self.padding,
                                  height: hairline)).fill()
        y += Self.section
        x = Self.padding
        for key in nav {
            draw(key, at: CGPoint(x: x, y: y), width: nil, font: Self.navFont,
                 text: Design.Color.hintPanelTextSecondary, raised: false)
            x += Self.width(key, font: Self.navFont) + Self.navSpacing
        }
    }

    /// A key and its words; with a `width` (a grid column), its count sits
    /// at the column's right edge so the counts line up.
    private func draw(_ key: Key, at origin: CGPoint, width: CGFloat?, font: NSFont, text: NSColor, raised: Bool) {
        let size = HintKeycap.size(for: key.key, large: key.kind)
        HintKeycap.draw(key.key, in: CGRect(x: origin.x, y: origin.y + ((Self.row - size.height) / 2).rounded(),
                                            width: size.width, height: size.height),
                        raised: raised || key.kind, large: key.kind, enabled: key.enabled)
        var x = origin.x + size.width + Self.gap
        let muted = Design.Color.hintPanelTextSecondary
        let color = key.enabled ? text : muted.withAlphaComponent(muted.alphaComponent * Design.Alpha.dimmed)
        draw(key.text, at: x, y: origin.y, height: Self.row, font: font, color: color)
        if key.count > 0 {
            let count = String(key.count)
            x = width.map { origin.x + $0 - Self.textWidth(count, font) } ?? x + Self.textWidth(key.text, font) + Self.gap
            draw(count, at: x, y: origin.y, height: Self.row, font: font,
                 color: key.enabled ? muted : color)
        }
    }

    private func draw(_ text: String, at x: CGFloat, y: CGFloat, height: CGFloat, font: NSFont, color: NSColor) {
        let attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color]
        let size = (text as NSString).size(withAttributes: attributes)
        (text as NSString).draw(at: CGPoint(x: x, y: y + (height - size.height) / 2), withAttributes: attributes)
    }
}

@MainActor
private final class HintActionButton: NSButton {
    static let width: CGFloat = 52
    private let shortcut: String

    init(_ shortcut: String) {
        self.shortcut = shortcut
        super.init(frame: .zero)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        cell?.draw(withFrame: CGRect(x: 0, y: 0, width: 28, height: bounds.height), in: self)
        let size = HintKeycap.size(for: shortcut)
        HintKeycap.draw(shortcut, in: CGRect(x: 30, y: ((bounds.height - size.height) / 2).rounded(),
                                             width: size.width, height: size.height), enabled: isEnabled)
    }
}

@MainActor
private final class HintCanvas: NSView {
    weak var overlay: TerminalHintOverlay?
    enum Kind { case main, ghost, reach }
    /// main: highlights and labels; ghost: labels a keystroke ruled out,
    /// fading; reach: the labels ⇧ shows with an item selected.
    var kind: Kind = .main
    override var isFlipped: Bool { true }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
    override func draw(_ dirtyRect: NSRect) { overlay?.drawHints(kind) }
}
