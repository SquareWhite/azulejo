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
//! style = "crop"         # moving tiles: "crop" (content at 1:1, revealed or cut off), "scale", "outline"
//! tab_switch = "fade"    # tab/workspace switches: "fade", "slide" or "none"
//! dim_inactive = false   # dim tiles without focus
//! reduce_motion = "system"  # "system": off when macOS Reduce Motion is on; "ignore"
//!
//! [herdr]
//! session = "default"    # herdr session for terminal tiles: "default" (herdr's own) or a name
//! enabled = true         # false: terminal tiles stay unbound (no herdr)
//!
//! [terminal]
//! on_exit = "close"      # the shell exits (ctrl+d, `exit`): "close" the tile, or "keep" it to respawn
//! ```

mod action;
mod catalogue;
mod chord;
mod diag;
pub mod ghostty;
mod keymap;

use std::path::{Path, PathBuf};

use azulejo_protocol::OnExit;
use toml::de::{DeTable, DeValue};

pub use action::{Action, PLANNED_METHODS, Target};
pub use catalogue::{INDEXED_ENTRIES, INTERNAL_METHODS, catalogue};
pub use chord::{Chord, ChordPattern, Key, Mods, Platform};
pub use diag::{Diagnostic, Location, Severity, Source};
pub use keymap::{Context, KeyOutcome, KeyRouter, Keymap, PrefixState};

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
    pub dim_inactive: bool,
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
            style: Style::Crop,
            tab_switch: TabSwitch::Fade,
            dim_inactive: false,
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
named!(Style { Crop = "crop", Scale = "scale", Outline = "outline" });
named!(TabSwitch { Fade = "fade", Slide = "slide", None = "none" });
named!(ReduceMotion { System = "system", Ignore = "ignore" });

/// Which herdr session terminal tiles live in (ADR 0008).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrConfig {
    pub enabled: bool,
    /// The session name. `None`, the default, means herdr's default session:
    /// the one the user's own `herdr` TUI and phone use.
    pub session: Option<String>,
}

impl HerdrConfig {
    /// What `session = "default"` means: herdr's default session.
    pub const HERDR_DEFAULT: &str = "default";
}

impl Default for HerdrConfig {
    fn default() -> Self {
        HerdrConfig {
            enabled: true,
            session: None,
        }
    }
}

/// Terminal tile settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalConfig {
    /// What happens when the process in a tile's pane exits (ADR 0009).
    pub on_exit: OnExit,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub keymap: Keymap,
    pub animation: Animation,
    pub herdr: HerdrConfig,
    pub terminal: TerminalConfig,
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
        let mut herdr = HerdrConfig::default();
        let mut terminal = TerminalConfig::default();

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
                    _ => false,
                },
            );
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
            };
            (rank, d.location.map(|l| (l.line, l.column)))
        });
        Config {
            keymap,
            animation,
            herdr,
            terminal,
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
            ("dim_inactive", DeValue::Boolean(b)) => out.dim_inactive = *b,
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
            ("dim_inactive", _) => sink.error(
                value.span(),
                "`animation.dim_inactive` must be true or false",
            ),
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
            let mut config = Config::defaults(platform);
            config.path = Some(path.to_path_buf());
            config
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
