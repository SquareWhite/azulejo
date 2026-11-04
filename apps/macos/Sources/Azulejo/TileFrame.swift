import AppKit

/// A tile view that puts controls in its header strip (F28): the preview
/// tile's Preview | Source toggle, the files tile's filter button.
@MainActor
protocol TileHeaderAccessory {
    var headerAccessory: NSView? { get }
}

/// A header accessory that shows extra controls while the pointer is over the
/// header strip (the preview tile's Open and Reveal).
@MainActor
protocol HeaderHoverAware {
    func headerHover(_ inside: Bool)
}

/// A tile view whose own toolbar is its top strip (the browser's chrome, D.6):
/// the frame hides its header strip for it.
@MainActor
protocol TileOwnsChrome {}

/// A tile view with strips across its top that are chrome, not content (an
/// open find bar, the browser's toolbar): mid-motion they ride the top of
/// the visible rect with the header, rather than being cut off with the
/// content when it sticks to the bottom edge. Tiles that are `Findable`
/// get their open find bar without this.
@MainActor
protocol RidesClipTop {
    var clipTopStrips: [NSView] { get }
}

/// Every tile in the container: a card (D.5) with a header strip (kind icon,
/// bold title, muted subtitle — what the core's `tile.header` says) above the
/// tile's own view.
///
/// The container only ever sets this view's frame; it sizes the content below
/// the header in one go, so a terminal's PTY still resizes once per layout
/// change (ADR 0004). Everything a tile view does is forwarded.
@MainActor
class TileFrame: NSView, TileView, ThemeAware {
    static let headerHeight = Design.Space.headerHeight

    let content: any TileView
    let header: TileHeaderView
    var tileID: String { content.tileID }
    var kind: TileKind { content.kind }
    var keyContext: String { content.keyContext }

    /// Tiles with their own chrome (the browser): no header strip.
    private let ownsChrome: Bool
    /// Margin round a terminal's text (`Design.Space.tileContentPad`); other
    /// kinds fill the card edge to edge.
    private let pad: CGFloat
    /// Rounds the card's corners without masking the content (D.5): the
    /// window colour drawn over the four corners. Rounded masking of a layer
    /// tree that holds a Metal surface or a web view costs an offscreen pass.
    let corners = CornerCapLayer()
    /// The focus indicator: unfocused tiles fade towards `Motion.dimFill`
    /// under this layer, header strip and all, while the focused tile stays
    /// at full brightness (`setDim`). It sits under the corner caps and
    /// follows the crop clip with them mid-motion.
    private let dim = CALayer()
    /// The dim's target opacity (0: not dimmed).
    private(set) var dimLevel: CGFloat = 0

    /// Wraps `content`; tiles that take part in motion get a frame that does too.
    static func wrap(_ content: any TileView) -> TileFrame {
        content is MotionAware ? MotionTileFrame(content) : TileFrame(content)
    }

    init(_ content: any TileView) {
        self.content = content
        header = TileHeaderView(kind: content.kind)
        ownsChrome = content is TileOwnsChrome
        pad = content.kind == .terminal ? Design.Space.tileContentPad : 0
        super.init(frame: .zero)
        wantsLayer = true
        if let accessory = (content as? TileHeaderAccessory)?.headerAccessory {
            header.setAccessory(accessory)
        }
        header.isHidden = ownsChrome
        header.menuProvider = { [weak self] in self?.contextMenu() }
        // The header stays above the content, so a terminal's 1:1 motion
        // snapshot (crop style) can't cover it mid-motion; the dim and then
        // the corner caps sit over both, in a subview of their own that comes
        // last. (As sublayers of this view they'd end up under the subviews'
        // layers, which AppKit attaches later: fine on screen with a
        // zPosition, wrong in captures, which draw in list order.)
        addSubview(content)
        addSubview(header)
        addSubview(overlays)
        dim.opacity = 0
        dim.actions = ["opacity": NSNull(), "position": NSNull(), "bounds": NSNull(), "backgroundColor": NSNull()]
        overlays.layer?.addSublayer(dim)
        overlays.layer?.addSublayer(corners)
        Theme.track(self)
    }

