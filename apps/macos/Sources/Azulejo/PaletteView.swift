import AppKit

/// One row in the ⌘K quick-open menu: a command or a place to jump to.
///
/// A row reads left to right: an icon (the agent's state, else the kind's
/// symbol), the crumbs (`window 2 › azulejo › `) and the leaf (`title`), the
/// detail (a task, a command, a branch, a folder), and on the right the
/// agent's wait, a note (why a command is off) or the keys that reach it.
struct PaletteItem {
    enum Kind: Equatable {
        case command
        case workspace
        case tab
        case tile(TileKind)
        /// A row of the "Set theme…" list.
        case theme

        var isCommand: Bool { self == .command || self == .theme }

        /// The kind's name (automation: `palette.topkind`, `palette.kinds`).
        var label: String {
            switch self {
            case .command: return "Command"
            case .workspace: return "Workspace"
            case .tab: return "Tab"
            case .theme: return "Theme"
            case .tile(let kind):
                switch kind {
                case .terminal: return "Terminal"
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

    /// Where an agent row goes on an empty query: "Needs you" (blocked,
    /// then finished and not read yet) or "Working".
    enum Bucket: Int, Comparable {
        case blocked, attention, working
        static func < (a: Bucket, b: Bucket) -> Bool { a.rawValue < b.rawValue }
    }

    /// Stable key for "recent first" ordering.
    var id: String
    /// The leaf: the row's name.
    var title: String
    var kind: Kind = .command
    /// Grey text after the title: a task, a command, a branch, a folder.
    var detail: String = ""
    var chords: [String] = []
    var enabled = true
    /// Right-hand note ("planned", "needs a tile", "current").
    var note: String?
    /// Needs input first: the placeholder for the inline second step.
    var prompt: String?
    var initialInput: String?
    /// Also searched, never highlighted (ids, paths, URLs).
    var searchExtra = ""
    /// Where the leaf lives, outermost first (`azulejo`, `review`).
    var crumbs: [String] = []
    /// A remote machine's label, drawn as a chip before the crumbs.
    var machine: String?
    /// The detail is a command, branch or path: monospaced.
    var detailMono = false
    /// A command's category (`commands.list`): its section in `>`, its
    /// detail in a flat list.
    var category: String?
    /// The agents' state icon, drawn instead of the kind's (the same icons
    /// as the sidebar and tab strip).
    var status: Design.Status?
    /// Keys that reach it, already pretty (`⌘3` for workspace 3).
    var keyHint: String?
    /// How long an agent has been in its state ("2m", "1h 12m").
    var wait: String?
    var bucket: Bucket?
    /// When the agent's state last changed (ms since the epoch).
    var since: UInt64 = 0

    /// The crumbs as drawn and matched: `"azulejo › review › "`.
    var crumbText: String { crumbs.map { $0 + " › " }.joined() }
}

/// Which characters of a row a query matched, by part (character offsets
/// in `crumbText`, `title` and the detail as drawn).
struct PaletteHits: Equatable {
    var crumbs: Set<Int> = []
    var title: Set<Int> = []
    var detail: Set<Int> = []
}

struct PaletteResult {
    var item: PaletteItem
    var hits = PaletteHits()
}

/// A section header in the list, before row `start`.
struct PaletteSection: Equatable {
    var title: String
    var count: Int
    var start: Int
    /// "Needs you": the label in the primary text colour.
    var strong = false
}

/// The ⌘K quick-open menu: a search field over one fuzzy-filtered list of
/// commands and jump targets (agents, workspaces, tabs, tiles), centred over
/// the window under a scrim (not a separate window). A leading `>` limits it
/// to commands, `@` to jump targets; either shows as a token in the field,
/// and ⌫ on an empty query removes it. An empty query lists what needs you
/// first (blocked agents, then unread finishes), then working agents, then
/// recent rows, then every other place; `>` lists recent commands, then
/// each category. Typing gives one flat ranked list. Its keys are the
/// keymap's `palette` context: the key router resolves them and calls
/// `perform(keyAction:)`; plain typing goes to the field.
///
/// It grows in from its top centre (a little smaller and transparent) and
/// shrinks back out, on its layer only (`Motion.paletteOpenDuration`,
/// `paletteCloseDuration`), the scrim fading with it; motion off or Reduce
/// Motion, it just shows and hides. Open and closed are immediate either
/// way: the field takes keys from the first frame, and a closing menu no
/// longer counts as open (the router, clicks and the run that closed it go
/// on at once) while it fades. An open or close mid-motion starts from where
/// the menu shows.
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

        /// The field's token for a scoped query.
        var token: String? {
            switch self {
            case .all: return nil
            case .commands: return "Commands"
            case .jump: return "Places"
            }
        }

        var placeholder: String {
            switch self {
            case .all: return "Search agents, places and commands"
            case .commands: return "Search commands"
            case .jump: return "Search places"
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

    static let fieldHeight = Design.Space.paletteFieldHeight
    static let footerHeight = Design.Space.paletteFooterHeight
    static let inset = Design.Space.paletteInset
    /// The prompt step's field block: its label over the field.
    static let promptHeight: CGFloat = 66
    /// The no-results block.
    static let emptyHeight: CGFloat = 96
    /// Rows a recent section shows at most on an empty query.
    static let recentCount = 5

    private(set) var isOpen = false
    /// The scope the current query asks for: the field's token.
    private(set) var scope: Scope = .all
    /// The query as typed, scope prefix included (`>zoom`).
    var query: String { scope.prefix + field.stringValue }
    /// Runs the chosen item, with the prompt's input for two-step items.
    var onRun: ((PaletteItem, String?) -> Void)?
    var onClose: (() -> Void)?
    /// A list opened with `openList`: the selected row changed (the theme
    /// picker previews it).
    var onSelect: ((PaletteItem) -> Void)?
    /// Closed without running a row (Esc, a click outside): before `onClose`.
    var onCancel: (() -> Void)?
    /// Opened or closed (the toolbar's ⌘ button stays pressed while open).
    var onShownChange: ((Bool) -> Void)?
    /// `quick_open`'s chord ("⌘K"), shown in the field row on an empty query.
    var chord = ""
    /// The shade over the window, shown and hidden with the menu (not for
    /// `openList`: the theme picker previews the window as it is).
    weak var scrim: NSView?

    let field = NSTextField()
    private let magnifier = NSImageView()
    private let token = PaletteTokenView()
    private let countLabel = NSTextField(labelWithString: "")
    private let promptLabel = NSTextField(labelWithString: "")
    private let list = PaletteListView()
    private let footer = PaletteFooterView()
    private let empty = PaletteEmptyView()
    private let backdrop = NSView()
    private let fieldRule = NSView()
    private var items: [PaletteItem] = []
    private var recents: [String] = []
    private var promptItem: PaletteItem?
    private var clickMonitor: Any?
    private var customPlaceholder: String?

    private(set) var results: [PaletteResult] = []
    private(set) var sections: [PaletteSection] = []
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

        // Solid, not a material: the rows read the same over any tile.
        backdrop.wantsLayer = true
        backdrop.layer?.cornerRadius = Design.Radius.palette
        backdrop.layer?.cornerCurve = .continuous
        backdrop.layer?.masksToBounds = true
        backdrop.layer?.borderWidth = Design.Border.hairline
        addSubview(backdrop)
        fieldRule.wantsLayer = true
        addSubview(fieldRule)

        magnifier.imageScaling = .scaleNone
        addSubview(magnifier)
        addSubview(token)

        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = Design.Font.paletteField
        field.textColor = Design.Color.textPrimary
        field.delegate = self
        field.cell?.isScrollable = true
        field.cell?.wraps = false
        addSubview(field)

        countLabel.font = Design.Font.detail
        countLabel.textColor = Design.Color.textSecondary
        countLabel.alignment = .right
        addSubview(countLabel)

        promptLabel.font = Design.Font.palettePrompt
        promptLabel.textColor = Design.Color.textSecondary
        addSubview(promptLabel)

        list.onClick = { [weak self] row in
            guard let self else { return }
            self.selected = row
            self.runSelected()
        }
        addSubview(list)
        addSubview(empty)
        addSubview(footer)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func applyTheme() {
        layer?.shadowColor = Design.Color.shadow.cgColor
        backdrop.layer?.backgroundColor = Design.Color.palette.cgColor
        backdrop.layer?.borderColor = Design.Color.border.cgColor
        fieldRule.layer?.backgroundColor = Design.Color.border.cgColor
        let config = NSImage.SymbolConfiguration(pointSize: Design.Symbol.paletteSearch, weight: .medium)
            .applying(.init(paletteColors: [Theme.resolved(Design.Color.textSecondary)]))
        magnifier.image = NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: nil)?
            .withSymbolConfiguration(config)
        updatePlaceholder()
        (field.currentEditor() as? NSTextView)?.insertionPointColor = Design.Color.accent
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    // MARK: Open / close

    static let placeholder = Scope.all.placeholder

    /// Opens with `query` in the field (`">"` or `"@"` for a scoped open).
    func open(items: [PaletteItem], recents: [String], query: String = "") {
        show(items: items, recents: recents, query: query, listID: nil, placeholder: nil)
    }

    /// A list of its own (the theme picker): `items` in their order, the
    /// field filtering them, `selectedID` selected; `placeholder` says what
    /// it's for. Set `onSelect` and `onCancel` after this.
    func openList(id: String, items: [PaletteItem], selectedID: String?, placeholder: String) {
        show(items: items, recents: [], query: "", listID: id, placeholder: placeholder)
        if let selectedID, let index = results.firstIndex(where: { $0.item.id == selectedID }) {
            selected = index
            list.selected = index
        }
    }

    private func show(items: [PaletteItem], recents: [String], query: String, listID: String?,
                      placeholder: String?) {
        onSelect = nil
        onCancel = nil
        self.listID = listID
        customPlaceholder = placeholder
        isOpen = true
        self.items = items
        self.recents = recents
        promptItem = nil
        let (scope, rest) = Scope.of(query)
        self.scope = scope
        field.stringValue = rest
        updatePlaceholder()
        let look = currentLook
        isHidden = false
        onShownChange?(true)
        refilter()
        window?.makeFirstResponder(field)
        field.currentEditor()?.moveToEndOfDocument(nil)
        (field.currentEditor() as? NSTextView)?.insertionPointColor = Design.Color.accent
        installClickMonitor()
        startMotion(.opening, from: look)
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

    /// Whether the scrim shows with the menu now.
    private var scrimShown: Bool { isOpen && listID == nil }

    /// Grows in or shrinks out, from `look` (where it shows now; mid-motion,
    /// part way). Motion off, or nothing to travel: shows or hides at once.
    private func startMotion(_ phase: MotionPhase, from look: Look) {
        motionGeneration += 1
        layer?.removeAnimation(forKey: Self.motionKey)
        scrim?.layer?.removeAnimation(forKey: Self.motionKey)
        let opening = phase == .opening
        // How much of the way is left, by opacity.
        let left = Double(opening ? 1 - look.opacity : look.opacity)
        guard Motion.active, window != nil, layer != nil, left > 0.01 else {
            motion = .none
            if !opening { isHidden = true }
            scrim?.isHidden = !(opening && scrimShown)
            return
        }
        motion = phase
        motionStart = CACurrentMediaTime()
        motionFrom = look
        motionDuration = max(0.03, left * (opening ? Motion.paletteOpenDuration : Motion.paletteCloseDuration))
        if opening { scrim?.isHidden = !scrimShown }
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
    /// The default ease-out, whatever `[animation] curve` says (the menu's
    /// motion is fixed). Its gentle finish stays out of sight here: the
    /// scale travels 4 %, so the last 1 % of the way is under a pixel.
    private static var curve: CAMediaTimingFunction { Motion.easeOut }

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
    /// the list changes height mid-open. The scrim fades along.
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
        if let scrimLayer = scrim?.layer, scrim?.isHidden == false {
            let shade = fade.copy() as! CABasicAnimation
            shade.beginTime = scrimLayer.convertTime(motionStart, from: nil)
            shade.preferredFrameRateRange = group.preferredFrameRateRange
            if !opening {
                shade.fillMode = .forwards
                shade.isRemovedOnCompletion = false
            }
            scrimLayer.add(shade, forKey: Self.motionKey)
        }
    }

    private func endMotion() {
        lastMotionMs[motion] = Int(((CACurrentMediaTime() - motionStart) * 1000).rounded())
        if motion == .closing {
            isHidden = true
            scrim?.isHidden = true
        }
        motion = .none
        layer?.removeAnimation(forKey: Self.motionKey)
        scrim?.layer?.removeAnimation(forKey: Self.motionKey)
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

    /// Automation: whether the scrim shows.
    var scrimVisible: Bool { scrim?.isHidden == false }

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
            move(list.pageRows)
        case "palette_page_up":
            move(-list.pageRows)
        case "palette_complete":
            // Complete the field with the selected title (keeping the scope).
            if promptItem == nil, results.indices.contains(selected) {
                field.stringValue = results[selected].item.title
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

    /// Sets the query as if typed (automation), `>` or `@` first for a scope.
    func setQuery(_ text: String) {
        let (scope, rest) = Scope.of(text)
        self.scope = scope
        field.stringValue = rest
        updatePlaceholder()
        refilter()
    }

    func controlTextDidChange(_ obj: Notification) {
        guard promptItem == nil else { return }
        // A leading `>` or `@` becomes the field's token.
        let (scope, rest) = Scope.of(field.stringValue)
        if scope != .all, listID == nil {
            self.scope = scope
            field.stringValue = rest
            updatePlaceholder()
        }
        refilter()
    }

    /// ⌫ on an empty query removes the scope's token.
    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        guard selector == #selector(NSResponder.deleteBackward(_:)), promptItem == nil,
              scope != .all, field.stringValue.isEmpty else { return false }
        scope = .all
        updatePlaceholder()
        refilter()
        return true
    }

    private func updatePlaceholder() {
        let text = promptItem?.prompt ?? customPlaceholder ?? scope.placeholder
        field.placeholderAttributedString = NSAttributedString(string: text, attributes: [
            .font: Design.Font.paletteField, .foregroundColor: Theme.resolved(Design.Color.textSecondary),
        ])
        token.text = promptItem == nil && listID == nil ? scope.token : nil
        needsLayout = true
    }

    private func refilter() {
        let query = field.stringValue.trimmingCharacters(in: .whitespaces)
        let rank = Dictionary(recents.enumerated().map { ($1, $0) }, uniquingKeysWith: { a, _ in a })
        let pool = items.enumerated().filter { scope.includes($0.element.kind) }
        sections = []
        if !query.isEmpty {
            results = Self.ranked(query, pool: pool, rank: rank)
        } else if listID != nil {
            results = pool.map { PaletteResult(item: $0.element) }
        } else {
            (results, sections) = Self.grouped(pool.map(\.element), scope: scope, recents: recents)
        }
        selected = results.firstIndex { $0.item.enabled } ?? 0
        list.set(rows: results, sections: sections)
        list.selected = selected
        list.isHidden = results.isEmpty
        empty.isHidden = !results.isEmpty
        empty.query = query
        empty.scoped = scope != .all
        promptLabel.isHidden = true
        updateCount(query: query)
        updateFooter()
        needsLayout = true
        superview?.needsLayout = true
        selectionChanged()
    }

    private func updateCount(query: String) {
        if promptItem != nil || (listID != nil && query.isEmpty) {
            countLabel.stringValue = ""
        } else if !query.isEmpty {
            let n = results.count
            countLabel.stringValue = "\(n) result\(n == 1 ? "" : "s")"
        } else {
            switch scope {
            case .all: countLabel.stringValue = chord
            case .commands: countLabel.stringValue = "\(results.count) commands"
            case .jump: countLabel.stringValue = "\(results.count) places"
            }
        }
    }

    private func updateFooter() {
        if promptItem != nil {
            footer.keys = [("↵", "done"), ("esc", "cancel")]
        } else if results.isEmpty {
            footer.keys = [("esc", "close")]
        } else if listID != nil {
            footer.keys = [("↑↓", "preview"), ("↵", "keep"), ("esc", "go back")]
        } else if scope == .commands {
            footer.keys = [("↑↓", "move"), ("↵", "run"), ("⌫", "all results"), ("esc", "close")]
        } else {
            footer.keys = [("↑↓", "move"), ("↵", "go to"), (">", "commands"), ("@", "places"), ("esc", "close")]
        }
    }

    /// A typed query: one flat list, best first, kinds mixed. A hit in the
    /// title beats one in the crumbs or detail, which beats one only in what
    /// isn't drawn (ids, paths).
    static func ranked(_ query: String, pool: [(offset: Int, element: PaletteItem)],
                       rank: [String: Int]) -> [PaletteResult] {
        let lowered = query.lowercased()
        var scored: [(result: PaletteResult, score: Int, order: Int)] = []
        for (order, item) in pool {
            let crumbs = item.crumbText
            let detail = Self.detailText(item, flat: true)
            var hits = PaletteHits()
            var score: Int
            if let m = PaletteFuzzy.match(query, in: item.title) {
                score = m.score
                hits.title = Set(m.indices)
                // Names typed out in full or from the start beat scattered hits.
                let title = item.title.lowercased()
                if title == lowered { score += 60 } else if title.hasPrefix(lowered) { score += 30 }
            } else if let m = PaletteFuzzy.match(query, in: crumbs + item.title + " " + detail) {
                // Matched round the title: half weight.
                score = m.score / 2
                let c = crumbs.count, t = item.title.count
                for i in m.indices {
                    if i < c { hits.crumbs.insert(i) } else if i < c + t { hits.title.insert(i - c) } else if i > c + t {
                        hits.detail.insert(i - c - t - 1)
                    }
                }
            } else if let m = PaletteFuzzy.match(query, in: [crumbs + item.title, detail, item.searchExtra]
                .joined(separator: " ")) {
                // Matched only in what isn't drawn: half weight, no highlight.
                score = m.score / 2
            } else {
                continue
            }
            // The query typed as is lights up wherever it shows.
            for term in lowered.split(separator: " ") where term.count >= 2 {
                hits.crumbs.formUnion(Self.occurrences(of: term, in: crumbs))
                hits.title.formUnion(Self.occurrences(of: term, in: item.title))
                hits.detail.formUnion(Self.occurrences(of: term, in: detail))
            }
            // A name-like query more often means a place than a command.
            if !item.kind.isCommand { score += 4 }
            if let r = rank[item.id] { score += max(0, 30 - r * 3) }
            if !item.enabled { score -= 8 }
            scored.append((PaletteResult(item: item, hits: hits), score, order))
        }
        scored.sort { $0.score != $1.score ? $0.score > $1.score : $0.order < $1.order }
        return scored.map(\.result)
    }

    private static func occurrences(of term: Substring, in text: String) -> Set<Int> {
        let chars = Array(text.lowercased()), needle = Array(term)
        guard chars.count == text.count, needle.count <= chars.count else { return [] }
        var out = Set<Int>()
        for start in 0...(chars.count - needle.count) where Array(chars[start..<start + needle.count]) == needle {
            out.formUnion(start..<start + needle.count)
        }
        return out
    }

    /// The detail as a row draws it: a command's category in a flat list
    /// (in `>` its section says it).
    static func detailText(_ item: PaletteItem, flat: Bool) -> String {
        if item.kind == .command, item.detail.isEmpty, flat, let category = item.category {
            return category.prefix(1).uppercased() + category.dropFirst()
        }
        return item.detail
    }

    /// An empty query, in sections. Places: "Needs you" (blocked agents,
    /// longest waiting first, then unread finishes), "Working", "Recent"
    /// (the last few rows run, not already listed), then every other place
    /// in the session's order; commands (`>`): "Recent", then each category
    /// in the keymap's order.
    static func grouped(_ items: [PaletteItem], scope: Scope,
                        recents: [String]) -> (rows: [PaletteResult], sections: [PaletteSection]) {
        var rows: [PaletteResult] = []
        var sections: [PaletteSection] = []
        var listed = Set<String>()
        func add(_ title: String, _ part: [PaletteItem], strong: Bool = false) {
            let part = part.filter { !listed.contains($0.id) }
            guard !part.isEmpty else { return }
            sections.append(PaletteSection(title: title, count: part.count, start: rows.count, strong: strong))
            rows += part.map { PaletteResult(item: $0) }
            listed.formUnion(part.map(\.id))
        }
        let byID = Dictionary(items.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        if scope == .commands {
            add("Recent", Array(recents.compactMap { byID[$0] }.prefix(recentCount)))
            var order: [String] = []
            var byCategory: [String: [PaletteItem]] = [:]
            for item in items where !listed.contains(item.id) {
                let category = item.category.map { $0.prefix(1).uppercased() + $0.dropFirst() } ?? "Other"
                if byCategory[category] == nil { order.append(category) }
                byCategory[category, default: []].append(item)
            }
            for category in order { add(category, byCategory[category] ?? []) }
            return (rows, sections)
        }
        let agents = items.filter { $0.bucket != nil }
        func oldest(_ bucket: PaletteItem.Bucket) -> [PaletteItem] {
            agents.filter { $0.bucket == bucket }.sorted { $0.since < $1.since }
        }
        add("Needs you", oldest(.blocked) + oldest(.attention), strong: true)
        add("Working", oldest(.working))
        add("Recent", Array(recents.compactMap { byID[$0] }.filter { !listed.contains($0.id) }.prefix(recentCount)))
        add("Places", items.filter { !$0.kind.isCommand })
        return (rows, sections)
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
        if item.prompt != nil {
            // Second step: ask for the input inline.
            promptItem = item
            promptLabel.stringValue = item.title.uppercased()
            promptLabel.isHidden = false
            list.isHidden = true
            empty.isHidden = true
            field.stringValue = item.initialInput ?? ""
            updatePlaceholder()
            updateCount(query: "")
            updateFooter()
            field.currentEditor()?.selectAll(nil)
            needsLayout = true
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

    /// Automation: the section headers, `Needs you 2|Working 1`.
    var describeSections: String {
        sections.isEmpty ? "-" : sections.map { "\($0.title) \($0.count)" }.joined(separator: "|")
    }

    // MARK: Layout

    /// Height for the current state, for the window's layout (which caps it;
    /// the list scrolls in what's left).
    var preferredHeight: CGFloat {
        if promptItem != nil { return Self.promptHeight + Self.footerHeight }
        if results.isEmpty { return Self.fieldHeight + Self.emptyHeight + Self.footerHeight }
        // The list's content holds its own top and bottom padding.
        return Self.fieldHeight + list.contentHeight + Self.footerHeight
    }

    override func layout() {
        super.layout()
        layer?.shadowPath = CGPath(roundedRect: bounds, cornerWidth: Design.Radius.palette,
                                   cornerHeight: Design.Radius.palette, transform: nil)
        // Opening and the list changed height: the same motion, pivoting
        // round the new top centre.
        if motion != .none, bounds.size != motionBounds { applyMotion() }
        backdrop.frame = bounds
        let top = bounds.maxY, w = bounds.width
        let prompting = promptItem != nil
        let head = prompting ? Self.promptHeight : Self.fieldHeight
        fieldRule.frame = NSRect(x: 0, y: top - head, width: w, height: Design.Border.hairline)
        footer.frame = NSRect(x: 0, y: 0, width: w, height: Self.footerHeight)

        let fieldHeight = ceil(field.intrinsicContentSize.height)
        magnifier.isHidden = prompting
        let countWidth = ceil(countLabel.intrinsicContentSize.width)
        let labelHeight = ceil(countLabel.intrinsicContentSize.height)
        countLabel.frame = NSRect(x: w - 14 - countWidth, y: top - head / 2 - labelHeight / 2,
                                  width: countWidth, height: labelHeight)
        if prompting {
            promptLabel.frame = NSRect(x: 16, y: top - 24, width: w - 32, height: 14)
            field.frame = NSRect(x: 16, y: top - 18 - (head - 18) / 2 - fieldHeight / 2,
                                 width: w - 32, height: fieldHeight)
            token.isHidden = true
        } else {
            let mid = top - head / 2
            magnifier.frame = NSRect(x: 16, y: mid - 7, width: 14, height: 14)
            var x: CGFloat = 40
            token.isHidden = token.text == nil
            if !token.isHidden {
                let size = token.intrinsicContentSize
                token.frame = NSRect(x: x, y: mid - size.height / 2, width: size.width, height: size.height)
                x = token.frame.maxX + 6
            }
            let right = countWidth > 0 ? countLabel.frame.minX - 10 : w - 14
            field.frame = NSRect(x: x - 2, y: mid - fieldHeight / 2, width: max(40, right - x + 2), height: fieldHeight)
        }
        let body = NSRect(x: 0, y: Self.footerHeight, width: w, height: max(0, top - head - Self.footerHeight))
        // Rule to rule: the list clips there, so a row scrolled part way
        // out is cut at the field row's rule and the footer's (its
        // padding is inside, `PaletteListView.padY`).
        list.frame = body.insetBy(dx: Self.inset, dy: 0)
        empty.frame = body
    }

    /// Automation: how far what the list paints reaches past its area, into
    /// the field row or the footer, in points: `0`, else `field 6`,
    /// `footer 14` or both (`field 6,footer 14`).
    var describeOverdraw: String {
        guard !list.isHidden else { return "0" }
        let fieldBottom = fieldRule.frame.minY, footerTop = footer.frame.maxY
        var intoField: CGFloat = 0, intoFooter: CGFloat = 0
        for rect in list.paintedRects where !rect.isEmpty {
            let r = convert(rect, from: list)
            intoField = max(intoField, r.maxY - fieldBottom)
            intoFooter = max(intoFooter, footerTop - r.minY)
        }
        var parts: [String] = []
        if intoField > 0.5 { parts.append("field \(Int(intoField.rounded()))") }
        if intoFooter > 0.5 { parts.append("footer \(Int(intoFooter.rounded()))") }
        return parts.isEmpty ? "0" : parts.joined(separator: ",")
    }

    /// Automation: the selected row, `shown` (all of it between the field
    /// row's rule and the footer's), `cut` (part of it) or `hidden`; `-`
    /// with no list.
    var describeSelectedRow: String {
        guard !list.isHidden, let rect = list.rowRect(selected) else { return "-" }
        let r = convert(rect, from: list)
        let area = NSRect(x: r.minX, y: footer.frame.maxY, width: r.width,
                          height: fieldRule.frame.minY - footer.frame.maxY)
        if r.minY >= area.minY - 0.5, r.maxY <= area.maxY + 0.5 { return "shown" }
        return r.intersects(area) ? "cut" : "hidden"
    }

    /// Automation: `front` when the footer is over the list (later in the
    /// subviews), else `behind`.
    var describeFooter: String {
        guard let f = subviews.firstIndex(of: footer), let l = subviews.firstIndex(of: list) else { return "-" }
        return f > l ? "front" : "behind"
    }

    /// Automation: the list's scroll (`none`, `top`, `mid`, `end`).
    var describeScroll: String { list.isHidden ? "-" : list.describeScroll }

    /// Automation: scrolls the list by `dy` points, as a wheel would.
    func scrollList(by dy: CGFloat) { list.scroll(by: dy) }
}

/// The scrim: shades the window behind quick open. Never takes clicks (a
/// click outside the menu closes it and goes on to what's under it).
@MainActor
final class PaletteScrimView: NSView, ThemeAware {
    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        isHidden = true
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func applyTheme() {
        layer?.backgroundColor = Design.Color.paletteScrim.cgColor
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

/// The scope's token in the field: "Commands" for `>`, "Places" for `@`.
@MainActor
private final class PaletteTokenView: NSView {
    var text: String? { didSet { if text != oldValue { invalidateIntrinsicContentSize(); needsDisplay = true } } }

    private var label: NSAttributedString {
        NSAttributedString(string: text ?? "", attributes: [
            .font: Design.Font.paletteScope, .foregroundColor: Design.Color.accentText,
        ])
    }

    override var intrinsicContentSize: NSSize {
        NSSize(width: ceil(label.size().width) + 16, height: 22)
    }

    override func draw(_ dirtyRect: NSRect) {
        Design.Color.accent.withAlphaComponent(Design.Alpha.paletteScope).setFill()
        NSBezierPath(roundedRect: bounds, xRadius: Design.Radius.pill, yRadius: Design.Radius.pill).fill()
        let text = label, size = text.size()
        text.draw(at: NSPoint(x: (bounds.width - size.width) / 2, y: (bounds.height - size.height) / 2))
    }
}

/// The menu's footer: its keys and what they do, over a hairline. Opaque
/// (the menu's own fill) and over the list, whatever the list draws.
@MainActor
private final class PaletteFooterView: NSView {
    var keys: [(key: String, label: String)] = [] { didSet { needsDisplay = true } }

    override init(frame: NSRect) {
        super.init(frame: frame)
        clipsToBounds = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        // Inside the menu's border, its bottom corners rounded as the
        // menu's are, so the border still shows round it.
        let edge = Design.Border.hairline, radius = max(0, Design.Radius.palette - edge)
        Design.Color.palette.setFill()
        NSBezierPath(roundedRect: NSRect(x: edge, y: -radius, width: bounds.width - 2 * edge,
                                         height: bounds.height + radius - edge),
                     xRadius: radius, yRadius: radius).fill()
        Design.Color.border.setFill()
        NSRect(x: 0, y: 0, width: bounds.width, height: Design.Border.hairline).fill()
        var x: CGFloat = 12
        for (key, label) in keys {
            x = PaletteKeycap.draw(key, font: Design.Font.hintKey, at: x, midY: bounds.midY, height: 17, padding: 5)
            x += 5
            let text = NSAttributedString(string: label, attributes: [
                .font: Design.Font.detail, .foregroundColor: Design.Color.textSecondary,
            ])
            let size = text.size()
            text.draw(at: NSPoint(x: x, y: bounds.midY - size.height / 2))
            x += ceil(size.width) + 14
        }
    }
}

/// No rows: what was searched for and how to widen it.
@MainActor
private final class PaletteEmptyView: NSView {
    var query = "" { didSet { needsDisplay = true } }
    var scoped = false { didSet { needsDisplay = true } }

    override var isFlipped: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        let title = NSAttributedString(string: "No matches for “\(query)”", attributes: [
            .font: Design.Font.paletteEmpty, .foregroundColor: Design.Color.textPrimary,
        ])
        let hint = scoped ? "Delete the token (⌫) to search everything."
            : "Start with > for commands or @ for places."
        let sub = NSAttributedString(string: hint, attributes: [
            .font: Design.Font.paletteDetail, .foregroundColor: Design.Color.textSecondary,
        ])
        let a = title.size(), b = sub.size(), gap: CGFloat = 6
        let y = (min(bounds.height, PaletteView.emptyHeight) - a.height - b.height - gap) / 2
        title.draw(at: NSPoint(x: (bounds.width - a.width) / 2, y: y))
        sub.draw(at: NSPoint(x: (bounds.width - b.width) / 2, y: y + a.height + gap))
    }
}

/// A key drawn as a keycap: its glyphs in a hairline box.
@MainActor
enum PaletteKeycap {
    /// Draws `key` with its left edge at `x`, centred on `midY`; returns its
    /// right edge.
    static func draw(_ key: String, font: NSFont, at x: CGFloat, midY: CGFloat, height: CGFloat,
                     padding: CGFloat) -> CGFloat {
        let text = NSAttributedString(string: key, attributes: [
            .font: font, .foregroundColor: Design.Color.textSecondary,
        ])
        let size = text.size()
        let box = NSRect(x: x, y: (midY - height / 2).rounded(), width: ceil(size.width) + 2 * padding, height: height)
        Design.Color.hintBadgeBorder.setStroke()
        let path = NSBezierPath(roundedRect: box.insetBy(dx: 0.5, dy: 0.5), xRadius: Design.Radius.chip,
                                yRadius: Design.Radius.chip)
        path.lineWidth = 1
        path.stroke()
        text.draw(at: NSPoint(x: box.minX + padding, y: box.midY - size.height / 2))
        return box.maxX
    }

    static func width(_ key: String, font: NSFont, padding: CGFloat) -> CGFloat {
        ceil(NSAttributedString(string: key, attributes: [.font: font]).size().width) + 2 * padding
    }
}

/// The result rows and their section headers, drawn by hand so highlighted
/// characters, crumbs and keycaps line up. Scrolls by the pixel.
///
/// It runs from the field row's rule down to the footer's, its top and
/// bottom padding inside (`padY`), and clips to its bounds: a row scrolled
/// part way out is cut at those rules, never drawn over the field row or
/// the footer (views don't clip by default since macOS 14).
@MainActor
final class PaletteListView: NSView {
    override init(frame: NSRect) {
        super.init(frame: frame)
        clipsToBounds = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private(set) var rows: [PaletteResult] = []
    private(set) var sections: [PaletteSection] = []
    var selected = 0 {
        didSet {
            reveal(selected)
            needsDisplay = true
        }
    }
    var onClick: ((Int) -> Void)?
    private var hover: Int?
    private var scrollY: CGFloat = 0
    /// Each row's top and each header's, in content coordinates (y down).
    private var rowTops: [CGFloat] = []
    private var headerTops: [(section: PaletteSection, top: CGFloat)] = []
    private(set) var contentHeight: CGFloat = 0
    private var tracking: NSTrackingArea?

    static let rowHeight = Design.Space.paletteRowHeight
    static let sectionHeight = Design.Space.paletteSectionHeight
    /// Space over every section header but the first.
    static let sectionGap: CGFloat = 6
    static let padX = Design.Space.paletteRowPaddingX
    /// Space over the first row and under the last, scrolled with them.
    static let padY = Design.Space.paletteInset

    override var isFlipped: Bool { true }

    /// Rows a page (page up / down) moves.
    var pageRows: Int { max(1, Int(bounds.height / Self.rowHeight) - 1) }

    func set(rows: [PaletteResult], sections: [PaletteSection]) {
        self.rows = rows
        self.sections = sections
        let starts = Dictionary(sections.map { ($0.start, $0) }, uniquingKeysWith: { a, _ in a })
        var y = Self.padY
        rowTops = []
        headerTops = []
        for i in rows.indices {
            if let section = starts[i] {
                if i > 0 { y += Self.sectionGap }
                headerTops.append((section, y))
                y += Self.sectionHeight
            }
            rowTops.append(y)
            y += Self.rowHeight
        }
        contentHeight = rows.isEmpty ? 0 : y + Self.padY
        scrollY = 0
        hover = nil
        needsDisplay = true
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        scrollY = min(scrollY, maxScroll)
        reveal(selected)
    }

    /// How far it scrolls: the last row and the padding under it end at
    /// the bottom edge.
    private var maxScroll: CGFloat { max(0, contentHeight - bounds.height) }

    /// Scrolls by `dy` points (positive: towards the end), as a wheel does.
    func scroll(by dy: CGFloat) {
        scrollY = max(0, min(maxScroll, scrollY + dy))
        needsDisplay = true
    }

    /// Scrolls row `index` into view, with its section header when it's
    /// the section's first row, and the padding when it's the first or last.
    private func reveal(_ index: Int) {
        guard rowTops.indices.contains(index), bounds.height > 0 else { return }
        var top = rowTops[index]
        if let header = headerTops.first(where: { $0.top + Self.sectionHeight == top }) { top = header.top }
        if index == 0 { top = 0 }
        let bottom = index == rowTops.count - 1 ? contentHeight : rowTops[index] + Self.rowHeight
        if top < scrollY {
            scrollY = top
        } else if bottom > scrollY + bounds.height {
            scrollY = bottom - bounds.height
        }
    }

    private func row(at point: NSPoint) -> Int? {
        let y = point.y + scrollY
        guard let i = rowTops.lastIndex(where: { $0 <= y }), y < rowTops[i] + Self.rowHeight else { return nil }
        return i
    }

    /// Content tops (y down) of what's at least partly in view.
    private var visibleTops: ClosedRange<CGFloat> { scrollY - Self.rowHeight...(scrollY + bounds.height) }

    /// Automation: where each header and row it draws now lands, in its own
    /// coordinates, cut to its bounds as the clip cuts them (`rect` empty:
    /// nothing of it shows).
    var paintedRects: [NSRect] {
        let visible = visibleTops
        let headers = headerTops.filter { visible.contains($0.top) }
            .map { NSRect(x: 0, y: $0.top - scrollY, width: bounds.width, height: Self.sectionHeight) }
        let rows = rowTops.filter { visible.contains($0) }
            .map { NSRect(x: 0, y: $0 - scrollY, width: bounds.width, height: Self.rowHeight) }
        return clipsToBounds ? (headers + rows).map { $0.intersection(bounds) } : headers + rows
    }

    /// Automation: where row `index` is, in its own coordinates (unclipped).
    func rowRect(_ index: Int) -> NSRect? {
        guard rowTops.indices.contains(index) else { return nil }
        return NSRect(x: 0, y: rowTops[index] - scrollY, width: bounds.width, height: Self.rowHeight)
    }

    /// Automation: `top`, `end` (scrolled all the way down) or `mid`;
    /// `none` when it all fits.
    var describeScroll: String {
        maxScroll < 0.5 ? "none" : scrollY < 0.5 ? "top" : scrollY > maxScroll - 0.5 ? "end" : "mid"
    }

    override func draw(_ dirtyRect: NSRect) {
        let visible = visibleTops
        for (section, top) in headerTops where visible.contains(top) {
            drawHeader(section, in: NSRect(x: 0, y: top - scrollY, width: bounds.width, height: Self.sectionHeight))
        }
        for (index, top) in rowTops.enumerated() where visible.contains(top) {
            let rect = NSRect(x: 0, y: top - scrollY, width: bounds.width, height: Self.rowHeight)
            if index == selected || index == hover {
                let fill = Design.Color.paletteSelected
                (index == selected ? fill : Theme.faded(fill, Design.Alpha.rowHover)).setFill()
                NSBezierPath(roundedRect: rect, xRadius: Design.Radius.pill, yRadius: Design.Radius.pill).fill()
            }
            draw(rows[index], selected: index == selected, in: rect)
        }
    }

    private func drawHeader(_ section: PaletteSection, in rect: NSRect) {
        let label = NSAttributedString(string: section.title, attributes: [
            .font: Design.Font.paletteSection,
            .foregroundColor: section.strong ? Design.Color.textPrimary : Design.Color.textSecondary,
        ])
        let size = label.size()
        label.draw(at: NSPoint(x: Self.padX, y: rect.midY - size.height / 2))
        Self.drawChip("\(section.count)", at: Self.padX + ceil(size.width) + 6, midY: rect.midY,
                      color: Design.Color.textSecondary)
    }

    /// A small rounded chip (section counts, machine labels); returns its
    /// right edge.
    @discardableResult
    private static func drawChip(_ text: String, at x: CGFloat, midY: CGFloat, color: NSColor) -> CGFloat {
        let label = NSAttributedString(string: text, attributes: [.font: Design.Font.paletteChip, .foregroundColor: color])
        let size = label.size()
        let height = ceil(size.height) + 1
        let box = NSRect(x: x, y: (midY - height / 2).rounded(), width: ceil(size.width) + 10, height: height)
        Design.Color.chip.setFill()
        NSBezierPath(roundedRect: box, xRadius: Design.Radius.chip, yRadius: Design.Radius.chip).fill()
        label.draw(at: NSPoint(x: box.minX + 5, y: box.midY - size.height / 2))
        return box.maxX
    }

    private static func chipWidth(_ text: String) -> CGFloat {
        ceil(NSAttributedString(string: text, attributes: [.font: Design.Font.paletteChip]).size().width) + 10
    }

    private func draw(_ row: PaletteResult, selected: Bool, in rect: NSRect) {
        let item = row.item
        var x = Self.padX
        let slot: CGFloat = 14
        let fade: CGFloat = item.enabled ? 1 : 0.35
        if let status = item.status, let image = SidebarView.icon(status, pointSize: Design.Symbol.paletteStatus) {
            let size = image.size
            image.draw(in: NSRect(x: x + (slot - size.width) / 2, y: rect.midY - size.height / 2,
                                  width: size.width, height: size.height),
                       from: .zero, operation: .sourceOver, fraction: fade, respectFlipped: true, hints: nil)
        } else if let image = Self.icon(item.kind, selected: selected) {
            let size = image.size
            image.draw(in: NSRect(x: x + (slot - size.width) / 2, y: rect.midY - size.height / 2,
                                  width: size.width, height: size.height),
                       from: .zero, operation: .sourceOver, fraction: item.enabled ? (selected ? 1 : 0.9) : 0.35,
                       respectFlipped: true, hints: nil)
        }
        x += slot + 8

        // Right side first, so the rest knows how much room it has.
        var right = rect.maxX - Self.padX
        if let note = item.note, !note.isEmpty {
            right = drawRight(note, color: Design.Color.textSecondary, right: right, midY: rect.midY)
        } else if let wait = item.wait {
            let tone: NSColor = item.status == .blocked ? Design.Color.statusBlocked
                : item.status == .done ? Design.Color.statusDone : Design.Color.textSecondary
            right = drawRight(wait, color: tone, right: right, midY: rect.midY)
        } else {
            let keys = item.keyHint.map { [$0] } ?? item.chords.prefix(2).map(PaletteFuzzy.prettyChord)
            for key in keys.reversed() {
                let w = PaletteKeycap.width(key, font: Design.Font.chord, padding: 6)
                _ = PaletteKeycap.draw(key, font: Design.Font.chord, at: right - w, midY: rect.midY, height: 18,
                                       padding: 6)
                right -= w + 4
            }
            if !keys.isEmpty { right += 4 }
        }
        let limit = right - (right < rect.maxX - Self.padX ? 12 : 0)

        // The title: machine chip, crumbs, leaf.
        let match = selected ? Design.Color.paletteMatchSelected : Design.Color.accentText
        let titleFont = Design.Font.paletteTitle
        let titleHeight = ceil(NSAttributedString(string: "Ag", attributes: [.font: titleFont]).size().height)
        let titleTop = rect.midY - titleHeight / 2
        let baseline = titleTop + titleFont.ascender
        if let machine = item.machine, x + Self.chipWidth(machine) < limit {
            x = Self.drawChip(machine, at: x, midY: rect.midY, color: Design.Color.chipText) + 6
        }
        let title = NSMutableAttributedString()
        title.append(Self.lit(item.crumbText, hits: row.hits.crumbs, font: Design.Font.paletteCrumb,
                              color: Design.Color.textSecondary, match: match, enabled: item.enabled))
        title.append(Self.lit(item.title, hits: row.hits.title, font: titleFont,
                              color: item.enabled ? Design.Color.textPrimary : Design.Color.textMuted,
                              match: match, enabled: item.enabled))
        let titleWidth = ceil(title.size().width)
        let room = max(0, limit - x)
        Self.drawLine(title, x: x, top: titleTop, width: min(titleWidth, room))
        x += min(titleWidth, room) + 8

        // The detail, in what's left: it truncates before the title does.
        let detail = PaletteView.detailText(item, flat: sections.isEmpty)
        guard !detail.isEmpty, limit - x > 24 else { return }
        let font = item.detailMono ? Design.Font.monoDetail : Design.Font.paletteDetail
        let text = Self.lit(detail, hits: row.hits.detail, font: font, color: Design.Color.textSecondary,
                            match: match, enabled: item.enabled)
        Self.drawLine(text, x: x, top: baseline - font.ascender, width: limit - x)
    }

    /// Right-aligned grey text ending at `right`; returns its left edge.
    private func drawRight(_ text: String, color: NSColor, right: CGFloat, midY: CGFloat) -> CGFloat {
        let label = NSAttributedString(string: text, attributes: [.font: Design.Font.detail, .foregroundColor: color])
        let size = label.size()
        label.draw(at: NSPoint(x: right - ceil(size.width), y: midY - size.height / 2))
        return right - ceil(size.width)
    }

    /// `text` with the characters at `hits` in the bold match colour.
    private static func lit(_ text: String, hits: Set<Int>, font: NSFont, color: NSColor, match: NSColor,
                            enabled: Bool) -> NSAttributedString {
        let out = NSMutableAttributedString(string: text, attributes: [.font: font, .foregroundColor: color])
        guard enabled, !hits.isEmpty else { return out }
        // paletteMatch for the 13 pt title and crumbs; the detail's own
        // size, bold, for the detail.
        let bold: NSFont = font.isFixedPitch ? .monospacedSystemFont(ofSize: font.pointSize, weight: .bold)
            : font.pointSize == Design.Font.paletteMatch.pointSize ? Design.Font.paletteMatch
            : .systemFont(ofSize: font.pointSize, weight: .bold)
        var offset = 0
        for (i, c) in text.enumerated() {
            let length = String(c).utf16.count
            if hits.contains(i) {
                out.addAttributes([.font: bold, .foregroundColor: match], range: NSRange(location: offset, length: length))
            }
            offset += length
        }
        return out
    }

    /// One line, cut with an ellipsis at `width`.
    private static func drawLine(_ text: NSAttributedString, x: CGFloat, top: CGFloat, width: CGFloat) {
        guard width > 0 else { return }
        let line = NSMutableAttributedString(attributedString: text)
        let style = NSMutableParagraphStyle()
        style.lineBreakMode = .byTruncatingTail
        line.addAttribute(.paragraphStyle, value: style, range: NSRange(location: 0, length: line.length))
        line.draw(with: NSRect(x: x, y: top, width: width, height: ceil(line.size().height) + 2),
                  options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
    }

    private static var icons: [String: NSImage] = [:]

    static func icon(_ kind: PaletteItem.Kind, selected: Bool) -> NSImage? {
        // By theme too: the image holds its tint.
        let key = Theme.current.id + "/" + kind.symbol + (selected ? "/on" : "")
        if let cached = icons[key] { return cached }
        let tint = selected ? Design.Color.textPrimary : Design.Color.textSecondary
        let config = NSImage.SymbolConfiguration(pointSize: Design.Symbol.paletteKind, weight: .regular)
            .applying(.init(paletteColors: [Theme.resolved(tint)]))
        guard let image = NSImage(systemSymbolName: kind.symbol, accessibilityDescription: kind.label)?
            .withSymbolConfiguration(config) else { return nil }
        icons[key] = image
        return image
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: .zero, options: [.mouseMoved, .mouseEnteredAndExited, .activeInActiveApp,
                                                         .inVisibleRect], owner: self, userInfo: nil)
        addTrackingArea(area)
        tracking = area
    }

    private func setHover(_ event: NSEvent?) {
        let new = event.flatMap { row(at: convert($0.locationInWindow, from: nil)) }
        if new != hover {
            hover = new
            needsDisplay = true
        }
    }

    override func mouseMoved(with event: NSEvent) { setHover(event) }
    override func mouseEntered(with event: NSEvent) { setHover(event) }
    override func mouseExited(with event: NSEvent) { setHover(nil) }

    override func mouseDown(with event: NSEvent) {
        if let index = row(at: convert(event.locationInWindow, from: nil)) { onClick?(index) }
    }

    override func scrollWheel(with event: NSEvent) {
        let dy = event.hasPreciseScrollingDeltas ? event.scrollingDeltaY : event.scrollingDeltaY * Self.rowHeight
        scroll(by: -dy)
        setHover(event)
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
