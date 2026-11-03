//! Adoption from a herdr survey, without a running core.

use azulejo_herdr as herdr;
use azulejo_model::{Axis, Node, Session, TileContent};
use azulejo_protocol::{Command, Event, TileClose};
use serde_json::json;

use crate::{Engine, Internal};

/// The user's herdr: workspace `api` with a three-pane tab (p1 | p2 / p3) and
/// a one-pane tab; an unlabelled workspace with a two-pane tab herdr gave no
/// layout for.
pub(crate) fn fixture() -> herdr::Survey {
    let pane = |id: &str, tab: &str, ws: &str, focused: bool| -> herdr::PaneInfo {
        serde_json::from_value(json!({
            "pane_id": id,
            "terminal_id": format!("term_{}", id.replace(':', "_")),
            "workspace_id": ws,
            "tab_id": tab,
            "focused": focused,
            "cwd": "/Users/u/src",
        }))
        .unwrap()
    };
    herdr::Survey {
        workspaces: vec![
            serde_json::from_value(json!({
                "workspace_id": "w1", "number": 1, "label": "api", "active_tab_id": "w1:t2"
            }))
            .unwrap(),
            serde_json::from_value(json!({ "workspace_id": "w2", "number": 2 })).unwrap(),
        ],
        tabs: vec![
            serde_json::from_value(json!({
                "tab_id": "w1:t1", "workspace_id": "w1", "number": 1, "label": "1"
            }))
            .unwrap(),
            serde_json::from_value(json!({
                "tab_id": "w1:t2", "workspace_id": "w1", "number": 2, "label": "logs"
            }))
            .unwrap(),
            serde_json::from_value(json!({
                "tab_id": "w2:t1", "workspace_id": "w2", "number": 1
            }))
            .unwrap(),
        ],
        panes: vec![
            pane("w1:p1", "w1:t1", "w1", false),
            pane("w1:p2", "w1:t1", "w1", true),
            pane("w1:p3", "w1:t1", "w1", false),
            pane("w1:p4", "w1:t2", "w1", true),
            pane("w2:p1", "w2:t1", "w2", true),
            pane("w2:p2", "w2:t1", "w2", false),
        ],
        // The shape `layout.export` returns (herdr 0.9.1).
        layouts: vec![
            serde_json::from_value(json!({
                "workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
                "focused_pane_id": "w1:p2",
                "root": {
                    "type": "split", "direction": "right", "ratio": 0.5,
                    "first": { "type": "pane", "pane_id": "w1:p1", "cwd": "/Users/u" },
                    "second": {
                        "type": "split", "direction": "down", "ratio": 0.3,
                        "first": { "type": "pane", "pane_id": "w1:p2" },
                        "second": { "type": "pane", "pane_id": "w1:p3" }
                    }
                }
            }))
            .unwrap(),
        ],
        focused_workspace_id: Some("w2".into()),
    }
}

fn survey(engine: &mut Engine, s: &herdr::Survey) -> Vec<Event> {
    engine.apply_internal(Internal::Surveyed(Box::new(s.clone())))
}

fn pane_of(session: &Session, tile: azulejo_model::TileId) -> String {
    match &session.tile(tile).unwrap().content {
        TileContent::Terminal { herdr: Some(h), .. } => h.pane_id.clone(),
        other => panic!("not a bound terminal: {other:?}"),
    }
}

/// The tree with pane IDs at the leaves, for comparing.
fn shape(session: &Session, node: &Node) -> String {
    match node {
        Node::Leaf { id } => pane_of(session, *id),
        Node::Split { axis, ratio, a, b } => format!(
            "({} {} {:.2} {})",
            shape(session, a),
            match axis {
                Axis::Horizontal => "|",
                Axis::Vertical => "/",
            },
            ratio,
            shape(session, b)
        ),
    }
}

