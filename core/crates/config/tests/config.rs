use std::collections::BTreeSet;
use std::path::Path;

use azulejo_config::ghostty::GhosttyConfig;
use azulejo_config::{
    Action, Chord, Config, Context, KeyOutcome, KeyRouter, Keymap, PLANNED_METHODS, Platform,
    PrefixState, Severity, Target, reload,
};
use azulejo_protocol::{Direction, METHODS, TileKind};
use proptest::prelude::*;

const MAC: Platform = Platform::Macos;
const LINUX: Platform = Platform::Linux;

fn chord(s: &str) -> Chord {
    Chord::parse(s).unwrap()
}

fn user(text: &str, platform: Platform) -> Config {
    Config::from_toml(text, None, platform)
}

fn resolve(config: &Config, c: &str, tile: Option<TileKind>) -> Option<Action> {
    config.keymap.resolve(
        &chord(c),
        &Context {
            tile,
            prefix: PrefixState::Idle,
        },
    )
}

fn warnings(config: &Config) -> Vec<String> {
    config
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .map(|d| d.message.clone())
        .collect()
}

fn errors(config: &Config) -> Vec<String> {
    config
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d| d.message.clone())
        .collect()
}

/// Every binding in every context, for comparing keymaps.
fn snapshot(keymap: &Keymap) -> BTreeSet<String> {
    let contexts = [
        None,
        Some(TileKind::Terminal),
        Some(TileKind::Browser),
        Some(TileKind::Preview),
        Some(TileKind::Files),
    ];
    let mut out = BTreeSet::new();
    for tile in contexts {
        for (c, a) in keymap.bindings(tile) {
            out.insert(format!("{tile:?} {c} {a:?}"));
        }
    }
    for (c, a) in keymap.prefix_bindings() {
        out.insert(format!("prefix {c} {a:?}"));
    }
    out.insert(format!("prefix key {:?}", keymap.prefix_key()));
    out
}

// ----- defaults -----

#[test]
fn default_keymap_loads_cleanly_on_both_platforms() {
    for platform in Platform::ALL {
        let config = Config::defaults(platform);
        assert!(
            config.diagnostics.is_empty(),
            "{platform:?}: {:#?}",
            config.diagnostics
        );
        assert!(config.keymap.bindings(None).count() > 30);
        assert!(!config.keymap.prefix_enabled());
    }
}

#[test]
fn default_keymap_with_prefix_mode_on_is_clean_too() {
    for platform in Platform::ALL {
        let config = user("[prefix]\nenabled = true\n", platform);
        assert!(
            config.diagnostics.is_empty(),
            "{platform:?}: {:#?}",
            config.diagnostics
        );
        assert!(config.keymap.prefix_key().is_some());
    }
}

#[test]
fn defaults_follow_the_herdr_bindings_on_macos() {
    let config = Config::defaults(MAC);
    assert_eq!(
        resolve(&config, "alt+h", None),
        Some(Action::FocusTile(Direction::Left))
    );
    assert_eq!(
        resolve(&config, "alt+shift+l", None),
        Some(Action::SwapTile(Direction::Right))
    );
    assert_eq!(
        resolve(&config, "cmd+3", None),
        Some(Action::SwitchWorkspace(3))
    );
    assert_eq!(
        resolve(&config, "ctrl+cmd+t", None),
        Some(Action::SplitAuto)
    );
    assert_eq!(resolve(&config, "cmd+t", None), Some(Action::NewWorkspace));
}

#[test]
fn every_default_action_maps_to_a_protocol_method_or_shell_action() {
    for platform in Platform::ALL {
        let config = user("[prefix]\nenabled = true\n", platform);
        let keymap = &config.keymap;
        let contexts = [
            None,
            Some(TileKind::Terminal),
            Some(TileKind::Browser),
            Some(TileKind::Preview),
            Some(TileKind::Files),
        ];
        let actions = contexts
            .into_iter()
            .flat_map(|t| keymap.bindings(t).collect::<Vec<_>>())
            .chain(keymap.prefix_bindings())
            .map(|(_, a)| a);
        for action in actions {
            match action.target() {
                Target::Shell => {}
                Target::Method(m) => assert!(
                    METHODS.contains(&m) || PLANNED_METHODS.contains(&m),
                    "{action:?} → unknown method {m}"
                ),
            }
            if let Some(command) = action.command() {
                assert_eq!(action.target(), Target::Method(command.method()));
            }
        }
    }
}

