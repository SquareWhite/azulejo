import AppKit

// Pieces the full sidebar's Workspaces view and the rail share: what a
// workspace row shows (`WorkspaceInfo`), text helpers (a branch or a path
// cut in the middle, time in state), the branch and commit glyphs, the
// hover card that shows a row's full values, and the edge fades and
// overflow pills of a list taller than its room.

// MARK: - What a row shows

/// A workspace as its row (sidebar or rail) and its hover card show it.
@MainActor
struct WorkspaceInfo: Equatable {
    let id: String
    let name: String
    /// The agents' state (blocked wins, then an unread finish, then
    /// working, else idle): what automation reads as the row's status.
    let status: Design.Status
    /// herdr hasn't made the workspace's terminals yet.
    let starting: Bool
    /// When the winning state began (ms since the epoch); nil when idle.
    let sinceMs: UInt64?
    /// How many agents share the winning state.
    let count: Int
    /// The winning agent's name, when one agent holds that state.
    let agent: String?
    /// The branch, or the short hash when detached; nil outside a repo.
    let branch: String?
    let detached: Bool
    let dirty: Bool
    /// The workspace's folder (repository, else cwd), `~` for home.
    let path: String?

    init(_ ws: WorkspaceModel) {
        id = ws.id
        name = ws.name
        let status = SidebarView.status(ws)
        self.status = status
        let tiles = ws.tabs.flatMap(\.tiles)
        let terminals = tiles.filter { $0.kind == .terminal }
        starting = !terminals.isEmpty && terminals.allSatisfy { $0.herdr == nil && $0.ended != true }
        let agents = tiles.compactMap(\.agent)
        let holding: [AgentModel] = switch status {
        case .blocked: agents.filter { $0.status == .blocked }
        case .done: agents.filter { $0.attention && $0.status != .blocked }
        case .working: agents.filter { $0.status == .working }
        default: []
        }
        count = holding.count
        sinceMs = holding.compactMap(\.sinceMs).filter { $0 > 0 }.min()
        let names = Set(holding.compactMap(\.name))
        agent = names.count == 1 ? names.first : nil
        let git = ws.git
        if let git, let branch = git.branch, !branch.isEmpty {
            self.branch = branch
            detached = git.detached == true
            dirty = git.dirty == true
        } else {
            branch = nil
            detached = false
            dirty = false
        }
        path = [ws.git?.repo, ws.cwd].compactMap { $0 }.first { !$0.isEmpty }.map(SidebarView.tilde)
    }

    /// The state the row's icon shows: starting until herdr has the
    /// workspace, else the agents'.
    var shown: Design.Status { starting && status == .idle ? .starting : status }

    /// The state in words, as the hover card says it ("Waiting", "Working").
    var stateWord: String {
        switch shown {
        case .blocked: "Waiting"
        case .working: "Working"
        case .done: "Done"
        case .starting: "Starting…"
        default: "Idle"
        }
    }

    /// The time-in-state colour: the state's own for the ones that need you.
    var timeColor: NSColor {
        switch status {
        case .blocked: Design.Color.statusBlocked
        case .done: Design.Color.statusDone
        default: Design.Color.textMuted
        }
    }
}

// MARK: - Text

@MainActor
enum SidebarText {
    /// How long a state has held, short: "now", "4m", "2h", "3d".
    static func age(sinceMs: UInt64, now: Date = Date()) -> String {
        let seconds = max(0, now.timeIntervalSince1970 - Double(sinceMs) / 1000)
        switch seconds {
        case ..<60: return "now"
        case ..<3600: return "\(Int(seconds / 60))m"
        case ..<86400: return "\(Int(seconds / 3600))h"
        default: return "\(Int(seconds / 86400))d"
        }
    }

    /// `text` cut in the middle to `keep` characters (the ellipsis one of
    /// them), keeping about 40 % at the head, or a short prefix up to its
    /// first `/` (`feature/`, `renovate/`) when that's under half: the
    /// prefix and the distinguishing tail stay. AppKit's
    /// `byTruncatingMiddle` cuts at the centre.
    static func middle(_ text: String, keep: Int) -> String {
        let chars = Array(text)
        guard chars.count > keep else { return text }
        guard keep >= 2 else { return keep == 1 ? "…" : "" }
        let room = keep - 1
        var head = room * 2 / 5
        if let slash = chars.firstIndex(of: "/"), slash > 0, slash + 1 <= room / 2 { head = max(head, slash + 1) }
        head = max(0, min(head, room - 4))
        let tail = room - head
        return String(chars[..<head]) + "…" + String(chars[(chars.count - tail)...])
    }

    /// `text` cut in the middle (`middle`) to fit `width` points in `font`.
    static func middle(_ text: String, font: NSFont, width: CGFloat) -> String {
        let attrs: [NSAttributedString.Key: Any] = [.font: font]
        func fits(_ s: String) -> Bool { ceil((s as NSString).size(withAttributes: attrs).width) <= width }
        if fits(text) { return text }
        var lo = 0, hi = text.count - 1
        while lo < hi {
            let mid = (lo + hi + 1) / 2
            if fits(middle(text, keep: mid)) { lo = mid } else { hi = mid - 1 }
        }
        return middle(text, keep: lo)
    }