#[test]
fn fresh_session_adopts_herdr_and_drops_the_placeholder() {
    let mut engine = Engine::new();
    let events = survey(&mut engine, &fixture());
    assert!(!events.is_empty());
    let s = engine.session();
    assert_eq!(s.check_invariants(), Ok(()));
    let ws = &s.windows[0].workspaces;
    let names: Vec<&str> = ws.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["api", "herdr 2"], "placeholder `main` is gone");
    assert_eq!(ws[0].herdr_workspace.as_deref(), Some("w1"));
    assert_eq!(ws[1].herdr_workspace.as_deref(), Some("w2"));

    let api = &ws[0];
    assert_eq!(api.tabs.len(), 2);
    assert_eq!(
        shape(s, &api.tabs[0].root),
        "(w1:p1 | 0.50 (w1:p2 / 0.30 w1:p3))"
    );
    assert_eq!(pane_of(s, api.tabs[0].focused), "w1:p2");
    assert_eq!(api.tabs[1].name.as_deref(), Some("logs"));
    assert_eq!(api.active_tab, Some(api.tabs[1].id), "herdr's active tab");
    // No layout from herdr: side by side.
    assert_eq!(shape(s, &ws[1].tabs[0].root), "(w2:p1 | 0.50 w2:p2)");
    // herdr's focused workspace is focused here too.
    assert_eq!(s.windows[0].active_workspace, ws[1].id);

    let refs: Vec<_> = s
        .tiles()
        .map(|(_, t)| match &t.content {
            TileContent::Terminal {
                herdr: Some(h),
                ended: false,
                cwd,
                ..
            } => {
                assert_eq!(cwd.as_deref(), Some("/Users/u/src"));
                (h.pane_id.clone(), h.terminal_id.clone(), h.tab_id.clone())
            }
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(refs.len(), 6);
    assert!(refs.contains(&("w1:p3".into(), "term_w1_p3".into(), "w1:t1".into())));
}

#[test]
fn adopting_again_changes_nothing() {
    let mut engine = Engine::new();
    survey(&mut engine, &fixture());
    let before = engine.session().clone();
    assert!(survey(&mut engine, &fixture()).is_empty());
    assert_eq!(engine.session(), &before);
}

#[test]
fn placeholder_stays_when_herdr_is_empty() {
    let mut engine = Engine::new();
    assert!(survey(&mut engine, &herdr::Survey::default()).is_empty());
    assert_eq!(engine.session().windows[0].workspaces[0].name, "main");
    // Only the first survey may drop it.
    survey(&mut engine, &fixture());
    let names: Vec<_> = engine.session().windows[0]
        .workspaces
        .iter()
        .map(|w| w.name.clone())
        .collect();
    assert_eq!(names, ["main", "api", "herdr 2"]);
}

#[test]
fn saved_sessions_keep_focus_and_only_gain_new_tabs() {
    let mut engine = Engine::new();
    survey(&mut engine, &fixture());
    let mut saved = engine.session().clone();
    // Focus the first workspace, then relaunch from the saved session.
    saved.windows[0].active_workspace = saved.windows[0].workspaces[0].id;
    let mut engine = Engine::with_session(saved.clone());

    // Someone opened a new tab in `api` meanwhile.
    let mut more = fixture();
    more.tabs.push(
        serde_json::from_value(json!({ "tab_id": "w1:t3", "workspace_id": "w1", "number": 3 }))
            .unwrap(),
    );
    more.panes.push(
        serde_json::from_value(json!({
            "pane_id": "w1:p9", "terminal_id": "term_new", "workspace_id": "w1", "tab_id": "w1:t3"
        }))
        .unwrap(),
    );
    survey(&mut engine, &more);
    let s = engine.session();
    assert_eq!(s.check_invariants(), Ok(()));
    assert_eq!(s.windows[0].workspaces.len(), 2);
    assert_eq!(
        s.windows[0].active_workspace,
        saved.windows[0].workspaces[0].id
    );
    let api = &s.windows[0].workspaces[0];
    assert_eq!(api.tabs.len(), 3);
    assert_eq!(api.active_tab, saved.windows[0].workspaces[0].active_tab);
    assert_eq!(pane_of(s, api.tabs[2].focused), "w1:p9");
}

#[test]
fn closed_tiles_stay_closed_until_herdr_drops_the_pane() {
    let mut engine = Engine::new();
    survey(&mut engine, &fixture());
    // Close (detach) every tile of the `api` 3-pane tab.
    let tab = engine.session().windows[0].workspaces[0].tabs[0].clone();
    for tile in &tab.tiles {
        engine
            .execute(Command::TileClose(TileClose {
                tile: Some(tile.id),
                kill: false,
            }))
            .unwrap();
    }
    assert_eq!(engine.session().detached_herdr.len(), 3);
    assert!(survey(&mut engine, &fixture()).is_empty(), "not re-adopted");
    assert_eq!(engine.session().windows[0].workspaces[0].tabs.len(), 1);

    // herdr closed one of those panes: it's forgotten, the others stay.
    let mut fewer = fixture();
    fewer.panes.retain(|p| p.pane_id.as_str() != "w1:p1");
    fewer.layouts.clear();
    survey(&mut engine, &fewer);
    assert_eq!(engine.session().detached_herdr.len(), 2);
    assert_eq!(engine.session().windows[0].workspaces[0].tabs.len(), 1);
}

fn extra_pane(id: &str, tab: &str) -> herdr::PaneInfo {
    serde_json::from_value(json!({
        "pane_id": id,
        "terminal_id": format!("term_{}", id.replace(':', "_")),
        "workspace_id": "w1",
        "tab_id": tab,
    }))
    .unwrap()
}

#[test]
fn a_pane_split_off_in_herdr_lands_beside_its_sibling() {
    let mut engine = Engine::new();
    survey(&mut engine, &fixture());
    let focus = engine.session().focus();

    // In herdr, p3 was split to the left at 0.4: (p5 | p3) where p3 was.
    let mut more = fixture();
    more.panes.push(extra_pane("w1:p5", "w1:t1"));
    more.layouts[0] = serde_json::from_value(json!({
        "workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
        "focused_pane_id": "w1:p5",
        "root": {
            "type": "split", "direction": "right", "ratio": 0.5,
            "first": { "type": "pane", "pane_id": "w1:p1" },
            "second": {
                "type": "split", "direction": "down", "ratio": 0.3,
                "first": { "type": "pane", "pane_id": "w1:p2" },
                "second": {
                    "type": "split", "direction": "right", "ratio": 0.4,
                    "first": { "type": "pane", "pane_id": "w1:p5" },
                    "second": { "type": "pane", "pane_id": "w1:p3" }
                }
            }
        }
    }))
    .unwrap();
    let events = survey(&mut engine, &more);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::TileCreated { .. })),
        "{events:?}"
    );
    let s = engine.session();
    assert_eq!(s.check_invariants(), Ok(()));
    assert_eq!(
        shape(s, &s.windows[0].workspaces[0].tabs[0].root),
        "(w1:p1 | 0.50 (w1:p2 / 0.30 (w1:p5 | 0.40 w1:p3)))"
    );
    assert_eq!(s.focus(), focus, "focus stays put");
    // Seen again: nothing new.
    assert!(survey(&mut engine, &more).is_empty());
}