/// Actions that act on whatever is focused must build their command in the
/// keymap; only ones that need session state, a prompt or the shell may not.
#[test]
fn stateless_actions_build_commands() {
    use Action::*;
    let needs_shell_or_state = |a: Action| {
        matches!(
            a,
            ResizeMode
                | CommandPalette
                | Goto
                | Find
                | OpenConfig
                | ReloadConfig
                | ToggleSidebar
                | EnterPrefix
                | EditScrollback
                | RenameTile
                | RenameWorkspace
                | RenameTab
                | CycleTileNext
                | CycleTilePrevious
                | SwitchWorkspace(_)
                | LastWorkspace
                | NextWorkspace
                | PreviousWorkspace
                | SwitchTab(_)
                | LastTab
                | NextTab
                | PreviousTab
                | CloseWindow
        )
    };
    for platform in Platform::ALL {
        let config = user("[prefix]\nenabled = true\n", platform);
        let keymap = &config.keymap;
        let actions = [
            None,
            Some(TileKind::Terminal),
            Some(TileKind::Browser),
            Some(TileKind::Preview),
            Some(TileKind::Files),
        ]
        .into_iter()
        .flat_map(|t| keymap.bindings(t).collect::<Vec<_>>())
        .chain(keymap.prefix_bindings())
        .map(|(_, a)| a);
        for action in actions {
            if needs_shell_or_state(action) {
                continue;
            }
            let command = action
                .command()
                .unwrap_or_else(|| panic!("{action:?} should build a command"));
            assert!(
                METHODS.contains(&command.method()),
                "{action:?} → {} isn't a protocol method",
                command.method()
            );
        }
    }
}

#[test]
fn herdr_session_defaults_to_the_users_own() {
    let config = Config::defaults(MAC);
    assert_eq!(config.herdr, azulejo_config::HerdrConfig::default());
    assert_eq!(config.herdr.session, None, "herdr's default session");
    assert!(config.herdr.enabled);

    let config = user("[herdr]\nsession = \"default\"\n", MAC);
    assert_eq!(config.herdr.session, None);
    assert!(config.diagnostics.is_empty());

    let config = user("[herdr]\nsession = \"azulejo\"\n", MAC);
    assert_eq!(config.herdr.session.as_deref(), Some("azulejo"));

    let config = user("[herdr]\nsession = \"work\"\nenabled = false\n", LINUX);
    assert_eq!(config.herdr.session.as_deref(), Some("work"));
    assert!(!config.herdr.enabled);

    let config = user("[herdr]\nsession = \"a b/c\"\n", MAC);
    assert_eq!(config.herdr.session, None, "a bad name keeps the default");
    assert!(config.has_errors());
}

#[test]
fn a_terminal_closes_when_its_shell_exits_unless_kept() {
    use azulejo_protocol::OnExit;
    assert_eq!(Config::defaults(MAC).terminal.on_exit, OnExit::Close);

    let config = user("[terminal]\non_exit = \"keep\"\n", MAC);
    assert_eq!(config.terminal.on_exit, OnExit::Keep);
    assert!(config.diagnostics.is_empty());

    let config = user("[terminal]\non_exit = \"linger\"\n", LINUX);
    assert_eq!(
        config.terminal.on_exit,
        OnExit::Close,
        "a bad value keeps the default"
    );
    assert!(config.has_errors());

    let config = user("[terminal]\nonexit = \"keep\"\n", MAC);
    assert!(!config.has_errors());
    assert!(
        config
            .diagnostics
            .iter()
            .any(|d| d.message.contains("terminal.onexit")),
        "{:?}",
        config.diagnostics
    );
}

// ----- context -----

#[test]
fn cmd_a_toggles_the_sidebar_only_in_terminal_tiles() {
    let config = Config::defaults(MAC);
    assert_eq!(
        resolve(&config, "cmd+a", Some(TileKind::Terminal)),
        Some(Action::ToggleSidebar)
    );
    for tile in [
        None,
        Some(TileKind::Browser),
        Some(TileKind::Preview),
        Some(TileKind::Files),
    ] {
        assert_eq!(resolve(&config, "cmd+a", tile), None, "{tile:?}");
    }
    // Same rule on Linux, whatever the chord is there.
    let linux = Config::defaults(LINUX);
    assert!(
        !linux
            .keymap
            .chords_for(Action::ToggleSidebar, Some(TileKind::Terminal))
            .is_empty()
    );
    assert!(
        linux
            .keymap
            .chords_for(Action::ToggleSidebar, Some(TileKind::Browser))
            .is_empty()
    );
}