    /// Holds the dim and the corner caps over everything else in the card.
    private let overlays = TileOverlaysView()

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { false }

    /// Where the content sits in a card of `size`: below the header, inside
    /// a terminal's margin (at the sides and bottom, half of it under the
    /// header).
    func contentFrame(for size: NSSize) -> NSRect {
        let h = ownsChrome ? 0 : min(Self.headerHeight, max(size.height, 0))
        let body = max(size.height - h, 0)
        let p = min(pad, size.width / 4, body / 4)
        return NSRect(x: p, y: p, width: max(size.width - 2 * p, 0), height: max(body - p - p / 2, 0))
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        let h = ownsChrome ? 0 : min(Self.headerHeight, max(newSize.height, 0))
        header.frame = NSRect(x: 0, y: newSize.height - h, width: newSize.width, height: h)
        content.frame = contentFrame(for: newSize)
        // Same coordinates as this view's bounds, so the caps and the dim
        // can follow a crop clip in them.
        overlays.frame = NSRect(origin: .zero, size: newSize)
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        corners.frame = CGRect(origin: .zero, size: newSize)
        dim.frame = CGRect(origin: .zero, size: newSize)
        CATransaction.commit()
    }

    // MARK: Focus dim

    /// Dims the tile to `level` (0: full brightness), cross-fading over
    /// `duration` with the motion curve, or at once when it's 0.
    func setDim(_ level: CGFloat, duration: CFTimeInterval) {
        let fill = Motion.dimFill
        var colour: CGColor?
        // theme: re-read on every focus change; applyTheme() sets it too.
        effectiveAppearance.performAsCurrentDrawingAppearance { colour = fill.cgColor }
        if dim.backgroundColor != colour { dim.backgroundColor = colour }
        guard level != dimLevel else { return }
        let from = dim.presentation()?.opacity ?? dim.opacity
        dimLevel = level
        dim.opacity = Float(level)
        dim.removeAnimation(forKey: Self.dimKey)
        guard duration > 0 else { return }
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = from
        fade.toValue = Float(level)
        Motion.configure(fade, duration: duration)
        dim.add(fade, forKey: Self.dimKey)
    }

