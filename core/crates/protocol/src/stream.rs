//! Event streams on the API socket (F22).
//!
//! `events.subscribe` keeps the connection open: after its reply, the server
//! writes JSON-RPC notifications on the same connection, interleaved with
//! responses to any further requests:
//!
//! ```text
//! → {"jsonrpc":"2.0","id":1,"method":"events.subscribe","params":{"filter":["tile.","focus.changed"]}}
//! ← {"jsonrpc":"2.0","id":1,"result":{"type":"subscribed","subscription":1}}
//! ← {"jsonrpc":"2.0","method":"events.event","params":{"subscription":1,"event":"tile.created","data":{…}}}
//! ← {"jsonrpc":"2.0","method":"events.lagged","params":{"subscription":1,"dropped":12}}
//! ```
//!
//! Events arrive in the order the core produced them. Each subscription has a
//! bounded buffer; a reader that falls behind loses events rather than slowing
//! the core, and is told how many with `events.lagged` before the next event
//! it does get. After a lag, re-read whatever state you track (`session.get`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use azulejo_model::{TileId, WorkspaceId};

use crate::event::Event;

/// Parameters of `events.subscribe`. Every filter is optional; together they
/// narrow the stream (an event must pass all of them).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventsSubscribe {
    /// Event names or prefixes. `tile.created` matches that event only;
    /// `tile.`, `tile.*` and `tile` match every `tile.…` event. Empty or absent
    /// means every event.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filter: Vec<String>,
    /// Only events about this workspace: its own events, its tabs' and its
    /// tiles', and `focus.changed` when focus lands in it. Events about no
    /// particular workspace (`herdr.status`, window events) are left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceId>,
    /// Only events about this tile: its own `tile.*`, `preview.updated` and
    /// `agent.state_changed` events, layout changes of its tab, and focus
    /// changes to or from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<TileId>,
}

/// Parameters of `events.unsubscribe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventsUnsubscribe {
    /// The `subscription` from `events.subscribe`'s reply.
    pub subscription: u64,
}

/// A message the server sends without being asked, on a connection with a
/// subscription. On the wire it's a JSON-RPC notification: `method` plus
/// `params`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "method", content = "params")]
// Short-lived wire messages; boxing the event would only add an allocation.
#[allow(clippy::large_enum_variant)]
pub enum Notification {
    /// One event. `event` and `data` are exactly the event's own fields.
    #[serde(rename = "events.event")]
    Event(EventNotification),
    /// The subscriber fell behind and `dropped` events were skipped just
    /// before the next one it receives.
    #[serde(rename = "events.lagged")]
    Lagged(LaggedNotification),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventNotification {
    pub subscription: u64,
    #[serde(flatten)]
    pub event: Event,
}

// By hand, so the schema refers to `Event` instead of inlining a second copy
// of every event.
impl JsonSchema for EventNotification {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "EventNotification".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let event = generator.subschema_for::<Event>();
        schemars::json_schema!({
            "description": "An event plus the subscription it's for: the event's own `event` and `data` fields, and `subscription`.",
            "allOf": [
                event,
                {
                    "type": "object",
                    "properties": {
                        "subscription": {"type": "integer", "format": "uint64", "minimum": 0}
                    },
                    "required": ["subscription"]
                }
            ]
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LaggedNotification {
    pub subscription: u64,
    pub dropped: u64,
}

/// Whether `name` (e.g. `tile.created`) passes `filter` (see
/// [`EventsSubscribe::filter`]).
pub fn filter_matches(filter: &[String], name: &str) -> bool {
    filter.is_empty() || filter.iter().any(|entry| entry_matches(entry, name))
}

fn entry_matches(entry: &str, name: &str) -> bool {
    let prefix = entry.trim().trim_end_matches('*').trim_end_matches('.');
    prefix.is_empty()
        || name == prefix
        || name
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn filters() {
        assert!(filter_matches(&[], "tile.created"));
        assert!(filter_matches(&f(&["tile.created"]), "tile.created"));
        assert!(!filter_matches(&f(&["tile.created"]), "tile.closed"));
        for prefix in ["tile", "tile.", "tile.*"] {
            assert!(filter_matches(&f(&[prefix]), "tile.created"), "{prefix}");
            assert!(filter_matches(&f(&[prefix]), "tile.closed"), "{prefix}");
            assert!(!filter_matches(&f(&[prefix]), "tab.created"), "{prefix}");
            assert!(!filter_matches(&f(&[prefix]), "tiles.x"), "{prefix}");
        }
        assert!(filter_matches(&f(&["*"]), "focus.changed"));
        assert!(filter_matches(
            &f(&["focus.changed", "tile."]),
            "focus.changed"
        ));
        assert!(!filter_matches(&f(&["layout"]), "focus.changed"));
    }
}
