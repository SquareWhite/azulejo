//! Commands, events and errors shared by the shells, the API server and the CLI.

use serde::{Deserialize, Serialize};

/// Version of the public API schema.
pub const API_VERSION: u32 = 1;

/// Where a split puts the new tile, relative to the focused one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Right,
    Down,
}

/// How a split lays out its two children: `horizontal` puts them side by side,
/// `vertical` stacks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// A layout tree as shells see it. Spike-grade (step 0.6): the real model lands in 1.1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum LayoutNode {
    Split {
        axis: Axis,
        /// Share of the space given to `a`, in (0, 1).
        ratio: f32,
        a: Box<LayoutNode>,
        b: Box<LayoutNode>,
    },
    Leaf {
        id: String,
    },
}

/// A request sent to the core, by a shell or through the socket API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Command {
    #[serde(rename = "core.ping")]
    Ping,
    /// Ask for the current layout; answered with `layout.changed`.
    #[serde(rename = "layout.get")]
    LayoutGet,
    /// Split the focused tile; the new tile takes focus.
    #[serde(rename = "layout.split")]
    LayoutSplit { direction: Direction },
}

/// Something that happened in the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum Event {
    #[serde(rename = "core.pong")]
    Pong { api_version: u32 },
    #[serde(rename = "layout.changed")]
    LayoutChanged { tree: LayoutNode, focused: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_round_trips_through_json() {
        let json = serde_json::to_string(&Command::Ping).unwrap();
        assert_eq!(json, r#"{"method":"core.ping"}"#);
        assert_eq!(
            serde_json::from_str::<Command>(&json).unwrap(),
            Command::Ping
        );
    }

    #[test]
    fn split_command_parses_from_shell_json() {
        let command: Command =
            serde_json::from_str(r#"{"method":"layout.split","params":{"direction":"down"}}"#)
                .unwrap();
        assert_eq!(
            command,
            Command::LayoutSplit {
                direction: Direction::Down
            }
        );
    }

    #[test]
    fn layout_event_has_a_tagged_tree() {
        let event = Event::LayoutChanged {
            tree: LayoutNode::Split {
                axis: Axis::Horizontal,
                ratio: 0.5,
                a: Box::new(LayoutNode::Leaf { id: "t1".into() }),
                b: Box::new(LayoutNode::Leaf { id: "t2".into() }),
            },
            focused: "t2".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(
            json,
            r#"{"event":"layout.changed","data":{"tree":{"type":"split","axis":"horizontal","ratio":0.5,"a":{"type":"leaf","id":"t1"},"b":{"type":"leaf","id":"t2"}},"focused":"t2"}}"#
        );
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event);
    }
}