    private static let dimKey = "azj.dim"

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        Theme.reapply(self)
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        Theme.reapply(self)
    }

    func applyTheme() {
        // A terminal's margin shows the terminal's own background, so it
        // reads as part of the terminal whatever its colours: the theme's
        // palette or the user's Ghostty theme.
        let body = content.kind == .terminal
            ? (TerminalRuntime.shared?.background ?? Design.Color.tile) : Design.Color.tile
        layer?.backgroundColor = body.cgColor
        corners.update(fill: Design.Color.window.cgColor, radius: Design.Radius.tile,
                       scale: window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2)
        dim.backgroundColor = Motion.dimFill.cgColor
    }

    // MARK: Corners in motion

    /// Crop motion animates this frame's bounds (a clip over 1:1 content);
    /// the caps, the dim and the header strip follow the visible rect from
    /// `from` to `to` (bounds coordinates) with the motion's timing.
    func animateCorners(from: CGRect, to: CGRect, duration: CFTimeInterval, holdAtEnd: Bool) {
        animateHeader(from: from, to: to, duration: duration, holdAtEnd: holdAtEnd)
        let position = CABasicAnimation(keyPath: "position")
        position.fromValue = NSValue(point: CGPoint(x: from.midX, y: from.midY))
        position.toValue = NSValue(point: CGPoint(x: to.midX, y: to.midY))
        let size = CABasicAnimation(keyPath: "bounds.size")
        size.fromValue = NSValue(size: from.size)
        size.toValue = NSValue(size: to.size)
        let group = CAAnimationGroup()
        group.animations = [position, size]
        if holdAtEnd {
            group.fillMode = .forwards
            group.isRemovedOnCompletion = false
        }
        Motion.configure(group, duration: duration)
        corners.add(group, forKey: Self.cornerMotionKey)
        dim.add(group, forKey: Self.cornerMotionKey)
        animateStrips(from: from, to: to, duration: duration, holdAtEnd: holdAtEnd)
    }

    func stopCornerMotion() {
        corners.removeAnimation(forKey: Self.cornerMotionKey)
        dim.removeAnimation(forKey: Self.cornerMotionKey)
        header.layer?.removeAnimation(forKey: Self.cornerMotionKey)
        header.badge?.layer?.removeAnimation(forKey: Self.cornerMotionKey)
        for strip in stripsInMotion { strip.layer?.removeAnimation(forKey: Self.cornerMotionKey) }
        stripsInMotion = []
    }

    /// The content's chrome strips (an open find bar, the browser's toolbar).
    private var clipTopStrips: [NSView] {
        if let own = content as? RidesClipTop { return own.clipTopStrips }
        guard let bar = (content as? Findable)?.findBar, bar.isOpen, bar.superview != nil else { return [] }
        return [bar]
    }

    private var stripsInMotion: [NSView] = []

    /// Find bars and the browser's toolbar keep to the top of the visible
    /// rect, under the header, as the header does: each moves by as much
    /// as the clip's top edge is off its resting place. Vertically only,
    /// so a find bar stays over the columns it searches.
    private func animateStrips(from: CGRect, to: CGRect, duration: CFTimeInterval, holdAtEnd: Bool) {
        let strips = clipTopStrips
        stripsInMotion = strips
        // `to` is where the clip rests at the end (the full bounds, or a
        // leaving tile's last clip); strips sit at their place for it.
        let rest = CGRect(origin: .zero, size: bounds.size)
        func lift(_ clip: CGRect) -> CGFloat { clip.maxY - rest.maxY }
        for strip in strips {
            guard let layer = strip.layer, let parent = strip.superview else { continue }
            // Layer positions follow the parent view's flip.
            let sign: CGFloat = parent.isFlipped ? -1 : 1
            let y = layer.position.y
            let move = CABasicAnimation(keyPath: "position.y")
            move.fromValue = y + sign * lift(from)
            move.toValue = y + sign * lift(to)
            if holdAtEnd {
                move.fillMode = .forwards
                move.isRemovedOnCompletion = false
            }
            Motion.configure(move, duration: duration)
            layer.add(move, forKey: Self.cornerMotionKey)
        }
    }

    /// The header strip is chrome, not content: it rides the top of the
    /// visible rect. Content sticks to the edge that moves least, so a tile
    /// growing upwards (a swap out of the stack) would otherwise show its
    /// body with the strip still cut off above the clip (B4b). The strip
    /// spans the visible width, its title centred in it (left-aligned when
    /// it has controls, as at rest).
    private func animateHeader(from: CGRect, to: CGRect, duration: CFTimeInterval, holdAtEnd: Bool) {
        guard !header.isHidden, let layer = header.layer else { return }
        let h = header.frame.height, w = header.frame.width
        guard h > 0, w > 0 else { return }
        let a = layer.anchorPoint
        let centred = header.centred
        func strip(_ clip: CGRect) -> (position: CGPoint, bounds: CGRect) {
            let frame = CGRect(x: clip.minX, y: clip.maxY - h, width: clip.width, height: h)
            let bounds = CGRect(x: centred ? (w - clip.width) / 2 : 0, y: 0, width: clip.width, height: h)
            return (Motion.position(of: frame, anchorPoint: a), bounds)
        }
        let start = strip(from), end = strip(to)
        let position = CABasicAnimation(keyPath: "position")
        position.fromValue = NSValue(point: start.position)
        position.toValue = NSValue(point: end.position)
        let bounds = CABasicAnimation(keyPath: "bounds")
        bounds.fromValue = NSValue(rect: start.bounds)
        bounds.toValue = NSValue(rect: end.bounds)
        let group = CAAnimationGroup()
        group.animations = [position, bounds]
        if holdAtEnd {
            group.fillMode = .forwards
            group.isRemovedOnCompletion = false
        }
        Motion.configure(group, duration: duration)
        layer.add(group, forKey: Self.cornerMotionKey)
        animateBadge(from: from, to: to, width: w, centred: centred, duration: duration, holdAtEnd: holdAtEnd)
    }

    /// The badge keeps to the visible strip's right end. The strip's bounds
    /// shift to keep a centred title centred, which would carry the badge
    /// past the clip; this moves it back by as much.
    private func animateBadge(from: CGRect, to: CGRect, width w: CGFloat, centred: Bool,
                              duration: CFTimeInterval, holdAtEnd: Bool) {
        guard let layer = header.badge?.layer else { return }
        func shift(_ clip: CGRect) -> CGFloat { centred ? (clip.width - w) / 2 : clip.width - w }
        let x = layer.position.x
        let move = CABasicAnimation(keyPath: "position.x")
        move.fromValue = x + shift(from)
        move.toValue = x + shift(to)
        if holdAtEnd {
            move.fillMode = .forwards
            move.isRemovedOnCompletion = false
        }
        Motion.configure(move, duration: duration)
        layer.add(move, forKey: Self.cornerMotionKey)
    }

    private static let cornerMotionKey = "azj.corners"

    // MARK: TileView

    /// What the content view last got, without the header and agent state:
    /// both change with every command a terminal runs (the header strip and
    /// the attention overlay show them), and the content doesn't use them.
    /// Forwarding them re-ran a terminal's attach bookkeeping on each change,
    /// mid-motion too.
    private var forwarded: TileModel?

    private static func bare(_ tile: TileModel) -> TileModel {
        var bare = tile
        bare.header = nil
        bare.agent = nil
        return bare
    }

    /// The model the content view was made with.
    func prime(_ tile: TileModel) {
        header.show(tile)
        forwarded = Self.bare(tile)
    }

    func update(_ tile: TileModel) {
        header.show(tile)
        let bare = Self.bare(tile)
        guard bare != forwarded else { return }
        forwarded = bare
        content.update(tile)
    }

    func setFocused(_ focused: Bool) {
        header.focused = focused
        if focused { TileFocus.returnToFindBar(of: content) }
        content.setFocused(focused)
    }

    func tileWillClose() { content.tileWillClose() }
}

