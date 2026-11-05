//! Themes: the built-ins from tokens.json, user themes, `[appearance]`, the
//! terminal colour precedence and contrast (docs/themes.md).

use std::path::Path;

use azulejo_config::ghostty::GhosttyConfig;
use azulejo_config::theme::{
    self, Appearance, ColorValue, Rgba, TerminalColors, Theme, builtin, builtin_themes,
    color_tokens,
};
use azulejo_config::{AppIcon, Config, Platform, Severity, reload};

const MAC: Platform = Platform::Macos;

fn rgba(hex: &str) -> Rgba {
    Rgba::parse(hex).unwrap()
}

fn messages(config: &Config, severity: Severity) -> Vec<String> {
    config
        .diagnostics
        .iter()
        .filter(|d| d.severity == severity)
        .map(|d| d.message.clone())
        .collect()
}

/// A config dir with `config.toml` and `themes/<id>.toml` files.
fn with_files(config: &str, themes: &[(&str, &str)]) -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().unwrap();
    let themes_dir = dir.path().join("themes");
    std::fs::create_dir_all(&themes_dir).unwrap();
    for (id, text) in themes {
        std::fs::write(themes_dir.join(format!("{id}.toml")), text).unwrap();
    }
    let path = dir.path().join("config.toml");
    std::fs::write(&path, config).unwrap();
    let config = reload(&path, MAC);
    (dir, config)
}

#[test]
fn four_built_ins_define_every_token() {
    let ids: Vec<&str> = builtin_themes().iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "atom-one-dark",
            "atom-one-light",
            "macos-dark",
            "macos-light"
        ]
    );
    assert!(color_tokens().len() > 50, "{}", color_tokens().len());
    for theme in builtin_themes() {
        for token in color_tokens() {
            assert!(
                theme.colors.contains_key(token),
                "{} lacks {token}",
                theme.id
            );
        }
        assert_eq!(theme.colors.len(), color_tokens().len(), "{}", theme.id);
        assert!(theme::SYNTAXES.contains(&theme.syntax.as_str()));
        assert!(theme.is_builtin());
    }
    let appearance = |id: &str| builtin(id).unwrap().appearance;
    assert_eq!(appearance("atom-one-dark"), Appearance::Dark);
    assert_eq!(appearance("atom-one-light"), Appearance::Light);
    assert_eq!(appearance("macos-dark"), Appearance::Dark);
    assert_eq!(appearance("macos-light"), Appearance::Light);
}

/// The default theme is the look that shipped before themes: the mockup's
/// One Dark values.
#[test]
fn atom_one_dark_keeps_todays_values() {
    let t = builtin("atom-one-dark").unwrap();
    for (token, hex) in [
        ("window", "#1b1d23"),
        ("sidebar", "#21252b"),
        ("tile", "#282c34"),
        ("tileAlt", "#252930"),
        ("sunken", "#202329"),
        ("selected", "#35393e"),
        ("textPrimary", "#d7dae0"),
        ("textSecondary", "#9da5b4"),
        ("textMuted", "#7f848e"),
        ("accent", "#61afef"),
        ("dimFill", "#1b1d23"),
        ("header", "#21252b"),
        ("headerHairline", "#181a1f"),
        ("border", "#ffffff14"),
        ("tabStrip", "#24262c"),
        ("searchBorderActive", "#61afef80"),
        ("overlay", "#1b1d23eb"),
    ] {
        assert_eq!(
            t.colors[token],
            ColorValue::Hex(rgba(hex)),
            "atom-one-dark {token}"
        );
    }
    assert_eq!(t.terminal.background, rgba("#282c34"), "the tile body");
    assert_eq!(t.terminal.foreground, rgba("#abb2bf"));
    assert_eq!(t.sidebar_material, None);
}

#[test]
fn macos_themes_use_system_colours() {
    for id in ["macos-dark", "macos-light"] {
        let t = builtin(id).unwrap();
        let system = |token: &str| match &t.colors[token] {
            ColorValue::System { name, .. } => name.clone(),
            other => panic!("{id} {token}: {other:?}"),
        };
        assert_eq!(system("accent"), "controlAccentColor");
        assert_eq!(system("selected"), "controlAccentColor");
        assert_eq!(system("textPrimary"), "labelColor");
        assert_eq!(system("tile"), "textBackgroundColor");
        assert_eq!(system("border"), "separatorColor");
        assert_eq!(t.sidebar_material.as_deref(), Some("sidebar"));
    }
}

