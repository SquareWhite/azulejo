//! herdr events, typed. Lifecycle events arrive as
//! `{"event":"pane_created","data":{"type":"pane_created",…}}`; per-pane
//! subscription events may use dotted kinds (`pane.agent_status_changed`).
//! Both spellings are accepted.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::types::{
    AgentStatus, PaneId, PaneInfo, ScrollInfo, TabId, TabInfo, WorkspaceId, WorkspaceInfo,
};

/// One thing to subscribe to. herdr fixes the set per connection, so
/// subscribing to another pane means opening another stream.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Subscription {
    WorkspaceCreated,
    WorkspaceUpdated,
    WorkspaceRenamed,
    WorkspaceClosed,
    WorkspaceFocused,
    TabCreated,
    TabRenamed,
    TabClosed,
    TabFocused,
    PaneCreated,
    PaneUpdated,
    PaneClosed,
    PaneExited,
    PaneFocused,
    PaneMoved,
    /// A tab's split tree, zoom or focus changed (ADR 0012).
    LayoutUpdated,
    /// Agent status for one pane (herdr requires a pane ID).
    AgentStatus(PaneId),
    /// Scroll position for one pane (herdr requires a pane ID).
    Scroll(PaneId),
}

impl Subscription {
    /// Everything Azulejo needs to mirror herdr's structure (F15). Agent
    /// status also arrives through `PaneUpdated`; subscribe per pane with
    /// [`Subscription::AgentStatus`] for dedicated transitions.
    pub fn lifecycle() -> Vec<Subscription> {
        use Subscription::*;
        vec![
            WorkspaceCreated,
            WorkspaceUpdated,
            WorkspaceRenamed,
            WorkspaceClosed,
            TabCreated,
            TabRenamed,
            TabClosed,
            PaneCreated,
            PaneUpdated,
            PaneClosed,
            PaneExited,
            PaneMoved,
            LayoutUpdated,
        ]
    }

    pub(crate) fn to_wire(&self) -> Value {
        use Subscription::*;
        let kind = match self {
            WorkspaceCreated => "workspace.created",
            WorkspaceUpdated => "workspace.updated",
            WorkspaceRenamed => "workspace.renamed",
            WorkspaceClosed => "workspace.closed",
            WorkspaceFocused => "workspace.focused",
            TabCreated => "tab.created",
            TabRenamed => "tab.renamed",
            TabClosed => "tab.closed",
            TabFocused => "tab.focused",
            PaneCreated => "pane.created",
            PaneUpdated => "pane.updated",
            PaneClosed => "pane.closed",
            PaneExited => "pane.exited",
            PaneFocused => "pane.focused",
            PaneMoved => "pane.moved",
            LayoutUpdated => "layout.updated",
            AgentStatus(pane) => {
                return json!({"type": "pane.agent_status_changed", "pane_id": pane});
            }
            Scroll(pane) => return json!({"type": "pane.scroll_changed", "pane_id": pane}),
        };
        json!({ "type": kind })
    }
}

/// A herdr event Azulejo understands. Anything else lands in `Other`.
// Events are rare (human-scale), so unboxed pane payloads are fine.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum HerdrEvent {
    WorkspaceCreated(WorkspaceInfo),
    WorkspaceUpdated(WorkspaceInfo),
    WorkspaceRenamed {
        workspace_id: WorkspaceId,
        label: String,
    },
    /// Also fires when the last tab of a workspace closes.
    WorkspaceClosed {
        workspace_id: WorkspaceId,
    },
    WorkspaceFocused {
        workspace_id: WorkspaceId,
    },
    TabCreated(TabInfo),
    TabRenamed {
        tab_id: TabId,
        workspace_id: WorkspaceId,
        label: String,
    },
    /// Closing a tab does not emit `PaneClosed` for its panes.
    TabClosed {
        tab_id: TabId,
        workspace_id: WorkspaceId,
    },
    TabFocused {
        tab_id: TabId,
        workspace_id: WorkspaceId,
    },
    PaneCreated(PaneInfo),
    /// Title, cwd, agent status or scroll changed.
    PaneUpdated(PaneInfo),
    PaneClosed {
        pane_id: PaneId,
        workspace_id: WorkspaceId,
    },
    PaneExited {
        pane_id: PaneId,
        workspace_id: WorkspaceId,
    },
    PaneFocused {
        pane_id: PaneId,
        workspace_id: WorkspaceId,
    },
    /// The pane keeps its process and terminal ID; its pane ID may change.
    PaneMoved {
        previous_pane_id: PaneId,
        previous_tab_id: TabId,
        previous_workspace_id: WorkspaceId,
        pane: PaneInfo,
    },
    AgentStatusChanged {
        pane_id: PaneId,
        workspace_id: WorkspaceId,
        agent: Option<String>,
        status: AgentStatus,
    },
    /// A tab's split tree, zoom or focus changed. Read the tree with
    /// [`crate::Client::layout_export`].
    LayoutUpdated {
        workspace_id: WorkspaceId,
        tab_id: TabId,
        zoomed: bool,
    },
    ScrollChanged {
        pane_id: PaneId,
        workspace_id: WorkspaceId,
        scroll: ScrollInfo,
    },
    Other {
        kind: String,
        data: Value,
    },
}