/// Right-click on the header strip: the tile's menu (Split, Zoom, Rename,
/// Copy, Reveal, Close, Kill). A tile with its own chrome (the browser)
/// merges these into its chrome's menu. The terminal's body keeps its own
/// right-click (libghostty, mouse-reporting apps).
extension TileFrame: ContextMenuProviding {
    func contextMenu() -> NSMenu? {
        if let own = content as? ContextMenuProviding { return own.contextMenu() }
        guard let controller = ContextMenus.app?.controller(for: window) else { return nil }
        return ContextMenus.tileMenu(tileID, in: controller)
    }

    /// Where the menu opens, in window coordinates: the header strip, or the
    /// top of a tile that draws its own chrome.
    var menuAnchor: NSRect {
        let strip = header.isHidden
            ? NSRect(x: 0, y: bounds.height - Self.headerHeight, width: bounds.width, height: Self.headerHeight)
            : header.frame
        return convert(strip, to: nil)
    }
}

/// A frame around a tile that freezes behind a snapshot while it moves. The
/// container speaks in this frame's coordinates; the snapshot lives in the
/// content's, below the header and inside a terminal's margin.
@MainActor
final class MotionTileFrame: TileFrame, MotionAware {
    private var aware: MotionAware { content as! MotionAware }

    @discardableResult
    func motionWillStart(freeze: Bool, fill: Bool) -> Bool {
        aware.motionWillStart(freeze: freeze, fill: fill)
    }