#[test]
fn context_sections_override_and_can_unbind() {
    let config = user(
        r#"
[keys]
find = "cmd+f"
[keys.browser]
find = "cmd+shift+f"
[keys.terminal]
find = ""
"#,
        MAC,
    );
    assert!(config.diagnostics.is_empty(), "{:#?}", config.diagnostics);
    assert_eq!(resolve(&config, "cmd+f", None), Some(Action::Find));
    assert_eq!(resolve(&config, "cmd+f", Some(TileKind::Browser)), None);
    assert_eq!(
        resolve(&config, "cmd+shift+f", Some(TileKind::Browser)),
        Some(Action::Find)
    );
    assert_eq!(resolve(&config, "cmd+f", Some(TileKind::Terminal)), None);
    // Other bindings still apply in a context.
    assert_eq!(
        resolve(&config, "alt+h", Some(TileKind::Browser)),
        Some(Action::FocusTile(Direction::Left))
    );
}

// ----- overrides -----

#[test]
fn user_bindings_replace_defaults() {
    let config = user("[keys]\nsplit_right = \"cmd+d\"\n", MAC);
    assert!(config.diagnostics.is_empty(), "{:#?}", config.diagnostics);
    assert_eq!(
        resolve(&config, "cmd+d", None),
        Some(Action::Split(Direction::Right))
    );
    assert_eq!(resolve(&config, "ctrl+cmd+v", None), None);
}

#[test]
fn platform_sections_beat_the_generic_one() {
    let text = r#"
[keys]
split_right = "cmd+d"
[keys.linux]
split_right = ["ctrl+shift+d", "super+d"]
"#;
    let mac = user(text, MAC);
    let linux = user(text, LINUX);
    assert_eq!(
        resolve(&mac, "cmd+d", None),
        Some(Action::Split(Direction::Right))
    );
    assert_eq!(
        resolve(&linux, "ctrl+shift+d", None),
        Some(Action::Split(Direction::Right))
    );
    // `super` and `cmd` are the same modifier.
    assert_eq!(
        resolve(&linux, "cmd+d", None),
        Some(Action::Split(Direction::Right))
    );
    assert_eq!(
        linux
            .keymap
            .chords_for(Action::Split(Direction::Right), None)
            .len(),
        2
    );
}

#[test]
fn empty_string_and_none_unbind() {
    for value in ["\"\"", "\"none\"", "\"NONE\""] {
        let config = user(&format!("[keys]\nnew_workspace = {value}\n"), MAC);
        assert!(config.diagnostics.is_empty(), "{:#?}", config.diagnostics);
        assert_eq!(resolve(&config, "cmd+t", None), None);
        assert!(
            config
                .keymap
                .chords_for(Action::NewWorkspace, None)
                .is_empty()
        );
    }
}

#[test]
fn indexed_bindings() {
    let config = user(
        "[keys]\nswitch_tab = [\"ctrl+alt+1..3\", \"ctrl+alt+9\"]\n",
        MAC,
    );
    assert!(config.diagnostics.is_empty(), "{:#?}", config.diagnostics);
    assert_eq!(
        resolve(&config, "ctrl+alt+2", None),
        Some(Action::SwitchTab(2))
    );
    assert_eq!(
        resolve(&config, "ctrl+alt+9", None),
        Some(Action::SwitchTab(9))
    );
    assert_eq!(resolve(&config, "ctrl+2", None), None);

    let bad = user(
        "[keys]\nzoom_tile = \"cmd+1..3\"\nswitch_tab = \"cmd+x\"\n",
        MAC,
    );
    assert_eq!(errors(&bad).len(), 2, "{:#?}", bad.diagnostics);
    // Both rejected, so the defaults stay.
    assert_eq!(resolve(&bad, "alt+shift+m", None), Some(Action::ZoomTile));
    assert_eq!(resolve(&bad, "ctrl+2", None), Some(Action::SwitchTab(2)));
}

// ----- errors -----

