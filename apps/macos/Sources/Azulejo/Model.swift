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
    /// Collapsed: a rail or hidden (`sidebar` says which).
    var sidebarHidden: Bool?
    /// How the sidebar shows; absent while it's full.
    var sidebar: SidebarState?

    enum CodingKeys: String, CodingKey {
        case id, workspaces, frame, sidebar
        case activeWorkspace = "active_workspace"
        case sidebarHidden = "sidebar_hidden"
    }

    var workspace: WorkspaceModel? { workspaces.first { $0.id == activeWorkspace } }

    /// How the sidebar shows (a core without `sidebar` only knew hidden).
    var sidebarState: SidebarState {
        sidebar ?? (sidebarHidden == true ? .hidden : .full)
    }
}

/// How a window's sidebar shows (`azulejo_model::SidebarState`): the full
/// list, a narrow rail of numbers and agent states, or nothing.
enum SidebarState: String, Decodable {
    case full, rail, hidden

    init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = SidebarState(rawValue: raw) ?? .full
    }
}

struct WorkspaceModel: Decodable, Equatable {
    var id: String
    var name: String
    var cwd: String?
    var tabs: [TabModel]
    var activeTab: String?
    /// The herdr machine its terminals run on; nil for Local (ADR 0011).
    var machine: String?
    /// The repository and branch the sidebar shows (F26).
    var git: GitModel?
    /// The folder set with `workspace.set_repo`, if any.
    var repo: String?

    enum CodingKeys: String, CodingKey {
        case id, name, cwd, tabs, machine, git, repo
        case activeTab = "active_tab"
    }

    /// The machine ID shells use: `"local"` for Local.
    var machineID: String { machine ?? MachineModel.local }

    var tab: TabModel? { tabs.first { $0.id == activeTab } ?? tabs.first }
}

struct TabModel: Decodable, Equatable {
    var id: String
    var name: String?
    /// The split tree: the source of truth in a manual tab, derived from the
    /// layout rule in a dynamic one, where it reproduces the core's rects
    /// exactly (ADR 0010).
    var root: LayoutNode
    var tiles: [TileModel]
    var focused: String
    var zoomed: String?
    /// Absent in old state and in plain manual tabs.
    var layout: TabLayoutModel?

    func tile(_ id: String) -> TileModel? { tiles.first { $0.id == id } }

    var kind: LayoutKind { layout?.kind ?? .manual }

    /// Tiles in cycling order: a dynamic tab's order (masters first), else
    /// the tree's reading order (depth-first, left/top first).
    var order: [String] {
        if let order = layout?.order, !order.isEmpty, kind != .manual { return order }
        return root.leaves
    }

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

/// How a tab arranges its tiles (ADR 0010).
enum LayoutKind: String, Decodable {
    case manual, tile, monocle
    case threeColumn = "three_column"

    init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = LayoutKind(rawValue: raw) ?? .manual
    }

    /// A small hint for the tab bar; manual tabs show none.
    var glyph: String? {
        switch self {
        case .manual: nil
        case .tile: "▤"
        case .monocle: "▣"
        case .threeColumn: "▥"
        }
    }

    /// The kinds the layout picker and the tab menu offer, in their order.
    static let picker: [LayoutKind] = [.manual, .tile, .threeColumn, .monocle]

    /// The kind's name in the layout picker and the tab menu.
    var title: String {
        switch self {
        case .manual: "Manual"
        case .tile: "Master (Tile)"
        case .threeColumn: "Three Column"
        case .monocle: "Monocle"
        }
    }

    /// The kind's SF Symbol in the layout picker and the tab menu.
    var symbol: String {
        switch self {
        case .manual: Design.Icon.layoutManual
        case .tile: Design.Icon.layoutTile
        case .threeColumn: Design.Icon.layoutThreeColumn
        case .monocle: Design.Icon.layoutMonocle
        }
    }

    /// The keymap action that switches a tab to this kind (`layout.toggle`;
    /// the picker only runs it for a kind the tab isn't in, where toggling
    /// is setting).
    var toggleAction: String { "toggle_layout_\(rawValue)" }
}

struct TabLayoutModel: Decodable, Equatable {
    var kind: LayoutKind
    var order: [String]?
    var params: LayoutParamsModel?
    /// Per-tile stack weights that aren't 1.0 (automation traces them).
    var weights: [String: Double]?
}

/// A dynamic tab's parameters, for automation and later the tab bar.
struct LayoutParamsModel: Decodable, Equatable {
    struct Tile: Decodable, Equatable {
        var masters: Int
        var ratio: Double
        var orientation: String?
        var rotatePart: Bool?