    var snapshotFrame: CGRect? { aware.snapshotFrame.map { content.convert($0, to: self) } }

    func placeSnapshot(_ frame: CGRect, cover: CGRect) {
        aware.placeSnapshot(convert(frame, to: content), cover: convert(cover, to: content))
    }

    func liveResizeWillStart(final: CGSize) { aware.liveResizeWillStart(final: contentFrame(for: final).size) }
    func liveResizeDidEnd() { aware.liveResizeDidEnd() }
    var resizesLive: Bool { aware.resizesLive }
    func revealWhenFresh(clock: MotionClock) { aware.revealWhenFresh(clock: clock) }
    func motionDidEnd(clock: MotionClock) { aware.motionDidEnd(clock: clock) }
    func motionCancelled() { aware.motionCancelled() }
    var surfaceResizes: Int { aware.surfaceResizes }
    var headerInset: CGFloat { bounds.height - content.frame.maxY }
    var contentLeft: CGFloat { content.frame.minX }
}

extension TileFrame: AutomatableTile {
    func automate(_ verb: String, _ arg: String) -> String? {
        guard let inner = content as? AutomatableTile else {
            return "the focused tile takes no automation verbs"
        }
        return inner.automate(verb, arg)
    }

    func automationState() -> [String: String] {
        var state = (content as? AutomatableTile)?.automationState() ?? [:]
        state["tile.header.title"] = header.titleText
        state["tile.header.subtitle"] = header.subtitleText
        state["tile.header.shown"] = header.shownSubtitle
        state["tile.header.focused"] = header.focused ? "yes" : "no"
        // Room the strip keeps clear at its right end (the badge; 0 without).
        state["tile.header.inset"] = "\(Int(header.trailingInset.rounded()))"
        state["tile.dim"] = dimLevel > 0 ? "yes" : "no"
        state["tile.overlays.top"] = subviews.last === overlays && layer?.sublayers?.last === overlays.layer
            ? "yes" : "no"
        return state
    }
}

/// The strip across the top of a tile: icon, bold title, " — " and a muted
/// subtitle (a path keeps its last folder), then the tile's own controls. Centred
/// like the mockup's terminal tiles; left-aligned when the tile puts controls
/// on the right (preview, files); a hairline underneath. Every strip is drawn
/// the same: the focused tile's is set apart by the dim over the others
/// (`TileFrame.setDim`), which turns the mockup's focused strip colour into
/// its unfocused one.
@MainActor
final class TileHeaderView: NSView, ThemeAware {
    /// The tile's right-click menu (`TileFrame.contextMenu`).
    var menuProvider: (() -> NSMenu?)?
    private let icon = NSImageView()
    private let label = NSTextField(labelWithString: "")
    private let hairline = CALayer()
    private var accessory: NSView?
    private(set) var titleText = ""
    private(set) var subtitleText = ""

    /// Whether this is the focused tile's strip (automation; the strip looks
    /// the same either way).
    var focused = false

