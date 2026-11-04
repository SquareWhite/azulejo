//! ADR 0012: targets, split spots and absorbing herdr changes.

use azulejo_model::{LayoutKind, Tile};

use super::*;

fn terminal() -> Tile {
    Tile::new(TileContent::terminal())
}

fn browser() -> Tile {
    Tile::new(TileContent::Browser {
        url: None,
        page_title: None,
    })
}

/// Binds `tile` to pane `pane` in herdr tab `tab` on Local.
fn bind(s: &mut Session, tile: TileId, pane: &str, tab: &str) {
    if let TileContent::Terminal { herdr, .. } = &mut s.tile_mut(tile).unwrap().content {
        *herdr = Some(HerdrRef::local(pane, format!("term_{pane}"), tab));
    }
}

/// A manual tab: A | (B / C), with A and B terminals and C as given.
fn manual_tab(c: Tile) -> (Session, TabId, [TileId; 3]) {
    let mut s = Session::new();
    let (_, ws) = s.create_window("main", None);
    let a = terminal();
    let ids = [a.id, TileId::new(), TileId::new()];
    s.create_tab(ws, a, None).unwrap();
    let tab = s.focus().tab.unwrap();
    let mut b = terminal();
    b.id = ids[1];
    s.split(ids[0], Direction::Right, b).unwrap();
    let mut c = c;
    c.id = ids[2];
    s.split(ids[1], Direction::Down, c).unwrap();
    (s, tab, ids)
}

fn p(id: &str) -> herdr::LayoutNode {
    herdr::LayoutNode::pane(id)
}

fn right(r: f64, a: herdr::LayoutNode, b: herdr::LayoutNode) -> herdr::LayoutNode {
    herdr::LayoutNode::split(herdr::SplitDirection::Right, r, a, b)
}

fn down(r: f64, a: herdr::LayoutNode, b: herdr::LayoutNode) -> herdr::LayoutNode {
    herdr::LayoutNode::split(herdr::SplitDirection::Down, r, a, b)
}

fn export(tab: &str, root: herdr::LayoutNode) -> herdr::TabLayout {
    herdr::TabLayout {
        workspace_id: "w1".into(),
        tab_id: tab.into(),
        zoomed: false,
        focused_pane_id: root.first_pane().cloned(),
        root,
    }
}

#[test]
fn a_browser_gives_its_space_to_its_neighbour() {
    let (mut s, tab, [a, b, _]) = manual_tab(browser());
    // The old mapping: a herdr tab each.
    bind(&mut s, a, "w1:p1", "w1:t1");
    bind(&mut s, b, "w1:p2", "w1:t2");
    let t = &targets(&s, &None)[&tab];
    assert_eq!(
        t.tab,
        Some("w1:t1".into()),
        "ties go to the first pane's tab"
    );
    assert_eq!(t.root, right(0.5, p("w1:p1"), p("w1:p2")));
    assert_eq!(t.zoomed, None);
}

#[test]
fn tabs_with_terminals_still_waiting_for_panes_wait() {
    let (mut s, tab, [a, _, _]) = manual_tab(terminal());
    bind(&mut s, a, "w1:p1", "w1:t1");
    assert!(!targets(&s, &None).contains_key(&tab));
}

#[test]
fn a_herdr_tab_goes_to_the_azulejo_tab_with_most_panes_there() {
    let (mut s, tab, [a, b, c]) = manual_tab(terminal());
    bind(&mut s, a, "w1:p1", "w1:t1");
    bind(&mut s, b, "w1:p2", "w1:t1");
    bind(&mut s, c, "w1:p3", "w1:t2");
    let ws = s.focus().workspace.unwrap();
    let d = terminal();
    let did = d.id;
    let other = s.create_tab(ws, d, None).unwrap();
    bind(&mut s, did, "w1:p4", "w1:t1");
    // A closed tile's pane is known: a push moves it out, never closes it.
    s.detached_herdr
        .push(HerdrRef::local("w1:p9", "term_w1:p9", "w1:t1"));
    let all = targets(&s, &None);
    assert_eq!(all[&tab].tab, Some("w1:t1".into()));
    assert_eq!(all[&other].tab, None, "w1:t1 is taken: it makes its own");
    assert!(all[&tab].known.contains(&"w1:p9".into()));
    assert!(all[&tab].known.contains(&"w1:p4".into()));
    assert!(!all[&tab].known.contains(&"w1:p1".into()));
}