/// The background a translucent token sits on.
fn under(theme: &Theme, token: &str) -> Rgba {
    let window = theme.rgba("window").unwrap();
    let get = |t: &str| theme.rgba(t).unwrap();
    match token {
        "selected" | "control" => get("sidebar").over(window),
        "header" | "sunken" | "tileAlt" | "overlay" | "browserField" => get("tile").over(window),
        "tabSelected" => get("tabStrip").over(window),
        _ => window,
    }
}

/// A token as drawn: composited over what it sits on.
fn solid(theme: &Theme, token: &str) -> Rgba {
    theme.rgba(token).unwrap().over(under(theme, token))
}

/// WCAG AA for every built-in: body text 4.5:1, muted and secondary text
/// 3:1, against every background they're drawn on.
#[test]
fn text_meets_wcag_aa_in_every_built_in() {
    let mut report = Vec::new();
    for theme in builtin_themes() {
        let checks: &[(&str, &[&str], f64)] = &[
            (
                "textPrimary",
                &[
                    "window", "sidebar", "tile", "tileAlt", "header", "sunken", "selected",
                ],
                4.5,
            ),
            // Preview body text (the webview's --azj-fg).
            ("textSecondary", &["tile", "tileAlt"], 4.5),
            ("textSecondary", &["sidebar", "header", "window"], 3.0),
            (
                "textMuted",
                &["window", "sidebar", "tile", "tileAlt", "header", "selected"],
                3.0,
            ),
            ("tabText", &["tabStrip"], 3.0),
            ("tabTextSelected", &["tabSelected"], 4.5),
            ("searchText", &["searchField"], 3.0),
            ("overlayText", &["overlay"], 4.5),
        ];
        for (text, backgrounds, minimum) in checks {
            for bg in *backgrounds {
                let back = solid(theme, bg);
                let fore = theme.rgba(text).unwrap().over(back);
                let ratio = fore.contrast(back);
                report.push(format!(
                    "{:<15} {text:<16} on {bg:<12} {ratio:5.2} (min {minimum})",
                    theme.id
                ));
                assert!(
                    ratio >= *minimum,
                    "{}: {text} {} on {bg} {} is {ratio:.2}:1, under {minimum}:1",
                    theme.id,
                    fore.hex(),
                    back.hex()
                );
            }
        }
        let term = &theme.terminal;
        let ratio = term.foreground.contrast(term.background);
        report.push(format!(
            "{:<15} terminal fg      on terminal bg {ratio:5.2} (min 4.5)",
            theme.id
        ));
        assert!(ratio >= 4.5, "{} terminal: {ratio:.2}", theme.id);
    }
    // `cargo test -p azulejo-config --test themes -- --nocapture` prints the table.
    println!("{}", report.join("\n"));
}

#[test]
fn user_theme_inherits_and_overrides() {
    let (_dir, config) = with_files(
        "[appearance]\ntheme = \"dusk\"\n",
        &[(
            "dusk",
            r##"
name = "Dusk"
inherits = "atom-one-dark"
syntax = "github-dark"

[color]
window = "#101114"
text_muted = "#8a8f99"      # snake_case works too
accent = "#ff00ff80"

[terminal]
background = "#1d2026"
selection_foreground = ""
palette = { 1 = "#ff6c6b", 9 = "#ff8080" }
"##,
        )],
    );
    assert!(config.diagnostics.is_empty(), "{:?}", config.diagnostics);
    assert_eq!(config.appearance.theme, "dusk");
    let dusk = config.themes.get("dusk").unwrap();
    let base = builtin("atom-one-dark").unwrap();
    assert_eq!(dusk.name, "Dusk");
    assert_eq!(dusk.appearance, Appearance::Dark);
    assert_eq!(dusk.syntax, "github-dark");
    assert_eq!(dusk.rgba("window"), Some(rgba("#101114")));
    assert_eq!(dusk.rgba("textMuted"), Some(rgba("#8a8f99")));
    assert_eq!(dusk.rgba("accent"), Some(rgba("#ff00ff80")));
    assert_eq!(dusk.colors["tile"], base.colors["tile"], "inherited");
    assert_eq!(dusk.terminal.background, rgba("#1d2026"));
    assert_eq!(dusk.terminal.foreground, base.terminal.foreground);
    assert_eq!(dusk.terminal.selection_foreground, None);
    assert_eq!(dusk.terminal.palette[1], rgba("#ff6c6b"));
    assert_eq!(dusk.terminal.palette[2], base.terminal.palette[2]);
    assert!(!dusk.is_builtin());
    // Listed after the built-ins.
    assert_eq!(config.themes.ids().last(), Some(&"dusk"));
}

