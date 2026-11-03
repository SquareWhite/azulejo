import AppKit

/// One row in the command palette (⌘⇧P) or the picker (⌘P).
struct PaletteItem {
    /// Stable key for "recent first" ordering.
    var id: String
    var title: String
    /// Grey text after the title: the category, or where a tile lives.
    var detail: String = ""
    var chords: [String] = []
    var enabled = true
    /// Right-hand note for disabled rows ("planned", "needs a tile").
    var note: String?
    /// Needs input first: the placeholder for the inline second step.
    var prompt: String?
    var initialInput: String?
    /// Also searched, never highlighted (ids, paths, URLs).
    var searchExtra = ""
    /// A coloured dot before the title (agent attention in the picker).
    var dot: NSColor?
}

/// The palette overlay: a search field over a fuzzy-filtered list, drawn over
/// the tiles (not a separate window). Keys reach it through `handle(chord:)`
/// from the key router; plain typing goes to the field.
@MainActor
final class PaletteView: NSView, NSTextFieldDelegate {
    enum Mode: String { case commands, goto }

    static let maxRows = 10
    static let rowHeight: CGFloat = 28
    static let fieldHeight: CGFloat = 44

    private(set) var mode: Mode?
    var isOpen: Bool { mode != nil }
    /// Runs the chosen item, with the prompt's input for two-step items.
    var onRun: ((PaletteItem, String?) -> Void)?
    var onClose: (() -> Void)?

    let field = NSTextField()
    private let promptLabel = NSTextField(labelWithString: "")
    private let list = PaletteListView()
    private let backdrop = NSVisualEffectView()
    private var items: [PaletteItem] = []
    private var recents: [String] = []
    private var promptItem: PaletteItem?
    private var clickMonitor: Any?

    private(set) var results: [(item: PaletteItem, match: PaletteFuzzy.Match?)] = []
    private(set) var selected = 0

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        isHidden = true
        layer?.shadowColor = NSColor.black.cgColor
        layer?.shadowOpacity = 0.35
        layer?.shadowRadius = 18
        layer?.shadowOffset = CGSize(width: 0, height: -6)

