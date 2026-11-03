import AppKit

/// One row in the ⌘K quick-open menu: a command or a place to jump to.
struct PaletteItem {
    enum Kind: Equatable {
        case command
        case workspace
        case tab
        case tile(TileKind)

        var isCommand: Bool { self == .command }

        /// Short tag drawn on the row.
        var label: String {
            switch self {
            case .command: return "Command"
            case .workspace: return "Workspace"
            case .tab: return "Tab"
            case .tile(let kind):
                switch kind {
                case .terminal: return "Terminal"
                case .browser: return "Browser"
                case .preview: return "Preview"
                case .files: return "Files"
                }
            }
        }

        /// SF Symbol for the row's icon.
        var symbol: String {
            switch self {
            case .command: return Design.Icon.command
            case .workspace: return Design.Icon.workspace
            case .tab: return Design.Icon.tab
            case .tile(let kind): return Design.Icon.for(kind)
            }
        }
    }

    /// Stable key for "recent first" ordering.
    var id: String
    var title: String
    var kind: Kind = .command
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
    /// A coloured dot before the title (agent attention on jump targets).
    var dot: NSColor?
    /// The agents' state icon before the title, drawn instead of the dot
    /// (the same icons as the sidebar and tab strip).
    var status: Design.Status?
}

/// The ⌘K quick-open menu: a search field over one fuzzy-filtered list of
/// commands and jump targets (workspaces, tabs, tiles), drawn over the tiles
/// (not a separate window). A leading `>` limits it to commands, `@` to jump
/// targets. Keys reach it through `handle(chord:)` from the key router; plain
/// typing goes to the field.
@MainActor
final class PaletteView: NSView, NSTextFieldDelegate {
    /// What the current query lists, from its leading character.
    enum Scope: String {
        case all, commands, jump

        static func of(_ query: String) -> (scope: Scope, rest: String) {
            if query.hasPrefix(">") { return (.commands, String(query.dropFirst())) }
            if query.hasPrefix("@") { return (.jump, String(query.dropFirst())) }
            return (.all, query)
        }

        var prefix: String {
            switch self {
            case .all: return ""
            case .commands: return ">"
            case .jump: return "@"
            }
        }

        func includes(_ kind: PaletteItem.Kind) -> Bool {
            switch self {
            case .all: return true
            case .commands: return kind.isCommand
            case .jump: return !kind.isCommand
            }
        }
    }

    static let maxRows = 10
    static let rowHeight: CGFloat = 28
    static let fieldHeight: CGFloat = 44

    private(set) var isOpen = false
    /// The scope the current query asks for.
    var scope: Scope { Scope.of(field.stringValue).scope }
    /// Runs the chosen item, with the prompt's input for two-step items.
    var onRun: ((PaletteItem, String?) -> Void)?
    var onClose: (() -> Void)?
    /// Opened or closed (the toolbar search field highlights while open).
    var onShownChange: ((Bool) -> Void)?

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
        layer?.shadowColor = Design.Color.shadow.cgColor
        layer?.shadowOpacity = 0.35
        layer?.shadowRadius = 18
        layer?.shadowOffset = CGSize(width: 0, height: -6)

        backdrop.material = .popover
        backdrop.blendingMode = .withinWindow
        backdrop.state = .active
        backdrop.wantsLayer = true
        backdrop.layer?.cornerRadius = Design.Radius.palette
        backdrop.layer?.masksToBounds = true
        backdrop.layer?.borderWidth = Design.Border.hairline
        backdrop.layer?.borderColor = Design.Color.border.cgColor
        addSubview(backdrop)

        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = Design.Font.paletteField
        field.delegate = self
        field.cell?.isScrollable = true
        field.cell?.wraps = false
        addSubview(field)

        promptLabel.font = Design.Font.palettePrompt
        promptLabel.textColor = Design.Color.textSecondary
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

    static let placeholder = "Commands, workspaces, tabs and tiles    > commands   @ go to"

    /// Opens with `query` in the field (`">"` or `"@"` for a scoped open).
    func open(items: [PaletteItem], recents: [String], query: String = "") {
        isOpen = true
        self.items = items
        self.recents = recents
        promptItem = nil
        field.stringValue = query
        field.placeholderString = Self.placeholder
        isHidden = false
        onShownChange?(true)
        refilter()
        window?.makeFirstResponder(field)
        field.currentEditor()?.moveToEndOfDocument(nil)
        installClickMonitor()
    }