    /// `text` cut at its end (an ellipsis last) to fit `width` points in `font`.
    static func end(_ text: String, font: NSFont, width: CGFloat) -> String {
        guard Self.width(text, font: font) > width else { return text }
        let chars = Array(text)
        var lo = 0, hi = chars.count - 1
        while lo < hi {
            let mid = (lo + hi + 1) / 2
            if Self.width(String(chars[..<mid]) + "…", font: font) <= width { lo = mid } else { hi = mid - 1 }
        }
        return String(chars[..<lo]).trimmingCharacters(in: .whitespaces) + "…"
    }

    /// A path cut to fit `width` points in `font`: folders out of its
    /// middle for an ellipsis, as many of its last ones kept whole as fit
    /// (the last always), then as many of its first ones
    /// (`~/…/clients/sample-api`, `/private/…/example-studio/reports`).
    /// When even its last folder won't fit, that's cut in the middle.
    static func path(_ text: String, font: NSFont, width: CGFloat) -> String {
        func fits(_ s: String) -> Bool { Self.width(s, font: font) <= width }
        if fits(text) { return text }
        let parts = text.split(separator: "/", omittingEmptySubsequences: false).map(String.init)
        // A leading "/" keeps its first folder: "/private/…", not "/…".
        let first = parts.first?.isEmpty == true ? 2 : 1
        if parts.count >= first + 2 {
            for tail in stride(from: parts.count - first - 1, through: 1, by: -1) {
                let end = parts[(parts.count - tail)...].joined(separator: "/")
                for head in stride(from: parts.count - tail - 1, through: first, by: -1) {
                    let cut = parts[..<head].joined(separator: "/") + "/…/" + end
                    if fits(cut) { return cut }
                }
            }
        }
        guard let last = parts.last, parts.count > 1 else { return middle(text, font: font, width: width) }
        return "…/" + middle(last, font: font, width: width - Self.width("…/", font: font))
    }

    /// The width `text` takes in `font`.
    static func width(_ text: String, font: NSFont) -> CGFloat {
        ceil((text as NSString).size(withAttributes: [.font: font]).width)
    }

    /// A one-line label: no wrapping, its text cut at the end.
    static func label(_ text: String = "", font: NSFont, color: NSColor) -> NSTextField {
        let field = NSTextField(labelWithString: text)
        field.font = font
        field.textColor = color
        field.lineBreakMode = .byTruncatingTail
        field.maximumNumberOfLines = 1
        return field
    }
}

// MARK: - Glyphs

@MainActor
enum SidebarGlyph {
    /// A branch (git's fork) for the branch line, in `color`.
    static func branch(_ color: NSColor, size: CGFloat = 10) -> NSImage? {
        let color = Theme.resolved(color)
        // The mockup's glyph, on its 10-point grid (top down): two commits on
        // a line and a third branching off to the right.
        let image = NSImage(size: NSSize(width: size, height: size), flipped: true) { rect in
            let k = rect.width / 10
            func p(_ x: CGFloat, _ y: CGFloat) -> NSPoint { NSPoint(x: x * k, y: y * k) }
            let path = NSBezierPath()
            for (x, y) in [(2.6, 2.0), (2.6, 8.2), (7.6, 3.2)] as [(CGFloat, CGFloat)] {
                path.appendOval(in: NSRect(x: (x - 1.2) * k, y: (y - 1.2) * k, width: 2.4 * k, height: 2.4 * k))
            }
            path.move(to: p(2.6, 3.2))
            path.line(to: p(2.6, 7))
            path.move(to: p(7.6, 4.4))
            path.curve(to: p(3.2, 7.3), controlPoint1: p(7.6, 6.2), controlPoint2: p(5.2, 6.0))
            path.lineWidth = max(1, k)
            color.setStroke()
            path.stroke()
            return true
        }
        image.accessibilityDescription = "branch"
        return image
    }

    /// A commit (a dot on a line) for a detached HEAD's hash.
    static func commit(_ color: NSColor, size: CGFloat = 10) -> NSImage {
        let color = Theme.resolved(color)
        let card = Theme.resolved(Design.Color.sidebar)
        let image = NSImage(size: NSSize(width: size, height: size), flipped: false) { rect in
            color.setStroke()
            let line = NSBezierPath()
            line.move(to: NSPoint(x: rect.minX + 0.5, y: rect.midY))
            line.line(to: NSPoint(x: rect.maxX - 0.5, y: rect.midY))
            line.lineWidth = 1
            line.stroke()
            let dot = NSBezierPath(ovalIn: NSRect(x: rect.midX - 2.2, y: rect.midY - 2.2, width: 4.4, height: 4.4))
            card.setFill()
            dot.fill()
            dot.lineWidth = 1
            dot.stroke()
            return true
        }
        image.accessibilityDescription = "commit"
        return image
    }