    init(kind: TileKind) {
        super.init(frame: .zero)
        wantsLayer = true
        icon.image = NSImage(systemSymbolName: Self.symbol(kind), accessibilityDescription: kind.rawValue)
        icon.symbolConfiguration = .init(pointSize: Design.Symbol.header, weight: .regular)
        // Paths are fitted before they get here (`fitPath`); what's still too
        // long (a long title or command) loses its end.
        label.lineBreakMode = .byTruncatingTail
        label.maximumNumberOfLines = 1
        label.cell?.truncatesLastVisibleLine = true
        addSubview(icon)
        addSubview(label)
        layer?.addSublayer(hairline)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func menu(for event: NSEvent) -> NSMenu? { menuProvider?() }

    /// Title and icon centred in the strip (no controls on the right).
    var centred: Bool { accessory == nil }

    func setAccessory(_ view: NSView) {
        accessory?.removeFromSuperview()
        accessory = view
        view.translatesAutoresizingMaskIntoConstraints = true
        addSubview(view)
        needsLayout = true
    }

    /// Shows the core's header for `tile`, or a fallback before it has one.
    func show(_ tile: TileModel) {
        let header = tile.header
        let title = header?.title ?? tile.title ?? tile.kind.rawValue
        let subtitle = header?.subtitle ?? ""
        guard title != titleText || subtitle != subtitleText else { return }
        titleText = title
        subtitleText = subtitle
        toolTip = subtitle.isEmpty ? title : "\(title) — \(subtitle)"
        render()
    }

    /// The agent badge, or a blocked tile's icon (the attention overlay makes
    /// and fills it), at the strip's right end; the title keeps clear of it.
    /// None by default: the title gets the whole strip. A subview, so it
    /// rides with the strip when the strip follows a crop clip mid-motion (F6).
    var badge: NSView? {
        didSet {
            if oldValue !== badge, oldValue?.superview === self { oldValue?.removeFromSuperview() }
            if let badge, badge.superview !== self { addSubview(badge) }
            needsLayout = true
        }
    }

    /// Room kept clear at the strip's right end for the badge (0 without one).
    private(set) var trailingInset: CGFloat = 0

    /// Automation: the subtitle as drawn after fitting ("…/T/azj/azulejo").
    private(set) var shownSubtitle = ""

    private func render() {
        labelKey = nil
        widths = [:]
        needsLayout = true
    }

    /// Text widths for the current title and subtitle, and what the label
    /// shows with its natural size. Reflow motion lays the strip out every
    /// frame, so it measures and redraws text only when that changes.
    private var widths: [String: CGFloat] = [:]
    private var labelKey: String?
    private var labelNatural = CGSize.zero

    private func width(_ string: String, _ font: NSFont) -> CGFloat {
        let key = "\(font.pointSize)\u{0}\(font.fontName)\u{0}\(string)"
        if let w = widths[key] { return w }
        if widths.count > 64 { widths = [:] }
        let w = NSAttributedString(string: string, attributes: [.font: font]).size().width
        widths[key] = w
        return w
    }

    /// The subtitle as it fits in `room` points after the title. A path
    /// loses folders from its front ("…/work/azulejo"), so the folder that
    /// matters stays; anything else, and a path whose last folder alone is
    /// too long, is cut at the end by the label.
    private func fittedSubtitle(room: CGFloat) -> String {
        guard !subtitleText.isEmpty else { return "" }
        let subtitleRoom = room - ceil(width(titleText, Design.Font.headerTitle)) - 4
        return Self.fitPath(subtitleText, room: subtitleRoom) { width(" — \($0)", Design.Font.headerSubtitle) }
    }

    private func text(subtitle: String) -> NSAttributedString {
        let text = NSMutableAttributedString(string: titleText, attributes: [
            .font: Design.Font.headerTitle,
            .foregroundColor: Design.Color.textPrimary,
        ])
        if !subtitleText.isEmpty {
            text.append(NSAttributedString(string: " — \(subtitle)", attributes: [
                .font: Design.Font.headerSubtitle, .foregroundColor: Design.Color.textMuted,
            ]))
        }
        return text
    }

    /// `path` as it fits in `room` (measured by `width`): whole, else with
    /// leading folders swapped for "…", always keeping the last one. Text
    /// that isn't a path (a command) comes back unchanged.
    static func fitPath(_ path: String, room: CGFloat, width: (String) -> CGFloat) -> String {
        guard path.hasPrefix("/") || path.hasPrefix("~/"), width(path) > room else { return path }
        let parts = path.split(separator: "/", omittingEmptySubsequences: true)
        guard parts.count > 1 else { return path }
        for start in 1..<parts.count {
            let candidate = "…/" + parts[start...].joined(separator: "/")
            if width(candidate) <= room { return candidate }
        }
        // Not even "…/last" fits: the last folder alone, cut at its end.
        return String(parts[parts.count - 1])
    }

    override func layout() {
        super.layout()
        Perf.measure("header.layout") { layoutStrip() }
    }

    private func layoutStrip() {
        let h = bounds.height
        let iconSize: CGFloat = 14, spacing: CGFloat = 6, margin: CGFloat = 10
        trailingInset = 0
        if let badge {
            let size = badge.fittingSize, inset = Design.Space.badgeInset
            badge.frame = NSRect(x: bounds.width - inset - size.width, y: (h - size.height) / 2,
                                 width: size.width, height: size.height)
            trailingInset = size.width + inset
        }
        var right = bounds.width - margin - trailingInset
        if let accessory {
            // The tile's controls show whole or not at all: below
            // `headerTitleMin` of room for the title they hide (Preview |
            // Source, the files buttons), never clipped to "Prev".
            let size = accessory.fittingSize
            let titleRoom = right - size.width - spacing - margin - iconSize - spacing
            accessory.isHidden = titleRoom < Design.Space.headerTitleMin
            if !accessory.isHidden {
                accessory.frame = NSRect(x: right - size.width, y: (h - size.height) / 2,
                                         width: size.width, height: size.height)
                right -= size.width + spacing
            }
        }
        let room = max(right - margin - iconSize - spacing, 0)
        shownSubtitle = fittedSubtitle(room: room - 4)
        let key = "\(titleText)\u{0}\(shownSubtitle)"
        if key != labelKey {
            labelKey = key
            label.attributedStringValue = text(subtitle: shownSubtitle)
            // Measured from the text itself: a truncating label's intrinsic
            // width follows its current frame, so a strip that grew kept the "…".
            labelNatural = label.attributedStringValue.size()
        }
        let natural = labelNatural
        let labelHeight = ceil(natural.height)
        let labelWidth = min(ceil(natural.width) + 4, room)
        // Centred over the whole strip unless controls sit on the right; kept
        // clear of anything over the strip's right end.
        let group = iconSize + spacing + labelWidth
        let left = accessory != nil ? margin : max(margin, min((bounds.width - group) / 2, right - group))
        icon.frame = NSRect(x: left, y: (h - iconSize) / 2, width: iconSize, height: iconSize)
        label.frame = NSRect(x: left + iconSize + spacing, y: (h - labelHeight) / 2,
                             width: labelWidth, height: labelHeight)
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        hairline.frame = CGRect(x: 0, y: 0, width: bounds.width, height: 1 / max(window?.backingScaleFactor ?? 2, 1))
        CATransaction.commit()
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    // MARK: Hover

    private var tracking: NSTrackingArea?

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        guard accessory is HeaderHoverAware else { return }
        let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect],
                                  owner: self, userInfo: nil)
        addTrackingArea(area)
        tracking = area
    }

    override func mouseEntered(with event: NSEvent) { hover(true) }
    override func mouseExited(with event: NSEvent) { hover(false) }

    /// Shows or hides the accessory's hover controls.
    func hover(_ inside: Bool) {
        guard let aware = accessory as? HeaderHoverAware else { return }
        aware.headerHover(inside)
        needsLayout = true
    }

    func applyTheme() {
        layer?.backgroundColor = Design.Color.header.cgColor
        hairline.backgroundColor = Design.Color.headerHairline.cgColor
        icon.contentTintColor = Design.Color.textSecondary
        render()
    }

    private static func symbol(_ kind: TileKind) -> String { Design.Icon.for(kind) }
}

