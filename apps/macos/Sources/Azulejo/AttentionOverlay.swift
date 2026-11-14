import AppKit

/// Colours for agent state, shared by tile badges, the sidebar, the tab bar
/// and the picker. Calm on purpose: only "blocked" is loud.
@MainActor
enum AttentionStyle {
    static let blocked = Design.Status.blocked.color
    static let done = Design.Status.done.color
    static let working = Design.Status.working.color
    static let idle = Design.Status.idle.color

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
                (color, count) = (AttentionStyle.working.withAlphaComponent(0.6), 0)
            } else {
                return nil
            }
            return NSAttributedString(
                string: count > 0 ? " ● \(count)" : " ●",
                attributes: [.foregroundColor: color, .font: Design.Font.badge])
        }
    }
}

/// Badges over the tiles: the agent's name and state in each terminal
/// tile's top-right corner, and a thin coloured border on tiles that need a
/// look. Lies over the tile container and passes every click through.
@MainActor
final class AttentionOverlayView: NSView {
    private var badges: [String: AgentBadgeView] = [:]
    private var borders: [String: NSView] = [:]
    /// What's drawn, for automation: tile id → "blocked!" / "working" / …
    private(set) var shown: [String: String] = [:]

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    private var tab: TabModel?

    /// Puts the badges where the tiles are now (window resized, motion ended).
    func relayout(frames: [String: CGRect]) { apply(tab, frames: frames) }

    /// `frames` are the visible tiles' rects in this view's coordinates (the
    /// overlay has the container's frame).
    func apply(_ tab: TabModel?, frames: [String: CGRect]) {
        self.tab = tab
        var seen = Set<String>()
        var state: [String: String] = [:]
        for (id, frame) in frames {
            guard let tile = tab?.tile(id), let agent = tile.agent,
                  agent.status != .unknown || agent.attention
            else { continue }
            seen.insert(id)
            let badge = badges[id] ?? {
                let b = AgentBadgeView()
                addSubview(b)
                badges[id] = b
                return b
            }()
            badge.apply(tile)
            let size = badge.fittingSize
            // Just below the tile's header strip (F28), clear of its controls.
            let top = frame.maxY - min(TileFrame.headerHeight, frame.height)
            badge.frame = NSRect(x: frame.maxX - size.width - 8, y: top - size.height - 6,
                                 width: size.width, height: size.height)

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
                border.layer?.borderColor = (agent.status == .blocked
                    ? AttentionStyle.blocked : AttentionStyle.done).withAlphaComponent(0.85).cgColor
                border.frame = frame.insetBy(dx: -1, dy: -1)
            } else if let border = borders.removeValue(forKey: id) {
                border.removeFromSuperview()
            }
            state[Short.id(id)] = agent.status.rawValue + (agent.attention ? "!" : "")
        }
        for (id, badge) in badges where !seen.contains(id) {
            badge.removeFromSuperview()
            badges[id] = nil
        }
        for (id, border) in borders where !seen.contains(id) {
            border.removeFromSuperview()
            borders[id] = nil
        }
        shown = state
    }
}

/// "● claude · blocked" in a tile's corner. Working agents get a slow,
/// gentle pulse on the dot; nothing else moves.
@MainActor
final class AgentBadgeView: NSView {
    private let dot = CALayer()
    private let label = NSTextField(labelWithString: "")
    private var pulsing = false

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.badge
        layer?.backgroundColor = Design.Color.badge.cgColor
        dot.cornerRadius = 4
        layer?.addSublayer(dot)
        label.font = Design.Font.badgeLabel
        label.textColor = Design.Color.textOnAccent
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 20),
            label.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            label.topAnchor.constraint(equalTo: topAnchor, constant: 2),
            label.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -2),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    func apply(_ tile: TileModel) {
        guard let agent = tile.agent else { return }
        let name = agent.name.flatMap { $0.isEmpty ? nil : $0 } ?? "agent"
        let status: String = switch agent.status {
        case .blocked: "blocked"
        case .done: "done"
        case .working: "working"
        case .idle: agent.attention ? "done" : "idle"
        case .unknown: "…"
        }
        label.stringValue = "\(name) · \(status)"
        let color = AttentionStyle.color(for: tile) ?? AttentionStyle.idle
        dot.backgroundColor = color.cgColor
        setPulse(agent.status == .working)
        needsLayout = true
    }

    override func layout() {
        super.layout()
        dot.frame = CGRect(x: 8, y: bounds.midY - 4, width: 8, height: 8)
    }

    private func setPulse(_ on: Bool) {
        guard on != pulsing else { return }
        pulsing = on
        if on, !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            let pulse = CABasicAnimation(keyPath: "opacity")
            pulse.fromValue = 1
            pulse.toValue = 0.35
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
/// telling the core a tile was seen when it's focused on screen.
@MainActor
enum AttentionTracker {
    /// Automation: treat background windows as visible to the user.
    static var assumeVisible = false
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

    /// For each window the user can see, a focused tile that needs a look
    /// has now been seen.
    static func markSeen(_ session: SessionModel, controllers: [WindowController], send: ([String: Any]) -> Void) {
        var still: Set<String> = []
        for controller in controllers {
            guard let tab = session.window(controller.windowID)?.workspace?.tab,
                  let tile = tab.tile(tab.focused), tile.needsAttention
            else { continue }
            let visible = assumeVisible
                || (NSApp.isActive && controller.window.isKeyWindow
                    && controller.window.occlusionState.contains(.visible))
            guard visible, !controller.palette.isOpen else { continue }
            still.insert(tile.id)
            if !pending.contains(tile.id) {
                send(["method": "tile.seen", "params": ["tile": tile.id]])
            }
        }
        pending = still
    }
}