/// What a subscription line turned out to be.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Line {
    Started,
    Event(HerdrEvent),
    /// herdr dropped events for a slow reader (0.9.2+): resync from a snapshot.
    Lost,
    Error {
        code: String,
        message: String,
    },
}

#[derive(Deserialize)]
struct WsData {
    workspace: WorkspaceInfo,
}
#[derive(Deserialize)]
struct WsIdData {
    workspace_id: WorkspaceId,
}
#[derive(Deserialize)]
struct WsRenamed {
    workspace_id: WorkspaceId,
    label: String,
}
#[derive(Deserialize)]
struct TabData {
    tab: TabInfo,
}
#[derive(Deserialize)]
struct TabIdData {
    tab_id: TabId,
    workspace_id: WorkspaceId,
}
#[derive(Deserialize)]
struct TabRenamed {
    tab_id: TabId,
    workspace_id: WorkspaceId,
    label: String,
}
#[derive(Deserialize)]
struct PaneData {
    pane: PaneInfo,
}
#[derive(Deserialize)]
struct PaneIdData {
    pane_id: PaneId,
    workspace_id: WorkspaceId,
}
#[derive(Deserialize)]
struct PaneMovedData {
    previous_pane_id: PaneId,
    previous_tab_id: TabId,
    previous_workspace_id: WorkspaceId,
    pane: PaneInfo,
}
#[derive(Deserialize)]
struct AgentData {
    pane_id: PaneId,
    workspace_id: WorkspaceId,
    #[serde(default)]
    agent: Option<String>,
    agent_status: AgentStatus,
}
#[derive(Deserialize)]
struct LayoutData {
    layout: LayoutHead,
}
#[derive(Deserialize)]
struct LayoutHead {
    workspace_id: WorkspaceId,
    tab_id: TabId,
    #[serde(default)]
    zoomed: bool,
}
#[derive(Deserialize)]
struct ScrollData {
    pane_id: PaneId,
    workspace_id: WorkspaceId,
    scroll: ScrollInfo,
}

#[derive(Deserialize)]
struct RawLine {
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<RawError>,
}

#[derive(Deserialize, Serialize)]
struct RawError {
    code: String,
    message: String,
}

pub(crate) fn parse_line(line: &str) -> Result<Line, String> {
    let raw: RawLine = serde_json::from_str(line).map_err(|e| format!("{e}: {line}"))?;
    if let Some(err) = raw.error {
        if err.code == "events_lost" {
            return Ok(Line::Lost);
        }
        return Ok(Line::Error {
            code: err.code,
            message: err.message,
        });
    }
    if let Some(result) = raw.result {
        if result.get("type").and_then(Value::as_str) == Some("subscription_started") {
            return Ok(Line::Started);
        }
        return Err(format!("unexpected result on a subscription: {line}"));
    }
    let kind = raw
        .event
        .ok_or_else(|| format!("no `event` field: {line}"))?;
    let data = raw.data.unwrap_or(Value::Null);
    if kind.replace('.', "_") == "events_lost" {
        return Ok(Line::Lost);
    }
    Ok(Line::Event(parse_event(&kind, data)))
}

