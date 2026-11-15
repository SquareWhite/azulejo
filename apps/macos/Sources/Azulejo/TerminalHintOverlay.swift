import AppKit
import IOSurface

/// One immutable, window-local selection session. Live output continues
/// behind captured terminal frames; labels and action payloads never retarget.
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
        var badge: CGRect = .zero
        var side = TerminalHints.BadgeSide.left
    }
    private struct Capture {
        let tile: TileModel
        let surface: TerminalSurfaceView
        let frame: CGRect
    }
    private weak var controller: WindowController?
    private weak var app: AppController?
    private weak var previousResponder: NSResponder?
    private var captures: [Capture] = []
    private var entries: [Entry] = []
    private var interactionMonitor: Any?
    private let canvas = HintCanvas()
    private let panel = NSView()
    private let emptyLabel = NSTextField(labelWithString: "No selectable items · Esc to return")
    private let copyButton = HintActionButton("C")
    private let openButton = HintActionButton("O")
    private let runButton = HintActionButton("R")
    private var selected: Int?
    private var prefix = ""
    private var tabID: String?
    private var focusedID: String?
    private var machineID: String?
    private var workspaceCwd: String?
    private(set) var lastOpened = ""
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
            let rows = (0..<grid.rows).map { surface.hintRow($0, through: grid.cols - 1)?.text ?? "" }
            let cwd = tile.terminalCwd ?? workspace.cwd
            var joins: Set<Int> = []
            if grid.rows > 1 {
                for row in 0..<(grid.rows - 1) where rows[row].last?.isWhitespace == false
                    && rows[row + 1].first?.isWhitespace == false {
                    if let last = surface.hintRow(row, from: grid.cols - 1, through: grid.cols - 1)?.text,
                       !last.isEmpty, !last.allSatisfy(\.isWhitespace) { joins.insert(row) }
                }
            }
            let items = TerminalHints.detectWrapped(rows: rows, joins: joins, cwd: cwd, local: local,
                                                   branch: workspace.git?.branch,
                                                   colors: surface.hintColors(rows: rows, columns: grid.cols),
                                                   columns: grid.cols)
            let surfaceFrame = surface.convert(surface.bounds, to: self)
            var positioned: [Entry] = []
            for item in items {
                let rects = item.spans.compactMap { span -> CGRect? in
                    guard let row = surface.hintRow(span.row, through: grid.cols - 1), row.text == rows[span.row] else { return nil }
                    let start = surface.hintColumn(at: span.range.location, row: span.row, text: row.text, columns: grid.cols)
                    let end = surface.hintColumn(at: NSMaxRange(span.range), row: span.row, text: row.text, columns: grid.cols)
                    let rect = CGRect(x: row.x + CGFloat(start) * cell.width,
                                      y: surface.bounds.height - row.baseline - cell.height * 0.22,
                                      width: CGFloat(end - start) * cell.width, height: cell.height)
                    return surface.convert(rect, to: self).intersection(surfaceFrame)
                }
                guard rects.count == item.spans.count, rects.allSatisfy({ !$0.isEmpty }) else { continue }
                let cells = item.spans.map { span in
                    (row: span.row,
                     start: surface.hintColumn(at: span.range.location, row: span.row, text: rows[span.row], columns: grid.cols),
                     end: surface.hintColumn(at: NSMaxRange(span.range), row: span.row, text: rows[span.row], columns: grid.cols))
                }
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
            captures.append(Capture(tile: tile, surface: surface, frame: surfaceFrame))
            found += positioned
        }
        let labels = TerminalHints.labels(count: found.count)
        entries = found
        for index in entries.indices { entries[index].label = labels[index] }
        placeBadges()
        canvas.frame = bounds
        canvas.autoresizingMask = [.width, .height]
        canvas.overlay = self
        addSubview(canvas)
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
            } else if self.selected != nil {
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
        removeFromSuperview()
        for view in subviews { view.removeFromSuperview() }
        captures.removeAll()
        entries.removeAll()
        selected = nil
        prefix = ""
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

    func handle(_ chord: String) {
        guard isOpen else { return }
        if chord == "escape" { close(); return }
        if selected != nil {
            switch chord {
            case "c", "cmd+c": copySelection()
            case "o": openSelection()
            case "r": runSelection()
            default: break
            }
            return
        }
        if chord == "backspace" || chord == "delete" {
            if !prefix.isEmpty { prefix.removeLast() }
        } else if chord.count == 1, chord.first?.isLetter == true {
            let next = prefix + chord
            if entries.contains(where: { $0.label.hasPrefix(next) }) {
                prefix = next
                selected = entries.firstIndex { $0.label == next }
            }
        }
        refresh()
    }

    /// Labels sit just outside their item's first cell (left, else right),
    /// one cell per letter, joined to the highlight. Placement runs per pane
    /// in reading order (`TerminalHints.badgeColumns`).
    private func placeBadges() {
        for capture in captures {
            let indices = entries.indices.filter { entries[$0].tileID == capture.tile.id }
            guard let columns = indices.first.map({ entries[$0].columns }) else { continue }
            let placed = TerminalHints.badgeColumns(indices.map { index in
                TerminalHints.BadgeRequest(cells: entries[index].cells, label: entries[index].label.count,
                                           command: entries[index].item.kind == .command)
            }, columns: columns)
            for (index, place) in zip(indices, placed) {
                let entry = entries[index]
                guard let rect = entry.rects.first, let first = entry.cells.first else { continue }
                entries[index].badge = CGRect(x: rect.minX + CGFloat(place.start - first.start) * entry.cell.width,
                                              y: rect.minY, width: CGFloat(entry.label.count) * entry.cell.width,
                                              height: rect.height)
                entries[index].side = place.side
            }
        }
    }

    private func refresh() {
        let entry = selected.map { entries[$0] }
        emptyLabel.isHidden = !entries.isEmpty
        copyButton.isEnabled = entry != nil
        openButton.isEnabled = entry?.item.openURL != nil
        openButton.contentTintColor = Design.Color.textSecondary
        openButton.toolTip = openButton.isEnabled
            ? "Open in Default App (O)" : "Open (O) · Only URLs and existing local paths"
        runButton.isHidden = entry?.item.kind != .command
        runButton.isEnabled = entry?.item.kind == .command && entry?.cwd != nil
        runButton.toolTip = runButton.isEnabled
            ? "Run in New Pane (R)" : "Run in New Pane (R) · Working directory unavailable"
        layoutPanel()
        canvas.needsDisplay = true
    }

    private func layoutPanel() {
        guard let selected, let first = entries[selected].rects.first,
              let last = entries[selected].rects.last,
              let pane = captures.first(where: { $0.tile.id == entries[selected].tileID })?.frame else {
            panel.isHidden = true
            return
        }
        let buttonSize: CGFloat = 28, padding: CGFloat = 4, spacing: CGFloat = 2, gap: CGFloat = 6
        var x = padding
        for button in [copyButton, openButton, runButton] where !button.isHidden {
            button.frame = CGRect(x: x, y: padding, width: HintActionButton.width, height: buttonSize)
            x += HintActionButton.width + spacing
        }
        let size = CGSize(width: x - spacing + padding, height: buttonSize + padding * 2)
        let belowX = min(max(pane.minX, last.minX), pane.maxX - size.width)
        let aboveX = min(max(pane.minX, first.minX), pane.maxX - size.width)
        let positions = [
            CGPoint(x: last.maxX + gap, y: last.midY - size.height / 2),
            CGPoint(x: belowX, y: last.maxY + gap),
            CGPoint(x: aboveX, y: first.minY - size.height - gap),
        ]
        let candidates = positions.map { CGRect(origin: $0, size: size) }
        panel.frame = candidates.first { candidate in
            pane.contains(candidate) && !entries[selected].rects.contains(where: { $0.intersects(candidate) })
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
        return captures.contains(where: { $0.frame.contains(point) }) ? self : nil
    }

    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        if selected == nil, let index = entries.firstIndex(where: { $0.badge.contains(point) }) {
            selected = index; refresh()
        } else { close() }
    }


    @objc private func copySelection() {
        guard let selected else { return }
        let value = entries[selected].item.value
        TerminalRuntime.pasteboard.clearContents()
        TerminalRuntime.pasteboard.setString(value, forType: .string)
        close()
    }

    @objc private func openSelection() {
        guard let selected, let url = entries[selected].item.openURL else { return }
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
        guard let selected, let app, let controller else { return }
        let entry = entries[selected]
        guard entry.item.kind == .command, let cwd = entry.cwd else { return }
        validate()
        guard isOpen else { return }
        close()
        let command: [String: Any] = ["method": "layout.split", "params": [
            "tile": entry.tileID, "direction": "right",
            "content": ["kind": "terminal", "cwd": cwd, "command": [entry.item.value]],
        ]]
        if case .failure(let error) = app.send(command) { controller.report(error) }
    }

    func applyTheme() {
        panel.layer?.backgroundColor = Design.Color.sidebar.cgColor
        panel.layer?.borderColor = Design.Color.border.cgColor
        panel.layer?.shadowColor = Design.Color.border.cgColor
        emptyLabel.textColor = Design.Color.textPrimary
        emptyLabel.backgroundColor = Design.Color.overlay
        for button in [copyButton, openButton, runButton] {
            button.contentTintColor = Design.Color.textSecondary
        }
        canvas.needsDisplay = true
    }

    fileprivate func drawHints() {
        let visible = entries.enumerated().filter { index, entry in
            selected.map { $0 == index } ?? entry.label.hasPrefix(prefix)
        }.map(\.element)
        let radius = Design.Radius.hintBadge
        for entry in visible {
            for (index, rect) in entry.rects.enumerated() {
                // The label extends the first row's box: one outline, rounded
                // on the label's outer side.
                let labelled = index == 0 && selected == nil
                let box = labelled ? rect.union(entry.badge) : rect
                let path = labelled
                    ? Self.path(box, left: entry.badge.minX <= box.minX ? radius : 0,
                                right: entry.badge.maxX >= box.maxX ? radius : 0)
                    : NSBezierPath(rect: rect)
                Design.Color.accent.withAlphaComponent(selected == nil ? 0.18 : 0.35).setFill()
                path.fill()
                Design.Color.accent.setStroke()
                path.lineWidth = 1
                path.stroke()
            }
        }
        guard selected == nil else { return }
        for entry in visible {
            let box = entry.rects[0].union(entry.badge)
            Design.Color.hintBadge.setFill()
            Self.path(entry.badge, left: entry.badge.minX <= box.minX ? radius : 0,
                      right: entry.badge.maxX >= box.maxX ? radius : 0).fill()
            let font = NSFont.monospacedSystemFont(ofSize: entry.cell.height * 0.62, weight: .bold)
            let attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: Design.Color.hintBadgeText]
            for (i, ch) in entry.label.uppercased().enumerated() {
                let text = String(ch) as NSString
                let size = text.size(withAttributes: attributes)
                text.draw(at: CGPoint(x: entry.badge.minX + CGFloat(i) * entry.cell.width + (entry.cell.width - size.width) / 2,
                                      y: entry.badge.midY - size.height / 2), withAttributes: attributes)
            }
        }
    }

    /// A rectangle with its left and/or right corners rounded.
    private static func path(_ rect: CGRect, left: CGFloat, right: CGFloat) -> NSBezierPath {
        let path = NSBezierPath()
        path.move(to: CGPoint(x: rect.minX + left, y: rect.minY))
        path.line(to: CGPoint(x: rect.maxX - right, y: rect.minY))
        if right > 0 {
            path.appendArc(from: CGPoint(x: rect.maxX, y: rect.minY), to: CGPoint(x: rect.maxX, y: rect.maxY), radius: right)
            path.appendArc(from: CGPoint(x: rect.maxX, y: rect.maxY), to: CGPoint(x: rect.minX, y: rect.maxY), radius: right)
        } else {
            path.line(to: CGPoint(x: rect.maxX, y: rect.minY))
            path.line(to: CGPoint(x: rect.maxX, y: rect.maxY))
        }
        path.line(to: CGPoint(x: rect.minX + left, y: rect.maxY))
        if left > 0 {
            path.appendArc(from: CGPoint(x: rect.minX, y: rect.maxY), to: CGPoint(x: rect.minX, y: rect.minY), radius: left)
            path.appendArc(from: CGPoint(x: rect.minX, y: rect.minY), to: CGPoint(x: rect.maxX, y: rect.minY), radius: left)
        } else {
            path.line(to: CGPoint(x: rect.minX, y: rect.maxY))
            path.line(to: CGPoint(x: rect.minX, y: rect.minY))
        }
        path.close()
        return path
    }

    var automationState: [String: String] {
        let entry = selected.map { entries[$0] }
        let popupVisible = isOpen && !panel.isHidden
        let popupFits = entry.flatMap { item in captures.first { $0.tile.id == item.tileID } }
            .map { $0.frame.contains(panel.frame) } ?? false
        var popupPosition = "hidden"
        if popupVisible, let first = entry?.rects.first, let last = entry?.rects.last {
            if panel.frame.maxY <= first.minY { popupPosition = "above" }
            else if panel.frame.minY >= last.maxY { popupPosition = "below" }
            else if panel.frame.minX >= last.maxX { popupPosition = "right" }
            else { popupPosition = "overlap" }
        }
        return [
            "hints.state": isOpen ? (selected == nil ? "choosing" : "selected") : "closed",
            "hints.count": String(entries.count),
            "hints.types": entries.map { $0.item.kind.rawValue }.joined(separator: ","),
            "hints.selected": entry?.item.value ?? "-",
            "hints.kind": entry?.item.kind.rawValue ?? "-",
            "hints.prefix": prefix.isEmpty ? "-" : prefix,
            "hints.actions": entry.map { e in
                (["copy"] + (e.item.openURL == nil ? [] : ["open"]) + (e.item.kind == .command && e.cwd != nil ? ["run"] : [])).joined(separator: ",")
            } ?? "-",
            "hints.opened": lastOpened.isEmpty ? "-" : lastOpened,
            "hints.panes": String(captures.count),
            "hints.label": entry?.label ?? "-",
            "hints.popup": popupVisible ? "visible" : "hidden",
            "hints.popup.fits": popupVisible ? (popupFits ? "yes" : "no") : "-",
            "hints.popup.position": popupPosition,
            // Labels over their own item, or over any non-command item.
            "hints.badge.covers": String(entries.filter { entry in
                entries.contains { other in
                    (other.item == entry.item || other.item.kind != .command)
                        && other.rects.contains { $0.insetBy(dx: 0.5, dy: 0.5).intersects(entry.badge) }
                }
            }.count),
            "hints.badge.clash": String(entries.indices.reduce(0) { total, i in
                total + entries[(i + 1)...].filter { $0.badge.insetBy(dx: 0.5, dy: 0.5).intersects(entries[i].badge) }.count
            }),
        ]
    }

    func automationSelect(_ value: String) -> String? {
        guard isOpen, let entry = entries.first(where: { $0.item.value == value }) else { return "no such visible hint" }
        selected = nil; prefix = ""
        for ch in entry.label { handle(String(ch)) }
        return nil
    }

    /// `hints badge <side> <value>`: where the visible item's label sits.
    func automationBadge(_ arg: String) -> String? {
        let parts = arg.split(separator: " ", maxSplits: 1).map(String.init)
        guard isOpen, parts.count == 2, let entry = entries.first(where: { $0.item.value == parts[1] })
        else { return "no such visible hint" }
        return entry.side.rawValue == parts[0] ? nil : "label is \(entry.side.rawValue), not \(parts[0])"
    }

    func automationClick(_ action: String) -> String? {
        let button: NSButton? = switch action {
        case "copy": copyButton
        case "open": openButton
        case "run": runButton
        default: nil
        }
        guard isOpen, let button, button.isEnabled else { return "hint action is unavailable" }
        button.performClick(nil)
        return nil
    }

    func automationClickAway(chrome: Bool) -> String? {
        guard isOpen, selected != nil, let window else { return "no selected hint popup" }
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

/// Both item labels and action keys use this exact bubble renderer.
@MainActor
private enum HintBubble {
    static func size(for label: String) -> CGSize {
        CGSize(width: CGFloat(label.count) * 8 + 10, height: 16)
    }

    static func draw(_ label: String, in rect: CGRect, enabled: Bool = true) {
        Design.Color.accent.withAlphaComponent(enabled ? 1 : 0.35).setFill()
        NSBezierPath(roundedRect: rect, xRadius: 3, yRadius: 3).fill()
        (label.uppercased() as NSString).draw(at: CGPoint(x: rect.minX + 5, y: rect.minY + 1),
            withAttributes: [.font: Design.Font.chord,
                             .foregroundColor: Design.Color.textOnAccent.withAlphaComponent(enabled ? 1 : 0.5)])
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
        let size = HintBubble.size(for: shortcut)
        HintBubble.draw(shortcut, in: CGRect(x: 30, y: (bounds.height - size.height) / 2,
                                            width: size.width, height: size.height), enabled: isEnabled)
    }
}

@MainActor
private final class HintCanvas: NSView {
    weak var overlay: TerminalHintOverlay?
    override var isFlipped: Bool { true }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
    override func draw(_ dirtyRect: NSRect) { overlay?.drawHints() }
}