#[test]
fn monocle_and_zoom_zoom_the_pane() {
    let mut s = Session::new();
    s.prefs.default = LayoutKind::Tile;
    let (_, ws) = s.create_window("main", None);
    let a = terminal();
    let aid = a.id;
    let tab = s.create_tab(ws, a, None).unwrap();
    let b = terminal();
    let bid = b.id;
    s.split(aid, Direction::Right, b).unwrap();
    bind(&mut s, aid, "w1:p1", "w1:t1");
    bind(&mut s, bid, "w1:p2", "w1:t1");
    s.set_layout(tab, LayoutKind::Monocle).unwrap();
    s.focus_tile(aid).unwrap();
    let t = &targets(&s, &None)[&tab];
    assert_eq!(t.zoomed, Some("w1:p1".into()));
    // Monocle's tree is Tile's: herdr unzoomed shows the Tile arrangement.
    assert_eq!(t.root, right(0.5, p("w1:p1"), p("w1:p2")));
    s.set_layout(tab, LayoutKind::Tile).unwrap();
    s.zoom(bid, Some(true)).unwrap();
    assert_eq!(targets(&s, &None)[&tab].zoomed, Some("w1:p2".into()));
}

#[test]
fn a_new_terminal_splits_off_the_pane_beside_it() {
    let (mut s, _, [a, b, c]) = manual_tab(terminal());
    bind(&mut s, a, "w1:p1", "w1:t1");
    bind(&mut s, b, "w1:p2", "w1:t1");
    let spec = split_for(&s, c).unwrap();
    assert_eq!((spec.tile, spec.pane.as_deref()), (b, Some("w1:p2")));
    assert_eq!(spec.direction, herdr::SplitDirection::Down);
    // B still getting its pane: C names B, and the worker finds the pane it
    // made for B just before (three quick splits land in place).
    let (mut s, _, [a, b, c]) = manual_tab(terminal());
    bind(&mut s, a, "w1:p1", "w1:t1");
    let spec = split_for(&s, c).unwrap();
    assert_eq!((spec.tile, spec.pane), (b, None));
    assert_eq!(spec.direction, herdr::SplitDirection::Down);
    // A browser beside it: the nearest terminal is across the root split.
    let (mut s, _, [a, b, _]) = manual_tab(browser());
    bind(&mut s, a, "w1:p1", "w1:t1");
    let spec = split_for(&s, b).unwrap();
    assert_eq!((spec.tile, spec.pane.as_deref()), (a, Some("w1:p1")));
    assert_eq!(spec.direction, herdr::SplitDirection::Right);
    // A tab's only terminal gets a herdr tab of its own.
    let mut s = Session::new();
    let (_, ws) = s.create_window("main", None);
    let only = terminal();
    let id = only.id;
    s.create_tab(ws, only, None).unwrap();
    assert_eq!(split_for(&s, id), None);
}

#[test]
fn a_tile_stack_grows_by_splitting_its_last_pane() {
    let mut s = Session::new();
    s.prefs.default = LayoutKind::Tile;
    let (_, ws) = s.create_window("main", None);
    let a = terminal();
    let aid = a.id;
    s.create_tab(ws, a, None).unwrap();
    let mut ids = vec![aid];
    for i in 0..3 {
        let t = terminal();
        ids.push(t.id);
        s.split(aid, Direction::Right, t).unwrap();
        bind(&mut s, ids[i], &format!("w1:p{}", i + 1), "w1:t1");
    }
    let spec = split_for(&s, ids[3]).unwrap();
    assert_eq!(spec.pane.as_deref(), Some("w1:p3"), "the stack's last pane");
    assert_eq!(spec.direction, herdr::SplitDirection::Down);
}

/// The manual tab of `manual_tab(c)`, mirrored into w1:t1.
fn mirrored(c: Tile) -> (Session, TabId, [TileId; 3]) {
    let (mut s, tab, ids) = manual_tab(c);
    bind(&mut s, ids[0], "w1:p1", "w1:t1");
    bind(&mut s, ids[1], "w1:p2", "w1:t1");
    if matches!(
        s.tile(ids[2]).unwrap().content,
        TileContent::Terminal { .. }
    ) {
        bind(&mut s, ids[2], "w1:p3", "w1:t1");
    }
    (s, tab, ids)
}

