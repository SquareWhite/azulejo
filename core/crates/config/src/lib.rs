//! Azulejo's config: `~/.config/azulejo/config.toml` (XDG on both platforms)
//! layered over the bundled default keymap, plus the subset of the user's
//! Ghostty config the Linux terminal tile reads.
//!
//! Loading never fails. A missing file means defaults; a bad file means
//! defaults plus [`Diagnostic`]s for a non-blocking banner. A bad value only
//! drops that value: the default for it stays.
//!
//! ```toml
//! [keys]                 # all platforms
//! split_right = "cmd+d"
//! goto = ""              # unbind
//!
//! [keys.linux]           # one platform; beats [keys]
//! split_right = "super+d"
//!
//! [keys.terminal]        # only while a terminal tile has focus
//! toggle_sidebar = "cmd+a"
//!
//! [prefix]
//! enabled = true
//! key = "cmd+b"          # or key.macos / key.linux
//!
//! [animation]
//! enabled = true
//! duration_ms = 200
//! curve = "ease_out"     # or "ease_in_out", "linear"
//! style = "reflow"       # moving tiles: "reflow" (content resizes live), "crop" (content at 1:1, revealed or cut off), "scale", "outline"
//! tab_switch = "fade"    # tab/workspace switches: "fade", "slide" or "none"
//! reduce_motion = "system"  # "system": off when macOS Reduce Motion is on; "ignore"
//!
//! [herdr]
//! session = "default"    # herdr session for terminal tiles: "default" (herdr's own) or a name
//! enabled = true         # false: terminal tiles stay unbound (no herdr)
//! mirror_layout = true   # tabs' splits, zoom and Monocle mirrored into herdr, both ways (ADR 0012)
//!
//! [terminal]
//! on_exit = "close"      # the shell exits (ctrl+d, `exit`): "close" the tile, or "keep" it to respawn
//!
//! [appearance]           # themes (docs/themes.md)
//! theme = "atom-one-dark"   # a theme id, or "auto" to follow the system
//! theme_light = "atom-one-light"  # with "auto"
//! theme_dark = "atom-one-dark"
//! terminal_colors = "theme"  # or "ghostty"; default: "ghostty" if its config sets colours
//! tile_agent_badge = false  # true: "● claude · working" in terminal tile headers
//!                           # (off, a blocked tile still gets a yellow mark there)
//! [sidebar]
//! collapsed = "rail"     # what toggle_sidebar collapses it to: "rail" (workspace numbers and
//!                        # agent states down a narrow column) or "hidden"
//!
//! [layout]               # dynamic layouts (ADR 0010)
//! default = "tile"       # new tabs: "tile", "monocle", "three_column" or "manual"
//! insert = "end"         # new tiles in dynamic tabs: "end", "master" or "after_focused"
//! cycle = ["tile", "monocle", "three_column"]
//! [layout.tile]
//! masters = 1            # 0 to 10
//! ratio = 0.5            # master area, 0.2 to 0.8
//! orientation = "left"   # "left", "top", "right" or "bottom"
//! [layout.three_column]
//! masters = 1            # 1 to 10
//! ratio = 0.6            # centre column, 0.2 to 0.75
//! ```

mod action;
mod catalogue;
mod chord;
mod diag;
pub mod ghostty;
mod keymap;
mod listing;
pub mod theme;

use std::path::{Path, PathBuf};

use azulejo_protocol::{InsertAt, LayoutKind, LayoutPrefs, OnExit, Orientation, SidebarState};
use toml::de::{DeTable, DeValue};

pub use action::{Action, PLANNED_METHODS, SEND_TEXT, Target, control_key, quote_text};
pub use catalogue::{INDEXED_ENTRIES, INTERNAL_METHODS, catalogue};
pub use chord::{Chord, ChordPattern, Key, Mods, Platform};
pub use diag::{Diagnostic, Location, Severity, Source};
pub use keymap::{Context, KeyContext, KeyOutcome, KeyRouter, Keymap, PrefixState, Written};
pub use listing::{
    DOC_BEGIN, DOC_END, GLOBAL, chord_patterns, key_listing, keys_template, reference_markdown,
    splice_reference,
};
pub use theme::{AppearanceConfig, TerminalColors, Theme, Themes};

use diag::Sink;
use keymap::{Layer, LayerInfo, LayerParser};

