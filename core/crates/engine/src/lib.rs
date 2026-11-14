//! Core state machine: commands in, events out.

use azulejo_protocol::{API_VERSION, Axis, Command, Direction, Event, LayoutNode};

/// Spike-grade (step 0.6): one layout tree with string tile IDs. Step 1.1 replaces
/// this with the real object model.
#[derive(Debug)]
pub struct Engine {
    tree: LayoutNode,
    focused: String,
    next_tile: u32,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self {
            tree: LayoutNode::Leaf { id: "t1".into() },
            focused: "t1".into(),
            next_tile: 2,
        }
    }

    pub fn handle(&mut self, command: Command) -> Vec<Event> {
        match command {
            Command::Ping => vec![Event::Pong {
                api_version: API_VERSION,
            }],
            Command::LayoutGet => vec![self.layout_changed()],
            Command::LayoutSplit { direction } => {
                let id = format!("t{}", self.next_tile);
                self.next_tile += 1;
                let axis = match direction {
                    Direction::Right => Axis::Horizontal,
                    Direction::Down => Axis::Vertical,
                };
                split_leaf(&mut self.tree, &self.focused, axis, &id);
                self.focused = id;
                vec![self.layout_changed()]
            }
        }
    }

    fn layout_changed(&self) -> Event {
        Event::LayoutChanged {
            tree: self.tree.clone(),
            focused: self.focused.clone(),
        }
    }
}

/// Replaces the leaf `target` with a split of it and a new leaf `new_id`.
fn split_leaf(node: &mut LayoutNode, target: &str, axis: Axis, new_id: &str) -> bool {
    match node {
        LayoutNode::Leaf { id } if id == target => {
            let old = std::mem::replace(node, LayoutNode::Leaf { id: String::new() });
            *node = LayoutNode::Split {
                axis,
                ratio: 0.5,
                a: Box::new(old),
                b: Box::new(LayoutNode::Leaf { id: new_id.into() }),
            };
            true
        }
        LayoutNode::Leaf { .. } => false,
        LayoutNode::Split { a, b, .. } => {
            split_leaf(a, target, axis, new_id) || split_leaf(b, target, axis, new_id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(id: &str) -> Box<LayoutNode> {
        Box::new(LayoutNode::Leaf { id: id.into() })
    }

    #[test]
    fn ping_answers_pong() {
        let events = Engine::new().handle(Command::Ping);
        assert_eq!(
            events,
            vec![Event::Pong {
                api_version: API_VERSION
            }]
        );
    }

    #[test]
    fn splits_nest_under_the_focused_tile() {
        let mut engine = Engine::new();
        engine.handle(Command::LayoutSplit {
            direction: Direction::Right,
        });
        let events = engine.handle(Command::LayoutSplit {
            direction: Direction::Down,
        });
        assert_eq!(
            events,
            vec![Event::LayoutChanged {
                tree: LayoutNode::Split {
                    axis: Axis::Horizontal,
                    ratio: 0.5,
                    a: leaf("t1"),
                    b: Box::new(LayoutNode::Split {
                        axis: Axis::Vertical,
                        ratio: 0.5,
                        a: leaf("t2"),
                        b: leaf("t3"),
                    }),
                },
                focused: "t3".into(),
            }]
        );
    }
}