fn parse_event(kind: &str, data: Value) -> HerdrEvent {
    fn de<T: serde::de::DeserializeOwned>(v: &Value) -> Option<T> {
        serde_json::from_value(v.clone()).ok()
    }
    let normal = kind.replace('.', "_");
    let parsed = match normal.as_str() {
        "workspace_created" => {
            de::<WsData>(&data).map(|d| HerdrEvent::WorkspaceCreated(d.workspace))
        }
        "workspace_updated" => {
            de::<WsData>(&data).map(|d| HerdrEvent::WorkspaceUpdated(d.workspace))
        }
        "workspace_renamed" => de::<WsRenamed>(&data).map(|d| HerdrEvent::WorkspaceRenamed {
            workspace_id: d.workspace_id,
            label: d.label,
        }),
        // herdr 0.9.1 sends the whole workspace; accept a bare ID too.
        "workspace_closed" => de::<WsIdData>(&data)
            .map(|d| d.workspace_id)
            .or_else(|| de::<WsData>(&data).map(|d| d.workspace.workspace_id))
            .map(|workspace_id| HerdrEvent::WorkspaceClosed { workspace_id }),
        "workspace_focused" => de::<WsIdData>(&data).map(|d| HerdrEvent::WorkspaceFocused {
            workspace_id: d.workspace_id,
        }),
        "tab_created" => de::<TabData>(&data).map(|d| HerdrEvent::TabCreated(d.tab)),
        "tab_renamed" => de::<TabRenamed>(&data).map(|d| HerdrEvent::TabRenamed {
            tab_id: d.tab_id,
            workspace_id: d.workspace_id,
            label: d.label,
        }),
        "tab_closed" => de::<TabIdData>(&data).map(|d| HerdrEvent::TabClosed {
            tab_id: d.tab_id,
            workspace_id: d.workspace_id,
        }),
        "tab_focused" => de::<TabIdData>(&data).map(|d| HerdrEvent::TabFocused {
            tab_id: d.tab_id,
            workspace_id: d.workspace_id,
        }),
        "pane_created" => de::<PaneData>(&data).map(|d| HerdrEvent::PaneCreated(d.pane)),
        "pane_updated" => de::<PaneData>(&data).map(|d| HerdrEvent::PaneUpdated(d.pane)),
        "pane_closed" => de::<PaneIdData>(&data).map(|d| HerdrEvent::PaneClosed {
            pane_id: d.pane_id,
            workspace_id: d.workspace_id,
        }),
        "pane_exited" => de::<PaneIdData>(&data).map(|d| HerdrEvent::PaneExited {
            pane_id: d.pane_id,
            workspace_id: d.workspace_id,
        }),
        "pane_focused" => de::<PaneIdData>(&data).map(|d| HerdrEvent::PaneFocused {
            pane_id: d.pane_id,
            workspace_id: d.workspace_id,
        }),
        "pane_moved" => de::<PaneMovedData>(&data).map(|d| HerdrEvent::PaneMoved {
            previous_pane_id: d.previous_pane_id,
            previous_tab_id: d.previous_tab_id,
            previous_workspace_id: d.previous_workspace_id,
            pane: d.pane,
        }),
        "pane_agent_status_changed" => {
            de::<AgentData>(&data).map(|d| HerdrEvent::AgentStatusChanged {
                pane_id: d.pane_id,
                workspace_id: d.workspace_id,
                agent: d.agent,
                status: d.agent_status,
            })
        }
        "layout_updated" => de::<LayoutData>(&data).map(|d| HerdrEvent::LayoutUpdated {
            workspace_id: d.layout.workspace_id,
            tab_id: d.layout.tab_id,
            zoomed: d.layout.zoomed,
        }),
        "pane_scroll_changed" => de::<ScrollData>(&data).map(|d| HerdrEvent::ScrollChanged {
            pane_id: d.pane_id,
            workspace_id: d.workspace_id,
            scroll: d.scroll,
        }),
        _ => None,
    };
    parsed.unwrap_or(HerdrEvent::Other { kind: normal, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Lines captured from herdr 0.9.1 (protocol 22).
    const STARTED: &str = r#"{"id":"sub","result":{"type":"subscription_started"}}"#;
    const PANE_CREATED: &str = r#"{"data":{"pane":{"agent_status":"unknown","cwd":"/Users/example","focused":false,"foreground_cwd":"/Users/example","pane_id":"w1:p2","revision":0,"scroll":{"max_offset_from_bottom":0,"offset_from_bottom":0,"viewport_rows":40},"tab_id":"w1:t2","terminal_id":"term_0123456789abcd","workspace_id":"w1"},"type":"pane_created"},"event":"pane_created"}"#;
    const TAB_CLOSED: &str = r#"{"data":{"tab_id":"w1:t2","type":"tab_closed","workspace_id":"w1"},"event":"tab_closed"}"#;
    const WS_CREATED: &str = r#"{"data":{"type":"workspace_created","workspace":{"active_tab_id":"w1:t1","agent_status":"unknown","focused":true,"label":"azj","number":1,"pane_count":1,"tab_count":1,"workspace_id":"w1"}},"event":"workspace_created"}"#;

    #[test]
    fn parses_subscription_started() {
        assert_eq!(parse_line(STARTED).unwrap(), Line::Started);
    }

    #[test]
    fn parses_pane_created() {
        let Line::Event(HerdrEvent::PaneCreated(p)) = parse_line(PANE_CREATED).unwrap() else {
            panic!("not a pane_created");
        };
        assert_eq!(p.pane_id.as_str(), "w1:p2");
        assert_eq!(p.terminal_id.as_str(), "term_0123456789abcd");
        assert_eq!(p.scroll.unwrap().viewport_rows, 40);
    }

    #[test]
    fn parses_tab_closed() {
        assert_eq!(
            parse_line(TAB_CLOSED).unwrap(),
            Line::Event(HerdrEvent::TabClosed {
                tab_id: "w1:t2".into(),
                workspace_id: "w1".into()
            })
        );
    }

    #[test]
    fn parses_workspace_closed_with_the_whole_workspace() {
        // Captured from herdr 0.9.1 after closing a workspace's last tab.
        let line = r#"{"data":{"type":"workspace_closed","workspace":{"active_tab_id":"w2:t2","agent_status":"unknown","focused":false,"label":"b","number":2,"pane_count":1,"tab_count":1,"workspace_id":"w2"}},"event":"workspace_closed"}"#;
        assert_eq!(
            parse_line(line).unwrap(),
            Line::Event(HerdrEvent::WorkspaceClosed {
                workspace_id: "w2".into()
            })
        );
    }

    #[test]
    fn parses_workspace_created() {
        let Line::Event(HerdrEvent::WorkspaceCreated(w)) = parse_line(WS_CREATED).unwrap() else {
            panic!("not a workspace_created");
        };
        assert_eq!(w.label.as_deref(), Some("azj"));
        assert_eq!(w.active_tab_id, Some("w1:t1".into()));
    }

    #[test]
    fn parses_dotted_agent_status() {
        let line = r#"{"event":"pane.agent_status_changed","data":{"pane_id":"w1:p1","workspace_id":"w1","agent":"claude","agent_status":"blocked"}}"#;
        assert_eq!(
            parse_line(line).unwrap(),
            Line::Event(HerdrEvent::AgentStatusChanged {
                pane_id: "w1:p1".into(),
                workspace_id: "w1".into(),
                agent: Some("claude".into()),
                status: AgentStatus::Blocked,
            })
        );
    }

    #[test]
    fn unknown_status_maps_to_unknown() {
        let line = r#"{"event":"pane_agent_status_changed","data":{"pane_id":"w1:p1","workspace_id":"w1","agent_status":"pondering"}}"#;
        let Line::Event(HerdrEvent::AgentStatusChanged { status, .. }) = parse_line(line).unwrap()
        else {
            panic!("not an agent status event");
        };
        assert_eq!(status, AgentStatus::Unknown);
    }

    #[test]
    fn unknown_kinds_are_kept() {
        let line = r#"{"event":"pane_output_changed","data":{"type":"pane_output_changed","pane_id":"w1:p1"}}"#;
        assert!(matches!(
            parse_line(line).unwrap(),
            Line::Event(HerdrEvent::Other { kind, .. }) if kind == "pane_output_changed"
        ));
    }

    #[test]
    fn parses_layout_updated() {
        // Captured from herdr 0.9.1 after a zoom (trimmed).
        let line = r#"{"data":{"layout":{"area":{"height":40,"width":120,"x":0,"y":0},"focused_pane_id":"w1:p2","panes":[{"focused":true,"pane_id":"w1:p2","rect":{"height":10,"width":48,"x":72,"y":0}}],"splits":[{"direction":"right","id":"split_0_root","ratio":0.6,"rect":{"height":40,"width":120,"x":0,"y":0}}],"tab_id":"w1:t1","workspace_id":"w1","zoomed":true},"type":"layout_updated"},"event":"layout_updated"}"#;
        assert_eq!(
            parse_line(line).unwrap(),
            Line::Event(HerdrEvent::LayoutUpdated {
                workspace_id: "w1".into(),
                tab_id: "w1:t1".into(),
                zoomed: true,
            })
        );
    }

    #[test]
    fn events_lost_is_detected() {
        let line = r#"{"id":"sub","error":{"code":"events_lost","message":"slow reader"}}"#;
        assert_eq!(parse_line(line).unwrap(), Line::Lost);
    }

    #[test]
    fn subscriptions_serialise() {
        assert_eq!(
            Subscription::PaneCreated.to_wire(),
            json!({"type": "pane.created"})
        );
        assert_eq!(
            Subscription::AgentStatus("w1:p3".into()).to_wire(),
            json!({"type": "pane.agent_status_changed", "pane_id": "w1:p3"})
        );
    }
}