/// The bundled default keymap (`shared/config/keymap.default.toml`).
pub const DEFAULT_KEYMAP: &str = include_str!("../../../../shared/config/keymap.default.toml");

/// Layout animation settings (F4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Animation {
    pub enabled: bool,
    pub duration_ms: u32,
    pub curve: Curve,
    pub style: Style,
    pub tab_switch: TabSwitch,
    pub reduce_motion: ReduceMotion,
}

impl Animation {
    pub const MAX_DURATION_MS: u32 = 1000;
}

impl Default for Animation {
    fn default() -> Self {
        Animation {
            enabled: true,
            duration_ms: 200,
            curve: Curve::EaseOut,
            style: Style::Reflow,
            tab_switch: TabSwitch::Fade,
            reduce_motion: ReduceMotion::System,
        }
    }
}

/// The timing curve of layout motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    EaseOut,
    EaseInOut,
    Linear,
}

/// What a tile's content does while its frame moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// The content lays out at every step of the motion: a terminal's grid
    /// follows the moving frame (whole cells, a few PTY resizes per motion).
    Reflow,
    /// The frame is a clip: content stays at 1:1, revealed or cut off.
    Crop,
    /// The content scales with the frame (text stretches mid-motion).
    Scale,
    /// Only outlines move; content fades in at its new place at the end.
    Outline,
}

/// How switching tab or workspace looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabSwitch {
    Fade,
    Slide,
    None,
}

/// Whether the OS "reduce motion" setting turns animation off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReduceMotion {
    /// Follow the OS: no motion while it asks for reduced motion.
    System,
    /// Animate anyway.
    Ignore,
}

macro_rules! named {
    ($ty:ident { $($variant:ident = $name:literal),+ $(,)? }) => {
        impl $ty {
            pub const NAMES: &[&str] = &[$($name),+];
            pub fn name(self) -> &'static str {
                match self { $($ty::$variant => $name),+ }
            }
            pub fn from_name(name: &str) -> Option<$ty> {
                match name { $($name => Some($ty::$variant),)+ _ => None }
            }
        }
    };
}
named!(Curve { EaseOut = "ease_out", EaseInOut = "ease_in_out", Linear = "linear" });
named!(Style { Reflow = "reflow", Crop = "crop", Scale = "scale", Outline = "outline" });
named!(TabSwitch { Fade = "fade", Slide = "slide", None = "none" });
named!(ReduceMotion { System = "system", Ignore = "ignore" });

/// Which herdr session terminal tiles live in (ADR 0008).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrConfig {
    pub enabled: bool,
    /// The session name. `None`, the default, means herdr's default session:
    /// the one the user's own `herdr` TUI and phone use.
    pub session: Option<String>,
    /// Connect herdr's saved machines too (ADR 0011). `None` means the
    /// default: on with herdr's default session, off with a named one, so an
    /// isolated setup (tests, dev profiles) never reaches real machines.
    pub machines: Option<bool>,
    /// Per machine, by herdr label or ID: presentation only.
    pub machine: std::collections::BTreeMap<String, MachinePrefs>,
    /// Tests and dev profiles: read the machine list from this file (the
    /// JSON `herdr machine list --json` prints) instead of herdr.
    pub machines_file: Option<PathBuf>,
    /// Tests and dev profiles: run this instead of `ssh`.
    pub ssh: Option<PathBuf>,
    /// Mirror each tab's layout into herdr, both ways: one herdr tab per
    /// Azulejo tab, its splits, zoom and Monocle (ADR 0012). Off: one herdr
    /// tab per terminal tile, herdr's tree left alone. Applies live.
    pub mirror_layout: bool,
}

/// What Azulejo does with one of herdr's saved machines. Connection details
/// stay in herdr.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MachinePrefs {
    /// Not listed and not connected.
    pub hidden: bool,
    /// Listed as "off" but not connected.
    pub no_connect: bool,
}

impl HerdrConfig {
    /// What `session = "default"` means: herdr's default session.
    pub const HERDR_DEFAULT: &str = "default";

    /// Whether herdr's saved machines are connected (see `machines`).
    pub fn machines_enabled(&self) -> bool {
        self.enabled && self.machines.unwrap_or(self.session.is_none())
    }