        backdrop.material = .popover
        backdrop.blendingMode = .withinWindow
        backdrop.state = .active
        backdrop.wantsLayer = true
        backdrop.layer?.cornerRadius = 10
        backdrop.layer?.masksToBounds = true
        backdrop.layer?.borderWidth = 1
        backdrop.layer?.borderColor = NSColor.separatorColor.cgColor
        addSubview(backdrop)

        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: 15)
        field.delegate = self
        field.cell?.isScrollable = true
        field.cell?.wraps = false
        addSubview(field)

        promptLabel.font = .systemFont(ofSize: 11, weight: .semibold)
        promptLabel.textColor = .secondaryLabelColor
        addSubview(promptLabel)

        list.onClick = { [weak self] row in
            guard let self else { return }
            self.selected = row
            self.runSelected()
        }
        addSubview(list)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    // MARK: Open / close

    func open(_ mode: Mode, items: [PaletteItem], recents: [String]) {
        self.mode = mode
        self.items = items
        self.recents = recents
        promptItem = nil
        field.stringValue = ""
        field.placeholderString = mode == .commands
            ? "Type a command…" : "Go to a tile, tab or workspace…"
        isHidden = false
        refilter()
        window?.makeFirstResponder(field)
        installClickMonitor()
    }

    func close() {
        guard isOpen else { return }
        mode = nil
        promptItem = nil
        isHidden = true
        if let clickMonitor { NSEvent.removeMonitor(clickMonitor) }
        clickMonitor = nil
        onClose?()
    }

    /// Clicking anywhere outside the palette closes it.
    private func installClickMonitor() {
        guard clickMonitor == nil else { return }
        clickMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) {
            [weak self] event in
            guard let self, self.isOpen, event.window === self.window else { return event }
            let point = self.convert(event.locationInWindow, from: nil)
            if !self.bounds.contains(point) { self.close() }
            return event
        }
    }

    // MARK: Keys

    /// Keys while open. Returns true when consumed; anything else goes on to
    /// the search field.
    func handle(chord: String) -> Bool {
        switch chord {
        case "escape":
            close()
        case "enter":
            runSelected()
        case "down", "ctrl+n", "ctrl+j":
            move(1)
        case "up", "ctrl+p", "ctrl+k":
            move(-1)
        case "pagedown":
            move(Self.maxRows)
        case "pageup":
            move(-Self.maxRows)
        case "tab":
            // Complete the field with the selected title.
            if promptItem == nil, results.indices.contains(selected) {
                field.stringValue = results[selected].item.title
                refilter()
                field.currentEditor()?.moveToEndOfDocument(nil)
            }
        default:
            return false
        }
        return true
    }

    private func move(_ delta: Int) {
        guard promptItem == nil, !results.isEmpty else { return }
        selected = max(0, min(results.count - 1, selected + delta))
        list.selected = selected
    }

    // MARK: Filtering

    /// Sets the query as if typed (automation).
    func setQuery(_ text: String) {
        field.stringValue = text
        refilter()
    }

    func controlTextDidChange(_ obj: Notification) {
        if promptItem == nil { refilter() }
    }

    private func refilter() {
        let query = field.stringValue.trimmingCharacters(in: .whitespaces)
        let rank = Dictionary(recents.enumerated().map { ($1, $0) }, uniquingKeysWith: { a, _ in a })
        var scored: [(item: PaletteItem, match: PaletteFuzzy.Match?, score: Int, order: Int)] = []
        for (order, item) in items.enumerated() {
            let titleMatch = PaletteFuzzy.match(query, in: item.title)
            var match = titleMatch
            var score: Int
            if let m = titleMatch {
                score = m.score
            } else if let m = PaletteFuzzy.match(query, in: item.title + " " + item.detail + " " + item.searchExtra) {
                // Matched outside the title: half weight, no highlight.
                score = m.score / 2
                match = PaletteFuzzy.Match(score: score, indices: [])
            } else {
                continue
            }
            if let r = rank[item.id] { score += max(0, 30 - r * 3) }
            if !item.enabled { score -= 8 }
            scored.append((item, match, score, order))
        }
        if query.isEmpty {
            // Recent first, then the catalogue's own order.
            scored.sort {
                let ra = rank[$0.item.id] ?? Int.max, rb = rank[$1.item.id] ?? Int.max
                return ra != rb ? ra < rb : $0.order < $1.order
            }
        } else {
            scored.sort { $0.score != $1.score ? $0.score > $1.score : $0.order < $1.order }
        }
        results = scored.map { ($0.item, $0.match) }
        selected = results.firstIndex { $0.item.enabled } ?? 0
        list.rows = results
        list.selected = selected
        list.isHidden = false
        promptLabel.isHidden = true
        superview?.needsLayout = true
    }

    // MARK: Running

    private func runSelected() {
        if let item = promptItem {
            let input = field.stringValue
            close()
            onRun?(item, input)
            return
        }
        guard results.indices.contains(selected) else { return }
        let item = results[selected].item
        guard item.enabled else {
            NSSound.beep()
            return
        }
        if let prompt = item.prompt {
            // Second step: ask for the input inline.
            promptItem = item
            promptLabel.stringValue = item.title.uppercased()
            promptLabel.isHidden = false
            list.isHidden = true
            field.stringValue = item.initialInput ?? ""
            field.placeholderString = prompt
            field.currentEditor()?.selectAll(nil)
            superview?.needsLayout = true
            return
        }
        close()
        onRun?(item, nil)
    }

    /// Whether the inline input step is showing, and for which item.
    var promptingFor: String? { promptItem?.id }

    // MARK: Layout

    /// Height for the current state, for the window's layout.
    var preferredHeight: CGFloat {
        if promptItem != nil { return Self.fieldHeight + 18 }
        let rows = min(Self.maxRows, max(results.count, 1))
        return Self.fieldHeight + CGFloat(rows) * Self.rowHeight + 8
    }

    override func layout() {
        super.layout()
        backdrop.frame = bounds
        let top = bounds.maxY
        if promptItem != nil {
            promptLabel.frame = NSRect(x: 16, y: top - 20, width: bounds.width - 32, height: 14)
            field.frame = NSRect(x: 14, y: top - Self.fieldHeight - 10, width: bounds.width - 28, height: 24)
        } else {
            field.frame = NSRect(x: 14, y: top - Self.fieldHeight + 10, width: bounds.width - 28, height: 24)
        }
        list.frame = NSRect(x: 6, y: 4, width: bounds.width - 12, height: max(0, top - Self.fieldHeight - 4))
    }
}