    func close() {
        guard isOpen else { return }
        isOpen = false
        promptItem = nil
        isHidden = true
        onShownChange?(false)
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
            // Complete the field with the selected title (keeping `>` / `@`).
            if promptItem == nil, results.indices.contains(selected) {
                field.stringValue = scope.prefix + results[selected].item.title
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
        let (scope, rest) = Scope.of(field.stringValue)
        let query = rest.trimmingCharacters(in: .whitespaces)
        let lowered = query.lowercased()
        let rank = Dictionary(recents.enumerated().map { ($1, $0) }, uniquingKeysWith: { a, _ in a })
        var scored: [(item: PaletteItem, match: PaletteFuzzy.Match?, score: Int, order: Int)] = []
        for (order, item) in items.enumerated() where scope.includes(item.kind) {
            let titleMatch = PaletteFuzzy.match(query, in: item.title)
            var match = titleMatch
            var score: Int
            if let m = titleMatch {
                score = m.score
                // Names typed out in full or from the start beat scattered hits.
                let title = item.title.lowercased()
                if !lowered.isEmpty {
                    if title == lowered { score += 60 } else if title.hasPrefix(lowered) { score += 30 }
                }
            } else if let m = PaletteFuzzy.match(query, in: item.title + " " + item.detail + " " + item.searchExtra) {
                // Matched outside the title: half weight, no highlight.
                score = m.score / 2
                match = PaletteFuzzy.Match(score: score, indices: [])
            } else {
                continue
            }
            // A name-like query more often means a place than a command.
            if !item.kind.isCommand { score += 4 }
            if let r = rank[item.id] { score += max(0, 30 - r * 3) }
            if !item.enabled { score -= 8 }
            scored.append((item, match, score, order))
        }
        if query.isEmpty {
            // Recent first (commands and places mixed), then places, then
            // commands, each in their own order.
            scored.sort {
                let ra = rank[$0.item.id] ?? Int.max, rb = rank[$1.item.id] ?? Int.max
                if ra != rb { return ra < rb }
                let ca = $0.item.kind.isCommand, cb = $1.item.kind.isCommand
                if ca != cb { return !ca }
                return $0.order < $1.order
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
                .font: Design.Font.paletteTitle, .foregroundColor: Design.Color.textMuted,
            ])
            text.draw(at: NSPoint(x: 12, y: (h - text.size().height) / 2))
            return
        }
        for (n, row) in rows.dropFirst(offset).prefix(PaletteView.maxRows).enumerated() {
            let index = n + offset
            let rect = NSRect(x: 0, y: CGFloat(n) * h, width: bounds.width, height: h)
            if index == selected {
                Design.Color.selected.setFill()
                NSBezierPath(roundedRect: rect.insetBy(dx: 2, dy: 1),
                             xRadius: Design.Radius.pill, yRadius: Design.Radius.pill).fill()
            }
            draw(row.item, match: row.match, in: rect)
        }
    }

    private func draw(_ item: PaletteItem, match: PaletteFuzzy.Match?, in rect: NSRect) {
        var x: CGFloat = 12
        if let image = Self.icon(item.kind) {
            let size: CGFloat = 14
            image.draw(in: NSRect(x: x, y: rect.midY - size / 2, width: size, height: size),
                       from: .zero, operation: .sourceOver, fraction: item.enabled ? 0.8 : 0.35,
                       respectFlipped: true, hints: nil)
            x += size + 8
        }
        if let status = item.status, let image = SidebarView.icon(status, pointSize: 11) {
            let size = image.size
            image.draw(in: NSRect(x: x, y: rect.midY - size.height / 2, width: size.width, height: size.height),
                       from: .zero, operation: .sourceOver, fraction: item.enabled ? 1 : 0.35,
                       respectFlipped: true, hints: nil)
            x += size.width + 6
        } else if let dot = item.dot {
            dot.setFill()
            NSBezierPath(ovalIn: NSRect(x: x, y: rect.midY - 4, width: 8, height: 8)).fill()
            x += 14
        }
        // Right side first, so the title knows how much room it has.
        let right = NSMutableAttributedString()
        // Kind tag first, so commands and places read apart at a glance.
        right.append(NSAttributedString(string: item.kind.label, attributes: [
            .font: Design.Font.paletteSection,
            .foregroundColor: Design.Color.textMuted,
        ]))
        if let note = item.note {
            right.append(NSAttributedString(string: "   "))
            right.append(NSAttributedString(string: note, attributes: [
                .font: Design.Font.detail, .foregroundColor: Design.Color.textMuted,
            ]))
        }
        if !item.chords.isEmpty {
            right.append(NSAttributedString(string: "   "))
            right.append(NSAttributedString(
                string: item.chords.prefix(2).map(PaletteFuzzy.prettyChord).joined(separator: "  "),
                attributes: [
                    .font: Design.Font.chord,
                    .foregroundColor: Design.Color.textSecondary,
                ]))
        }
        let rightSize = right.size()
        right.draw(at: NSPoint(x: rect.maxX - rightSize.width - 12, y: rect.midY - rightSize.height / 2))

        let titleColor = item.enabled ? Design.Color.textPrimary : Design.Color.textMuted
        let text = NSMutableAttributedString(string: item.title, attributes: [
            .font: Design.Font.paletteTitle, .foregroundColor: titleColor,
        ])
        let highlighted = Set(match?.indices ?? [])
        if !highlighted.isEmpty {
            for i in highlighted where i < item.title.count {
                let start = item.title.index(item.title.startIndex, offsetBy: i)
                let range = NSRange(start...start, in: item.title)
                text.addAttributes([
                    .font: Design.Font.paletteMatch,
                    .foregroundColor: item.enabled ? Design.Color.accent : titleColor,
                ], range: range)
            }
        }
        if !item.detail.isEmpty {
            text.append(NSAttributedString(string: "  " + item.detail, attributes: [
                .font: Design.Font.detail, .foregroundColor: Design.Color.textSecondary,
            ]))
        }
        let maxWidth = max(40, rect.maxX - rightSize.width - 24 - x)
        let size = text.size()
        text.draw(with: NSRect(x: x, y: rect.midY - size.height / 2, width: maxWidth, height: size.height),
                  options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
    }

    private static var icons: [String: NSImage] = [:]

    static func icon(_ kind: PaletteItem.Kind) -> NSImage? {
        if let cached = icons[kind.symbol] { return cached }
        let config = NSImage.SymbolConfiguration(pointSize: 12, weight: .regular)
            .applying(.init(paletteColors: [Design.Color.textSecondary]))
        guard let image = NSImage(systemSymbolName: kind.symbol, accessibilityDescription: kind.label)?
            .withSymbolConfiguration(config) else { return nil }
        icons[kind.symbol] = image
        return image
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