#[test]
fn user_theme_without_inherits_starts_from_its_appearance() {
    let (_dir, config) = with_files(
        "",
        &[(
            "paper",
            "appearance = \"light\"\n[color]\ntile = \"#fffff0\"\n",
        )],
    );
    let paper = config.themes.get("paper").unwrap();
    let light = builtin("atom-one-light").unwrap();
    assert_eq!(paper.appearance, Appearance::Light);
    assert_eq!(paper.colors["window"], light.colors["window"]);
    assert_eq!(paper.rgba("tile"), Some(rgba("#fffff0")));
    assert_eq!(paper.name, "paper", "the id when there's no name");
}

#[test]
fn bad_theme_values_warn_and_keep_the_inherited_ones() {
    let (_dir, config) = with_files(
        "[appearance]\ntheme = \"broken\"\n",
        &[(
            "broken",
            r##"
inherits = "atom-one-light"
appearance = "sepia"
syntax = "monokai"
shade = 3
[color]
window = "blue"
not_a_token = "#000000"
tile = "#12345"
[terminal]
foreground = 12
blink = true
palette = ["#000000", "nope"]
"##,
        )],
    );
    assert!(!config.has_errors(), "{:?}", config.diagnostics);
    let warnings = messages(&config, Severity::Warning);
    for needle in [
        "`appearance` must be",
        "`syntax` must be one of",
        "unknown setting `shade`",
        "`color.window` isn't a colour",
        "unknown colour token `not_a_token`",
        "`color.tile` isn't a colour",
        "`terminal.foreground` isn't a colour",
        "unknown setting `terminal.blink`",
        "`terminal.palette` 1 isn't a colour",
    ] {
        assert!(
            warnings.iter().any(|w| w.contains(needle)),
            "no warning with {needle:?} in {warnings:#?}"
        );
    }
    // Each problem points at the theme file and its line.
    let d = config
        .diagnostics
        .iter()
        .find(|d| d.message.contains("color.window"))
        .unwrap();
    assert!(d.to_string().contains("broken.toml:7:"), "{d}");
    let broken = config.themes.get("broken").unwrap();
    let light = builtin("atom-one-light").unwrap();
    assert_eq!(broken.appearance, Appearance::Light);
    assert_eq!(broken.syntax, light.syntax);
    assert_eq!(broken.colors["window"], light.colors["window"]);
    assert_eq!(broken.colors["tile"], light.colors["tile"]);
    assert_eq!(broken.terminal.foreground, light.terminal.foreground);
    assert_eq!(broken.terminal.palette[0], rgba("#000000"));
    assert_eq!(broken.terminal.palette[1], light.terminal.palette[1]);
    assert_eq!(config.appearance.theme, "broken");
}

#[test]
fn unusable_theme_files() {
    let (_dir, config) = with_files(
        "[appearance]\ntheme = \"half\"\n",
        &[
            ("half", "name = \"Half\"\n[color\n"),
            ("macos-dark", "[color]\nwindow = \"#000000\"\n"),
            ("odd name", "name = \"x\"\n"),
            ("unknown-base", "inherits = \"solarized\"\n"),
        ],
    );
    let warnings = messages(&config, Severity::Warning);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("not valid TOML, so theme `half`")),
        "{warnings:#?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("`macos-dark` is a built-in name"))
    );
    assert!(warnings.iter().any(|w| w.contains("`odd name` skipped")));
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("`inherits` must name a built-in"))
    );
    // The built-in stays itself.
    assert_eq!(
        config.themes.get("macos-dark"),
        builtin("macos-dark"),
        "a user file can't replace a built-in"
    );
    assert_eq!(
        config.themes.get("unknown-base").unwrap().source,
        theme::ThemeSource::User {
            path: _dir.path().join("themes/unknown-base.toml"),
            inherits: "atom-one-dark".into()
        }
    );
    // `half` doesn't exist, so the config names an unknown theme: an error
    // (a reload keeps the previous config), and the default applies.
    let errors = messages(&config, Severity::Error);
    assert!(
        errors.iter().any(|e| e.contains("unknown theme `half`")),
        "{errors:#?}"
    );
    assert_eq!(config.appearance.theme, "atom-one-dark");
}