/// A small two-way segmented pill (the mockup's Preview | Source): a track
/// with the selected segment filled.
@MainActor
final class PillToggle: NSView, ThemeAware {
    var onChange: (Int) -> Void = { _ in }
    var selectedSegment = 0 { didSet { if selectedSegment != oldValue { restyle() } } }
    private let buttons: [NSButton]
    private let fill = CALayer()

    init(labels: [String]) {
        buttons = labels.map { NSButton(title: $0, target: nil, action: nil) }
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.pill
        fill.cornerRadius = Design.Radius.pill - 2
        layer?.addSublayer(fill)
        for (i, button) in buttons.enumerated() {
            button.isBordered = false
            button.tag = i
            button.target = self
            button.action = #selector(clicked(_:))
            addSubview(button)
        }
        Theme.track(self)
    }

    func applyTheme() { restyle() }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private static let height: CGFloat = 20, inset: CGFloat = 2, padding: CGFloat = 8

    private func width(_ button: NSButton) -> CGFloat {
        ceil((button.title as NSString).size(withAttributes: [.font: Design.Font.caption]).width) + 2 * Self.padding
    }

    override var fittingSize: NSSize {
        NSSize(width: buttons.map(width).reduce(0, +) + 2 * Self.inset, height: Self.height)
    }