    /// A folder for a hover card's path line, in `color`.
    static func folder(_ color: NSColor, size: CGFloat = 10) -> NSImage? {
        NSImage(systemSymbolName: "folder", accessibilityDescription: "folder")?
            .withSymbolConfiguration(.init(pointSize: size, weight: .regular)
                .applying(.init(paletteColors: [Theme.resolved(color)])))
    }

    /// A machine: a laptop for Local, a server for the others.
    static func machine(local: Bool, color: NSColor) -> NSImage? {
        NSImage(systemSymbolName: local ? "laptopcomputer" : "server.rack", accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: Design.Symbol.sidebarGroup, weight: .medium)
                .applying(.init(paletteColors: [Theme.resolved(color)])))
    }

    /// A status icon for the sidebar and the rail. Filled symbols (blocked's
    /// `exclamationmark.circle.fill`, done's `checkmark.circle.fill`) get
    /// their mark in the card's colour on the status colour, as in the
    /// mockup, rather than one flat disc; idle is a thin ring as wide as
    /// them (`azulejo.idleCircle`).
    static func status(_ status: Design.Status, pointSize: CGFloat) -> NSImage? {
        if status == .idle {
            return Design.customSymbol("azulejo.idleCircle", color: status.color, pointSize: pointSize)
        }
        guard status.symbol.hasSuffix(".fill") else { return Design.StatusIcon.image(status, pointSize: pointSize) }
        let card = Theme.resolved(Design.Color.sidebar)
        let mark = card.alphaComponent < 1 ? Theme.resolved(Design.Color.window) : card
        let config = NSImage.SymbolConfiguration(pointSize: pointSize, weight: .semibold)
            .applying(.init(paletteColors: [mark, Theme.resolved(status.color)]))
        return NSImage(systemSymbolName: status.symbol, accessibilityDescription: status.rawValue)?
            .withSymbolConfiguration(config)
    }
}

/// A small filled circle: a machine's connection state, or a dirty work tree.
@MainActor
final class DotView: NSView, ThemeAware {
    private let color: NSColor

    init(color: NSColor, side: CGFloat = Design.Radius.machineDot * 2) {
        self.color = color
        super.init(frame: NSRect(x: 0, y: 0, width: side, height: side))
        wantsLayer = true
        layer?.cornerRadius = side / 2
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    func applyTheme() {
        layer?.backgroundColor = color.cgColor
    }
}

// MARK: - Hover cards

/// What a hover card says, line by line. Values (a name, a branch, a path)
/// keep to one line each and are cut to fit, never wrapped mid-word.
enum HoverLine {
    /// Bold, one line (cut at its end); its chord (⌘2) on a keycap at the
    /// right edge.
    case title(String, key: String?)
    /// The branch (or a detached HEAD's hash) in mono on one line, cut in
    /// the middle (`SidebarText.middle`, a `feature/` prefix kept); dirty:
    /// the sidebar's dot after it and "Uncommitted changes" on a line of
    /// its own under it.
    case branch(String, detached: Bool, dirty: Bool)
    /// A state icon, its words in the state's colour, then muted words.
    case state(Design.Status, String, color: NSColor, trailing: String?)
    /// A folder in mono, muted, on one line: cut in the middle, its last
    /// folders whole (`SidebarText.path`).
    case path(String)
    /// Plain words, muted, wrapping between words (a message, a list).
    case text(String)
}

/// The card that shows a rail row's full values after the pointer rests
/// on it (`motion.hoverCardDelay`): name and ⌘ key, the branch, the state
/// with its time and agent, the path; a machine letter's state and herdr's
/// message; the footer's counts. It sits over everything in the window,
/// right of the rail (or under its anchor, `.below`), and goes as the
/// pointer leaves. The full sidebar's rows have none.
/// It floats as ⌘K's menu does: the menu's fill, hairline, shadow and
/// corner radius, its keycap for the chord. It's as wide as its longest
/// line (`space.hoverCardMinWidth` to `hoverCardMaxWidth`); past that a
/// value is cut to fit its one line.
@MainActor
final class HoverCard: NSView, ThemeAware {
    /// Where it sits against its row.
    enum Placement { case below, right }

    /// One line of the card: the width it takes uncut, the gap under it,
    /// and how it lays out (at `x`, `y`, `width` wide; its height).
    private struct Line {
        var natural: CGFloat
        var gapAfter: CGFloat = Design.Space.hoverCardLineGap
        /// Its labels (automation: how many text lines each takes).
        var fields: [NSTextField]
        /// Automation: what else it shows (a keycap's chord, the dirty dot).
        var marks: String = ""
        var place: @MainActor (_ x: CGFloat, _ y: CGFloat, _ width: CGFloat) -> CGFloat
    }

    private(set) static weak var shown: HoverCard?
    private static var pending: Timer?
    private static weak var pendingAnchor: NSView?
    private static var pendingCard: (placement: Placement, lines: () -> [HoverLine])?
    /// When the last card went: another row within this shows its card at once.
    private static var lastHidden = Date.distantPast
    /// A label's text sits this far inside its frame (the cell's inset).
    private static let inset: CGFloat = 2

