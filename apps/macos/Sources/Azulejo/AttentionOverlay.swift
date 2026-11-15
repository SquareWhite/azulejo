import AppKit

/// Colours for agent state, shared by tile badges, the sidebar, the tab bar
/// and the picker. Calm on purpose: only "blocked" is loud.
@MainActor
enum AttentionStyle {
    static let blocked = Design.Status.blocked.color
    static let done = Design.Status.done.color
    static let working = Design.Status.working.color
    static let idle = Design.Status.idle.color

    /// `[appearance] tile_agent_badge` (off by default): the full badge
    /// ("● claude · working") in terminal tile headers. Off, a header shows
    /// only the blocked icon, on blocked tiles. Follows config reloads.
    static var tileBadge = false {
        didSet {
            if tileBadge != oldValue { NotificationCenter.default.post(name: tileBadgeChanged, object: nil) }
        }
    }
    static let tileBadgeChanged = Notification.Name("AzulejoTileBadgeChanged")

    /// The dot for a tile, or nil when there's no agent worth showing.
    static func color(for tile: TileModel) -> NSColor? {
        guard let agent = tile.agent else { return nil }
        switch agent.status {
        case .blocked: return blocked
        case .done: return agent.attention ? done : idle
        case .working: return working
        case .idle: return agent.attention ? done : nil
        case .unknown: return nil
        }
    }

    /// Per-scope rollup for the sidebar and tab bar.
    struct Rollup: Equatable {
        var blocked = 0
        var attention = 0
        var working = 0

        init(_ tiles: [TileModel]) {
            for tile in tiles {
                guard let agent = tile.agent else { continue }
                if agent.status == .blocked { blocked += 1 }
                if agent.attention { attention += 1 }
                if agent.status == .working { working += 1 }
            }
        }

        /// A coloured "● n" suffix, or nil when nothing's going on.
        @MainActor var marker: NSAttributedString? {
            let (color, count): (NSColor, Int)
            if blocked > 0 {
                (color, count) = (AttentionStyle.blocked, blocked)
            } else if attention > 0 {
                (color, count) = (AttentionStyle.done, attention)
            } else if working > 0 {
                (color, count) = (AttentionStyle.working.withAlphaComponent(Design.Alpha.workingMarker), 0)
            } else {
                return nil
            }
            return NSAttributedString(
                string: count > 0 ? " ● \(count)" : " ●",
                attributes: [.foregroundColor: color, .font: Design.Font.badge])
        }
    }
}

/// Agent state on the tiles: a thin coloured border on tiles that need a
/// look, and at the right end of a terminal tile's header strip either the
/// blocked icon (blocked tiles only; the default) or, with `[appearance]
/// tile_agent_badge = true`, the agent's name and state on every agent tile.
/// Either sits clear of the terminal's text: it's the strip's subview, so it
/// moves with it and the title keeps clear of it. The overlay lies over the
/// tile container and passes every click through.
@MainActor
final class AttentionOverlayView: NSView, ThemeAware {
    /// A tile's header strip, so the badge can sit in it.
    var header: ((String) -> TileHeaderView?)?
    /// Badges by tile (`tile_agent_badge` on); each lives in its tile's
    /// header strip.
    private var badges: [String: AgentBadgeView] = [:]
    /// Blocked icons by tile (`tile_agent_badge` off), in the header strips.
    private var marks: [String: StatusIconView] = [:]
    private var borders: [String: NSView] = [:]
    /// Agent state, for automation: tile id → "blocked!" / "working" / …
    private(set) var shown: [String: String] = [:]
    /// What each tile's header carries, for automation: tile id → "badge",
    /// "dot" (a narrow tile's badge) or "blocked" (the blocked icon).
    private(set) var inHeader: [String: String] = [:]

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        Theme.track(self)
        NotificationCenter.default.addObserver(
            self, selector: #selector(tileBadgeChanged), name: AttentionStyle.tileBadgeChanged, object: nil)
    }

    @objc private func tileBadgeChanged() { apply(tab, frames: frames) }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    private var tab: TabModel?
    private var frames: [String: CGRect] = [:]

    /// Borders and badge dots take their colours again.
    func applyTheme() { apply(tab, frames: frames) }

    /// Puts the badges where the tiles are now (window resized, motion ended).
    func relayout(frames: [String: CGRect]) { apply(tab, frames: frames) }