        enum CodingKeys: String, CodingKey {
            case masters, ratio, orientation
            case rotatePart = "rotate_part"
        }
    }

    struct ThreeColumn: Decodable, Equatable {
        var masters: Int
        var ratio: Double
    }

    var tile: Tile
    var threeColumn: ThreeColumn

    enum CodingKeys: String, CodingKey {
        case tile
        case threeColumn = "three_column"
    }
}

enum TileKind: String, Decodable, CaseIterable {
    case terminal, browser, preview, files
}

struct HerdrRefModel: Decodable, Equatable {
    var paneId: String
    var terminalId: String
    var tabId: String
    /// The herdr machine the pane runs on; nil for Local (ADR 0011).
    var machine: String?

    enum CodingKeys: String, CodingKey {
        case paneId = "pane_id"
        case terminalId = "terminal_id"
        case tabId = "tab_id"
        case machine
    }
}

/// A herdr machine and the core's link to it (`machines.list`, ADR 0011).
struct MachineModel: Decodable, Equatable {
    static let local = "local"

    enum State: String, Decodable {
        case connecting, connected, reconnecting, error, off
    }

    var id: String
    var label: String
    var state: State
    var message: String?

    var isLocal: Bool { id == Self.local }
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
    /// Terminal tiles: the agent herdr reports in the pane (F11, F16).
    var agent: AgentModel?
    /// What the header strip shows, worked out by the core (F28).
    var header: TileHeaderModel?
    /// Preview tiles: `rendered` (default) or `source` (F29).
    var previewMode: String?

    enum CodingKeys: String, CodingKey {
        case id, kind, title, cwd, command, herdr, ended, url, path, root, agent, header
        case pageTitle = "page_title"
        case previewMode = "preview_mode"
    }

    var showsSource: Bool { previewMode == "source" }

    /// Needs a look: the agent became blocked or finished and nobody has seen it.
    var needsAttention: Bool { agent?.attention == true }
}

/// A tile's header text (`azulejo_model::TileHeader`).
struct TileHeaderModel: Decodable, Equatable {
    var title: String
    var subtitle: String?
}

/// A workspace's repository and branch (`azulejo_model::GitInfo`, F26).
struct GitModel: Decodable, Equatable {
    var repo: String
    var branch: String?
    var detached: Bool?
    var dirty: Bool?
}

/// What herdr says the agent in a terminal tile is doing.
struct AgentModel: Decodable, Equatable {
    enum Status: String, Decodable {
        case idle, working, blocked, done, unknown

        init(from decoder: Decoder) throws {
            let raw = try decoder.singleValueContainer().decode(String.self)
            self = Status(rawValue: raw) ?? .unknown
        }
    }

    var name: String?
    var status: Status
    var sinceMs: UInt64?
    var attention: Bool

    enum CodingKeys: String, CodingKey {
        case name, status, attention
        case sinceMs = "since_ms"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        name = try c.decodeIfPresent(String.self, forKey: .name)
        status = try c.decodeIfPresent(Status.self, forKey: .status) ?? .unknown
        sinceMs = try c.decodeIfPresent(UInt64.self, forKey: .sinceMs)
        attention = try c.decodeIfPresent(Bool.self, forKey: .attention) ?? false
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
            let (first, second, _) = Self.split(bounds, axis: axis, ratio: ratio, gap: gap)
            return a.frames(in: first, gap: gap).merging(b.frames(in: second, gap: gap)) { x, _ in x }
        }
    }

    /// One split's two sides inside `bounds` and the gutter between them
    /// (`gap` wide), as `frames(in:gap:)` lays them out.
    static func split(_ bounds: CGRect, axis: Axis, ratio: Double, gap: CGFloat)
        -> (first: CGRect, second: CGRect, gutter: CGRect) {
        let r = CGFloat(ratio)
        var first = bounds, second = bounds
        switch axis {
        case .horizontal:
            let w = max(0, bounds.width - gap)
            first.size.width = (w * r).rounded()
            second.origin.x = bounds.minX + first.width + gap
            second.size.width = max(0, bounds.maxX - second.minX)
            return (first, second, CGRect(x: first.maxX, y: bounds.minY, width: gap, height: bounds.height))
        case .vertical:
            let h = max(0, bounds.height - gap)
            let top = (h * r).rounded()
            first.origin.y = bounds.maxY - top
            first.size.height = top
            second.size.height = max(0, first.minY - gap - bounds.minY)
            return (first, second, CGRect(x: bounds.minX, y: first.minY - gap, width: bounds.width, height: gap))
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