#[test]
fn appearance_section() {
    let config = Config::from_toml(
        "[appearance]\ntheme = \"macos-light\"\nterminal_colors = \"ghostty\"\n",
        None,
        MAC,
    );
    assert!(config.diagnostics.is_empty(), "{:?}", config.diagnostics);
    assert_eq!(config.appearance.theme, "macos-light");
    assert!(!config.appearance.is_auto());
    assert_eq!(
        config.appearance.terminal_colors,
        Some(TerminalColors::Ghostty)
    );
    assert_eq!(
        config.appearance.theme_for(Appearance::Dark),
        "macos-light",
        "a fixed theme ignores the system"
    );

    // Defaults.
    let plain = Config::defaults(MAC);
    assert_eq!(plain.appearance.theme, "atom-one-dark");
    assert_eq!(plain.appearance.terminal_colors, None);
    assert!(
        !plain.appearance.tile_agent_badge,
        "no agent badge in headers by default"
    );

    // The header badge comes back on request; anything but a bool is an error.
    let badge = Config::from_toml("[appearance]\ntile_agent_badge = true\n", None, MAC);
    assert!(badge.diagnostics.is_empty(), "{:?}", badge.diagnostics);
    assert!(badge.appearance.tile_agent_badge);
    let wrong = Config::from_toml("[appearance]\ntile_agent_badge = \"yes\"\n", None, MAC);
    assert!(
        messages(&wrong, Severity::Error)
            .iter()
            .any(|e| e.contains("tile_agent_badge` must be true or false"))
    );
    assert!(!wrong.appearance.tile_agent_badge);

    // The Dock icon: light unless the config says otherwise.
    assert_eq!(
        plain.appearance.app_icon,
        AppIcon::Light,
        "light by default"
    );
    for (name, icon) in [
        ("light", AppIcon::Light),
        ("dark", AppIcon::Dark),
        ("auto", AppIcon::Auto),
    ] {
        let set = Config::from_toml(&format!("[appearance]\napp_icon = \"{name}\"\n"), None, MAC);
        assert!(set.diagnostics.is_empty(), "{name}: {:?}", set.diagnostics);
        assert_eq!(set.appearance.app_icon, icon);
        assert_eq!(icon.name(), name);
    }
    for bad in ["\"system\"", "true", "1"] {
        let wrong = Config::from_toml(&format!("[appearance]\napp_icon = {bad}\n"), None, MAC);
        assert!(
            messages(&wrong, Severity::Error).iter().any(
                |e| e.contains("`appearance.app_icon` must be \"light\", \"dark\" or \"auto\"")
            ),
            "{bad}: {:?}",
            wrong.diagnostics
        );
        assert_eq!(
            wrong.appearance.app_icon,
            AppIcon::Light,
            "{bad} keeps the default"
        );
    }

    // auto follows the system, with the atom pair by default.
    let auto = Config::from_toml("[appearance]\ntheme = \"auto\"\n", None, MAC);
    assert!(auto.appearance.is_auto());
    assert_eq!(
        auto.appearance.theme_for(Appearance::Light),
        "atom-one-light"
    );
    assert_eq!(auto.appearance.theme_for(Appearance::Dark), "atom-one-dark");
    let mac = Config::from_toml(
        "[appearance]\ntheme = \"auto\"\ntheme_light = \"macos-light\"\ntheme_dark = \"macos-dark\"\n",
        None,
        MAC,
    );
    assert_eq!(mac.appearance.theme_for(Appearance::Light), "macos-light");
    assert_eq!(mac.appearance.theme_for(Appearance::Dark), "macos-dark");

    // Unknown names are errors and fall back; a light slot naming a dark
    // theme is only a warning.
    let bad = Config::from_toml(
        "[appearance]\ntheme = \"solarized\"\ntheme_dark = \"nope\"\ntheme_light = \"atom-one-dark\"\nterminal_colors = \"both\"\nfont = 3\n",
        None,
        MAC,
    );
    let errors = messages(&bad, Severity::Error);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("unknown theme `solarized`"))
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("unknown theme `nope` for `theme_dark`"))
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("terminal_colors` must be"))
    );
    let warnings = messages(&bad, Severity::Warning);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("`theme_light` names `atom-one-dark`, a dark theme"))
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("unknown setting `appearance.font`"))
    );
    assert_eq!(bad.appearance.theme, "atom-one-dark");
    assert_eq!(bad.appearance.theme_dark, "atom-one-dark");
    assert_eq!(bad.appearance.terminal_colors, None);
    // The error points at the value.
    let d = bad
        .diagnostics
        .iter()
        .find(|d| d.message.contains("solarized"))
        .unwrap();
    assert_eq!(d.location.map(|l| (l.line, l.column)), Some((2, 9)));
}