    /// `frames` are the visible tiles' rects in this view's coordinates (the
    /// overlay has the container's frame).
    func apply(_ tab: TabModel?, frames: [String: CGRect]) {
        self.tab = tab
        self.frames = frames
        var seen = Set<String>()
        var withBadge = Set<String>(), withMark = Set<String>()
        var state: [String: String] = [:]
        var carried: [String: String] = [:]
        for (id, frame) in frames {
            guard let tile = tab?.tile(id), let agent = tile.agent,
                  agent.status != .unknown || agent.attention
            else { continue }
            seen.insert(id)
            if AttentionStyle.tileBadge {
                withBadge.insert(id)
                let badge = badges[id] ?? AgentBadgeView()
                badges[id] = badge
                badge.apply(tile)
                // A narrow tile gets the dot alone, so the title keeps some room.
                badge.compact = false
                if badge.fittingSize.width > frame.width * 0.45 { badge.compact = true }
                place(badge, tile: id, frame: frame)
                carried[Short.id(id)] = badge.compact ? "dot" : "badge"
            } else if agent.status == .blocked {
                // F11: a blocked agent can be found at a glance in a busy
                // tab, attention seen or not. The icon alone, no text.
                withMark.insert(id)
                let mark = marks[id] ?? StatusIconView(.blocked) { Self.blockedIcon() }
                marks[id] = mark
                mark.toolTip = "\(agent.name.flatMap { $0.isEmpty ? nil : $0 } ?? "agent") · blocked"
                place(mark, tile: id, frame: frame)
                carried[Short.id(id)] = "blocked"
            }

            let focused = tab?.focused == id
            if agent.attention, !focused || agent.status == .blocked {
                let border = borders[id] ?? {
                    let v = NSView()
                    v.wantsLayer = true
                    v.layer?.cornerRadius = Design.Radius.focus
                    v.layer?.borderWidth = Design.Border.attention
                    addSubview(v, positioned: .below, relativeTo: nil)
                    borders[id] = v
                    return v
                }()
                // theme: apply() runs again from applyTheme().
                border.layer?.borderColor = (agent.status == .blocked
                    ? AttentionStyle.blocked : AttentionStyle.done).withAlphaComponent(Design.Alpha.attentionBorder).cgColor
                border.frame = frame.insetBy(dx: -1, dy: -1)
            } else if let border = borders.removeValue(forKey: id) {
                border.removeFromSuperview()
            }
            state[Short.id(id)] = agent.status.rawValue + (agent.attention ? "!" : "")
        }
        for (id, badge) in badges where !withBadge.contains(id) {
            unplace(badge, tile: id)
            badges[id] = nil
        }
        for (id, mark) in marks where !withMark.contains(id) {
            unplace(mark, tile: id)
            marks[id] = nil
        }
        for (id, border) in borders where !seen.contains(id) {
            border.removeFromSuperview()
            borders[id] = nil
        }
        shown = state
        inHeader = carried
    }

    /// The blocked symbol as the tab strip draws it: the "!" in the strip's
    /// colour (the window's when the strip is see-through), the disc in the
    /// status colour. A one-colour palette would fill the "!" in too.
    private static func blockedIcon() -> NSImage? {
        let status = Design.Status.blocked
        let strip = Theme.resolved(Design.Color.header)
        let mark = strip.alphaComponent < 1 ? Theme.resolved(Design.Color.window) : strip
        let config = NSImage.SymbolConfiguration(pointSize: Design.Symbol.header, weight: .bold)
            .applying(.init(paletteColors: [mark, Theme.resolved(status.color)]))
        return NSImage(systemSymbolName: status.symbol, accessibilityDescription: status.rawValue)?
            .withSymbolConfiguration(config)
    }

    /// `view` at the right end of the tile's header strip, which lays it out
    /// and keeps its title clear of it; over the tile's top right corner when
    /// it has no strip.
    private func place(_ view: NSView, tile id: String, frame: CGRect) {
        if let strip = header?(id), !strip.isHidden {
            strip.badge = view
            return
        }
        if view.superview !== self { addSubview(view) }
        let size = view.fittingSize, inset = Design.Space.badgeInset
        let top = min(TileFrame.headerHeight, frame.height)
        view.frame = NSRect(x: frame.maxX - size.width - inset, y: frame.maxY - top + (top - size.height) / 2,
                            width: size.width, height: size.height)
    }

    private func unplace(_ view: NSView, tile id: String) {
        if let strip = header?(id), strip.badge === view { strip.badge = nil }
        view.removeFromSuperview()
    }
}

/// "● claude · blocked" in a tile's header strip; just "●" when `compact`.
/// Working agents get a slow, gentle pulse on the dot; nothing else moves.
@MainActor
final class AgentBadgeView: NSView, ThemeAware {
    private let dot = CALayer()
    private let label = NSTextField(labelWithString: "")
    private var pulsing = false
    private var labelConstraints: [NSLayoutConstraint] = []
    private var compactConstraints: [NSLayoutConstraint] = []
    private static let dotSize = Design.Radius.badgeDot * 2