#[test]
fn a_ratio_moved_in_herdr_moves_the_matching_split() {
    let (mut s, tab, [_, b, c]) = mirrored(browser());
    // herdr has A | B (the browser isn't there); the TUI drags to 0.7.
    assert!(absorb(
        &mut s,
        &None,
        &export("w1:t1", right(0.7, p("w1:p1"), p("w1:p2")))
    ));
    let Node::Split {
        ratio, b: inner, ..
    } = &s.tab(tab).unwrap().root
    else {
        panic!("still split");
    };
    assert!((ratio - 0.7).abs() < 1e-9);
    // The browser's own split is untouched.
    assert!(matches!(inner.as_ref(), Node::Split { ratio, .. } if (ratio - 0.5).abs() < 1e-9));
    assert!(inner.contains(b) && inner.contains(c));
    // The same again: nothing changes.
    assert!(!absorb(
        &mut s,
        &None,
        &export("w1:t1", right(0.7, p("w1:p1"), p("w1:p2")))
    ));
}

#[test]
fn panes_swapped_in_herdr_swap_their_tiles() {
    let (mut s, tab, [a, b, c]) = mirrored(terminal());
    let swapped = right(0.5, p("w1:p3"), down(0.5, p("w1:p2"), p("w1:p1")));
    assert!(absorb(&mut s, &None, &export("w1:t1", swapped)));
    assert_eq!(s.tab(tab).unwrap().root.tiles(), vec![c, b, a]);
    s.check_invariants().unwrap();
}

#[test]
fn another_shape_from_herdr_replaces_the_tree_and_keeps_browsers() {
    let (mut s, tab, [a, b, c]) = mirrored(browser());
    // In herdr: A over B.
    assert!(absorb(
        &mut s,
        &None,
        &export("w1:t1", down(0.4, p("w1:p1"), p("w1:p2")))
    ));
    let root = &s.tab(tab).unwrap().root;
    let tiles = root.tiles();
    assert_eq!(tiles.len(), 3);
    assert!(tiles.contains(&c), "the browser is back beside B");
    assert!(matches!(
        root,
        Node::Split {
            axis: Axis::Vertical,
            ..
        }
    ));
    let _ = (a, b);
    s.check_invariants().unwrap();
}

#[test]
fn zoom_follows_herdr_and_monocle_leaves_when_unzoomed() {
    let (mut s, tab, [_, b, _]) = mirrored(terminal());
    let tree = right(0.5, p("w1:p1"), down(0.5, p("w1:p2"), p("w1:p3")));
    let mut l = export("w1:t1", tree);
    l.zoomed = true;
    l.focused_pane_id = Some("w1:p2".into());
    assert!(absorb(&mut s, &None, &l));
    assert_eq!(s.tab(tab).unwrap().zoomed, Some(b));
    l.zoomed = false;
    assert!(absorb(&mut s, &None, &l));
    assert_eq!(s.tab(tab).unwrap().zoomed, None);

    // Monocle: unzoomed in herdr → back to the previous kind.
    s.set_layout(tab, LayoutKind::Monocle).unwrap();
    let root = targets(&s, &None)[&tab].root.clone();
    let mut l = export("w1:t1", root);
    l.zoomed = false;
    assert!(absorb(&mut s, &None, &l));
    assert_eq!(s.tab(tab).unwrap().layout.kind, LayoutKind::Manual);
    s.check_invariants().unwrap();
}

#[test]
fn nothing_is_absorbed_while_panes_and_tiles_disagree() {
    let (mut s, _, _) = mirrored(terminal());
    // A pane split off in the TUI that isn't adopted yet.
    let tree = right(
        0.6,
        p("w1:p1"),
        down(0.5, p("w1:p2"), right(0.5, p("w1:p3"), p("w1:p8"))),
    );
    assert!(!absorb(&mut s, &None, &export("w1:t1", tree)));
    // A pane closed in herdr whose tile hasn't gone yet.
    assert!(!absorb(
        &mut s,
        &None,
        &export("w1:t1", right(0.6, p("w1:p1"), p("w1:p2")))
    ));
}

