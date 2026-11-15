import AppKit

/// One row in the ⌘K quick-open menu: a command or a place to jump to.
struct PaletteItem {
    enum Kind: Equatable {
        case command
        case workspace
        case tab
        case tile(TileKind)
        /// A row of the "Set theme…" list.
        case theme

        var isCommand: Bool { self == .command || self == .theme }

        /// Short tag drawn on the row.
        var label: String {
            switch self {
            case .command: return "Command"
            case .workspace: return "Workspace"
            case .tab: return "Tab"
            case .theme: return "Theme"
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
            case .theme: return "paintpalette"
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
/// targets. Its keys are the keymap's `palette` context: the key router
/// resolves them and calls `perform(keyAction:)`; plain typing goes to the
/// field.
///
/// It grows in from its top centre (a little smaller and transparent) and
/// shrinks back out, on its layer only (`Motion.paletteOpenDuration`,
/// `paletteCloseDuration`); motion off or Reduce Motion, it just shows and
/// hides. Open and closed are immediate either way: the field takes keys
/// from the first frame, and a closing menu no longer counts as open (the
/// router, clicks and the run that closed it go on at once) while it fades.
/// An open or close mid-motion starts from where the menu shows.
@MainActor
final class PaletteView: NSView, NSTextFieldDelegate, ThemeAware {
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
    /// A list opened with `openList`: the selected row changed (the theme
    /// picker previews it).
    var onSelect: ((PaletteItem) -> Void)?
    /// Closed without running a row (Esc, a click outside): before `onClose`.
    var onCancel: (() -> Void)?
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
        layer?.shadowOpacity = Float(Design.Alpha.paletteShadow)
        layer?.shadowRadius = 18
        layer?.shadowOffset = CGSize(width: 0, height: -6)
        // The shadow from a path (set in `layout`), not the content's alpha:
        // nothing to re-render offscreen while the menu scales and fades.

        backdrop.material = .popover
        backdrop.blendingMode = .withinWindow
        backdrop.state = .active
        backdrop.wantsLayer = true
        backdrop.layer?.cornerRadius = Design.Radius.palette
        backdrop.layer?.masksToBounds = true
        backdrop.layer?.borderWidth = Design.Border.hairline
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
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func applyTheme() {
        layer?.shadowColor = Design.Color.shadow.cgColor
        backdrop.layer?.borderColor = Design.Color.border.cgColor
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    // MARK: Open / close

    static let placeholder = "Commands, workspaces, tabs and tiles    > commands   @ go to"

    /// Opens with `query` in the field (`">"` or `"@"` for a scoped open).
    func open(items: [PaletteItem], recents: [String], query: String = "") {
        onSelect = nil
        onCancel = nil
        listID = nil
        isOpen = true
        self.items = items
        self.recents = recents
        promptItem = nil
        field.stringValue = query
        field.placeholderString = Self.placeholder
        let look = currentLook
        isHidden = false
        onShownChange?(true)
        refilter()
        window?.makeFirstResponder(field)
        field.currentEditor()?.moveToEndOfDocument(nil)
        installClickMonitor()
        startMotion(.opening, from: look)
    }

    /// A list of its own (the theme picker): `items` in their order, the
    /// field filtering them, `selectedID` selected; `placeholder` says what
    /// it's for. Set `onSelect` and `onCancel` after this.
    func openList(id: String, items: [PaletteItem], selectedID: String?, placeholder: String) {
        open(items: items, recents: [], query: "")
        listID = id
        field.placeholderString = placeholder
        if let selectedID, let index = results.firstIndex(where: { $0.item.id == selectedID }) {
            selected = index
            list.selected = index
        }
    }

    /// The `openList` id while one is showing.
    private(set) var listID: String?

    /// Esc and clicks outside: a cancel.
    func close() { close(cancelled: true) }

    private func close(cancelled: Bool) {
        guard isOpen else { return }
        if cancelled { onCancel?() }
        onCancel = nil
        onSelect = nil
        listID = nil
        isOpen = false
        promptItem = nil
        // Still on screen while it shrinks: let go of the keyboard now, so
        // the next key goes to the tile `onClose` focuses, not this field.
        if let editor = field.currentEditor(), window?.firstResponder === editor {
            window?.makeFirstResponder(nil)
        }
        startMotion(.closing, from: currentLook)
        onShownChange?(false)
        if let clickMonitor { NSEvent.removeMonitor(clickMonitor) }
        clickMonitor = nil
        onClose?()
    }

    /// Closing, it's only a picture: clicks go to what's under it.
    override func hitTest(_ point: NSPoint) -> NSView? {
        isOpen ? super.hitTest(point) : nil
    }

    // MARK: Motion

    enum MotionPhase: String { case none, opening, closing }

    /// What the menu's layer is doing now.
    private(set) var motion: MotionPhase = .none
    /// Automation: how long the last finished open and close took, in ms.
    private(set) var lastMotionMs: [MotionPhase: Int] = [:]
    /// Each open and close bumps it; a motion that ends only tidies up if
    /// it's still the latest.
    private var motionGeneration = 0
    /// The running motion: when it started (media time), how long it runs,
    /// and the scale and opacity it started from.
    private var motionStart: CFTimeInterval = 0
    private var motionDuration: CFTimeInterval = 0
    private var motionFrom: Look = (1, 1)
    /// The bounds the running motion's transform was built for.
    private var motionBounds: CGSize = .zero
    private static let motionKey = "azj.palette"
    /// How small (and transparent) it starts opening and ends closing.
    static let restScale: CGFloat = 0.96

    /// Grows in or shrinks out, from `look` (where it shows now; mid-motion,
    /// part way). Motion off, or nothing to travel: shows or hides at once.
    private func startMotion(_ phase: MotionPhase, from look: Look) {
        motionGeneration += 1
        layer?.removeAnimation(forKey: Self.motionKey)
        let opening = phase == .opening
        // How much of the way is left, by opacity.
        let left = Double(opening ? 1 - look.opacity : look.opacity)
        guard Motion.active, window != nil, layer != nil, left > 0.01 else {
            motion = .none
            if !opening { isHidden = true }
            return
        }
        motion = phase
        motionStart = CACurrentMediaTime()
        motionFrom = look
        motionDuration = max(0.03, left * (opening ? Motion.paletteOpenDuration : Motion.paletteCloseDuration))
        applyMotion()
        // Insurance: a layer that's never drawn may not report its
        // animation's end; the motion ends on time regardless.
        let generation = motionGeneration
        DispatchQueue.main.asyncAfter(deadline: .now() + motionDuration + 0.25) { [weak self] in
            guard let self, generation == self.motionGeneration, self.motion != .none else { return }
            self.endMotion()
        }
    }

    typealias Look = (scale: CGFloat, opacity: Float)
    private static let curve = CAMediaTimingFunction(controlPoints: 0.2, 0.9, 0.3, 1.0)

    /// Scale and opacity where the menu shows now, from the motion's own
    /// clock (a motion added this turn hasn't reached the presentation yet).
    private var currentLook: Look {
        if isHidden { return (Self.restScale, 0) }
        guard motion != .none, motionDuration > 0 else { return (1, 1) }
        let p = Motion.progress((CACurrentMediaTime() - motionStart) / motionDuration, curve: Self.curve)
        let to: Look = motion == .opening ? (1, 1) : (Self.restScale, 0)
        return (motionFrom.scale + (to.scale - motionFrom.scale) * CGFloat(p),
                motionFrom.opacity + (to.opacity - motionFrom.opacity) * Float(p))
    }

    /// (Re)adds the running motion's animation for the current bounds, as
    /// far along as the clock says: the pivot follows the top centre when
    /// the list changes height mid-open.
    private func applyMotion() {
        guard let layer, motion != .none else { return }
        motionBounds = bounds.size
        let opening = motion == .opening
        let scale = CABasicAnimation(keyPath: "transform")
        scale.fromValue = NSValue(caTransform3D: transform(scale: motionFrom.scale))
        scale.toValue = NSValue(caTransform3D: opening ? CATransform3DIdentity : transform(scale: Self.restScale))
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = motionFrom.opacity
        fade.toValue = opening ? 1 : 0
        for part in [scale, fade] {
            part.duration = motionDuration
            part.timingFunction = Self.curve
        }
        let group = CAAnimationGroup()
        group.animations = [scale, fade]
        group.duration = motionDuration
        let rate = Motion.refreshRate
        group.preferredFrameRateRange = CAFrameRateRange(minimum: rate / 2, maximum: rate, preferred: rate)
        group.beginTime = layer.convertTime(motionStart, from: nil)
        if !opening {
            // Held at nothing until the end hides it.
            group.fillMode = .forwards
            group.isRemovedOnCompletion = false
        }
        let generation = motionGeneration
        group.delegate = MotionEnd { [weak self] finished in
            guard let self, finished, generation == self.motionGeneration else { return }
            self.endMotion()
        }
        layer.add(group, forKey: Self.motionKey)
    }

    private func endMotion() {
        lastMotionMs[motion] = Int(((CACurrentMediaTime() - motionStart) * 1000).rounded())
        if motion == .closing { isHidden = true }
        motion = .none
        layer?.removeAnimation(forKey: Self.motionKey)
    }

    /// Scales by `scale` about the top centre (the layer's transform turns
    /// about its anchor point, in its superlayer's orientation: y up unless
    /// the superview is flipped).
    private func transform(scale: CGFloat) -> CATransform3D {
        guard let layer else { return CATransform3DIdentity }
        let a = layer.anchorPoint
        let size = layer.bounds.size
        let top: CGFloat = superview?.isFlipped == true ? 0 : 1
        let pivot = CGPoint(x: size.width * (0.5 - a.x), y: size.height * (top - a.y))
        var t = CATransform3DMakeScale(scale, scale, 1)
        t.m41 = pivot.x * (1 - scale)
        t.m42 = pivot.y * (1 - scale)
        return t
    }

    /// Automation: scale and opacity as Core Animation shows them now
    /// ("0.98@0.50"; hidden: "-").
    var describeLook: String {
        guard !isHidden, let layer else { return "-" }
        let p = layer.presentation() ?? layer
        return String(format: "%.2f@%.2f", p.transform.m11, p.opacity)
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

    /// Its keymap actions while open (`[keys.<platform>.palette]`; the key
    /// router resolves the chords). Unknown names do nothing.
    func perform(keyAction name: String) {
        switch name {
        case "palette_close":
            close()
        case "palette_run":
            runSelected()
        case "palette_next":
            move(1)
        case "palette_previous":
            move(-1)
        case "palette_page_down":
            move(Self.maxRows)
        case "palette_page_up":
            move(-Self.maxRows)
        case "palette_complete":
            // Complete the field with the selected title (keeping `>` / `@`).
            if promptItem == nil, results.indices.contains(selected) {
                field.stringValue = scope.prefix + results[selected].item.title
                refilter()
                field.currentEditor()?.moveToEndOfDocument(nil)
            }
        default:
            break
        }
    }

    private func move(_ delta: Int) {
        guard promptItem == nil, !results.isEmpty else { return }
        selected = max(0, min(results.count - 1, selected + delta))
        list.selected = selected
        selectionChanged()
    }

    private func selectionChanged() {
        guard let onSelect, results.indices.contains(selected) else { return }
        onSelect(results[selected].item)
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
        selectionChanged()
    }

    // MARK: Running

    private func runSelected() {
        if let item = promptItem {
            let input = field.stringValue
            close(cancelled: false)
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
        close(cancelled: false)
        onRun?(item, nil)
    }

    /// Automation: selects the row with `id`, as the arrow keys would.
    func select(id: String) -> Bool {
        guard let index = results.firstIndex(where: { $0.item.id == id }) else { return false }
        selected = index
        list.selected = index
        selectionChanged()
        return true
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
        layer?.shadowPath = CGPath(roundedRect: bounds, cornerWidth: Design.Radius.palette,
                                   cornerHeight: Design.Radius.palette, transform: nil)
        // Opening and the list changed height: the same motion, pivoting
        // round the new top centre.
        if motion != .none, bounds.size != motionBounds { applyMotion() }
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
        if let status = item.status, let image = SidebarView.icon(status, pointSize: Design.Symbol.paletteStatus) {
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
        // By theme too: the image holds its tint.
        let key = Theme.current.id + "/" + kind.symbol
        if let cached = icons[key] { return cached }
        let config = NSImage.SymbolConfiguration(pointSize: Design.Symbol.paletteKind, weight: .regular)
            .applying(.init(paletteColors: [Theme.resolved(Design.Color.textSecondary)]))
        guard let image = NSImage(systemSymbolName: kind.symbol, accessibilityDescription: kind.label)?
            .withSymbolConfiguration(config) else { return nil }
        icons[key] = image
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

/// A `CAAnimation` delegate that calls back once it stops (`finished`: it
/// ran to its end, not removed or replaced). Core Animation calls it on the
/// main thread.
private final class MotionEnd: NSObject, CAAnimationDelegate, @unchecked Sendable {
    private let done: @MainActor (Bool) -> Void
    init(_ done: @escaping @MainActor (Bool) -> Void) { self.done = done }
    func animationDidStop(_ anim: CAAnimation, finished flag: Bool) {
        MainActor.assumeIsolated { done(flag) }
    }
}
