import Foundation

// Mirrors of `azulejo_model` as the core reports it (`session.get`). IDs are
// opaque prefixed ULIDs (`tile_…`); the shell never makes them up.

struct SessionModel: Decodable, Equatable {
    var windows: [WindowModel]
    var focusedWindow: String?

    enum CodingKeys: String, CodingKey {
        case windows
        case focusedWindow = "focused_window"
    }

    func window(_ id: String) -> WindowModel? { windows.first { $0.id == id } }
}

struct FrameModel: Codable, Equatable {
    var x, y, width, height: Double
}

struct WindowModel: Decodable, Equatable {
    var id: String
    var workspaces: [WorkspaceModel]
    var activeWorkspace: String
    var frame: FrameModel?
    var sidebarHidden: Bool?

    enum CodingKeys: String, CodingKey {
        case id, workspaces, frame
        case activeWorkspace = "active_workspace"
        case sidebarHidden = "sidebar_hidden"
    }

    var workspace: WorkspaceModel? { workspaces.first { $0.id == activeWorkspace } }
}

struct WorkspaceModel: Decodable, Equatable {
    var id: String
    var name: String
    var cwd: String?
    var tabs: [TabModel]
    var activeTab: String?

    enum CodingKeys: String, CodingKey {
        case id, name, cwd, tabs
        case activeTab = "active_tab"
    }

    var tab: TabModel? { tabs.first { $0.id == activeTab } ?? tabs.first }
}

struct TabModel: Decodable, Equatable {
    var id: String
    var name: String?
    var root: LayoutNode
    var tiles: [TileModel]
    var focused: String
    var zoomed: String?

    func tile(_ id: String) -> TileModel? { tiles.first { $0.id == id } }

    /// Tiles in reading order (depth-first, left/top first): the order tile
    /// cycling follows.
    var order: [String] { root.leaves }

    /// What the tab bar shows: its name, else the focused tile's title, else
    /// the focused tile's kind.
    var label: String {
        if let name, !name.isEmpty { return name }
        let tile = tile(focused)
        if let title = tile?.title, !title.isEmpty { return title }
        if let page = tile?.pageTitle, !page.isEmpty { return page }
        return tile?.kind.rawValue ?? "tab"
    }
}

enum TileKind: String, Decodable, CaseIterable {
    case terminal, browser, preview, files
}

struct HerdrRefModel: Decodable, Equatable {
    var paneId: String
    var terminalId: String
    var tabId: String

    enum CodingKeys: String, CodingKey {
        case paneId = "pane_id"
        case terminalId = "terminal_id"
        case tabId = "tab_id"
    }
}

/// One tile with its kind-specific fields flattened in, as the core sends it.
struct TileModel: Decodable, Equatable {
    var id: String
    var kind: TileKind
    var title: String?
    // terminal
    var cwd: String?
    var command: [String]?
    var herdr: HerdrRefModel?
    var ended: Bool?
    // browser
    var url: String?
    var pageTitle: String?
    // preview
    var path: String?
    // files
    var root: String?

    enum CodingKeys: String, CodingKey {
        case id, kind, title, cwd, command, herdr, ended, url, path, root
        case pageTitle = "page_title"
    }
}

indirect enum LayoutNode: Decodable, Equatable {
    case split(axis: Axis, ratio: Double, a: LayoutNode, b: LayoutNode)
    case leaf(id: String)

    enum Axis: String, Decodable {
        /// Side by side: `a` left, `b` right.
        case horizontal
        /// Stacked: `a` on top, `b` below.
        case vertical
    }

    private enum CodingKeys: String, CodingKey {
        case type, axis, ratio, a, b, id
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        switch try c.decode(String.self, forKey: .type) {
        case "split":
            self = .split(
                axis: try c.decode(Axis.self, forKey: .axis),
                ratio: try c.decode(Double.self, forKey: .ratio),
                a: try c.decode(LayoutNode.self, forKey: .a),
                b: try c.decode(LayoutNode.self, forKey: .b))
        case "leaf":
            self = .leaf(id: try c.decode(String.self, forKey: .id))
        case let other:
            throw DecodingError.dataCorruptedError(
                forKey: .type, in: c, debugDescription: "unknown node type \(other)")
        }
    }

    var leaves: [String] {
        switch self {
        case .leaf(let id): [id]
        case .split(_, _, let a, let b): a.leaves + b.leaves
        }
    }

    /// Each leaf's rect inside `bounds`, leaving `gap` points between siblings.
    /// AppKit coordinates (origin bottom-left): `a` of a vertical split is on top.
    func frames(in bounds: CGRect, gap: CGFloat) -> [String: CGRect] {
        switch self {
        case .leaf(let id):
            return [id: bounds]
        case .split(let axis, let ratio, let a, let b):
            let r = CGFloat(ratio)
            var first = bounds, second = bounds
            switch axis {
            case .horizontal:
                let w = max(0, bounds.width - gap)
                first.size.width = (w * r).rounded()
                second.origin.x = bounds.minX + first.width + gap
                second.size.width = max(0, bounds.maxX - second.minX)
            case .vertical:
                let h = max(0, bounds.height - gap)
                let top = (h * r).rounded()
                first.origin.y = bounds.maxY - top
                first.size.height = top
                second.size.height = max(0, first.minY - gap - bounds.minY)
            }
            return a.frames(in: first, gap: gap).merging(b.frames(in: second, gap: gap)) { x, _ in x }
        }
    }

    /// A compact one-line rendering for logs: `H(t1 | V(t2 / t3))`.
    func describe(short: (String) -> String) -> String {
        switch self {
        case .leaf(let id): short(id)
        case .split(.horizontal, let r, let a, let b):
            "H\(String(format: "%.2f", r))(\(a.describe(short: short)) | \(b.describe(short: short)))"
        case .split(.vertical, let r, let a, let b):
            "V\(String(format: "%.2f", r))(\(a.describe(short: short)) / \(b.describe(short: short)))"
        }
    }
}