#[test]
fn a_dynamic_tab_freezes_when_herdr_makes_what_its_rule_cant() {
    let mut s = Session::new();
    s.prefs.default = LayoutKind::Tile;
    let (_, ws) = s.create_window("main", None);
    let a = terminal();
    let aid = a.id;
    let tab = s.create_tab(ws, a, None).unwrap();
    let mut ids = vec![aid];
    for _ in 0..2 {
        let t = terminal();
        ids.push(t.id);
        s.split(aid, Direction::Right, t).unwrap();
    }
    for (i, t) in ids.iter().enumerate() {
        bind(&mut s, *t, &format!("w1:p{}", i + 1), "w1:t1");
    }
    // The master edge moved: absorbed.
    let tree = right(0.65, p("w1:p1"), down(0.5, p("w1:p2"), p("w1:p3")));
    assert!(absorb(&mut s, &None, &export("w1:t1", tree)));
    let t = s.tab(tab).unwrap();
    assert_eq!(t.layout.kind, LayoutKind::Tile);
    assert!((t.layout.params.tile.ratio - 0.65).abs() < 1e-6);
    // Three side by side: Tile can't, so the tab freezes.
    let row = right(1.0 / 3.0, p("w1:p1"), right(0.5, p("w1:p2"), p("w1:p3")));
    assert!(absorb(&mut s, &None, &export("w1:t1", row.clone())));
    let t = s.tab(tab).unwrap();
    assert_eq!(t.layout.kind, LayoutKind::Manual);
    assert_eq!(t.layout.previous, Some(LayoutKind::Tile));
    // And the next target is exactly what herdr has: nothing to push.
    let target = &targets(&s, &None)[&tab];
    assert!(herdr::same_tree(&target.root, &row, herdr::RATIO_TOLERANCE));
    s.check_invariants().unwrap();
}

// ---- pulls: our own writes vs someone's change ----------------------------

fn set(panes: &[&str]) -> std::collections::BTreeSet<String> {
    panes.iter().map(|p| (*p).to_owned()).collect()
}

#[test]
fn a_resize_right_after_our_splits_is_a_change() {
    // The record after the push: one pane. Our two splits land (recorded),
    // then the TUI resizes before anything else is read.
    let mut before = export("w1:t1", p("w1:p1"));
    assert!(record_split(
        &mut before,
        &"w1:p1".into(),
        herdr::SplitDirection::Right,
        0.5,
        &"w1:p2".into()
    ));
    assert!(record_split(
        &mut before,
        &"w1:p2".into(),
        herdr::SplitDirection::Down,
        0.5,
        &"w1:p3".into()
    ));
    let now = export(
        "w1:t1",
        right(0.7, p("w1:p1"), down(0.5, p("w1:p2"), p("w1:p3"))),
    );
    let all = set(&["w1:p1", "w1:p2", "w1:p3"]);
    assert_eq!(judge(&before, &now, Some(&all)), Pulled::Change);
    // A pull that read herdr before the worker's word came already has the
    // pane: the late word changes nothing.
    let mut read = now.clone();
    assert!(!record_split(
        &mut read,
        &"w1:p2".into(),
        herdr::SplitDirection::Down,
        0.5,
        &"w1:p3".into()
    ));
    assert_eq!(read, now);
    // Our splits alone are an echo.
    let ours = export(
        "w1:t1",
        right(0.5, p("w1:p1"), down(0.5, p("w1:p2"), p("w1:p3"))),
    );
    assert_eq!(judge(&before, &ours, Some(&all)), Pulled::Echo);
}

