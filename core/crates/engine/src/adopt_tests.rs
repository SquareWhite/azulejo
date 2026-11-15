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
    engine.apply_internal(Internal::Surveyed(None, Box::new(s.clone()), 0))
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

/// ⌘W closes a workspace (F14): every pane detaches, nothing is killed, and
/// neither the next survey nor a relaunch brings the workspace back.
#[test]
fn a_closed_workspace_detaches_and_stays_closed() {
    use azulejo_protocol::WorkspaceTarget;
    let mut engine = Engine::new();
    survey(&mut engine, &fixture());
    let api = engine.session().windows[0].workspaces[0].clone();
    assert_eq!(api.herdr_workspace.as_deref(), Some("w1"));

    let outcome = engine
        .execute(Command::WorkspaceClose(WorkspaceTarget {
            workspace: Some(api.id),
        }))
        .unwrap();
    let closed: Vec<_> = outcome
        .events
        .iter()
        .filter_map(|e| match e {
            Event::TileClosed { kill, .. } => Some(*kill),
            _ => None,
        })
        .collect();
    assert_eq!(closed, [false; 4], "four tiles close, none killed");
    assert!(
        crate::link::effects(&outcome.events, engine.session()).is_empty(),
        "nothing reaches herdr"
    );
    let mut detached: Vec<&str> = engine
        .session()
        .detached_herdr
        .iter()
        .map(|r| r.pane_id.as_str())
        .collect();
    detached.sort_unstable();
    assert_eq!(detached, ["w1:p1", "w1:p2", "w1:p3", "w1:p4"]);

    // herdr still runs all of it; the next survey adopts nothing.
    assert!(survey(&mut engine, &fixture()).is_empty());
    let names = |s: &Session| -> Vec<String> {
        s.windows[0]
            .workspaces
            .iter()
            .map(|w| w.name.clone())
            .collect()
    };
    assert_eq!(names(engine.session()), ["herdr 2"]);

    // Relaunch from the saved session: still closed.
    let saved: Session =
        serde_json::from_value(serde_json::to_value(engine.session()).unwrap()).unwrap();
    let mut relaunched = Engine::with_session(saved);
    assert!(survey(&mut relaunched, &fixture()).is_empty());
    assert_eq!(names(relaunched.session()), ["herdr 2"]);
    assert_eq!(relaunched.session().check_invariants(), Ok(()));
}