    /// The dot alone (narrow tiles); the label stays in the tooltip.
    var compact = false {
        didSet {
            guard compact != oldValue else { return }
            label.isHidden = compact
            NSLayoutConstraint.deactivate(compact ? labelConstraints : compactConstraints)
            NSLayoutConstraint.activate(compact ? compactConstraints : labelConstraints)
            needsLayout = true
        }
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.badge
        Theme.track(self)
        dot.cornerRadius = Design.Radius.badgeDot
        layer?.addSublayer(dot)
        label.font = Design.Font.badgeLabel
        label.textColor = Design.Color.textOnAccent
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)
        labelConstraints = [
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 20),
            label.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            label.topAnchor.constraint(equalTo: topAnchor, constant: 2),
            label.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -2),
        ]
        let side = Self.dotSize + 10
        compactConstraints = [
            widthAnchor.constraint(equalToConstant: side),
            heightAnchor.constraint(equalToConstant: side),
        ]
        NSLayoutConstraint.activate(labelConstraints)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    /// The dot's colour comes back with the overlay's next apply().
    func applyTheme() {
        layer?.backgroundColor = Design.Color.badge.cgColor
    }

    func apply(_ tile: TileModel) {
        guard let agent = tile.agent else { return }
        let name = agent.name.flatMap { $0.isEmpty ? nil : $0 } ?? "agent"
        let status: String = switch agent.status {
        case .blocked: "blocked"
        case .done: agent.attention ? "done" : "idle"
        case .working: "working"
        case .idle: agent.attention ? "done" : "idle"
        case .unknown: "…"
        }
        label.stringValue = "\(name) · \(status)"
        toolTip = label.stringValue
        let color = AttentionStyle.color(for: tile) ?? AttentionStyle.idle
        // theme: AttentionOverlayView.applyTheme() runs this again.
        dot.backgroundColor = color.cgColor
        setPulse(agent.status == .working)
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let d = Self.dotSize, x = compact ? (bounds.width - d) / 2 : 8
        dot.frame = CGRect(x: x, y: bounds.midY - d / 2, width: d, height: d)
    }

    private func setPulse(_ on: Bool) {
        guard on != pulsing else { return }
        pulsing = on
        if on, !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            let pulse = CABasicAnimation(keyPath: "opacity")
            pulse.fromValue = 1
            pulse.toValue = Design.Alpha.badgePulse
            pulse.duration = 1.2
            pulse.autoreverses = true
            pulse.repeatCount = .infinity
            pulse.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            dot.add(pulse, forKey: "pulse")
        } else {
            dot.removeAnimation(forKey: "pulse")
        }
    }
}

/// App-wide attention bookkeeping: the Dock badge (blocked agents) and
/// telling the core the user read a finished agent's result.
@MainActor
enum AttentionTracker {
    /// Automation: treat background windows as visible to the user.
    static var assumeVisible = false
    /// How long a finished agent's tile has to stay in front of the user
    /// before its done mark clears. Longer than a keyboard hop through tiles
    /// or workspaces (a focus move plus its 150–250 ms motion), so passing
    /// through doesn't count as reading; short enough that a glance does.
    static let dwell: TimeInterval = 0.75
    /// Tiles in front of the user with an unread finish, and since when.
    private static var looking: [String: CFTimeInterval] = [:]
    /// Tiles we've sent `tile.seen` for and the core hasn't cleared yet.
    private static var pending: Set<String> = []

    /// The Dock badge shows blocked agents. On by default; turn off with
    /// `defaults write dev.azulejo.Azulejo AzulejoDockBadge -bool false`.
    static func updateDock(_ session: SessionModel) {
        let enabled = UserDefaults.standard.object(forKey: "AzulejoDockBadge") as? Bool ?? true
        let blocked = session.windows.flatMap(\.workspaces).flatMap(\.tabs).flatMap(\.tiles)
            .filter { $0.agent?.status == .blocked }.count
        let label = enabled && blocked > 0 ? "\(blocked)" : nil
        if NSApp.dockTile.badgeLabel != label { NSApp.dockTile.badgeLabel = label }
    }

    /// A finished agent's result counts as read once its tile has been the
    /// focused, visible tile of the key window (app active, palette closed)
    /// for `dwell`: then the core gets `tile.seen`. Blocked agents aren't
    /// answered by looking, so they're left alone. `again` runs this once
    /// more when a dwell is up, with whatever the session is by then.
    static func markSeen(
        _ session: SessionModel, controllers: [WindowController],
        send: ([String: Any]) -> Void, again: @escaping @MainActor () -> Void
    ) {
        let now = CACurrentMediaTime()
        var front: [String: CFTimeInterval] = [:]
        var still: Set<String> = []
        for controller in controllers {
            guard let tab = session.window(controller.windowID)?.workspace?.tab,
                  let tile = tab.tile(tab.focused), let agent = tile.agent,
                  agent.attention, agent.status != .blocked
            else { continue }
            let visible = assumeVisible
                || (NSApp.isActive && controller.window.isKeyWindow
                    && controller.window.occlusionState.contains(.visible))
            guard visible, !controller.palette.isOpen,
                  controller.tiles.isVisible(tile.id)
            else { continue }
            let since = looking[tile.id] ?? now
            front[tile.id] = since
            if now - since < dwell {
                if looking[tile.id] == nil {
                    DispatchQueue.main.asyncAfter(deadline: .now() + dwell + 0.02) { again() }
                }
                continue
            }
            still.insert(tile.id)
            if !pending.contains(tile.id) {
                send(["method": "tile.seen", "params": ["tile": tile.id]])
            }
        }
        looking = front
        pending = still
    }
}