    /// Automation: the lines of the card showing, in full ("" when none).
    static var text: String { shown?.summary ?? "" }

    let anchor: NSView
    private let summary: String
    private var lines: [Line] = []
    /// The value lines' glyph column (the branch, a state's icon, a
    /// folder), their words after it.
    private let column: CGFloat

    /// Shows `lines` for `anchor` once the pointer has rested there (at once
    /// with `now`, or right after another card went).
    static func schedule(_ anchor: NSView, placement: Placement, now: Bool = false,
                         lines: @escaping () -> [HoverLine]) {
        cancel()
        let warm = Date().timeIntervalSince(lastHidden) < 0.5 || shown != nil
        hide()
        if now || warm {
            show(anchor, placement: placement, lines: lines())
            return
        }
        pendingAnchor = anchor
        pendingCard = (placement, lines)
        pending = Timer.scheduledTimer(withTimeInterval: Design.Motion.hoverCardDelay, repeats: false) { _ in
            MainActor.assumeIsolated {
                guard let anchor = pendingAnchor, anchor.window != nil, let card = pendingCard else { return }
                pending = nil
                pendingCard = nil
                show(anchor, placement: card.placement, lines: card.lines())
            }
        }
    }

    /// The pointer left `anchor` (or anything): no card for it.
    static func leave(_ anchor: NSView) {
        if pendingAnchor === anchor { cancel() }
        if shown?.anchor === anchor { hide() }
    }

    static func cancel() {
        pending?.invalidate()
        pending = nil
        pendingAnchor = nil
        pendingCard = nil
    }

    static func hide() {
        guard let card = shown else { return }
        card.removeFromSuperview()
        shown = nil
        lastHidden = Date()
    }

    private static func show(_ anchor: NSView, placement: Placement, lines: [HoverLine]) {
        guard let root = anchor.window?.contentView, !anchor.isHiddenOrHasHiddenAncestor else { return }
        let card = HoverCard(anchor: anchor, lines: lines)
        let rect = root.convert(anchor.bounds, from: anchor)
        let size = placement == .below
            ? card.fit(min: rect.width, max: rect.width)
            : card.fit(min: Design.Space.hoverCardMinWidth, max: Design.Space.hoverCardMaxWidth)
        var origin = placement == .below
            ? NSPoint(x: rect.minX, y: (root.isFlipped ? rect.maxY + 4 : rect.minY - 4 - size.height))
            : NSPoint(x: rect.maxX + 8, y: root.isFlipped ? rect.minY : rect.maxY - size.height)
        // Inside the window, flipping above the row when there's no room below.
        let bounds = root.bounds.insetBy(dx: 4, dy: 4)
        if placement == .below, !root.isFlipped, origin.y < bounds.minY {
            origin.y = rect.maxY + 4
        }
        origin.x = min(max(origin.x, bounds.minX), bounds.maxX - size.width)
        origin.y = min(max(origin.y, bounds.minY), bounds.maxY - size.height)
        card.frame = NSRect(origin: origin, size: size)
        root.addSubview(card)
        shown = card
    }

