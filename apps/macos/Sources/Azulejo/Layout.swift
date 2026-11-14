import Foundation

/// Mirrors `azulejo_protocol::LayoutNode`. Spike-grade (step 0.6).
indirect enum LayoutNode: Decodable, Equatable {
    case split(axis: Axis, ratio: Double, a: LayoutNode, b: LayoutNode)
    case leaf(id: String)

    enum Axis: String, Decodable {
        case horizontal, vertical
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
                b: try c.decode(LayoutNode.self, forKey: .b)
            )
        case "leaf":
            self = .leaf(id: try c.decode(String.self, forKey: .id))
        case let other:
            throw DecodingError.dataCorruptedError(
                forKey: .type, in: c, debugDescription: "unknown node type \(other)")
        }
    }
}

/// The `layout.changed` event.
struct LayoutChanged: Decodable {
    let tree: LayoutNode
    let focused: String

    /// Returns nil for any other event.
    static func decode(_ json: String) -> LayoutChanged? {
        struct Envelope: Decodable {
            let event: String
        }
        struct Full: Decodable {
            let data: LayoutChanged
        }
        let data = Data(json.utf8)
        guard (try? JSONDecoder().decode(Envelope.self, from: data))?.event == "layout.changed"
        else { return nil }
        return try? JSONDecoder().decode(Full.self, from: data).data
    }
}