#[test]
fn a_change_among_known_panes_counts_whatever_came_or_went() {
    let before = export("w1:t1", right(0.5, p("w1:p1"), p("w1:p2")));
    // A pane split off in the TUI (not adopted yet), and the root dragged.
    let now = export(
        "w1:t1",
        right(0.7, p("w1:p1"), down(0.5, p("w1:p2"), p("w1:p9"))),
    );
    let tiles = set(&["w1:p1", "w1:p2"]);
    assert_eq!(
        judge(&before, &now, Some(&tiles)),
        Pulled::Wait,
        "until it's a tile"
    );
    let adopted = set(&["w1:p1", "w1:p2", "w1:p9"]);
    assert_eq!(judge(&before, &now, Some(&adopted)), Pulled::Change);
    // Only the split: the record follows, nothing to absorb.
    let split = export(
        "w1:t1",
        right(0.5, p("w1:p1"), down(0.5, p("w1:p2"), p("w1:p9"))),
    );
    assert_eq!(judge(&before, &split, Some(&tiles)), Pulled::Membership);
    // A pane closed in herdr collapses its split; the rest is unchanged.
    let three = export(
        "w1:t1",
        right(0.5, p("w1:p1"), down(0.3, p("w1:p2"), p("w1:p3"))),
    );
    let closed = export("w1:t1", right(0.5, p("w1:p1"), p("w1:p2")));
    assert_eq!(judge(&three, &closed, None), Pulled::Membership);
}

#[test]
fn zoom_counts_only_with_the_same_panes() {
    let mut before = export("w1:t1", right(0.5, p("w1:p1"), p("w1:p2")));
    before.zoomed = true;
    let mut now = before.clone();
    now.zoomed = false;
    assert_eq!(
        judge(&before, &now, None),
        Pulled::Change,
        "unzoomed in the TUI"
    );
    // A split clears herdr's zoom: not an unzoom.
    let split = export(
        "w1:t1",
        right(0.5, p("w1:p1"), down(0.5, p("w1:p2"), p("w1:p3"))),
    );
    assert_eq!(judge(&before, &split, None), Pulled::Membership);
}

/// A Monocle tab with one terminal and a browser, or two terminals and a
/// browser, focused on the browser.
fn monocle_with_browser(terminals: usize) -> (Session, TabId, Vec<TileId>, TileId) {
    let mut s = Session::new();
    s.prefs.default = LayoutKind::Tile;
    let (_, ws) = s.create_window("main", None);
    let a = terminal();
    let aid = a.id;
    let tab = s.create_tab(ws, a, None).unwrap();
    let mut terms = vec![aid];
    for _ in 1..terminals {
        let t = terminal();
        terms.push(t.id);
        s.split(aid, Direction::Right, t).unwrap();
    }
    for (i, t) in terms.iter().enumerate() {
        bind(&mut s, *t, &format!("w1:p{}", i + 1), "w1:t1");
    }
    let b = browser();
    let bid = b.id;
    s.split(aid, Direction::Right, b).unwrap();
    s.set_layout(tab, LayoutKind::Monocle).unwrap();
    s.focus_tile(terms[terms.len() - 1]).unwrap();
    s.focus_tile(bid).unwrap();
    (s, tab, terms, bid)
}

#[test]
fn monocle_on_a_browser_keeps_herdr_on_the_terminal_focused_last() {
    let (mut s, tab, terms, browser) = monocle_with_browser(2);
    let t = &targets(&s, &None)[&tab];
    assert_eq!(t.zoomed, Some("w1:p2".into()), "the terminal focused last");
    // herdr shows that pane, zoomed: nothing to absorb, the browser keeps
    // focus and Monocle stays.
    let mut l = export("w1:t1", t.root.clone());
    l.zoomed = true;
    l.focused_pane_id = Some("w1:p2".into());
    assert!(!absorb(&mut s, &None, &l));
    let tab_now = s.tab(tab).unwrap();
    assert_eq!(tab_now.layout.kind, LayoutKind::Monocle);
    assert_eq!(tab_now.focused, browser);
    // Zoomed onto the other terminal in the TUI: that one comes to the top.
    l.focused_pane_id = Some("w1:p1".into());
    assert!(absorb(&mut s, &None, &l));
    assert_eq!(s.tab(tab).unwrap().focused, terms[0]);
}

#[test]
fn monocle_with_one_terminal_and_a_browser_has_nothing_to_zoom() {
    let (mut s, tab, _, _) = monocle_with_browser(1);
    let t = &targets(&s, &None)[&tab];
    assert_eq!(t.zoomed, None, "herdr doesn't zoom a tab's only pane");
    // herdr reports it unzoomed: that isn't leaving Monocle.
    let l = export("w1:t1", t.root.clone());
    assert!(!absorb(&mut s, &None, &l));
    assert_eq!(s.tab(tab).unwrap().layout.kind, LayoutKind::Monocle);
    s.check_invariants().unwrap();
}