    private init(anchor: NSView, lines: [HoverLine]) {
        self.anchor = anchor
        summary = lines.flatMap(Self.words).joined(separator: " ¦ ")
        column = ceil(max(10, SidebarGlyph.status(.blocked, pointSize: 11)?.size.width ?? 10))
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.palette
        layer?.cornerCurve = .continuous
        layer?.borderWidth = Design.Border.hairline
        layer?.masksToBounds = false
        for line in lines { self.lines += build(line) }
        // A little more room under the title than between the values.
        if case .title = lines.first, !self.lines.isEmpty { self.lines[0].gapAfter = Design.Space.hoverCardTitleGap }
        setAccessibilityElement(true)
        setAccessibilityRole(.helpTag)
        setAccessibilityLabel(summary)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    /// Never in the way of the pointer: the row under it keeps its hover.
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    /// ⌘K's surface: `palette`, its `border` hairline, its shadow (a
    /// smaller spread for a smaller card).
    func applyTheme() {
        layer?.backgroundColor = Design.Color.palette.cgColor
        layer?.borderColor = Design.Color.border.cgColor
        layer?.shadowColor = Design.Color.shadow.cgColor
        layer?.shadowOpacity = Float(Design.Alpha.paletteShadow)
        layer?.shadowRadius = 9
        layer?.shadowOffset = CGSize(width: 0, height: -4)
    }

    /// Its size: as wide as its longest line, from `min` to `max`, each
    /// line laid out (and cut) to that.
    private func fit(min low: CGFloat, max high: CGFloat) -> NSSize {
        let padX = Design.Space.hoverCardPaddingX, padY = Design.Space.hoverCardPaddingY
        let natural = lines.map(\.natural).max() ?? 0
        let inner = min(max(ceil(natural), low - 2 * padX), high - 2 * padX)
        var y = padY
        for (i, line) in lines.enumerated() {
            y += line.place(padX, y, inner)
            if i < lines.count - 1 { y += line.gapAfter }
        }
        return NSSize(width: inner + 2 * padX, height: ceil(y + padY))
    }

    private static func words(_ line: HoverLine) -> [String] {
        switch line {
        case .title(let text, let key): [[text, key].compactMap { $0 }.joined(separator: " ")]
        case .branch(let text, let detached, let dirty):
            [text + (detached ? " detached" : "") + (dirty ? " •" : "")] + (dirty ? [Self.dirtyWords] : [])
        case .state(_, let text, _, let trailing): [[text, trailing].compactMap { $0 }.joined(separator: " · ")]
        case .path(let text), .text(let text): [text]
        }
    }

    private static let dirtyWords = "Uncommitted changes"

    // MARK: Lines

    /// A hyphen no line breaks after (U+2011).
    private static let hyphen = "\u{2011}"

    /// A filled circle `side` wide in `color`.
    private static func dot(_ color: NSColor, side: CGFloat) -> NSImage {
        let color = Theme.resolved(color)
        return NSImage(size: NSSize(width: side, height: side), flipped: false) { rect in
            color.setFill()
            NSBezierPath(ovalIn: rect).fill()
            return true
        }
    }

    /// A one-line label, cut by us (never by AppKit, never wrapped).
    private func label(_ text: String, font: NSFont, color: NSColor) -> NSTextField {
        let field = SidebarText.label(text, font: font, color: color)
        field.lineBreakMode = .byClipping
        addSubview(field)
        return field
    }

    private func image(_ image: NSImage?) -> NSImageView {
        let view = NSImageView(image: image ?? NSImage())
        view.imageScaling = .scaleNone
        addSubview(view)
        return view
    }

    private static var lineHeights: [NSFont: CGFloat] = [:]

    /// One text line's height in `font`, as a label lays it out.
    private static func lineHeight(_ font: NSFont) -> CGFloat {
        if let height = lineHeights[font] { return height }
        let probe = NSTextField(labelWithString: "Xg")
        probe.font = font
        let height = ceil(probe.cell?.cellSize.height ?? font.pointSize + 4)
        lineHeights[font] = height
        return height
    }

    /// `field` showing `text`, its words at `x`, centred on a line `height`
    /// tall from `y`.
    private static func put(_ field: NSTextField, _ text: String, x: CGFloat, y: CGFloat, height: CGFloat) {
        field.stringValue = text
        let font = field.font ?? Design.Font.detail
        let h = lineHeight(font)
        field.frame = NSRect(x: x - inset, y: y + ((height - h) / 2).rounded(),
                             width: SidebarText.width(text, font: font) + 2 * inset, height: h)
    }

    /// An image centred in a `side`-wide column at `x` on a line `height` tall.
    private static func centre(_ view: NSImageView, x: CGFloat, y: CGFloat, side: CGFloat, height: CGFloat) {
        let size = view.image?.size ?? .zero
        view.frame = NSRect(x: x + ((side - size.width) / 2).rounded(), y: y + ((height - size.height) / 2).rounded(),
                            width: size.width, height: size.height)
    }

    private func build(_ line: HoverLine) -> [Line] {
        let iconGap = Design.Space.hoverCardIconGap
        let column = self.column
        let lead = column + iconGap
        switch line {
        case .title(let text, let key):
            let font = Design.Font.hoverCardTitle
            let field = label(text, font: font, color: Design.Color.textPrimary)
            let chip = key.map { KeyChip($0) }
            if let chip { addSubview(chip) }
            let chipWidth = chip?.intrinsicContentSize.width ?? 0
            let keyRoom = chip == nil ? 0 : Design.Space.hoverCardKeyGap + chipWidth
            return [Line(natural: SidebarText.width(text, font: font) + keyRoom, fields: [field],
                         marks: key ?? "") { x, y, width in
                let height = max(Self.lineHeight(font), chip == nil ? 0 : KeyChip.height)
                Self.put(field, SidebarText.end(text, font: font, width: width - keyRoom), x: x, y: y, height: height)
                chip?.frame = NSRect(x: x + width - chipWidth, y: y + (height - KeyChip.height) / 2,
                                     width: chipWidth, height: KeyChip.height)
                return height
            }]
        case .branch(let text, let detached, let dirty):
            let mono = Design.Font.monoDetail, detail = Design.Font.detail
            let glyph = image(detached ? SidebarGlyph.commit(Design.Color.textSecondary)
                                       : SidebarGlyph.branch(Design.Color.textSecondary))
            let field = label(text, font: mono, color: Design.Color.textSecondary)
            let word = detached ? label("detached", font: detail, color: Design.Color.textSecondary) : nil
            let wordRoom = word == nil ? 0 : iconGap + SidebarText.width("detached", font: detail)
            let dot: DotView? = dirty ? DotView(color: Design.Color.gitModified, side: 5) : nil
            if let dot { addSubview(dot) }
            let dotRoom: CGFloat = dot == nil ? 0 : iconGap + 5
            var out = [Line(natural: lead + SidebarText.width(text, font: mono) + wordRoom + dotRoom,
                            fields: [field] + [word].compactMap { $0 }, marks: dirty ? "•" : "") { x, y, width in
                let height = max(Self.lineHeight(mono), word == nil ? 0 : Self.lineHeight(detail))
                Self.centre(glyph, x: x, y: y, side: column, height: height)
                let shown = SidebarText.middle(text, font: mono, width: width - lead - wordRoom - dotRoom)
                Self.put(field, shown, x: x + lead, y: y, height: height)
                var end = x + lead + SidebarText.width(shown, font: mono)
                if let word {
                    Self.put(word, "detached", x: end + iconGap, y: y, height: height)
                    end += wordRoom
                }
                dot?.frame = NSRect(x: end + iconGap, y: y + ((height - 5) / 2).rounded(), width: 5, height: 5)
                return height
            }]
            if dirty {
                let words = label(Self.dirtyWords, font: detail, color: Design.Color.textSecondary)
                out.append(Line(natural: lead + SidebarText.width(Self.dirtyWords, font: detail),
                                  fields: [words]) { x, y, width in
                    let height = Self.lineHeight(detail)
                    Self.put(words, SidebarText.end(Self.dirtyWords, font: detail, width: width - lead),
                             x: x + lead, y: y, height: height)
                    return height
                })
            }
            return out
        case .state(let status, let text, let color, let trailing):
            // A machine on its way up shows the sidebar's arc, not the
            // status symbol; a connected one the rail letter's dot.
            let busy = status == .connecting || status == .reconnecting
            let icon = image(busy ? Design.customSymbol("azulejo.arc", color: status.color, pointSize: 10)
                : status == .connected ? Self.dot(status.color, side: Design.Radius.machineDot * 2 - 1)
                : SidebarGlyph.status(status, pointSize: 11))
            let font = Design.Font.hud, rest = Design.Font.overlayBody
            let field = label(text, font: font, color: color)
            let after = trailing.map { "· " + $0 }
            let more = after.map { label($0, font: rest, color: Design.Color.textSecondary) }
            let wordsWidth = SidebarText.width(text, font: font)
            let natural = lead + wordsWidth + (after.map { iconGap + SidebarText.width($0, font: rest) } ?? 0)
            return [Line(natural: natural, fields: [field] + [more].compactMap { $0 }) { x, y, width in
                let height = max(Self.lineHeight(font), more == nil ? 0 : Self.lineHeight(rest),
                                 icon.image?.size.height ?? 0)
                Self.centre(icon, x: x, y: y, side: column, height: height)
                Self.put(field, SidebarText.end(text, font: font, width: width - lead), x: x + lead, y: y,
                         height: height)
                if let more, let after {
                    let start = lead + min(wordsWidth, width - lead) + iconGap
                    Self.put(more, SidebarText.end(after, font: rest, width: max(0, width - start)), x: x + start, y: y,
                             height: height)
                }
                return height
            }]
        case .path(let text):
            let mono = Design.Font.monoDetail
            let glyph = image(SidebarGlyph.folder(Design.Color.textSecondary))
            let field = label(text, font: mono, color: Design.Color.textSecondary)
            return [Line(natural: lead + SidebarText.width(text, font: mono), fields: [field]) { x, y, width in
                let height = Self.lineHeight(mono)
                Self.centre(glyph, x: x, y: y, side: column, height: height)
                Self.put(field, SidebarText.path(text, font: mono, width: width - lead), x: x + lead, y: y,
                         height: height)
                return height
            }]
        case .text(let text):
            let font = Design.Font.detail
            // Breaks only at spaces: a hyphenated name stays whole.
            let field = NSTextField(wrappingLabelWithString: text.replacingOccurrences(of: "-", with: Self.hyphen))
            field.font = font
            field.textColor = Design.Color.textSecondary
            field.lineBreakMode = .byWordWrapping
            addSubview(field)
            return [Line(natural: SidebarText.width(text, font: font), fields: [field]) { x, y, width in
                field.preferredMaxLayoutWidth = width
                let cell = field.cell?.cellSize(forBounds: NSRect(x: 0, y: 0, width: width + 2 * Self.inset,
                                                                  height: .greatestFiniteMagnitude))
                let height = ceil(cell?.height ?? Self.lineHeight(font))
                field.frame = NSRect(x: x - Self.inset, y: y, width: width + 2 * Self.inset, height: height)
                return height
            }]
        }
    }

    // MARK: Automation

    /// Each line as drawn (cut to fit), " ¦ " between ("" when none).
    static var drawn: String {
        shown?.lines.map { line in
            (line.fields.map(\.stringValue) + [line.marks].filter { !$0.isEmpty }).joined(separator: " ")
                .replacingOccurrences(of: hyphen, with: "-")
        }.joined(separator: " ¦ ") ?? ""
    }

    /// How many text lines each line's words take at their width
    /// (`1|1|1|1`; only a `.text` line may take more), "" when none.
    static var lineCounts: String {
        guard let card = shown else { return "" }
        return card.lines.map { line in
            "\(line.fields.map(Self.lineCount).max() ?? 0)"
        }.joined(separator: "|")
    }

    /// Lines AppKit would cut or wrap itself, or that leave the card's
    /// padding: "none", else their numbers (1-based).
    static var overflow: String {
        guard let card = shown else { return "" }
        let inner = card.bounds.insetBy(dx: Design.Space.hoverCardPaddingX - inset, dy: 0)
        let bad = card.lines.indices.filter { i in
            card.lines[i].fields.contains { field in
                let font = field.font ?? Design.Font.detail
                let room = field.frame.width - 2 * inset
                let wraps = field.cell?.wraps == true
                return (!wraps && SidebarText.width(field.stringValue, font: font) > room + 0.5)
                    || field.frame.minX < inner.minX - 0.5 || field.frame.maxX > inner.maxX + 0.5
                    || (!wraps && field.frame.height != lineHeight(font))
            }
        }
        return bad.isEmpty ? "none" : bad.map { "\($0 + 1)" }.joined(separator: ",")
    }

    /// Its width in points (rounded), "" when none.
    static var width: String { shown.map { "\(Int($0.frame.width.rounded()))" } ?? "" }

    /// How many lines `field`'s words need at its frame's width.
    private static func lineCount(_ field: NSTextField) -> Int {
        let font = field.font ?? Design.Font.detail
        let text = NSAttributedString(string: field.stringValue, attributes: [.font: font])
        func height(_ width: CGFloat) -> CGFloat {
            ceil(text.boundingRect(with: NSSize(width: width, height: .greatestFiniteMagnitude),
                                   options: [.usesLineFragmentOrigin, .usesFontLeading]).height)
        }
        return Int((height(field.frame.width - 2 * inset) / max(1, height(.greatestFiniteMagnitude))).rounded())
    }
}

/// A key chord on a keycap (⌘2): a hover card's, the empty list's hint.
/// Drawn as a ⌘K row draws its chords (`PaletteKeycap`): `chord` type in
/// the secondary text colour, a `hintBadgeBorder` outline, no fill.
@MainActor
final class KeyChip: NSView, ThemeAware {
    static let height: CGFloat = 18
    static let padding: CGFloat = 6
    let text: String