/// A workspace closed while herdr was still making a pane for one of its
/// tiles: the pane arrives for a tile that's gone, and counts as detached.
#[test]
fn a_pane_made_for_a_closed_workspace_isnt_adopted() {
    use azulejo_model::HerdrRef;
    use azulejo_protocol::{TabCreate, WorkspaceCreate, WorkspaceTarget};
    let mut engine = Engine::new();
    survey(&mut engine, &fixture());
    let created = engine
        .execute(Command::WorkspaceCreate(WorkspaceCreate::default()))
        .unwrap();
    let azulejo_protocol::Reply::Created(c) = created.reply else {
        panic!("{:?}", created.reply)
    };
    let workspace = c.workspace.unwrap();
    let tab = engine
        .execute(Command::TabCreate(TabCreate {
            workspace: Some(workspace),
            ..TabCreate::default()
        }))
        .unwrap();
    let azulejo_protocol::Reply::Created(t) = tab.reply else {
        panic!("{:?}", tab.reply)
    };
    let tile = t.tile.unwrap();
    engine
        .execute(Command::WorkspaceClose(WorkspaceTarget {
            workspace: Some(workspace),
        }))
        .unwrap();

    // herdr made the pane (in a new herdr workspace w3) after all.
    let late = HerdrRef {
        pane_id: "w3:p1".into(),
        terminal_id: "term_w3_p1".into(),
        tab_id: "w3:t1".into(),
        machine: None,
    };
    assert!(
        engine
            .apply_internal(Internal::BindTerminal {
                tile,
                herdr: late.clone(),
            })
            .is_empty()
    );
    assert!(engine.session().detached_herdr.contains(&late));
    let mut more = fixture();
    more.workspaces.push(
        serde_json::from_value(json!({ "workspace_id": "w3", "number": 3, "label": "late" }))
            .unwrap(),
    );
    more.tabs.push(
        serde_json::from_value(json!({ "tab_id": "w3:t1", "workspace_id": "w3", "number": 1 }))
            .unwrap(),
    );
    more.panes.push(
        serde_json::from_value(json!({
            "pane_id": "w3:p1", "terminal_id": "term_w3_p1", "workspace_id": "w3", "tab_id": "w3:t1"
        }))
        .unwrap(),
    );
    assert!(survey(&mut engine, &more).is_empty(), "not adopted");
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
fn a_layout_naming_a_pane_the_snapshot_lacks_is_taken_without_it() {
    let mut s = fixture();
    // herdr's tree names a pane the snapshot doesn't have: the tree without
    // it, (p1 | (p2 / p3)) less p3.
    s.panes.retain(|p| p.pane_id.as_str() != "w1:p3");
    let mut engine = Engine::new();
    survey(&mut engine, &s);
    let session = engine.session();
    assert_eq!(
        shape(session, &session.windows[0].workspaces[0].tabs[0].root),
        "(w1:p1 | 0.50 w1:p2)"
    );
}

/// A survey requested before herdr streamed a pane's newer agent status
/// can't put the older status back (or clear attention). A later survey can.
#[test]
fn a_survey_older_than_a_streamed_agent_status_keeps_the_streamed_one() {
    use azulejo_model::AgentStatus;
    let mut engine = Engine::new();
    let mut s = fixture();
    let set = |s: &mut herdr::Survey, status: herdr::AgentStatus| {
        let p = s
            .panes
            .iter_mut()
            .find(|p| p.pane_id.as_str() == "w1:p1")
            .unwrap();
        p.agent = Some("claude".into());
        p.agent_status = status;
    };
    set(&mut s, herdr::AgentStatus::Working);
    engine.apply_internal(Internal::Surveyed(None, Box::new(s.clone()), 1_000));
    let agent = |e: &Engine| {
        e.session()
            .tiles()
            .find(|(_, t)| pane_of_tile(t) == Some("w1:p1"))
            .and_then(|(_, t)| t.agent.clone())
            .unwrap()
    };
    assert_eq!(agent(&engine).status, AgentStatus::Working);

    // herdr streams "blocked" at 2,000 ms.
    engine.apply_internal(Internal::AgentStatus {
        machine: None,
        pane_id: "w1:p1".into(),
        agent: Some("claude".into()),
        status: AgentStatus::Blocked,
        at_ms: 2_000,
    });
    let a = agent(&engine);
    assert_eq!((a.status, a.attention), (AgentStatus::Blocked, true));

    // A survey requested at 1,500 (its snapshot still says working) lands late.
    engine.apply_internal(Internal::Surveyed(None, Box::new(s.clone()), 1_500));
    let a = agent(&engine);
    assert_eq!(
        (a.status, a.attention),
        (AgentStatus::Blocked, true),
        "stale survey won"
    );

    // One requested after the stream's report is believed.
    engine.apply_internal(Internal::Surveyed(None, Box::new(s), 2_500));
    let a = agent(&engine);
    assert_eq!((a.status, a.attention), (AgentStatus::Working, false));
}

fn pane_of_tile(t: &azulejo_model::Tile) -> Option<&str> {
    match &t.content {
        TileContent::Terminal { herdr: Some(h), .. } => Some(h.pane_id.as_str()),
        _ => None,
    }
}

/// A survey requested before herdr streamed a pane's new title and cwd can't
/// put the older ones back in the header. A later survey can.
#[test]
fn a_survey_older_than_streamed_pane_info_keeps_the_streamed_info() {
    let mut engine = Engine::new();
    let mut s = fixture();
    let set = |s: &mut herdr::Survey, title: &str, cwd: &str| {
        let p = s
            .panes
            .iter_mut()
            .find(|p| p.pane_id.as_str() == "w1:p1")
            .unwrap();
        p.terminal_title = Some(title.into());
        p.foreground_cwd = Some(cwd.into());
    };
    set(&mut s, "old title", "/old");
    engine.apply_internal(Internal::Surveyed(None, Box::new(s.clone()), 1_000));
    let meta = |e: &Engine| {
        e.session()
            .tiles()
            .find(|(_, t)| pane_of_tile(t) == Some("w1:p1"))
            .and_then(|(_, t)| t.pane.clone())
            .unwrap()
    };
    assert_eq!(meta(&engine).title.as_deref(), Some("old title"));

    // herdr streams the new title and cwd (stamped with the real clock, so
    // after any survey requested at the fixed times below).
    engine.apply_internal(Internal::PaneInfo {
        machine: None,
        pane_id: "w1:p1".into(),
        title: Some("vim plan.md".into()),
        cwd: Some("/new".into()),
    });
    assert_eq!(meta(&engine).title.as_deref(), Some("vim plan.md"));

    // A survey requested earlier (its snapshot still has the old ones) lands late.
    engine.apply_internal(Internal::Surveyed(None, Box::new(s.clone()), 1_500));
    let m = meta(&engine);
    assert_eq!(
        (m.title.as_deref(), m.cwd.as_deref()),
        (Some("vim plan.md"), Some("/new")),
        "stale survey won"
    );

    // One requested after the stream's report is believed.
    let later = crate::agent::now_ms() + 60_000;
    engine.apply_internal(Internal::Surveyed(None, Box::new(s), later));
    let m = meta(&engine);
    assert_eq!(
        (m.title.as_deref(), m.cwd.as_deref()),
        (Some("old title"), Some("/old"))
    );
}

/// The tile showing a pane, if any.
fn tile_of_pane(session: &Session, pane: &str) -> Option<azulejo_model::TileId> {
    session
        .tiles()
        .find(|(_, t)| pane_of_tile(t) == Some(pane))
        .map(|(_, t)| t.id)
}

/// A survey requested before herdr reported a pane, tab or workspace closed
/// still lists it; applied late, it mustn't bring the tiles back. A survey
/// requested after the close is believed.
#[test]
fn a_survey_older_than_a_close_in_herdr_doesnt_bring_the_pane_back() {
    let mut engine = Engine::new();
    survey(&mut engine, &fixture());
    let stale = fixture();

    // A one-pane tab (w1:t2) and a split-off pane (w1:p3 in w1:t1) close in
    // herdr; so does workspace w2.
    for pane in ["w1:p4", "w1:p3"] {
        let tile = tile_of_pane(engine.session(), pane).unwrap();
        engine.apply_internal(Internal::ClosedElsewhere { tile });
        assert!(tile_of_pane(engine.session(), pane).is_none());
    }
    engine.apply_internal(Internal::HerdrWorkspaceClosed {
        machine: None,
        herdr: "w2".into(),
    });
    assert!(tile_of_pane(engine.session(), "w2:p1").is_none());

    // The survey requested before those closes lands now.
    let events = engine.apply_internal(Internal::Surveyed(None, Box::new(stale.clone()), 1_000));
    for pane in ["w1:p4", "w1:p3", "w2:p1", "w2:p2"] {
        assert!(
            tile_of_pane(engine.session(), pane).is_none(),
            "stale survey re-adopted {pane}: {events:?}"
        );
    }
    assert_eq!(engine.session().check_invariants(), Ok(()));

    // One requested after the closes, still listing w1:p4, is believed.
    let later = crate::agent::now_ms() + 60_000;
    engine.apply_internal(Internal::Surveyed(None, Box::new(stale), later));
    assert!(tile_of_pane(engine.session(), "w1:p4").is_some());
}

// ---- bursts of splits made in herdr ----------------------------------------

pub(crate) fn hp(id: &str) -> herdr::LayoutNode {
    herdr::LayoutNode::pane(id)
}

pub(crate) fn hright(a: herdr::LayoutNode, b: herdr::LayoutNode) -> herdr::LayoutNode {
    herdr::LayoutNode::split(herdr::SplitDirection::Right, 0.5, a, b)
}

pub(crate) fn hdown(a: herdr::LayoutNode, b: herdr::LayoutNode) -> herdr::LayoutNode {
    herdr::LayoutNode::split(herdr::SplitDirection::Down, 0.5, a, b)
}

/// herdr running one workspace, `w9`, with one tab, `w9:t1`, holding
/// `panes`, laid out as `root` (`layout.export`, read after the snapshot,
/// may already hold panes the snapshot doesn't).
pub(crate) fn one_tab(panes: &[&str], root: &herdr::LayoutNode) -> herdr::Survey {
    herdr::Survey {
        workspaces: vec![
            serde_json::from_value(json!({
                "workspace_id": "w9", "number": 1, "label": "burst", "active_tab_id": "w9:t1"
            }))
            .unwrap(),
        ],
        tabs: vec![
            serde_json::from_value(json!({
                "tab_id": "w9:t1", "workspace_id": "w9", "number": 1
            }))
            .unwrap(),
        ],
        panes: panes
            .iter()
            .map(|id| {
                serde_json::from_value(json!({
                    "pane_id": id,
                    "terminal_id": format!("term_{}", id.replace(':', "_")),
                    "workspace_id": "w9",
                    "tab_id": "w9:t1",
                }))
                .unwrap()
            })
            .collect(),
        // herdr exports a tree only for tabs with more than one pane.
        layouts: (root.panes().len() > 1)
            .then(|| herdr::TabLayout {
                workspace_id: "w9".into(),
                tab_id: "w9:t1".into(),
                zoomed: false,
                focused_pane_id: root.first_pane().cloned(),
                root: root.clone(),
            })
            .into_iter()
            .collect(),
        focused_workspace_id: Some("w9".into()),
    }
}

/// herdr's tree in [`shape`]'s notation.
fn herdr_shape(node: &herdr::LayoutNode) -> String {
    match node {
        herdr::LayoutNode::Pane { pane_id } => pane_id.as_ref().unwrap().0.clone(),
        herdr::LayoutNode::Split {
            direction,
            ratio,
            first,
            second,
        } => format!(
            "({} {} {:.2} {})",
            herdr_shape(first),
            match direction {
                herdr::SplitDirection::Right => "|",
                herdr::SplitDirection::Down => "/",
            },
            ratio,
            herdr_shape(second)
        ),
    }
}

/// herdr's tree after each split of a burst: (panes so far, tree).
fn stages(splits: &[(usize, herdr::SplitDirection)]) -> Vec<(Vec<String>, herdr::LayoutNode)> {
    fn split_leaf(node: &mut herdr::LayoutNode, target: &str, d: herdr::SplitDirection, new: &str) {
        match node {
            herdr::LayoutNode::Pane { pane_id } if pane_id.as_ref().unwrap().as_str() == target => {
                *node = herdr::LayoutNode::split(d, 0.5, node.clone(), hp(new));
            }
            herdr::LayoutNode::Pane { .. } => {}
            herdr::LayoutNode::Split { first, second, .. } => {
                split_leaf(first, target, d, new);
                split_leaf(second, target, d, new);
            }
        }
    }
    let mut panes = vec!["w9:p1".to_string()];
    let mut root = hp("w9:p1");
    let mut out = vec![(panes.clone(), root.clone())];
    for (i, (target, d)) in splits.iter().enumerate() {
        let new = format!("w9:p{}", i + 2);
        split_leaf(&mut root, &panes[*target], *d, &new);
        panes.push(new);
        out.push((panes.clone(), root.clone()));
    }
    out
}

fn adopted_shape(engine: &Engine) -> String {
    let s = engine.session();
    let tab = s
        .windows
        .iter()
        .flat_map(|w| &w.workspaces)
        .find(|w| w.herdr_workspace.as_deref() == Some("w9"))
        .map(|w| &w.tabs[0])
        .expect("adopted");
    shape(s, &tab.root)
}

/// A burst of splits (herdr-grid.sh without its pauses, a script, an agent)
/// reaches a busy core's surveys half made: one survey sees the first pane
/// alone, the next all of them. Placing each pane beside its sibling can't
/// work then (two new siblings know nobody here); the tab still ends with
/// herdr's tree, whichever stages the surveys caught. It once came in as
/// "master on top over three across", and the mirror pushed that back.
#[test]
fn a_burst_seen_half_made_still_ends_in_herdrs_tree() {
    use herdr::SplitDirection::{Down, Right};
    let recipes: [&[(usize, herdr::SplitDirection)]; 3] = [
        // 2×2 grid: right, then down in each column.
        &[(0, Right), (0, Down), (1, Down)],
        // Three columns.
        &[(0, Right), (1, Right)],
        // Nested.
        &[(0, Right), (1, Down), (2, Right), (0, Down)],
    ];
    for splits in recipes {
        let stages = stages(splits);
        let last = stages.len() - 1;
        // Every set of in-between stages the surveys could have caught.
        for mask in 0..(1u32 << last) {
            let mut engine = Engine::new();
            let caught: Vec<usize> = (0..last)
                .filter(|i| mask & (1 << i) != 0)
                .chain([last])
                .collect();
            for &i in &caught {
                let (panes, root) = &stages[i];
                let panes: Vec<&str> = panes.iter().map(String::as_str).collect();
                survey(&mut engine, &one_tab(&panes, root));
                assert_eq!(engine.session().check_invariants(), Ok(()));
            }
            assert_eq!(
                adopted_shape(&engine),
                herdr_shape(&stages[last].1),
                "stages {caught:?} of {splits:?}"
            );
        }
    }
}

/// `layout.export` is read after the snapshot: a pane split off in between
/// is in the tree but not in the panes. The tab takes herdr's tree without
/// it, and the next survey brings it in, in place.
#[test]
fn a_tree_newer_than_the_snapshot_is_taken_without_the_newer_pane() {
    let grid = hright(
        hdown(hp("w9:p1"), hp("w9:p3")),
        hdown(hp("w9:p2"), hp("w9:p4")),
    );
    let mut engine = Engine::new();
    survey(&mut engine, &one_tab(&["w9:p1", "w9:p2", "w9:p3"], &grid));
    assert_eq!(
        adopted_shape(&engine),
        "((w9:p1 / 0.50 w9:p3) | 0.50 w9:p2)"
    );
    survey(
        &mut engine,
        &one_tab(&["w9:p1", "w9:p2", "w9:p3", "w9:p4"], &grid),
    );
    assert_eq!(adopted_shape(&engine), herdr_shape(&grid));
}
