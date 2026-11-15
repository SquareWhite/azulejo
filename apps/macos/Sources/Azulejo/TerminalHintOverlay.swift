import AppKit
import IOSurface

/// ⌘S smart select: one immutable, window-local selection session. Live
/// output continues behind captured terminal frames; labels and action
/// payloads never retarget.
///
/// Two modes. Choosing: every item has a label whose first letter is its
/// kind (`TerminalSelection.labels`); typing filters the labels, Backspace
/// takes a letter back, a complete label selects its item. Arrow keys move
/// a focus ring across the items still shown (starting at the terminal
/// cursor), Return selects the focused one and ⇧ + arrow starts a range
/// from it. Selected: c / o /
/// r act on the selection, arrows move it (⇧ extends it from the anchor),
/// ⇧ + another label extends it to that item, ⌥↑ / ⌥↓ grow it to the
/// containing item, line, paragraph, block or screen and shrink it back.
/// A legend in the focused tile's header strip shows the keys that apply.
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
    private let legend = HintLegend()
    /// The tile whose header strip shows the legend (the focused one), or
    /// nil: the bar at the bottom of the pane.
    private weak var legendHost: TileHeaderView?
    /// Bumped each time the legend shows, so an earlier fade-out's
    /// completion can't take down a newer legend.
    private var legendGeneration = 0
    /// Automation: how many times the legend moved after it first showed.
    private var legendMoves = 0
    private let panel = NSView()
    private let emptyLabel = NSTextField(labelWithString: "No selectable items · Esc to return")
    private let copyButton = HintActionButton("C")
    private let openButton = HintActionButton("O")
    private let runButton = HintActionButton("R")
    private var selection: Selection?
    /// While choosing, the entry the arrow keys have focused (Return
    /// selects it), or nil before the first arrow key.
    private var focused: Int?
    private var prefix = ""
    /// Letters typed with ⇧ while selected: another item's label.
    private var extendPrefix = ""
    private var tabID: String?
    private var focusedID: String?
    private var machineID: String?
    private var workspaceCwd: String?
    private(set) var lastOpened = ""
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
        emptyLabel.font = Design.Font.monoDetail
        emptyLabel.drawsBackground = true
        for (button, symbol, label, tip) in [
            (copyButton, "doc.on.doc", "Copy", "Copy (C)"),
            (openButton, "arrow.up.forward.app", "Open", "Open in Default App (O)"),
            (runButton, "play.fill", "Run in New Pane", "Run in New Pane (R)"),
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
            // A repaint during extraction must not produce labels for old
            // text over a newer frame. The user can invoke the mode again.
            guard (0..<grid.rows).allSatisfy({ surface.hintRow($0, through: grid.cols - 1)?.text == rows[$0] }),
                  let image = surface.snapshot() else { continue }
            let frozen = NSView(frame: surfaceFrame)
            frozen.wantsLayer = true
            frozen.layer?.contents = image
            frozen.layer?.contentsGravity = .resize
            frozen.layer?.contentsScale = controller.window.backingScaleFactor
            addSubview(frozen)
            hints.keep(kept)
            captures.append(Capture(tile: tile, surface: surface, frame: surfaceFrame, rows: rows, hints: hints,
                                    firstEntry: found.count, rowCells: rowCells, offsetColumns: offsetColumns, cols: grid.cols,
                                    margin: margin, text: text, cursor: surface.hintCursor()))
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
        showLegend()
        addSubview(panel)
        emptyLabel.sizeToFit()
        emptyLabel.frame.origin = CGPoint(x: bounds.midX - emptyLabel.frame.width / 2, y: bounds.midY)
        addSubview(emptyLabel)
        panel.isHidden = true
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
        hideLegend()
        cancelReachWait()
        reachShown = false
        removeFromSuperview()
        for view in subviews { view.removeFromSuperview() }
        captures.removeAll()
        entries.removeAll()
        ghostEntries.removeAll()
        selection = nil
        focused = nil
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

    private func choose(_ chord: String) {
        switch chord {
        case "backspace", "delete":
            if !prefix.isEmpty { prefix.removeLast() }
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
        guard entries.contains(where: { $0.label.hasPrefix(next) }) else { return }
        prefix = next
        // A complete label wins over the focus ring.
        if let index = entries.firstIndex(where: { $0.label == next }) { select(index); return }
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
        TerminalSelection.Unit(range: TerminalSelection.extent(entries[index].item),
                               level: entries[index].item.kind == .text ? .text : .item, entry: index)
    }

    private func act(_ chord: String) {
        guard var current = selection else { return }
        if let typed = Self.letter(chord), typed.shift {
            // ⇧ + a label: extend to that item (same pane only).
            let next = extendPrefix + typed.letter
            let pane = paneEntries(current.pane)
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

    /// Entries whose labels show while choosing (or, selected, the labels
    /// ⇧ can still reach: the pane's items outside the selection).
    private var visibleEntries: Set<Int> {
        if let selection {
            let range = selection.range
            return Set(paneEntries(selection.pane).filter {
                !entries[$0].label.isEmpty && !range.contains(TerminalSelection.extent(entries[$0].item))
                    && entries[$0].label.hasPrefix(extendPrefix)
            })
        }
        return Set(entries.indices.filter { !entries[$0].label.isEmpty && entries[$0].label.hasPrefix(prefix) })
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
    private func placeCallouts() {
        for capture in captures {
            // Unlabelled repeats get no callout: their text is text to
            // keep off like any other.
            let indices = entries.indices.filter { entries[$0].tileID == capture.tile.id && !entries[$0].label.isEmpty }
            guard let cell = indices.first.map({ entries[$0].cell }) else { continue }
            let metrics = Self.calloutMetrics(cell)
            let items = indices.map { index in
                let width = (entries[index].label.uppercased() as NSString).size(withAttributes: [.font: metrics.font]).width
                return TerminalSelection.CalloutItem(
                    anchor: entries[index].anchor, rects: entries[index].rects, prose: entries[index].item.kind == .text,
                    size: CGSize(width: max(metrics.height, ceil(width) + 2 * metrics.padding), height: metrics.height))
            }
            let placed = TerminalHints.placeCallouts(items, text: capture.text, bounds: capture.margin,
                                                     tail: metrics.tail, spacing: 2,
                                                     inset: metrics.radius + metrics.half, drift: cell.width / 2)
            for (index, callout) in zip(indices, placed) { entries[index].callout = callout }
        }
    }

    private func refresh() {
        emptyLabel.isHidden = !entries.isEmpty
        copyButton.isEnabled = selection != nil
        // Only the actions that apply: no greyed-out buttons.
        openButton.isHidden = !canOpen
        openButton.contentTintColor = Design.Color.textSecondary
        openButton.toolTip = "Open in Default App (O)"
        runButton.isHidden = !canRun
        layoutLegend()
        layoutPanel()
        canvas.needsDisplay = true
        reach.needsDisplay = true
    }

    /// The keys that apply right now: the key, what it does, and whether
    /// it's a kind letter (drawn as a little callout, like the labels).
    private var legendItems: [HintLegend.Item] {
        guard let selection else {
            let visible = visibleEntries
            var items: [HintLegend.Item] = TerminalSelection.kindOrder.compactMap { kind, name in
                visible.contains { entries[$0].item.kind == kind }
                    ? .init(key: String(TerminalSelection.letter(kind)), text: name, kind: true) : nil
            }
            // Arrows move a focus ring over what's shown; Return picks it.
            if !visible.isEmpty { items.append(.init(key: "↑↓←→", text: "move")) }
            if focused != nil { items.append(.init(key: "⏎", text: "select")) }
            if !prefix.isEmpty { items.append(.init(key: "⌫", text: "back")) }
            items.append(.init(key: "esc", text: "cancel"))
            return items
        }
        if !extendPrefix.isEmpty {
            return [.init(key: "⇧", text: "extend to \(extendPrefix.uppercased())…"), .init(key: "⌫", text: "back"),
                    .init(key: "esc", text: "cancel")]
        }
        var items: [HintLegend.Item] = [.init(key: "c", text: "copy")]
        if canOpen { items.append(.init(key: "o", text: "open")) }
        if canRun { items.append(.init(key: "r", text: "run")) }
        if captures[selection.pane].hints.hasTargets(selection.focus.level) { items.append(.init(key: "←→↑↓", text: "move")) }
        // Holding ⇧ shows the labels it can extend to.
        items.append(.init(key: "⇧", text: "hold to extend"))
        let grow = grownUnit(selection) != nil
        let shrink = !selection.history.isEmpty || shrunkUnit(selection) != nil
        if grow && shrink { items.append(.init(key: "⌥↑↓", text: "grow/shrink")) }
        else if grow { items.append(.init(key: "⌥↑", text: "grow")) }
        else if shrink { items.append(.init(key: "⌥↓", text: "shrink")) }
        items.append(.init(key: "esc", text: "cancel"))
        return items
    }

    /// The pane the legend belongs to for the whole session: the focused
    /// tile's, else the first one captured.
    private var legendPane: Capture? {
        captures.first { $0.tile.id == focusedID } ?? captures.first
    }

    /// The legend stands in for the legend pane's title content (its title,
    /// or its tabs in Monocle: `TileHeaderView.overrideContent`), clear of
    /// the strip's badge and controls, so it never covers terminal text and
    /// never moves during a session. It fades in (`motion.hintFilter`), and
    /// on the way out fades over the title or tabs coming back. A tile
    /// without a header strip gets a bar at the bottom of the pane instead.
    private func showLegend() {
        legendGeneration += 1
        legendMoves = 0
        legend.layer?.removeAllAnimations()
        if let host = legendHost, host.overrideContent === legend { host.overrideContent = nil }
        legend.removeFromSuperview()
        legend.frame = .zero
        legendHost = nil
        guard let pane = legendPane, !entries.isEmpty else { return }
        if let header = controller?.tiles.header(of: pane.tile.id), !header.isHiddenOrHasHiddenAncestor,
           header.bounds.height >= HintLegend.minimumStrip {
            legendHost = header
            legend.inStrip = true
            legend.alphaValue = 0
            header.overrideContent = legend
            header.layoutSubtreeIfNeeded()
            let duration = Motion.active ? Design.Motion.hintFilter : 0
            if duration > 0 {
                NSAnimationContext.runAnimationGroup { context in
                    context.duration = duration
                    context.timingFunction = Motion.timing
                    legend.animator().alphaValue = 1
                }
            } else { legend.alphaValue = 1 }
        } else {
            legend.inStrip = false
            legend.alphaValue = 1
            addSubview(legend)
        }
        legend.applyTheme()
    }

    /// Gives the strip its title or tabs back, the legend fading out over
    /// them; the bar goes with the overlay.
    private func hideLegend() {
        guard let host = legendHost, host.overrideContent === legend else { return }
        legendHost = nil
        let frame = legend.frame
        host.overrideContent = nil
        let generation = legendGeneration
        let duration = Motion.active ? Design.Motion.hintFilter : 0
        guard duration > 0, host.window != nil else { return }
        // Opaque in the strip's colour: fading, it uncovers what came back.
        legend.frame = frame
        host.addSubview(legend)
        NSAnimationContext.runAnimationGroup({ context in
            context.duration = duration
            context.timingFunction = Motion.timing
            legend.animator().alphaValue = 0
        }, completionHandler: { [weak self] in
            MainActor.assumeIsolated {
                guard let self, self.legendGeneration == generation, self.legendHost == nil,
                      self.legend.superview === host else { return }
                self.legend.removeFromSuperview()
            }
        })
    }

    /// The legend's frame in this view's coordinates (`.null` when hidden).
    private var legendFrame: CGRect {
        guard !legend.isHidden, let parent = legend.superview else { return .null }
        return parent === self ? legend.frame : parent.convert(legend.frame, to: self)
    }

    private func layoutLegend() {
        guard let pane = legendPane, !entries.isEmpty, legend.superview != nil else { legend.isHidden = true; return }
        legend.isHidden = false
        let before = legend.frame
        if legendHost != nil {
            // The strip lays it out in its content rect.
            legend.set(legendItems, maxWidth: legend.bounds.width)
        } else {
            legend.set(legendItems, maxWidth: pane.frame.width - 2 * HintLegend.margin)
            let size = legend.fittingSize
            legend.frame = CGRect(x: (pane.frame.midX - size.width / 2).rounded(),
                                  y: pane.frame.maxY - size.height - HintLegend.margin,
                                  width: size.width, height: size.height)
        }
        if before != .zero, before != legend.frame { legendMoves += 1 }
        legend.needsDisplay = true
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
        let legendFrame = legendFrame
        panel.frame = candidates.first { candidate in
            pane.contains(candidate) && !rects.contains(where: { $0.intersects(candidate) })
                && !candidate.intersects(legendFrame)
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

    @objc private func runSelection() {
        guard let entry = singleEntry, let app, let controller else { return }
        guard entry.item.kind == .command, let cwd = entry.cwd else { return }
        validate()
        guard isOpen else { return }
        close()
        let command: [String: Any] = ["method": "layout.split", "params": [
            "tile": entry.tileID, "direction": "right",
            "content": ["kind": "terminal", "cwd": cwd, "command": [TerminalHints.runLine(entry.item.value)]],
        ]]
        if case .failure(let error) = app.send(command) { controller.report(error) }
    }

    // MARK: Drawing

    func applyTheme() {
        panel.layer?.backgroundColor = Design.Color.sidebar.cgColor
        panel.layer?.borderColor = Design.Color.border.cgColor
        panel.layer?.shadowColor = Design.Color.border.cgColor
        emptyLabel.textColor = Design.Color.textPrimary
        emptyLabel.backgroundColor = Design.Color.overlay
        for button in [copyButton, openButton, runButton] {
            button.contentTintColor = Design.Color.textSecondary
        }
        legend.applyTheme()
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
        if let single = selection.single {
            for index in repeatsOf(single) { drawBoxes(entries[index]) }
        }
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
        for entry in shown {
            if entry.item.kind == .text {
                // A paragraph: a dashed outline round its rows, under the
                // items inside it.
                let box = entry.rects.dropFirst().reduce(entry.rects[0]) { $0.union($1) }
                let path = NSBezierPath(rect: box.insetBy(dx: -1, dy: 0))
                path.setLineDash([3, 2], count: 2, phase: 0)
                path.lineWidth = 1
                Design.Color.accent.setStroke()
                path.stroke()
                continue
            }
            drawBoxes(entry)
        }
        if let focus { drawFocusRing(entries[focus]) }
        // Callouts last: they float over every highlight and the text.
        for (index, entry) in zip(order, shown) {
            drawCallout(entry, alpha: 1, typed: typed, focused: index == focus)
        }
    }

    /// The focus ring the arrow keys move while choosing: a thicker
    /// outline (`hintFocus`) just outside the item's rects (a paragraph: its
    /// rows' box), distinct from the selected item's filled highlight.
    private func drawFocusRing(_ entry: Entry) {
        let rects = entry.item.kind == .text
            ? [entry.rects.dropFirst().reduce(entry.rects[0]) { $0.union($1) }] : entry.rects
        Design.Color.hintFocus.setStroke()
        for rect in rects {
            let path = NSBezierPath(roundedRect: rect.insetBy(dx: -2, dy: -1.5), xRadius: 2, yRadius: 2)
            path.lineWidth = 2
            path.stroke()
        }
    }

    /// An item's highlight while choosing: a light fill and an outline.
    private func drawBoxes(_ entry: Entry) {
        for rect in entry.rects {
            let path = NSBezierPath(rect: rect)
            Design.Color.accent.withAlphaComponent(0.18).setFill()
            path.fill()
            Design.Color.accent.setStroke()
            path.lineWidth = 1
            path.stroke()
        }
    }

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
    /// none when it sits over the item itself), a thin outline and a soft
    /// shadow; dark letters on yellow (`hintCallout*`). The first `typed`
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
        // Shadow under the whole shape, then the outline, then the fill
        // again over the outline's inner half so pill and tail read as one.
        NSGraphicsContext.saveGraphicsState()
        let shadow = NSShadow()
        shadow.shadowColor = Design.Color.hintCalloutShadow
        shadow.shadowBlurRadius = 3
        shadow.shadowOffset = CGSize(width: 0, height: -1)
        shadow.set()
        let body = focused ? Design.Color.hintCalloutFocus : Design.Color.hintCallout
        body.setFill()
        pill.fill()
        if !tail.isEmpty { tail.fill() }
        NSGraphicsContext.restoreGraphicsState()
        Design.Color.hintCalloutBorder.setStroke()
        for path in [pill, tail] where !path.isEmpty {
            path.lineWidth = 1.5
            path.lineJoinStyle = .round
            path.stroke()
        }
        body.setFill()
        pill.fill()
        if !tail.isEmpty { tail.fill() }
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
        let legendVisible = isOpen && !legend.isHidden && legend.superview != nil
        let legendPosition = legendVisible ? (legendHost != nil ? "header" : "bottom") : "hidden"
        // The legend's own room: the header strip, or the pane.
        let legendRoom: CGRect? = legendHost.map { $0.convert($0.bounds, to: self) } ?? legendPane?.frame
        let calloutStats = calloutStatistics
        let actions = selection.map { _ in
            (["copy"] + (canOpen ? ["open"] : []) + (canRun ? ["run"] : [])).joined(separator: ",")
        } ?? "-"
        let kind = selection.map { s in
            entry?.item.kind.rawValue ?? (s.anchor == s.focus ? s.focus.level.name : "range")
        } ?? "-"
        return [
            "hints.state": isOpen ? (selection == nil ? "choosing" : "selected") : "closed",
            "hints.count": String(entries.count),
            "hints.visible": String(isOpen ? visibleEntries.count : 0),
            "hints.types": entries.map { $0.item.kind.rawValue }.joined(separator: ","),
            "hints.labels": entries.isEmpty ? "-" : entries.map { $0.label.isEmpty ? "-" : $0.label }.joined(separator: ","),
            // Repeats left without a label (all but the newest copy).
            "hints.unlabelled": String(entries.filter(\.label.isEmpty).count),
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
            "hints.panes": String(captures.count),
            "hints.label": entry.map { $0.label.isEmpty ? "-" : $0.label } ?? "-",
            "hints.fade": String(Int((lastFade * 1000).rounded())),
            "hints.legend": legendVisible ? legendItems.map(\.text).joined(separator: ",") : "-",
            "hints.legend.position": legendPosition,
            "hints.legend.fits": legendVisible
                ? (legendRoom.map { $0.insetBy(dx: -0.5, dy: -0.5).contains(legendFrame) } == true
                    && legend.fittingSize.width <= legend.bounds.width + 0.5 ? "yes" : "no") : "-",
            "hints.legend.keys": legendVisible ? legend.shownKeys.joined(separator: ",") : "-",
            "hints.legend.tile": legendVisible ? (legendPane?.tile.id == focusedID ? "focused" : "other") : "-",
            "hints.legend.moves": String(legendMoves),
            "hints.popup": popupVisible ? "visible" : "hidden",
            "hints.popup.fits": popupVisible ? (popupFits ? "yes" : "no") : "-",
            "hints.popup.position": popupPosition,
            // Callouts overlapping each other, or covering another item's
            // first cell; and ones the placement couldn't clear.
            "hints.callout.clash": String(entries.indices.reduce(0) { total, i in
                total + entries[(i + 1)...].filter {
                    $0.tileID == entries[i].tileID
                        && $0.callout.frame.insetBy(dx: 0.5, dy: 0.5).intersects(entries[i].callout.frame)
                }.count
            }),
            // (Unlabelled repeats' first cells are plain text to placement.)
            "hints.callout.hides": String(entries.filter { entry in
                !entry.label.isEmpty && entries.contains { other in
                    other.tileID == entry.tileID && other.anchor != entry.anchor
                        && other.callout.frame.insetBy(dx: 0.5, dy: 0.5).intersects(entry.anchor)
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
    /// newest copy).
    private func labelledEntry(_ value: String) -> Entry? {
        entries.first { $0.item.value == value && !$0.label.isEmpty }
    }

    /// `hints select <value>`: types the label of the first item with that value.
    func automationSelect(_ value: String) -> String? {
        guard isOpen, let entry = labelledEntry(value) else { return "no such visible hint" }
        selection = nil; focused = nil; prefix = ""; extendPrefix = ""
        for ch in entry.label { handle(String(ch)) }
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

/// ⌘S keycaps, for the legend and the action popup alike: a small rounded
/// key (`hintBadge`, a hairline `hintBadgeBorder`, letters `hintBadgeText`)
/// in the callouts' shape. A kind letter is a little label callout instead
/// (`hintCallout*`): it's what the labels it picks start with.
@MainActor
private enum HintKeycap {
    static let height: CGFloat = 16

    /// Letters as the callouts show them (capitals); other keys as named.
    static func display(_ key: String) -> String {
        key.count == 1 && key.first?.isLetter == true ? key.uppercased() : key
    }

    private static func font(kind: Bool) -> NSFont {
        kind ? NSFont.systemFont(ofSize: Design.Font.hintKey.pointSize, weight: .bold) : Design.Font.hintKey
    }

    static func size(for key: String, kind: Bool = false) -> CGSize {
        let width = ceil((display(key) as NSString).size(withAttributes: [.font: font(kind: kind)]).width)
        return CGSize(width: max(height, width + 8), height: height)
    }

    /// `color` at `alpha` of its own opacity (the outline token is translucent).
    private static func faded(_ color: NSColor, _ alpha: CGFloat) -> NSColor {
        alpha >= 1 ? color : color.withAlphaComponent(color.alphaComponent * alpha)
    }

    static func draw(_ key: String, in rect: CGRect, kind: Bool = false, enabled: Bool = true) {
        let alpha: CGFloat = enabled ? 1 : 0.4
        let radius = Design.Radius.hintBadge
        let path = NSBezierPath(roundedRect: rect.insetBy(dx: 0.5, dy: 0.5), xRadius: radius, yRadius: radius)
        faded(kind ? Design.Color.hintCallout : Design.Color.hintBadge, alpha).setFill()
        path.fill()
        faded(kind ? Design.Color.hintCalloutBorder : Design.Color.hintBadgeBorder, alpha).setStroke()
        path.lineWidth = 1
        path.stroke()
        let color = kind ? Design.Color.hintCalloutText : Design.Color.hintBadgeText
        let text = display(key) as NSString
        let attributes: [NSAttributedString.Key: Any] = [.font: font(kind: kind),
                                                         .foregroundColor: faded(color, enabled ? 1 : 0.5)]
        let size = text.size(withAttributes: attributes)
        text.draw(at: CGPoint(x: rect.midX - size.width / 2, y: rect.midY - size.height / 2), withAttributes: attributes)
    }
}

/// The keys that apply right now, centred in the focused tile's header
/// strip over its title (`inStrip`), or, for a tile without a strip, in a
/// small bar at the bottom of the pane: a keycap (`HintKeycap`) and a quiet
/// word for each (`textMuted`).
@MainActor
private final class HintLegend: NSView {
    struct Item {
        let key: String
        let text: String
        var kind = false
    }

    /// Room kept between the bar and the pane's edge.
    static let margin: CGFloat = 10
    /// A strip lower than this can't hold the keycaps: the bar instead.
    static let minimumStrip: CGFloat = 20
    private static let gap: CGFloat = 4, spacing: CGFloat = 12, barPadding: CGFloat = 8, barHeight: CGFloat = 26
    var inStrip = false { didSet { if inStrip != oldValue { applyTheme() } } }
    private var items: [Item] = []
    /// Automation: the keys shown after fitting.
    var shownKeys: [String] { items.map(\.key) }
    override var isFlipped: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    func applyTheme() {
        // The bar: a card in the header's colour. In the strip, `draw`
        // fills it instead, short of the strip's hairline.
        layer?.backgroundColor = inStrip ? NSColor.clear.cgColor : Design.Color.header.cgColor
        layer?.cornerRadius = inStrip ? 0 : Design.Radius.sidebarSegment
        layer?.borderWidth = inStrip ? 0 : Design.Border.hairline
        layer?.borderColor = Design.Color.border.cgColor
        needsDisplay = true
    }

    private static func width(_ item: Item) -> CGFloat {
        HintKeycap.size(for: item.key, kind: item.kind).width + gap
            + ceil((item.text as NSString).size(withAttributes: [.font: Design.Font.hintLegend]).width)
    }

    private static func contentWidth(_ list: [Item]) -> CGFloat {
        list.map(width).reduce(0, +) + CGFloat(max(0, list.count - 1)) * spacing
    }

    /// Keeps the items that fit in `maxWidth`, dropping from the end but
    /// always keeping the last (Esc).
    func set(_ all: [Item], maxWidth: CGFloat) {
        var kept = all
        let padding = inStrip ? 0 : 2 * Self.barPadding
        while kept.count > 1, Self.contentWidth(kept) + padding > maxWidth { kept.remove(at: kept.count - 2) }
        items = kept
        needsDisplay = true
    }

    override var fittingSize: NSSize {
        let padding = inStrip ? 0 : 2 * Self.barPadding
        return NSSize(width: ceil(Self.contentWidth(items) + padding), height: inStrip ? bounds.height : Self.barHeight)
    }

    override func draw(_ dirtyRect: NSRect) {
        if inStrip {
            // Opaque, so fading out it uncovers the title coming back.
            Design.Color.header.setFill()
            let hairline = 1 / max(window?.backingScaleFactor ?? 2, 1)
            CGRect(x: 0, y: 0, width: bounds.width, height: bounds.height - hairline).fill()
        }
        var x = ((bounds.width - Self.contentWidth(items)) / 2).rounded()
        let attributes: [NSAttributedString.Key: Any] = [.font: Design.Font.hintLegend,
                                                         .foregroundColor: Design.Color.textMuted]
        for item in items {
            let size = HintKeycap.size(for: item.key, kind: item.kind)
            HintKeycap.draw(item.key, in: CGRect(x: x, y: ((bounds.height - size.height) / 2).rounded(),
                                                 width: size.width, height: size.height), kind: item.kind)
            x += size.width + Self.gap
            let text = item.text as NSString
            let textSize = text.size(withAttributes: attributes)
            text.draw(at: CGPoint(x: x, y: (bounds.height - textSize.height) / 2), withAttributes: attributes)
            x += ceil(textSize.width) + Self.spacing
        }
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