#[test]
fn malformed_toml_gives_defaults_and_a_located_error() {
    let text = "[keys]\nsplit_right = \"cmd+d\"\nbroken = = 3\n";
    let config = Config::from_toml(text, Some("/tmp/azulejo/config.toml".into()), MAC);
    assert_eq!(
        snapshot(&config.keymap),
        snapshot(&Config::defaults(MAC).keymap)
    );
    assert_eq!(config.diagnostics.len(), 1);
    let d = &config.diagnostics[0];
    assert!(d.is_error());
    assert_eq!(d.location.unwrap().line, 3, "{d}");
    assert!(
        d.to_string().starts_with("/tmp/azulejo/config.toml:3:"),
        "{d}"
    );
}

#[test]
fn bad_values_are_dropped_with_locations() {
    let text = "\
[keys]
split_right = \"hyper+d\"
zoom_tile = 5
promote_tile = [\"alt+enter\", 3]
not_an_action = \"cmd+y\"
[keys.windows]
goto = \"cmd+p\"
[unknown]
x = 1
";
    let config = user(text, MAC);
    let lines = |severity: Severity| {
        config
            .diagnostics
            .iter()
            .filter(|d| d.severity == severity)
            .map(|d| d.location.unwrap().line)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        lines(Severity::Error),
        vec![2, 3, 4],
        "{:#?}",
        config.diagnostics
    );
    assert_eq!(
        lines(Severity::Warning),
        vec![5, 6, 8],
        "{:#?}",
        config.diagnostics
    );
    // Rejected values leave the defaults in place.
    assert_eq!(
        resolve(&config, "ctrl+cmd+v", None),
        Some(Action::Split(Direction::Right))
    );
    assert_eq!(
        resolve(&config, "alt+shift+m", None),
        Some(Action::ZoomTile)
    );
    assert_eq!(
        resolve(&config, "alt+enter", None),
        Some(Action::PromoteTile)
    );
}

// ----- conflicts -----

#[test]
fn duplicate_bindings_warn_and_the_user_wins() {
    let config = user("[keys]\nsplit_right = \"cmd+t\"\n", MAC);
    let w = warnings(&config);
    assert_eq!(w.len(), 1, "{w:#?}");
    assert!(w[0].contains("split_right") && w[0].contains("new_workspace"));
    assert_eq!(config.diagnostics[0].location.unwrap().line, 2);
    assert_eq!(
        resolve(&config, "cmd+t", None),
        Some(Action::Split(Direction::Right))
    );
}

#[test]
fn shadowing_ctrl_b_and_control_keys_in_terminals_warns() {
    let config = user("[keys]\ngoto = \"ctrl+b\"\nfind = \"ctrl+k\"\n", MAC);
    let w = warnings(&config);
    assert_eq!(w.len(), 2, "{w:#?}");
    assert!(w.iter().any(|m| m.contains("ctrl+b") && m.contains("goto")));
    assert!(w.iter().any(|m| m.contains("ctrl+k") && m.contains("find")));

    // Fine outside terminals.
    let browser_only = user("[keys.browser]\nfind = \"ctrl+k\"\n", MAC);
    assert!(
        browser_only.diagnostics.is_empty(),
        "{:#?}",
        browser_only.diagnostics
    );
}

#[test]
fn reserved_os_chords_warn() {
    let mac = user("[keys]\nclose_tile = \"cmd+q\"\n", MAC);
    let w = warnings(&mac);
    assert_eq!(w.len(), 1, "{w:#?}");
    assert!(w[0].contains("Quit"));

    let linux = user("[keys]\nclose_tile = \"super+l\"\n", LINUX);
    assert_eq!(warnings(&linux).len(), 1, "{:#?}", linux.diagnostics);
    // cmd+q is only reserved on macOS.
    let linux_q = user("[keys]\nclose_tile = \"cmd+q\"\n", LINUX);
    assert!(linux_q.diagnostics.is_empty(), "{:#?}", linux_q.diagnostics);
}

#[test]
fn prefix_key_cannot_be_ctrl_b() {
    let config = user("[prefix]\nenabled = true\nkey = \"ctrl+b\"\n", MAC);
    assert_eq!(errors(&config).len(), 1, "{:#?}", config.diagnostics);
    assert_eq!(config.keymap.prefix_key(), Some(chord("cmd+b")));
}

#[test]
fn user_binding_on_the_prefix_key_warns_when_prefix_is_on() {
    let off = user("[keys]\ngoto = \"cmd+b\"\n", MAC);
    assert!(off.diagnostics.is_empty(), "{:#?}", off.diagnostics);
    assert_eq!(resolve(&off, "cmd+b", None), Some(Action::Goto));

    let on = user("[keys]\ngoto = \"cmd+b\"\n[prefix]\nenabled = true\n", MAC);
    assert_eq!(warnings(&on).len(), 1, "{:#?}", on.diagnostics);
    assert_eq!(resolve(&on, "cmd+b", None), Some(Action::EnterPrefix));
}

// ----- prefix mode -----

#[test]
fn router_runs_prefix_sequences() {
    let config = user("[prefix]\nenabled = true\n", MAC);
    let keymap = &config.keymap;
    let mut router = KeyRouter::default();
    let term = Some(TileKind::Terminal);

    assert_eq!(
        router.handle(keymap, chord("cmd+b"), term),
        KeyOutcome::PrefixArmed
    );
    assert_eq!(router.state(), PrefixState::Armed);
    assert_eq!(
        router.handle(keymap, chord("h"), term),
        KeyOutcome::Action(Action::FocusTile(Direction::Left))
    );
    assert_eq!(router.state(), PrefixState::Idle);

    // Plain keys pass through when the prefix isn't armed.
    assert_eq!(
        router.handle(keymap, chord("h"), term),
        KeyOutcome::Unhandled
    );

    router.handle(keymap, chord("cmd+b"), term);
    assert_eq!(
        router.handle(keymap, chord("4"), term),
        KeyOutcome::Action(Action::SwitchTab(4))
    );
    router.handle(keymap, chord("cmd+b"), term);
    assert_eq!(
        router.handle(keymap, chord("?"), term),
        KeyOutcome::Action(Action::CommandPalette)
    );
    router.handle(keymap, chord("cmd+b"), term);
    assert_eq!(
        router.handle(keymap, chord("y"), term),
        KeyOutcome::PrefixCancelled
    );
    router.handle(keymap, chord("cmd+b"), term);
    assert_eq!(
        router.handle(keymap, chord("escape"), term),
        KeyOutcome::PrefixCancelled
    );

    // Direct chords still work in prefix mode.
    assert_eq!(
        router.handle(keymap, chord("alt+l"), term),
        KeyOutcome::Action(Action::FocusTile(Direction::Right))
    );
}

#[test]
fn prefix_mode_is_off_by_default() {
    let config = Config::defaults(MAC);
    let mut router = KeyRouter::default();
    assert_eq!(
        router.handle(&config.keymap, chord("cmd+b"), Some(TileKind::Terminal)),
        KeyOutcome::Unhandled
    );
    assert_eq!(
        config.keymap.prefix_chords_for(Action::ZoomTile),
        vec![chord("z")]
    );
}

#[test]
fn prefix_keys_can_be_overridden_per_platform() {
    let text = r#"
[prefix]
enabled = true
key.linux = "ctrl+alt+space"
[prefix.keys]
zoom_tile = "prefix+m"
[prefix.keys.linux]
zoom_tile = "f"
"#;
    let mac = user(text, MAC);
    let linux = user(text, LINUX);
    assert!(mac.diagnostics.is_empty(), "{:#?}", mac.diagnostics);
    assert!(linux.diagnostics.is_empty(), "{:#?}", linux.diagnostics);
    assert_eq!(
        mac.keymap.prefix_chords_for(Action::ZoomTile),
        vec![chord("m")]
    );
    assert_eq!(
        linux.keymap.prefix_chords_for(Action::ZoomTile),
        vec![chord("f")]
    );
    assert_eq!(linux.keymap.prefix_key(), Some(chord("ctrl+alt+space")));
    assert_eq!(mac.keymap.prefix_key(), Some(chord("cmd+b")));
}

// ----- animation and files -----

#[test]
fn animation_settings() {
    let config = user("[animation]\nenabled = false\nduration_ms = 150\n", MAC);
    assert!(config.diagnostics.is_empty());
    assert!(!config.animation.enabled);
    assert_eq!(config.animation.duration_ms, 150);

    let bad = user("[animation]\nduration_ms = 5000\nspeed = 2\n", MAC);
    assert_eq!(errors(&bad).len(), 1);
    assert_eq!(warnings(&bad).len(), 1);
    assert_eq!(bad.animation.duration_ms, 200);
}

#[test]
fn reload_reads_the_file_and_tolerates_a_missing_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let missing = reload(&path, MAC);
    assert!(missing.diagnostics.is_empty());
    assert_eq!(missing.path.as_deref(), Some(path.as_path()));

    std::fs::write(&path, "[keys]\nzoom_tile = \"cmd+z\"\n").unwrap();
    let config = reload(&path, MAC);
    assert!(config.diagnostics.is_empty());
    assert_eq!(resolve(&config, "cmd+z", None), Some(Action::ZoomTile));

    std::fs::write(&path, "[keys\n").unwrap();
    let broken = reload(&path, MAC);
    assert!(broken.has_errors());
    assert_eq!(
        resolve(&broken, "alt+shift+m", None),
        Some(Action::ZoomTile)
    );
}