    /// The preferences for a machine, looked up by ID, then by label.
    pub fn prefs(&self, id: &str, label: &str) -> MachinePrefs {
        self.machine
            .get(id)
            .or_else(|| self.machine.get(label))
            .copied()
            .unwrap_or_default()
    }
}

impl Default for HerdrConfig {
    fn default() -> Self {
        HerdrConfig {
            enabled: true,
            session: None,
            machines: None,
            machine: Default::default(),
            machines_file: None,
            ssh: None,
            mirror_layout: true,
        }
    }
}

/// Terminal tile settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalConfig {
    /// What happens when the process in a tile's pane exits (ADR 0009).
    pub on_exit: OnExit,
}

/// The `[sidebar]` settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarConfig {
    /// What collapsing the sidebar (`toggle_sidebar`) gives: a rail or
    /// nothing. Never `Full`.
    pub collapsed: SidebarState,
}

impl Default for SidebarConfig {
    fn default() -> Self {
        SidebarConfig {
            collapsed: SidebarState::Rail,
        }
    }
}

/// The `[layout]` defaults: what new tabs start as. Unlike the model's own
/// default (manual, so a bare session behaves as before), new tabs default to
/// dynamic Tile here (ADR 0010).
pub fn default_layout() -> LayoutPrefs {
    LayoutPrefs {
        default: LayoutKind::Tile,
        ..LayoutPrefs::default()
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub keymap: Keymap,
    pub animation: Animation,
    pub layout: LayoutPrefs,
    pub herdr: HerdrConfig,
    pub terminal: TerminalConfig,
    /// `[appearance]`: the theme, and whose colours terminals use.
    pub appearance: AppearanceConfig,
    /// The built-in themes plus the user's (`<config dir>/themes/*.toml`).
    pub themes: Themes,
    pub sidebar: SidebarConfig,
    /// The file this came from, if any.
    pub path: Option<PathBuf>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Config {
    /// The bundled defaults alone.
    pub fn defaults(platform: Platform) -> Config {
        Config::build(platform, None, None)
    }

    /// `text` as the user's config. `path` is only used in diagnostics.
    pub fn from_toml(text: &str, path: Option<PathBuf>, platform: Platform) -> Config {
        Config::build(platform, Some(text), path)
    }

    fn build(platform: Platform, user: Option<&str>, path: Option<PathBuf>) -> Config {
        let mut diagnostics = Vec::new();
        let mut layers = vec![Layer::default()];
        let mut infos = vec![LayerInfo {
            text: DEFAULT_KEYMAP,
            source: Source::DefaultKeymap,
        }];
        let mut animation = Animation::default();
        let mut layout = default_layout();
        let mut herdr = HerdrConfig::default();
        let mut terminal = TerminalConfig::default();
        let mut appearance = AppearanceConfig::default();
        let mut appearance_spans = theme::AppearanceSpans::default();
        // User themes live next to the config file.
        let themes_dir = path
            .as_deref()
            .and_then(Path::parent)
            .map(|dir| dir.join("themes"));
        let themes = Themes::load(themes_dir.as_deref(), &mut diagnostics);
        let mut sidebar = SidebarConfig::default();

        parse_layer(
            DEFAULT_KEYMAP,
            Source::DefaultKeymap,
            0,
            &mut layers[0],
            &mut diagnostics,
            |_, _, _| false,
        );

        if let Some(text) = user {
            let source = Source::User(path.clone());
            let mut layer = Layer::default();
            parse_layer(
                text,
                source.clone(),
                1,
                &mut layer,
                &mut diagnostics,
                |sink, name, value| match name {
                    "animation" => {
                        parse_animation(sink, value, &mut animation);
                        true
                    }
                    "herdr" => {
                        parse_herdr(sink, value, &mut herdr);
                        true
                    }
                    "terminal" => {
                        parse_terminal(sink, value, &mut terminal);
                        true
                    }
                    "sidebar" => {
                        parse_sidebar(sink, value, &mut sidebar);
                        true
                    }
                    "layout" => {
                        parse_layout(sink, value, &mut layout);
                        true
                    }
                    "appearance" => {
                        theme::parse_appearance(
                            sink,
                            value,
                            &mut appearance,
                            &mut appearance_spans,
                        );
                        true
                    }
                    _ => false,
                },
            );
            let mut sink = Sink {
                text,
                source: source.clone(),
                out: &mut diagnostics,
            };
            theme::check_appearance(&mut sink, &mut appearance, &appearance_spans, &themes);
            layers.push(layer);
            infos.push(LayerInfo { text, source });
        }

        let keymap = Keymap::build(platform, &layers, &infos, &mut diagnostics);
        // Report in file order: defaults first, then the user's file, top to bottom.
        diagnostics.sort_by_key(|d| {
            let rank = match d.source {
                Source::DefaultKeymap => 0,
                Source::User(_) => 1,
                Source::Ghostty(_) => 2,
                Source::Theme(_) => 3,
            };
            (rank, d.location.map(|l| (l.line, l.column)))
        });
        Config {
            keymap,
            animation,
            layout,
            herdr,
            terminal,
            appearance,
            themes,
            sidebar,
            path,
            diagnostics,
        }
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// Parses one TOML file into `layer`. `extra` handles top-level sections other
/// than `keys` and `prefix`, returning `false` for unknown ones. A file that
/// isn't valid TOML contributes nothing.
fn parse_layer(
    text: &str,
    source: Source,
    index: usize,
    layer: &mut Layer,
    diagnostics: &mut Vec<Diagnostic>,
    mut extra: impl FnMut(&mut Sink<'_>, &str, &toml::Spanned<DeValue<'_>>) -> bool,
) {
    let mut sink = Sink {
        text,
        source,
        out: diagnostics,
    };
    let table = match DeTable::parse(text) {
        Ok(table) => table,
        Err(e) => {
            let message = format!(
                "not valid TOML, so none of it applies: {}",
                e.message().trim()
            );
            sink.push(Severity::Error, e.span(), message);
            return;
        }
    };
    for (key, value) in table.get_ref().iter() {
        let name = key.get_ref().as_ref();
        match (name, value.get_ref()) {
            ("keys", DeValue::Table(keys)) => LayerParser::new(index, &mut sink).keys(keys, layer),
            ("prefix", DeValue::Table(prefix)) => {
                LayerParser::new(index, &mut sink).prefix(prefix, layer)
            }
            ("keys" | "prefix", _) => sink.error(value.span(), format!("`{name}` must be a table")),
            _ => {
                if !extra(&mut sink, name, value) {
                    sink.warn(key.span(), format!("unknown section `{name}`"));
                }
            }
        }
    }
}

fn parse_animation(sink: &mut Sink<'_>, value: &toml::Spanned<DeValue<'_>>, out: &mut Animation) {
    let DeValue::Table(table) = value.get_ref() else {
        sink.error(value.span(), "`animation` must be a table");
        return;
    };
    fn choice<T: Copy>(
        sink: &mut Sink<'_>,
        value: &toml::Spanned<DeValue<'_>>,
        key: &str,
        names: &[&str],
        parse: fn(&str) -> Option<T>,
        out: &mut T,
    ) {
        let found = match value.get_ref() {
            DeValue::String(s) => parse(s.as_ref()),
            _ => None,
        };
        match found {
            Some(v) => *out = v,
            None => sink.error(
                value.span(),
                format!(
                    "`animation.{key}` must be one of {}",
                    names
                        .iter()
                        .map(|n| format!("\"{n}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
        }
    }
    for (key, value) in table.iter() {
        match (key.get_ref().as_ref(), value.get_ref()) {
            ("enabled", DeValue::Boolean(b)) => out.enabled = *b,
            // Unfocused tiles always dim now: dimming is the focus indicator.
            ("dim_inactive", _) => sink.warn(
                key.span(),
                "`animation.dim_inactive` is gone: unfocused tiles always dim now \
                 (set `unfocused-split-opacity = 1` in Ghostty's config to turn that off)",
            ),
            ("duration_ms", DeValue::Integer(i)) => {
                match i64::from_str_radix(i.as_str(), i.radix()) {
                    Ok(ms) if (0..=i64::from(Animation::MAX_DURATION_MS)).contains(&ms) => {
                        out.duration_ms = ms as u32
                    }
                    _ => sink.error(
                        value.span(),
                        format!(
                            "`animation.duration_ms` must be 0 to {}",
                            Animation::MAX_DURATION_MS
                        ),
                    ),
                }
            }
            ("curve", _) => choice(
                sink,
                value,
                "curve",
                Curve::NAMES,
                Curve::from_name,
                &mut out.curve,
            ),
            ("style", _) => choice(
                sink,
                value,
                "style",
                Style::NAMES,
                Style::from_name,
                &mut out.style,
            ),
            ("tab_switch", _) => choice(
                sink,
                value,
                "tab_switch",
                TabSwitch::NAMES,
                TabSwitch::from_name,
                &mut out.tab_switch,
            ),
            ("reduce_motion", _) => choice(
                sink,
                value,
                "reduce_motion",
                ReduceMotion::NAMES,
                ReduceMotion::from_name,
                &mut out.reduce_motion,
            ),
            ("enabled", _) => sink.error(value.span(), "`animation.enabled` must be true or false"),
            ("duration_ms", _) => sink.error(
                value.span(),
                "`animation.duration_ms` must be a whole number",
            ),
            (other, _) => sink.warn(key.span(), format!("unknown setting `animation.{other}`")),
        }
    }
}

fn parse_terminal(
    sink: &mut Sink<'_>,
    value: &toml::Spanned<DeValue<'_>>,
    out: &mut TerminalConfig,
) {
    let DeValue::Table(table) = value.get_ref() else {
        sink.error(value.span(), "`terminal` must be a table");
        return;
    };
    for (key, value) in table.iter() {
        match (key.get_ref().as_ref(), value.get_ref()) {
            ("on_exit", DeValue::String(s)) if s.as_ref() == "close" => out.on_exit = OnExit::Close,
            ("on_exit", DeValue::String(s)) if s.as_ref() == "keep" => out.on_exit = OnExit::Keep,
            ("on_exit", _) => sink.error(
                value.span(),
                "`terminal.on_exit` must be \"close\" or \"keep\"",
            ),
            (other, _) => sink.warn(key.span(), format!("unknown setting `terminal.{other}`")),
        }
    }
}

fn parse_sidebar(sink: &mut Sink<'_>, value: &toml::Spanned<DeValue<'_>>, out: &mut SidebarConfig) {
    let DeValue::Table(table) = value.get_ref() else {
        sink.error(value.span(), "`sidebar` must be a table");
        return;
    };
    for (key, value) in table.iter() {
        match (key.get_ref().as_ref(), value.get_ref()) {
            ("collapsed", DeValue::String(s)) if s.as_ref() == "rail" => {
                out.collapsed = SidebarState::Rail
            }
            ("collapsed", DeValue::String(s)) if s.as_ref() == "hidden" => {
                out.collapsed = SidebarState::Hidden
            }
            ("collapsed", _) => sink.error(
                value.span(),
                "`sidebar.collapsed` must be \"rail\" or \"hidden\"",
            ),
            (other, _) => sink.warn(key.span(), format!("unknown setting `sidebar.{other}`")),
        }
    }
}

fn layout_kind(
    sink: &mut Sink<'_>,
    value: &toml::Spanned<DeValue<'_>>,
    what: &str,
) -> Option<LayoutKind> {
    let found = match value.get_ref() {
        DeValue::String(s) => match s.as_ref() {
            // `tall` is a layout-file alias; config names the kinds.
            "tall" => None,
            name => LayoutKind::parse(name),
        },
        _ => None,
    };
    if found.is_none() {
        sink.error(
            value.span(),
            format!("`{what}` must be \"tile\", \"monocle\", \"three_column\" or \"manual\""),
        );
    }
    found
}

fn number(value: &DeValue<'_>) -> Option<f64> {
    match value {
        DeValue::Float(f) => f.as_str().parse().ok(),
        DeValue::Integer(i) => i64::from_str_radix(i.as_str(), i.radix())
            .ok()
            .map(|n| n as f64),
        _ => None,
    }
}

fn whole(value: &DeValue<'_>) -> Option<i64> {
    match value {
        DeValue::Integer(i) => i64::from_str_radix(i.as_str(), i.radix()).ok(),
        _ => None,
    }
}

fn parse_layout(sink: &mut Sink<'_>, value: &toml::Spanned<DeValue<'_>>, out: &mut LayoutPrefs) {
    let DeValue::Table(table) = value.get_ref() else {
        sink.error(value.span(), "`layout` must be a table");
        return;
    };
    for (key, value) in table.iter() {
        match (key.get_ref().as_ref(), value.get_ref()) {
            ("default", _) => {
                if let Some(kind) = layout_kind(sink, value, "layout.default") {
                    out.default = kind;
                }
            }
            ("insert", DeValue::String(s)) => match s.as_ref() {
                "end" => out.insert = InsertAt::End,
                "master" => out.insert = InsertAt::Master,
                "after_focused" => out.insert = InsertAt::AfterFocused,
                _ => sink.error(
                    value.span(),
                    "`layout.insert` must be \"end\", \"master\" or \"after_focused\"",
                ),
            },
            ("insert", _) => sink.error(value.span(), "`layout.insert` must be a string"),
            ("cycle", DeValue::Array(items)) => {
                let mut cycle = Vec::new();
                let mut ok = true;
                for item in items.iter() {
                    match layout_kind(sink, item, "layout.cycle") {
                        Some(kind) if !cycle.contains(&kind) => cycle.push(kind),
                        Some(_) => {}
                        None => ok = false,
                    }
                }
                if ok && cycle.is_empty() {
                    sink.error(value.span(), "`layout.cycle` needs at least one kind");
                } else if ok {
                    out.cycle = cycle;
                }
            }
            ("cycle", _) => sink.error(value.span(), "`layout.cycle` must be a list of kinds"),
            ("tile", DeValue::Table(t)) => {
                for (k, v) in t.iter() {
                    match (k.get_ref().as_ref(), v.get_ref()) {
                        ("masters", raw) => match whole(raw) {
                            Some(m @ 0..=10) => out.params.tile.masters = m as u8,
                            _ => sink.error(v.span(), "`layout.tile.masters` must be 0 to 10"),
                        },
                        ("ratio", raw) => match number(raw) {
                            Some(r) if (0.2..=0.8).contains(&r) => out.params.tile.ratio = r,
                            _ => sink.error(v.span(), "`layout.tile.ratio` must be 0.2 to 0.8"),
                        },
                        ("orientation", DeValue::String(o)) => match o.as_ref() {
                            "left" => out.params.tile.orientation = Orientation::Left,
                            "top" => out.params.tile.orientation = Orientation::Top,
                            "right" => out.params.tile.orientation = Orientation::Right,
                            "bottom" => out.params.tile.orientation = Orientation::Bottom,
                            _ => sink.error(
                                v.span(),
                                "`layout.tile.orientation` must be \"left\", \"top\", \"right\" or \"bottom\"",
                            ),
                        },
                        ("orientation", _) => sink.error(
                            v.span(),
                            "`layout.tile.orientation` must be a string",
                        ),
                        (other, _) => {
                            sink.warn(k.span(), format!("unknown setting `layout.tile.{other}`"))
                        }
                    }
                }
            }
            ("three_column", DeValue::Table(t)) => {
                for (k, v) in t.iter() {
                    match (k.get_ref().as_ref(), v.get_ref()) {
                        ("masters", raw) => match whole(raw) {
                            Some(m @ 1..=10) => out.params.three_column.masters = m as u8,
                            _ => sink
                                .error(v.span(), "`layout.three_column.masters` must be 1 to 10"),
                        },
                        ("ratio", raw) => match number(raw) {
                            Some(r) if (0.2..=0.75).contains(&r) => {
                                out.params.three_column.ratio = r
                            }
                            _ => sink
                                .error(v.span(), "`layout.three_column.ratio` must be 0.2 to 0.75"),
                        },
                        (other, _) => sink.warn(
                            k.span(),
                            format!("unknown setting `layout.three_column.{other}`"),
                        ),
                    }
                }
            }
            ("tile" | "three_column", _) => sink.error(
                value.span(),
                format!("`layout.{}` must be a table", key.get_ref().as_ref()),
            ),
            (other, _) => sink.warn(key.span(), format!("unknown setting `layout.{other}`")),
        }
    }
}

fn parse_herdr(sink: &mut Sink<'_>, value: &toml::Spanned<DeValue<'_>>, out: &mut HerdrConfig) {
    let DeValue::Table(table) = value.get_ref() else {
        sink.error(value.span(), "`herdr` must be a table");
        return;
    };
    for (key, value) in table.iter() {
        match (key.get_ref().as_ref(), value.get_ref()) {
            ("enabled", DeValue::Boolean(b)) => out.enabled = *b,
            ("enabled", _) => sink.error(value.span(), "`herdr.enabled` must be true or false"),
            ("session", DeValue::String(name)) => {
                let name = name.trim();
                if name == HerdrConfig::HERDR_DEFAULT {
                    out.session = None;
                } else if !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                {
                    out.session = Some(name.to_owned());
                } else {
                    sink.error(
                        value.span(),
                        "`herdr.session` must be \"default\" or a name made of letters, digits, `-`, `_` and `.`",
                    );
                }
            }
            ("session", _) => sink.error(value.span(), "`herdr.session` must be a string"),
            ("mirror_layout", DeValue::Boolean(b)) => out.mirror_layout = *b,
            ("mirror_layout", _) => {
                sink.error(value.span(), "`herdr.mirror_layout` must be true or false")
            }
            ("machines", DeValue::Boolean(b)) => out.machines = Some(*b),
            ("machines", _) => sink.error(value.span(), "`herdr.machines` must be true or false"),
            ("machine", DeValue::Table(machines)) => {
                for (name, prefs) in machines.iter() {
                    let name = name.get_ref().to_string();
                    let DeValue::Table(prefs) = prefs.get_ref() else {
                        sink.error(
                            prefs.span(),
                            format!("`herdr.machine.\"{name}\"` must be a table"),
                        );
                        continue;
                    };
                    let mut out_prefs = MachinePrefs::default();
                    for (k, v) in prefs.iter() {
                        match (k.get_ref().as_ref(), v.get_ref()) {
                            ("hidden", DeValue::Boolean(b)) => out_prefs.hidden = *b,
                            ("connect", DeValue::Boolean(b)) => out_prefs.no_connect = !*b,
                            ("hidden" | "connect", _) => sink.error(
                                v.span(),
                                format!("`herdr.machine.\"{name}\".{}` must be true or false", k.get_ref()),
                            ),
                            (other, _) => sink.warn(
                                k.span(),
                                format!(
                                    "unknown setting `herdr.machine.\"{name}\".{other}`; machines are set up in herdr (`herdr machine add`)"
                                ),
                            ),
                        }
                    }
                    out.machine.insert(name, out_prefs);
                }
            }
            ("machine", _) => sink.error(
                value.span(),
                "`herdr.machine` must be a table of machines by herdr label or ID",
            ),
            ("machines_file", DeValue::String(path)) => {
                out.machines_file = Some(PathBuf::from(path.as_ref()));
            }
            ("ssh", DeValue::String(path)) => out.ssh = Some(PathBuf::from(path.as_ref())),
            ("machines_file" | "ssh", _) => sink.error(
                value.span(),
                format!("`herdr.{}` must be a path", key.get_ref()),
            ),
            (other, _) => sink.warn(key.span(), format!("unknown setting `herdr.{other}`")),
        }
    }
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// `$XDG_CONFIG_HOME`, else `~/.config`. Used on macOS too.
pub(crate) fn xdg_config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home_dir().map(|h| h.join(".config")))
}

/// Where the user's config lives: `$XDG_CONFIG_HOME/azulejo/config.toml`.
pub fn config_path() -> Option<PathBuf> {
    xdg_config_home().map(|dir| dir.join("azulejo").join("config.toml"))
}

/// Loads the user's config from [`config_path`].
pub fn load(platform: Platform) -> Config {
    match config_path() {
        Some(path) => reload(&path, platform),
        None => Config::defaults(platform),
    }
}

/// Reads `path` again. A missing file means defaults with no diagnostics; an
/// unreadable one means defaults plus an error.
pub fn reload(path: &Path, platform: Platform) -> Config {
    match std::fs::read_to_string(path) {
        Ok(text) => Config::from_toml(&text, Some(path.to_path_buf()), platform),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No file, but user themes beside it still show up (⌘K lists them).
            Config::build(platform, None, Some(path.to_path_buf()))
        }
        Err(e) => {
            let mut config = Config::defaults(platform);
            config.path = Some(path.to_path_buf());
            config.diagnostics.push(Diagnostic {
                severity: Severity::Error,
                source: Source::User(Some(path.to_path_buf())),
                location: None,
                message: format!("can't read the config, using defaults: {e}"),
            });
            config
        }
    }
}
