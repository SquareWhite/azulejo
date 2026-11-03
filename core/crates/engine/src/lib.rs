//! Core state machine: commands in, events out.

use azulejo_protocol::{API_VERSION, Command, Event};

#[derive(Debug, Default)]
pub struct Engine;

impl Engine {
    pub fn new() -> Self {
        Self
    }

    pub fn handle(&mut self, command: Command) -> Vec<Event> {
        match command {
            Command::Ping => vec![Event::Pong {
                api_version: API_VERSION,
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