    init(_ text: String) {
        self.text = text
        super.init(frame: .zero)
        setAccessibilityElement(true)
        setAccessibilityRole(.staticText)
        setAccessibilityLabel(text)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var intrinsicContentSize: NSSize {
        NSSize(width: PaletteKeycap.width(text, font: Design.Font.chord, padding: Self.padding), height: Self.height)
    }

    override func draw(_ dirtyRect: NSRect) {
        _ = PaletteKeycap.draw(text, font: Design.Font.chord, at: 0, midY: bounds.midY, height: Self.height,
                               padding: Self.padding)
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    func applyTheme() { needsDisplay = true }
}

/// A workspace's key badge (⌘3) while ⌘ is held alone (`WorkspaceHints`),
/// in a sidebar row and on a rail row: `keyBadge` type, the row's text on
/// the Workspaces | Files track's fill (`chip` is meant for ⌘K's panel and
/// vanishes on a light sidebar).
@MainActor
final class HintBadge: NSView, ThemeAware {
    private let label = NSTextField(labelWithString: "")
    var text: String {
        get { label.stringValue }
        set { label.stringValue = newValue; invalidateIntrinsicContentSize() }
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.chip
        layer?.cornerCurve = .continuous
        label.font = Design.Font.keyBadge
        label.textColor = Design.Color.textPrimary
        label.alignment = .center
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)
        NSLayoutConstraint.activate([
            label.centerXAnchor.constraint(equalTo: centerXAnchor),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var intrinsicContentSize: NSSize {
        NSSize(width: ceil(label.intrinsicContentSize.width) + 12, height: Design.Space.keyBadgeHeight)
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    func applyTheme() {
        layer?.backgroundColor = Design.Color.control.cgColor
    }
}

// MARK: - Overflow

/// A list's edge fades: a mask over its scroll view that fades the rows
/// out over `space.sidebarEdgeFade` at an edge with more rows beyond it.
@MainActor
enum EdgeFade {
    static func apply(to view: NSView, top: Bool, bottom: Bool) {
        view.wantsLayer = true
        guard let layer = view.layer else { return }
        guard top || bottom else { layer.mask = nil; return }
        let mask = (layer.mask as? CAGradientLayer) ?? CAGradientLayer()
        let height = max(1, view.bounds.height)
        let fade = min(Design.Space.sidebarEdgeFade, height / 2) / height
        let clear = CGColor(gray: 0, alpha: 0), solid = CGColor(gray: 0, alpha: 1)
        // The outermost 15 % of each fade stays clear, as in the mockup.
        mask.colors = [top ? clear : solid, top ? clear : solid, solid, solid, bottom ? clear : solid,
                       bottom ? clear : solid]
        mask.locations = [0, NSNumber(value: Double(fade * 0.15)), NSNumber(value: Double(fade)),
                          NSNumber(value: Double(1 - fade)), NSNumber(value: Double(1 - fade * 0.15)), 1]
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        mask.frame = layer.bounds
        // A flipped view's layer runs top down already; an unflipped one's
        // bottom up.
        if !view.isFlipped {
            mask.startPoint = CGPoint(x: 0.5, y: 1)
            mask.endPoint = CGPoint(x: 0.5, y: 0)
        }
        CATransaction.commit()
        layer.mask = mask
    }
}

/// A pill at an edge of a list with rows beyond it: "↓ 9 more · 1 waiting"
/// or "↑ 2 waiting above" in the full sidebar, "↑2" in the rail. A click
/// scrolls to the nearest waiting row that way.
@MainActor
final class EdgePill: NSView, ThemeAware {
    enum Style { case pill, marker }

    var onPress: (() -> Void)?
    private let label = NSTextField(labelWithString: "")
    private let style: Style
    /// Automation: what it says.
    private(set) var text = ""

    init(style: Style) {
        self.style = style
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerCurve = .continuous
        layer?.borderWidth = Design.Border.hairline
        layer?.masksToBounds = false
        label.lineBreakMode = .byClipping
        addSubview(label)
        isHidden = true
        setAccessibilityRole(.button)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// `parts`: plain words, then words in the waiting colour.
    func set(arrow: String, plain: String?, waiting: String?) {
        let out = NSMutableAttributedString()
        let font = style == .pill ? Design.Font.detail : Design.Font.railMarker
        let strong = style == .pill ? Design.Font.sidebarPillStrong : Design.Font.railMarker
        let plainColor = style == .pill ? Design.Color.textSecondary : Design.Color.statusBlocked
        out.append(NSAttributedString(string: arrow + (style == .pill ? " " : ""),
                                      attributes: [.font: font, .foregroundColor: plainColor]))
        if let plain {
            out.append(NSAttributedString(string: plain, attributes: [.font: font, .foregroundColor: plainColor]))
        }
        if let waiting {
            if plain != nil {
                out.append(NSAttributedString(string: " · ", attributes: [.font: font, .foregroundColor: plainColor]))
            }
            out.append(NSAttributedString(string: waiting, attributes: [
                .font: strong, .foregroundColor: Design.Color.statusBlocked,
            ]))
        }
        label.attributedStringValue = out
        text = out.string
        setAccessibilityLabel(text)
        needsLayout = true
    }

    var height: CGFloat { style == .pill ? Design.Space.sidebarPillHeight : Design.Space.railEdgeMarker }

    var fittingWidth: CGFloat {
        ceil(label.attributedStringValue.size().width) + (style == .pill ? 18 : 8)
    }

    override func layout() {
        super.layout()
        layer?.cornerRadius = bounds.height / 2
        let size = label.attributedStringValue.size()
        label.frame = NSRect(x: (bounds.width - ceil(size.width) - 4) / 2, y: (bounds.height - ceil(size.height)) / 2,
                             width: ceil(size.width) + 4, height: ceil(size.height))
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    /// ⌘K's surface, as the hover cards: `palette`, its `border` hairline
    /// (the waiting colour's for a rail marker), a soft shadow on a pill.
    func applyTheme() {
        layer?.backgroundColor = Design.Color.palette.cgColor
        layer?.borderColor = (style == .pill ? Design.Color.border
            : Theme.faded(Design.Color.statusBlocked, 0.5)).cgColor
        if style == .pill {
            layer?.shadowColor = Design.Color.shadow.cgColor
            layer?.shadowOpacity = Float(Design.Alpha.paletteShadow) / 2
            layer?.shadowRadius = 6
            layer?.shadowOffset = CGSize(width: 0, height: -2)
        }
    }

    override func mouseDown(with event: NSEvent) {}
    override func mouseUp(with event: NSEvent) {
        if bounds.contains(convert(event.locationInWindow, from: nil)) { onPress?() }
    }
    override func accessibilityPerformPress() -> Bool { onPress?(); return true }
}

/// Which rows of a scrolling list lie beyond its visible part, and which of
/// them wait: a row counts as out of view once its middle is.
struct Overflow: Equatable {
    var above = 0, aboveWaiting = 0, below = 0, belowWaiting = 0

    init() {}

    init(rows: [(frame: NSRect, waiting: Bool)], visible: NSRect, flipped: Bool) {
        for row in rows {
            let mid = row.frame.midY
            let isAbove = flipped ? mid < visible.minY : mid > visible.maxY
            let isBelow = flipped ? mid > visible.maxY : mid < visible.minY
            if isAbove { above += 1; if row.waiting { aboveWaiting += 1 } }
            if isBelow { below += 1; if row.waiting { belowWaiting += 1 } }
        }
    }

    /// Automation: "above/aboveWaiting below/belowWaiting" ("0/0 9/1").
    var summary: String { "\(above)/\(aboveWaiting) \(below)/\(belowWaiting)" }
}