    override var intrinsicContentSize: NSSize { fittingSize }

    override func layout() {
        super.layout()
        var x = Self.inset
        let h = bounds.height - 2 * Self.inset
        for button in buttons {
            let w = width(button)
            button.frame = NSRect(x: x, y: Self.inset, width: w, height: h)
            x += w
        }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        if buttons.indices.contains(selectedSegment) { fill.frame = buttons[selectedSegment].frame }
        CATransaction.commit()
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    /// Also this view's applyTheme().
    private func restyle() {
        for (i, button) in buttons.enumerated() {
            button.attributedTitle = NSAttributedString(string: button.title, attributes: [
                .font: Design.Font.caption,
                .foregroundColor: i == selectedSegment ? Design.Color.textPrimary : Design.Color.textMuted,
            ])
        }
        // theme: restyle() is applyTheme().
        effectiveAppearance.performAsCurrentDrawingAppearance {
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            layer?.backgroundColor = Design.Color.previewToggleTrack.cgColor
            fill.backgroundColor = Design.Color.previewToggleSelected.cgColor
            CATransaction.commit()
        }
        needsLayout = true
    }

    @objc private func clicked(_ sender: NSButton) {
        guard sender.tag != selectedSegment else { return }
        selectedSegment = sender.tag
        onChange(sender.tag)
    }
}

/// The top of a tile card: the focus dim and the corner caps. Never takes
/// clicks, and doesn't clip, so the caps can follow a crop clip that
/// reaches past the card's resting bounds.
@MainActor
final class TileOverlaysView: NSView {
    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        clipsToBounds = false
        layer?.masksToBounds = false
        setAccessibilityElement(false)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { false }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

/// The window colour over a card's four corners, clear in between: a
/// nine-slice image, so it's one cheap layer at any size and its corners keep
/// their radius while the frame animates.
final class CornerCapLayer: CALayer {
    private var drawn: (fill: CGColor, radius: CGFloat, scale: CGFloat)?

    override init() {
        super.init()
        contentsGravity = .resize
    }

    override init(layer: Any) { super.init(layer: layer) }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func update(fill: CGColor, radius: CGFloat, scale: CGFloat) {
        if let drawn, drawn.fill == fill, drawn.radius == radius, drawn.scale == scale { return }
        drawn = (fill, radius, scale)
        let side = Int(((2 * radius + 2) * scale).rounded(.up))
        guard radius > 0, let space = CGColorSpace(name: CGColorSpace.sRGB), let ctx = CGContext(
            data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: 0,
            space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { contents = nil; return }
        let rect = CGRect(x: 0, y: 0, width: side, height: side)
        let r = radius * scale
        ctx.setFillColor(fill)
        ctx.fill(rect)
        ctx.setBlendMode(.clear)
        ctx.addPath(CGPath(roundedRect: rect, cornerWidth: r, cornerHeight: r, transform: nil))
        ctx.fillPath()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        contents = ctx.makeImage()
        contentsScale = scale
        let unit = 1 / CGFloat(side)
        let edge = r * unit
        contentsCenter = CGRect(x: edge, y: edge, width: max(1 - 2 * edge, unit), height: max(1 - 2 * edge, unit))
        CATransaction.commit()
    }
}