#[test]
fn split_panes_without_a_tree_go_to_the_stack_unless_closed_here() {
    let mut engine = Engine::new();
    survey(&mut engine, &fixture());
    // w1:t2 ("logs") had one pane; herdr gave no layout for it.
    let mut more = fixture();
    more.panes.push(extra_pane("w1:p5", "w1:t2"));
    more.panes.push(extra_pane("w1:p6", "w1:t2"));
    survey(&mut engine, &more);
    let s = engine.session();
    assert_eq!(s.check_invariants(), Ok(()));
    let logs = &s.windows[0].workspaces[0].tabs[1];
    assert_eq!(shape(s, &logs.root), "(w1:p4 | 0.50 (w1:p5 / 0.50 w1:p6))");

    // Closed here: stays out, even though herdr still runs it.
    let p6 = logs
        .tiles
        .iter()
        .find(|t| pane_of(s, t.id) == "w1:p6")
        .unwrap()
        .id;
    engine
        .execute(Command::TileClose(TileClose {
            tile: Some(p6),
            kill: false,
        }))
        .unwrap();
    assert!(survey(&mut engine, &more).is_empty());
    assert_eq!(engine.session().tiles().count(), 7);
}

#[test]
fn a_restarted_herdr_is_matched_by_pane_and_tab() {
    let mut engine = Engine::new();
    survey(&mut engine, &fixture());
    // Same slots, fresh terminals (herdr restarted): nothing new to adopt.
    let mut restarted = fixture();
    for p in &mut restarted.panes {
        p.terminal_id = format!("{}_new", p.terminal_id).as_str().into();
    }
    assert!(survey(&mut engine, &restarted).is_empty());
}

#[test]
fn zoom_and_odd_ratios_carry_over_safely() {
    let mut s = fixture();
    s.layouts[0].zoomed = true;
    if let herdr::LayoutNode::Split { ratio, .. } = &mut s.layouts[0].root {
        *ratio = 1.0;
    }
    let mut engine = Engine::new();
    survey(&mut engine, &s);
    let session = engine.session();
    assert_eq!(session.check_invariants(), Ok(()));
    let tab = &session.windows[0].workspaces[0].tabs[0];
    assert_eq!(tab.zoomed, Some(tab.focused));
    assert!(tab.root.ratios().iter().all(|r| *r < 1.0));
}

#[test]
fn a_layout_that_disagrees_with_the_panes_falls_back() {
    let mut s = fixture();
    // herdr's tree names a pane the snapshot doesn't have.
    s.panes.retain(|p| p.pane_id.as_str() != "w1:p3");
    let mut engine = Engine::new();
    survey(&mut engine, &s);
    let session = engine.session();
    assert_eq!(
        shape(session, &session.windows[0].workspaces[0].tabs[0].root),
        "(w1:p1 | 0.50 w1:p2)"
    );
}
