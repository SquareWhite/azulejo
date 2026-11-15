import AppKit

/// Mark as Unread / Mark as Read on agent tiles (`toggle_unread`): the tile
/// header's menu, ⌘K and the keymap. A marked tile looks like a finish
/// nobody has read (the done mark in its header, the sidebar's and tab
/// strip's markers, the footer's done count, the Dock badge) until the user
/// comes back to it (`AttentionTracker.markSeen`) or marks it read. The core
/// keeps the mark (`tile.mark_unread`) and saves it with the session.
extension AppController {
    /// The tile a menu was opened on, else the focused one.
    func toggleUnread(target: ActionTarget?, in controller: WindowController) {
        let id: String?
        if case .tile(let tile)? = target {
            id = tile
        } else {
            id = session?.window(controller.windowID)?.workspace?.tab?.focused
        }
        guard let id, let tile = tile(id) else { return controller.flash("no tile") }
        guard let agent = tile.agent else { return controller.flash("only agent tiles can be marked unread") }
        let method = agent.isUnread ? "tile.seen" : "tile.mark_unread"
        if case .failure(let error) = send(["method": method, "params": ["tile": id]]) {
            controller.report(error)
        }
    }

    /// Mark All as Read on a workspace row: every unread agent tile in it.
    func markAllRead(workspace id: String, in controller: WindowController) {
        guard let ws = session?.windows.flatMap(\.workspaces).first(where: { $0.id == id }) else { return }
        for tile in ws.tabs.flatMap(\.tiles) where tile.agent?.isUnread == true {
            if case .failure(let error) = send(["method": "tile.seen", "params": ["tile": tile.id]]) {
                controller.report(error)
            }
        }
    }

    /// The menu and ⌘K title for a tile: nil for a tile without an agent.
    static func unreadTitle(_ tile: TileModel?, menu: Bool) -> String? {
        guard let agent = tile?.agent else { return nil }
        switch (agent.isUnread, menu) {
        case (true, true): return "Mark as Read"
        case (false, true): return "Mark as Unread"
        case (true, false): return "Mark tile as read"
        case (false, false): return "Mark tile as unread"
        }
    }

    private func tile(_ id: String) -> TileModel? {
        session?.windows.lazy.flatMap(\.workspaces).flatMap(\.tabs).compactMap { $0.tile(id) }.first
    }
}

/// Automation keys for unread marks and the Dock badge (`Automation`):
/// `tile.unread` (the focused tab's tiles in reading order: `marked` (the
/// user marked it unread), `unread` (a finish nobody has read), `read` (an
/// agent with nothing unread, blocked included) or `none` (no agent)),
/// `unread.count` (unread agent tiles in every window), `dock.badge` (the
/// Dock tile's badge, `none` when empty), `dock.badge.setting`
/// (`[appearance] dock_badge`: `on` / `off`) and `menu.unread` (the last
/// built menu's Mark as Unread / Mark as Read item, or `none`).
@MainActor
enum UnreadAutomation {
    static func state(app: AppController, controller: WindowController) -> [String: String] {
        var s: [String: String] = [
            "unread.count": "\(AttentionTracker.unreadCount)",
            "dock.badge": NSApp.dockTile.badgeLabel ?? "none",
            "dock.badge.setting": AttentionTracker.dockBadge ? "on" : "off",
            "menu.unread": MenuAutomation.last?.menu.items.map(\.title)
                .first { $0 == "Mark as Unread" || $0 == "Mark as Read" } ?? "none",
        ]
        if let tab = app.session?.window(controller.windowID)?.workspace?.tab {
            s["tile.unread"] = tab.order.map { id in
                guard let agent = tab.tile(id)?.agent else { return "none" }
                if agent.unread { return "marked" }
                return agent.isUnread ? "unread" : "read"
            }.joined(separator: ",")
        }
        return s
    }
}
