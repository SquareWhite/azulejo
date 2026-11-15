//! Where ⌘S Run sends a command (`azulejo_hints::run`): the current pane
//! when it's an idle shell, else the nearest idle shell on the same
//! machine, else a new pane.

use azulejo_hints::callout::Rect;
use azulejo_hints::run::{Mode, Pane, Plan, Request, Target, plan};

fn pane(tile: &str, rect: [f64; 4], idle: bool) -> Pane {
    Pane {
        tile: tile.into(),
        rect: Rect::from(rect),
        machine: "local".into(),
        idle,
        cwd: Some("/w".into()),
    }
}

fn request(current: &str, panes: Vec<Pane>) -> Request {
    Request {
        mode: Mode::Reuse,
        value: "make test".into(),
        cwd: Some("/w".into()),
        machine: "local".into(),
        current: current.into(),
        panes,
        recent: vec![],
    }
}

fn target(r: &Request) -> Target {
    plan(r).target
}

fn reuse(tile: &str) -> Target {
    Target::Reuse(tile.into())
}

#[test]
fn an_idle_current_pane_runs_it_there() {
    let r = request(
        "a",
        vec![
            pane("a", [0., 0., 0.5, 1.], true),
            pane("b", [0.5, 0., 0.5, 1.], true),
        ],
    );
    assert_eq!(
        plan(&r),
        Plan {
            target: Target::Current,
            line: "make test".into()
        }
    );
}

#[test]
fn a_busy_current_pane_goes_to_the_nearest_idle_one() {
    // a | b over c, d far right: b and c both touch a.
    let panes = vec![
        pane("a", [0., 0., 0.25, 1.], false),
        pane("d", [0.75, 0., 0.25, 1.], true),
        pane("b", [0.25, 0., 0.5, 0.4], true),
        pane("c", [0.25, 0.4, 0.5, 0.6], true),
    ];
    // Both touch; c's centre is nearer a's.
    assert_eq!(target(&request("a", panes.clone())), reuse("c"));
    // Ties go to the most recent Run target, before the centre.
    let mut r = request("a", panes.clone());
    r.recent = vec!["d".into(), "b".into(), "c".into()];
    assert_eq!(
        target(&r),
        reuse("b"),
        "d is further away whatever its rank"
    );
    // Busy panes never count.
    let mut panes = panes;
    panes[2].idle = false;
    panes[3].idle = false;
    assert_eq!(target(&request("a", panes)), reuse("d"));
}

#[test]
fn equal_ties_fall_back_to_reading_order() {
    let panes = vec![
        pane("a", [0.25, 0.25, 0.5, 0.5], false),
        pane("l", [0., 0., 0.25, 1.], true),
        pane("r", [0.75, 0., 0.25, 1.], true),
    ];
    assert_eq!(target(&request("a", panes)), reuse("l"));
}

#[test]
fn no_idle_pane_makes_a_new_one() {
    let r = request(
        "a",
        vec![
            pane("a", [0., 0., 0.5, 1.], false),
            pane("b", [0.5, 0., 0.5, 1.], false),
        ],
    );
    assert_eq!(
        plan(&r),
        Plan {
            target: Target::New,
            line: "make test".into()
        }
    );
    // Nothing but the current pane.
    assert_eq!(
        target(&request("a", vec![pane("a", [0., 0., 1., 1.], false)])),
        Target::New
    );
    // The current tile isn't a terminal (or isn't known).
    assert_eq!(
        target(&request("x", vec![pane("b", [0., 0., 1., 1.], true)])),
        Target::New
    );
}

#[test]
fn only_panes_on_the_source_machine_count() {
    let mut remote = pane("a", [0., 0., 0.5, 1.], true);
    remote.machine = "build-box".into();
    let local = pane("b", [0.5, 0., 0.5, 1.], true);
    // A command from a remote pane: the local idle shell beside it won't do,
    // and nor will a local current pane.
    let mut r = request("b", vec![remote.clone(), local.clone()]);
    r.machine = "build-box".into();
    assert_eq!(target(&r), reuse("a"));
    r.panes[0].idle = false;
    assert_eq!(target(&r), Target::New);
    // A local command never runs in the remote shell.
    let r = request("a", vec![remote, pane("b", [0.5, 0., 0.5, 1.], false)]);
    assert_eq!(target(&r), Target::New);
}

#[test]
fn new_mode_always_makes_a_pane() {
    let mut r = request("a", vec![pane("a", [0., 0., 1., 1.], true)]);
    r.mode = Mode::New;
    assert_eq!(target(&r), Target::New);
}

#[test]
fn a_shell_elsewhere_changes_directory_first() {
    let mut r = request(
        "a",
        vec![
            pane("a", [0., 0., 0.5, 1.], false),
            pane("b", [0.5, 0., 0.5, 1.], true),
        ],
    );
    r.panes[1].cwd = Some("/elsewhere".into());
    r.cwd = Some("/w/it's here".into());
    assert_eq!(
        plan(&r),
        Plan {
            target: reuse("b"),
            line: r"cd '/w/it'\''s here' && make test".into()
        }
    );
    r.cwd = Some("/w/plain-dir_1.0".into());
    assert_eq!(plan(&r).line, "cd /w/plain-dir_1.0 && make test");
    // Unknown there: cd anyway.
    r.panes[1].cwd = None;
    assert_eq!(plan(&r).line, "cd /w/plain-dir_1.0 && make test");
    // A directory fish and sh would read differently: a new pane instead.
    r.cwd = Some(r"/w/back\slash".into());
    assert_eq!(target(&r), Target::New);
    // A new pane starts in the directory: no cd.
    assert_eq!(plan(&r).line, "make test");
}

#[test]
fn multi_line_commands_keep_the_sh_line() {
    let mut r = request("a", vec![pane("a", [0., 0., 1., 1.], true)]);
    r.value = "cat <<'EOF'\nhi\nEOF".into();
    let line = plan(&r).line;
    assert!(
        line.starts_with("/bin/sh -c 'eval ") && !line.contains('\n'),
        "{line}"
    );
}

#[test]
fn the_request_and_plan_cross_as_json() {
    let r: Request = serde_json::from_str(
        r#"{"mode":"reuse","value":"ls","cwd":"/w","machine":"local","current":"a",
            "panes":[{"tile":"a","rect":[0,0,1,1],"machine":"local","idle":false,"cwd":"/w"},
                     {"tile":"b","rect":[1,0,1,1],"machine":"local","idle":true,"cwd":"/w"}],
            "recent":["b"]}"#,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(plan(&r)).unwrap(),
        serde_json::json!({"target": "reuse", "tile": "b", "line": "ls"})
    );
    let mut r = r;
    r.mode = Mode::New;
    assert_eq!(
        serde_json::to_value(plan(&r)).unwrap(),
        serde_json::json!({"target": "new", "line": "ls"})
    );
}