#[test]
fn ghostty_includes_resolve_relative_and_skip_optional_missing() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("config");
    std::fs::create_dir(dir.path().join("themes")).unwrap();
    std::fs::write(
        &main,
        "font-size = 12\nconfig-file = themes/colours\nconfig-file = ?missing\nbackground = #000000\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("themes/colours"),
        "background = #111111\nfont-size = 14\nconfig-file = ../config\n",
    )
    .unwrap();
    let text = std::fs::read_to_string(&main).unwrap();
    let config = GhosttyConfig::parse(&text, &main);
    // Includes apply after the including file, so their values win.
    assert_eq!(config.font_size, Some(14.0));
    assert_eq!(
        config.background.map(|c| (c.r, c.g, c.b)),
        Some((0x11, 0x11, 0x11))
    );
    // The cycle back to `config` stops at the depth limit with a warning.
    assert!(
        config
            .diagnostics
            .iter()
            .any(|d| d.message.contains("nest"))
    );
    assert!(!config.files.iter().any(|f| f.ends_with("missing")));
    assert!(Path::new(&config.files[0]).ends_with("themes/colours"));
}

// ----- properties -----

fn action_name() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("split_right".to_string()),
        Just("switch_tab".to_string()),
        Just("toggle_sidebar".to_string()),
        Just("goto".to_string()),
        Just("zoom_tile".to_string()),
        Just("no_such_action".to_string()),
    ]
}