/// The result rows, drawn by hand so highlighted characters and chords line up.
@MainActor
final class PaletteListView: NSView {
    var rows: [(item: PaletteItem, match: PaletteFuzzy.Match?)] = [] { didSet { offset = 0; needsDisplay = true } }
    var selected = 0 {
        didSet {
            if selected < offset { offset = selected }
            if selected >= offset + PaletteView.maxRows { offset = selected - PaletteView.maxRows + 1 }
            needsDisplay = true
        }
    }
    var onClick: ((Int) -> Void)?
    private var offset = 0

    override var isFlipped: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        let h = PaletteView.rowHeight
        if rows.isEmpty {
            let text = NSAttributedString(string: "No matches", attributes: [
                .font: NSFont.systemFont(ofSize: 13), .foregroundColor: NSColor.tertiaryLabelColor,
            ])
            text.draw(at: NSPoint(x: 12, y: (h - text.size().height) / 2))
            return
        }
        for (n, row) in rows.dropFirst(offset).prefix(PaletteView.maxRows).enumerated() {
            let index = n + offset
            let rect = NSRect(x: 0, y: CGFloat(n) * h, width: bounds.width, height: h)
            if index == selected {
                NSColor.selectedContentBackgroundColor.withAlphaComponent(0.35).setFill()
                NSBezierPath(roundedRect: rect.insetBy(dx: 2, dy: 1), xRadius: 6, yRadius: 6).fill()
            }
            draw(row.item, match: row.match, in: rect)
        }
    }

    private func draw(_ item: PaletteItem, match: PaletteFuzzy.Match?, in rect: NSRect) {
        var x: CGFloat = 12
        if let dot = item.dot {
            dot.setFill()
            NSBezierPath(ovalIn: NSRect(x: x, y: rect.midY - 4, width: 8, height: 8)).fill()
            x += 14
        }
        // Right side first, so the title knows how much room it has.
        let right = NSMutableAttributedString()
        if let note = item.note {
            right.append(NSAttributedString(string: note, attributes: [
                .font: NSFont.systemFont(ofSize: 11), .foregroundColor: NSColor.tertiaryLabelColor,
            ]))
        }
        if !item.chords.isEmpty {
            if right.length > 0 { right.append(NSAttributedString(string: "   ")) }
            right.append(NSAttributedString(
                string: item.chords.prefix(2).map(PaletteFuzzy.prettyChord).joined(separator: "  "),
                attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .regular),
                    .foregroundColor: NSColor.secondaryLabelColor,
                ]))
        }
        let rightSize = right.size()
        right.draw(at: NSPoint(x: rect.maxX - rightSize.width - 12, y: rect.midY - rightSize.height / 2))

        let titleColor: NSColor = item.enabled ? .labelColor : .tertiaryLabelColor
        let text = NSMutableAttributedString(string: item.title, attributes: [
            .font: NSFont.systemFont(ofSize: 13), .foregroundColor: titleColor,
        ])
        let highlighted = Set(match?.indices ?? [])
        if !highlighted.isEmpty {
            for i in highlighted where i < item.title.count {
                let start = item.title.index(item.title.startIndex, offsetBy: i)
                let range = NSRange(start...start, in: item.title)
                text.addAttributes([
                    .font: NSFont.systemFont(ofSize: 13, weight: .bold),
                    .foregroundColor: item.enabled ? NSColor.controlAccentColor : titleColor,
                ], range: range)
            }
        }
        if !item.detail.isEmpty {
            text.append(NSAttributedString(string: "  " + item.detail, attributes: [
                .font: NSFont.systemFont(ofSize: 11), .foregroundColor: NSColor.secondaryLabelColor,
            ]))
        }
        let maxWidth = max(40, rect.maxX - rightSize.width - 24 - x)
        let size = text.size()
        text.draw(with: NSRect(x: x, y: rect.midY - size.height / 2, width: maxWidth, height: size.height),
                  options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
    }

    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        let row = Int(point.y / PaletteView.rowHeight) + offset
        if rows.indices.contains(row) { onClick?(row) }
    }

    override func scrollWheel(with event: NSEvent) {
        let delta = event.scrollingDeltaY > 0 ? -1 : (event.scrollingDeltaY < 0 ? 1 : 0)
        let maxOffset = max(0, rows.count - PaletteView.maxRows)
        offset = max(0, min(maxOffset, offset + delta))
        needsDisplay = true
    }
}