/// Terminal colours: the user's Ghostty config wins by default whenever it
/// sets colours of its own; `terminal_colors` overrides that either way.
#[test]
fn terminal_colour_precedence() {
    let path = Path::new("/tmp/ghostty");
    let plain = GhosttyConfig::parse("font-size = 13\nunfocused-split-opacity = 0.7\n", path);
    assert!(!plain.sets_colors());
    for line in [
        "theme = catppuccin-mocha",
        "background = #101010",
        "foreground = #eeeeee",
        "palette = 1=#ff0000",
    ] {
        let set = GhosttyConfig::parse(line, path);
        assert!(set.sets_colors(), "{line}");
        let config = Config::defaults(MAC);
        assert_eq!(
            config.appearance.terminal_colors(&set),
            TerminalColors::Ghostty,
            "{line}"
        );
        let forced = Config::from_toml("[appearance]\nterminal_colors = \"theme\"\n", None, MAC);
        assert_eq!(
            forced.appearance.terminal_colors(&set),
            TerminalColors::Theme,
            "{line}"
        );
    }
    let config = Config::defaults(MAC);
    assert_eq!(
        config.appearance.terminal_colors(&plain),
        TerminalColors::Theme
    );
    let forced = Config::from_toml("[appearance]\nterminal_colors = \"ghostty\"\n", None, MAC);
    assert_eq!(
        forced.appearance.terminal_colors(&plain),
        TerminalColors::Ghostty
    );
}

#[test]
fn terminal_palette_as_ghostty_config() {
    let text = builtin("atom-one-dark").unwrap().terminal.ghostty_config();
    assert!(text.contains("background = #282c34\n"), "{text}");
    assert!(text.contains("foreground = #abb2bf\n"));
    assert!(text.contains("selection-foreground = cell-foreground\n"));
    assert!(text.contains("palette = 0=#21252b\n"));
    assert!(text.contains("palette = 15=#abb2bf\n"));
    assert_eq!(text.matches("palette = ").count(), 16);
    // Ghostty reads it back as the same colours.
    let parsed = GhosttyConfig::parse(&text, Path::new("/tmp/theme"));
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(parsed.palette.len(), 16);
    let mac = builtin("macos-light").unwrap().terminal.ghostty_config();
    assert!(mac.contains("selection-foreground = #000000\n"), "{mac}");
}

#[test]
fn themes_json_carries_what_the_shells_need() {
    let json = builtin("macos-dark").unwrap().to_json();
    assert_eq!(json["id"], "macos-dark");
    assert_eq!(json["appearance"], "dark");
    assert_eq!(json["builtin"], true);
    assert_eq!(json["colors"]["accent"]["system"], "controlAccentColor");
    assert_eq!(json["colors"]["textMuted"]["alpha"], 0.45);
    assert_eq!(json["terminal"]["palette"].as_array().unwrap().len(), 16);
    let atom = builtin("atom-one-dark").unwrap().to_json();
    assert_eq!(atom["colors"]["window"], "#1b1d23");
    assert_eq!(atom["colors"]["border"], "#ffffff14");
}