fn chord_text() -> impl Strategy<Value = String> {
    let mods = proptest::collection::vec(
        prop_oneof![
            Just("cmd"),
            Just("ctrl"),
            Just("alt"),
            Just("shift"),
            Just("super"),
            Just("bogus")
        ],
        0..4,
    );
    let key = prop_oneof![
        Just("a".to_string()),
        Just("b".to_string()),
        Just("1..8".to_string()),
        Just("3".to_string()),
        Just("?".to_string()),
        Just("".to_string()),
        Just("none".to_string()),
        Just("f12".to_string()),
        "[a-z]{1,3}".prop_map(String::from),
    ];
    (mods, key).prop_map(|(mods, key)| {
        let mut parts: Vec<String> = mods.into_iter().map(String::from).collect();
        parts.push(key);
        parts.join("+")
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// Any mix of valid and invalid bindings loads without panicking, every
    /// resolved action has a target, and every error has a location.
    #[test]
    fn arbitrary_configs_never_panic(
        entries in proptest::collection::vec(
            (prop_oneof![Just(""), Just("macos."), Just("linux."), Just("terminal."), Just("browser.")],
             action_name(), chord_text()),
            0..8,
        ),
        prefix_on in any::<bool>(),
    ) {
        let mut text = format!("[prefix]\nenabled = {prefix_on}\n");
        for (section, name, chord) in &entries {
            // Dotted keys keep everything in one table, so duplicate keys are
            // the only way to make invalid TOML here.
            text.push_str(&format!("keys.{section}{name} = \"{chord}\"\n"));
        }
        for platform in Platform::ALL {
            let config = Config::from_toml(&text, None, platform);
            for d in &config.diagnostics {
                prop_assert!(d.location.is_some(), "{d}");
            }
            for (_, action) in config.keymap.bindings(Some(TileKind::Terminal)) {
                let _ = action.target();
            }
        }
    }
}

#[test]
fn actions_round_trip_through_their_names() {
    use azulejo_config::{Action, Platform};
    let config = azulejo_config::Config::defaults(Platform::Macos);
    for (_, action) in config.keymap.bindings(None) {
        assert_eq!(
            Action::from_name(&action.name(), action.index()),
            Some(action)
        );
    }
    assert_eq!(
        Action::from_name("switch_tab", None),
        None,
        "indexed needs an index"
    );
    assert_eq!(
        Action::from_name("split_auto", Some(2)),
        None,
        "plain takes none"
    );
    assert_eq!(Action::from_name("nope", None), None);
}
