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
//! ```

mod action;
mod chord;
mod diag;
pub mod ghostty;
mod keymap;

use std::path::{Path, PathBuf};

use toml::de::{DeTable, DeValue};

pub use action::{Action, PLANNED_METHODS, Target};
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
}

impl Animation {
    pub const MAX_DURATION_MS: u32 = 1000;
}

impl Default for Animation {
    fn default() -> Self {
        Animation {
            enabled: true,
            duration_ms: 200,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub keymap: Keymap,
    pub animation: Animation,
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
    for (key, value) in table.iter() {
        match (key.get_ref().as_ref(), value.get_ref()) {
            ("enabled", DeValue::Boolean(b)) => out.enabled = *b,
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
            ("enabled", _) => sink.error(value.span(), "`animation.enabled` must be true or false"),
            ("duration_ms", _) => sink.error(
                value.span(),
                "`animation.duration_ms` must be a whole number",
            ),
            (other, _) => sink.warn(key.span(), format!("unknown setting `animation.{other}`")),
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
