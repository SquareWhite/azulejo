//! Themes (docs/themes.md): the built-in ones from `shared/design/tokens.json`,
//! user themes from `<config dir>/themes/<id>.toml`, and the `[appearance]`
//! section that picks one.
//!
//! ```toml
//! # <config dir>/themes/dusk.toml
//! name = "Dusk"
//! inherits = "atom-one-dark"   # a built-in; omitted, the atom theme of `appearance`
//! appearance = "dark"          # "light" or "dark"; defaults to the inherited one
//! syntax = "one-dark"          # preview syntax colours
//!
//! [color]                      # colour tokens (tokens.json colorTokens), camelCase or snake_case
//! window = "#14161b"
//! text_muted = "#8a8f99"
//!
//! [terminal]
//! background = "#1d2026"
//! palette = { 1 = "#ff6c6b" }  # or a list of 16
//! ```
//!
//! Loading never fails: an unknown key or a bad colour is a warning and the
//! inherited value stays.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::Value;
use toml::de::{DeTable, DeValue};

use crate::diag::{Diagnostic, Severity, Sink, Source};
use crate::ghostty::GhosttyConfig;

/// `shared/design/tokens.json`: the built-in themes and the colour token list.
pub const TOKENS_JSON: &str = include_str!("../../../../shared/design/tokens.json");

/// The theme a config without `[appearance]` gets.
pub const DEFAULT_THEME: &str = "atom-one-dark";
/// `theme = "auto"` defaults.
pub const DEFAULT_LIGHT_THEME: &str = "atom-one-light";
pub const DEFAULT_DARK_THEME: &str = "atom-one-dark";
/// `theme = "auto"`: follow the system's light or dark appearance.
pub const AUTO: &str = "auto";

/// Preview syntax colours a theme can name (`azulejo-preview` builds them).
pub const SYNTAXES: &[&str] = &["one-dark", "one-light", "github-dark", "github-light"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

impl Appearance {
    pub fn name(self) -> &'static str {
        match self {
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }

    pub fn from_name(name: &str) -> Option<Appearance> {
        match name {
            "light" => Some(Appearance::Light),
            "dark" => Some(Appearance::Dark),
            _ => None,
        }
    }
}

/// An sRGB colour with alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba { r, g, b, a: 255 }
    }

    /// `#rgb`, `#rrggbb` or `#rrggbbaa` (the `#` is required).
    pub fn parse(text: &str) -> Option<Rgba> {
        let hex = text.trim().strip_prefix('#')?;
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        match hex.len() {
            3 => {
                let nibble = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok().map(|v| v * 17);
                Some(Rgba::rgb(nibble(0)?, nibble(1)?, nibble(2)?))
            }
            6 => Some(Rgba::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Some(Rgba {
                r: byte(0)?,
                g: byte(2)?,
                b: byte(4)?,
                a: byte(6)?,
            }),
            _ => None,
        }
    }

    /// `#rrggbb`, or `#rrggbbaa` when not opaque.
    pub fn hex(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }

    /// This colour drawn over `under` (which is treated as opaque).
    pub fn over(self, under: Rgba) -> Rgba {
        let a = f64::from(self.a) / 255.0;
        let mix = |top: u8, bottom: u8| {
            (f64::from(top) * a + f64::from(bottom) * (1.0 - a)).round() as u8
        };
        Rgba::rgb(
            mix(self.r, under.r),
            mix(self.g, under.g),
            mix(self.b, under.b),
        )
    }

    /// WCAG relative luminance (alpha ignored).
    pub fn luminance(self) -> f64 {
        let channel = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }

    /// WCAG contrast ratio between two opaque colours (1 to 21).
    pub fn contrast(self, other: Rgba) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }
}

/// A theme colour: a fixed value, or a system colour (macOS `NSColor`
/// name) that the macOS shell resolves live, so it follows the user's accent
/// colour; `hex` is how macOS draws it by default, for everything else.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorValue {
    Hex(Rgba),
    System {
        name: String,
        /// Replaces the system colour's own alpha.
        alpha: Option<f64>,
        hex: Rgba,
    },
}

impl ColorValue {
    /// The value as a plain colour (a system colour's default).
    pub fn rgba(&self) -> Rgba {
        match self {
            ColorValue::Hex(c) => *c,
            ColorValue::System { hex, .. } => *hex,
        }
    }

    /// The shape the shells read: `"#rrggbb[aa]"` or `{"system","alpha"?,"hex"}`.
    pub fn to_json(&self) -> Value {
        match self {
            ColorValue::Hex(c) => Value::String(c.hex()),
            ColorValue::System { name, alpha, hex } => {
                let mut map = serde_json::Map::new();
                map.insert("system".into(), name.clone().into());
                if let Some(a) = alpha {
                    map.insert("alpha".into(), (*a).into());
                }
                map.insert("hex".into(), hex.hex().into());
                Value::Object(map)
            }
        }
    }
}

/// A terminal palette, in Ghostty's terms.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalPalette {
    pub background: Rgba,
    pub foreground: Rgba,
    pub cursor: Option<Rgba>,
    pub cursor_text: Option<Rgba>,
    pub selection_background: Option<Rgba>,
    /// `None`: selected cells keep their own colour.
    pub selection_foreground: Option<Rgba>,
    /// ANSI 0–15.
    pub palette: [Rgba; 16],
}

impl TerminalPalette {
    /// The palette as Ghostty config lines (loaded after the user's own
    /// Ghostty config when `terminal_colors = "theme"`).
    pub fn ghostty_config(&self) -> String {
        let hex = |c: Rgba| Rgba { a: 255, ..c }.hex();
        let mut out = String::new();
        out.push_str(&format!("background = {}\n", hex(self.background)));
        out.push_str(&format!("foreground = {}\n", hex(self.foreground)));
        if let Some(c) = self.cursor {
            out.push_str(&format!("cursor-color = {}\n", hex(c)));
        }
        if let Some(c) = self.cursor_text {
            out.push_str(&format!("cursor-text = {}\n", hex(c)));
        }
        if let Some(c) = self.selection_background {
            out.push_str(&format!("selection-background = {}\n", hex(c)));
        }
        match self.selection_foreground {
            Some(c) => out.push_str(&format!("selection-foreground = {}\n", hex(c))),
            None => out.push_str("selection-foreground = cell-foreground\n"),
        }
        for (i, c) in self.palette.iter().enumerate() {
            out.push_str(&format!("palette = {i}={}\n", hex(*c)));
        }
        out
    }

    pub fn to_json(&self) -> Value {
        let opt = |c: Option<Rgba>| c.map_or(Value::Null, |c| c.hex().into());
        serde_json::json!({
            "background": self.background.hex(),
            "foreground": self.foreground.hex(),
            "cursor": opt(self.cursor),
            "cursor_text": opt(self.cursor_text),
            "selection_background": opt(self.selection_background),
            "selection_foreground": opt(self.selection_foreground),
            "palette": self.palette.iter().map(|c| c.hex()).collect::<Vec<_>>(),
            "ghostty": self.ghostty_config(),
        })
    }
}

/// Where a theme came from.
#[derive(Debug, Clone, PartialEq)]
pub enum ThemeSource {
    BuiltIn,
    User {
        path: PathBuf,
        /// The built-in it starts from.
        inherits: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub appearance: Appearance,
    /// One of [`SYNTAXES`].
    pub syntax: String,
    /// A vibrancy material for the sidebar card (macOS
    /// `NSVisualEffectView.Material` name), or none: a flat card.
    pub sidebar_material: Option<String>,
    /// Every token in [`color_tokens`].
    pub colors: BTreeMap<String, ColorValue>,
    pub terminal: TerminalPalette,
    pub source: ThemeSource,
}

impl Theme {
    pub fn is_builtin(&self) -> bool {
        self.source == ThemeSource::BuiltIn
    }

    /// A colour token's value (a system colour's default).
    pub fn rgba(&self, token: &str) -> Option<Rgba> {
        self.colors.get(token).map(ColorValue::rgba)
    }

    /// What the shells read (`azj_core_theme`).
    pub fn to_json(&self) -> Value {
        let colors: serde_json::Map<String, Value> = self
            .colors
            .iter()
            .map(|(k, v)| (k.clone(), v.to_json()))
            .collect();
        let (builtin, path, inherits) = match &self.source {
            ThemeSource::BuiltIn => (true, Value::Null, Value::Null),
            ThemeSource::User { path, inherits } => (
                false,
                path.display().to_string().into(),
                inherits.clone().into(),
            ),
        };
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "appearance": self.appearance.name(),
            "syntax": self.syntax,
            "sidebar_material": self.sidebar_material,
            "builtin": builtin,
            "path": path,
            "inherits": inherits,
            "colors": colors,
            "terminal": self.terminal.to_json(),
        })
    }
}

// MARK: Built-ins

struct BuiltIns {
    tokens: Vec<String>,
    themes: Vec<Theme>,
}

fn builtins_data() -> &'static BuiltIns {
    static DATA: OnceLock<BuiltIns> = OnceLock::new();
    DATA.get_or_init(|| parse_builtins(TOKENS_JSON).expect("shared/design/tokens.json is valid"))
}

/// The colour tokens every theme defines, in tokens.json order.
pub fn color_tokens() -> &'static [String] {
    &builtins_data().tokens
}

/// The built-in themes, in tokens.json order.
pub fn builtin_themes() -> &'static [Theme] {
    &builtins_data().themes
}

pub fn builtin(id: &str) -> Option<&'static Theme> {
    builtin_themes().iter().find(|t| t.id == id)
}

/// The built-in a user theme with no `inherits` starts from.
fn base_for(appearance: Appearance) -> &'static Theme {
    builtin(match appearance {
        Appearance::Light => DEFAULT_LIGHT_THEME,
        Appearance::Dark => DEFAULT_DARK_THEME,
    })
    .expect("the atom themes are built in")
}

/// Parses tokens.json's `colorTokens` and `themes`. Errors name the bad key;
/// `scripts/design-tokens` checks the same file, so this only fails on a
/// broken checkout.
fn parse_builtins(json: &str) -> Result<BuiltIns, String> {
    let root: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let tokens: Vec<String> = root["colorTokens"]
        .as_object()
        .ok_or("colorTokens missing")?
        .keys()
        .filter(|k| !k.starts_with('_'))
        .cloned()
        .collect();
    let mut themes = Vec::new();
    for (id, t) in root["themes"].as_object().ok_or("themes missing")? {
        if id.starts_with('_') {
            continue;
        }
        let err = |what: &str| format!("themes.{id}: {what}");
        let appearance = t["appearance"]
            .as_str()
            .and_then(Appearance::from_name)
            .ok_or_else(|| err("appearance"))?;
        let mut colors = BTreeMap::new();
        for token in &tokens {
            let v = &t["color"][token];
            colors.insert(
                token.clone(),
                json_color(v).ok_or_else(|| err(&format!("color.{token}")))?,
            );
        }
        let term = &t["terminal"];
        let color = |key: &str| term[key].as_str().and_then(Rgba::parse);
        let mut palette = [Rgba::rgb(0, 0, 0); 16];
        let list = term["palette"].as_array().ok_or_else(|| err("palette"))?;
        if list.len() != 16 {
            return Err(err("palette needs 16 colours"));
        }
        for (i, c) in list.iter().enumerate() {
            palette[i] = c
                .as_str()
                .and_then(Rgba::parse)
                .ok_or_else(|| err("palette"))?;
        }
        themes.push(Theme {
            id: id.clone(),
            name: t["name"].as_str().ok_or_else(|| err("name"))?.to_owned(),
            appearance,
            syntax: t["syntax"]
                .as_str()
                .filter(|s| SYNTAXES.contains(s))
                .ok_or_else(|| err("syntax"))?
                .to_owned(),
            sidebar_material: t["sidebarMaterial"].as_str().map(str::to_owned),
            colors,
            terminal: TerminalPalette {
                background: color("background").ok_or_else(|| err("terminal.background"))?,
                foreground: color("foreground").ok_or_else(|| err("terminal.foreground"))?,
                cursor: color("cursor"),
                cursor_text: color("cursorText"),
                selection_background: color("selectionBackground"),
                selection_foreground: color("selectionForeground"),
                palette,
            },
            source: ThemeSource::BuiltIn,
        });
    }
    Ok(BuiltIns { tokens, themes })
}

fn json_color(v: &Value) -> Option<ColorValue> {
    match v {
        Value::String(s) => Rgba::parse(s).map(ColorValue::Hex),
        Value::Object(o) => Some(ColorValue::System {
            name: o.get("system")?.as_str()?.to_owned(),
            alpha: match o.get("alpha") {
                None => None,
                Some(a) => Some(a.as_f64().filter(|a| (0.0..=1.0).contains(a))?),
            },
            hex: o.get("hex")?.as_str().and_then(Rgba::parse)?,
        }),
        _ => None,
    }
}

// MARK: The set of themes

/// Every theme a config can name: the built-ins, then user themes by id.
#[derive(Debug, Clone, PartialEq)]
pub struct Themes {
    list: Vec<Theme>,
}

impl Default for Themes {
    fn default() -> Self {
        Themes {
            list: builtin_themes().to_vec(),
        }
    }
}

impl Themes {
    /// The built-ins plus the user themes in `dir` (`<config dir>/themes`);
    /// a missing directory means none.
    pub fn load(dir: Option<&Path>, diagnostics: &mut Vec<Diagnostic>) -> Themes {
        let mut themes = Themes::default();
        let Some(dir) = dir else {
            return themes;
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return themes;
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "toml") && p.is_file())
            .collect();
        files.sort();
        for path in files {
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
                continue;
            };
            let warn = |diagnostics: &mut Vec<Diagnostic>, message: String| {
                diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    source: Source::Theme(path.clone()),
                    location: None,
                    message,
                })
            };
            if !valid_id(&id) {
                warn(
                    diagnostics,
                    format!(
                        "theme file names are the theme's id: letters, digits, `-`, `_` and `.`; \
                         `{id}` skipped"
                    ),
                );
                continue;
            }
            if id == AUTO || builtin(&id).is_some() {
                warn(
                    diagnostics,
                    format!(
                        "`{id}` is a built-in name; give the file another name \
                         (use `inherits = \"{id}\"` to start from it)"
                    ),
                );
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    if let Some(theme) = parse_user_theme(&id, &text, &path, diagnostics) {
                        themes.list.push(theme);
                    }
                }
                Err(e) => warn(diagnostics, format!("can't read: {e}")),
            }
        }
        themes
    }

    pub fn get(&self, id: &str) -> Option<&Theme> {
        self.list.iter().find(|t| t.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Theme> {
        self.list.iter()
    }

    pub fn ids(&self) -> Vec<&str> {
        self.list.iter().map(|t| t.id.as_str()).collect()
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A colour token by its tokens.json name or the snake_case spelling.
fn token_named(key: &str) -> Option<&'static str> {
    let wanted: String = key
        .chars()
        .filter(|c| *c != '_')
        .flat_map(char::to_lowercase)
        .collect();
    color_tokens()
        .iter()
        .find(|t| t.to_lowercase() == wanted)
        .map(String::as_str)
}

/// Parses one user theme. `None` only when the file isn't valid TOML (a
/// warning says so); everything else falls back to the inherited value.
pub fn parse_user_theme(
    id: &str,
    text: &str,
    path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Theme> {
    let mut sink = Sink {
        text,
        source: Source::Theme(path.to_path_buf()),
        out: diagnostics,
    };
    let table = match DeTable::parse(text) {
        Ok(table) => table,
        Err(e) => {
            sink.push(
                Severity::Warning,
                e.span(),
                format!(
                    "not valid TOML, so theme `{id}` isn't available: {}",
                    e.message().trim()
                ),
            );
            return None;
        }
    };
    let table = table.get_ref();
    let string = |key: &str| {
        table.iter().find_map(|(k, v)| {
            (k.get_ref().as_ref() == key).then(|| match v.get_ref() {
                DeValue::String(s) => Ok((s.to_string(), v.span())),
                _ => Err(v.span()),
            })
        })
    };

    // Appearance first: it picks the base when `inherits` is missing.
    let mut appearance = None;
    match string("appearance") {
        Some(Ok((name, span))) => match Appearance::from_name(&name) {
            Some(a) => appearance = Some(a),
            None => sink.warn(span, "`appearance` must be \"light\" or \"dark\""),
        },
        Some(Err(span)) => sink.warn(span, "`appearance` must be \"light\" or \"dark\""),
        None => {}
    }
    let base: &Theme = match string("inherits") {
        Some(Ok((name, span))) => match builtin(&name) {
            Some(theme) => theme,
            None => {
                let fallback = base_for(appearance.unwrap_or(Appearance::Dark));
                sink.warn(
                    span,
                    format!(
                        "`inherits` must name a built-in theme ({}); using {}",
                        quoted(builtin_themes().iter().map(|t| t.id.as_str())),
                        fallback.id
                    ),
                );
                fallback
            }
        },
        Some(Err(span)) => {
            let fallback = base_for(appearance.unwrap_or(Appearance::Dark));
            sink.warn(
                span,
                format!("`inherits` must be a theme name; using {}", fallback.id),
            );
            fallback
        }
        None => base_for(appearance.unwrap_or(Appearance::Dark)),
    };
    let mut theme = Theme {
        id: id.to_owned(),
        name: id.to_owned(),
        source: ThemeSource::User {
            path: path.to_path_buf(),
            inherits: base.id.clone(),
        },
        ..base.clone()
    };
    if let Some(a) = appearance {
        theme.appearance = a;
    }

    for (key, value) in table.iter() {
        let name = key.get_ref().as_ref();
        match (name, value.get_ref()) {
            ("appearance" | "inherits", _) => {}
            ("name", DeValue::String(s)) if !s.trim().is_empty() => theme.name = s.trim().into(),
            ("name", _) => sink.warn(value.span(), "`name` must be a non-empty string"),
            ("syntax", DeValue::String(s)) if SYNTAXES.contains(&s.as_ref()) => {
                theme.syntax = s.to_string()
            }
            ("syntax", _) => sink.warn(
                value.span(),
                format!(
                    "`syntax` must be one of {}; keeping {}",
                    quoted(SYNTAXES.iter().copied()),
                    theme.syntax
                ),
            ),
            ("color", DeValue::Table(colors)) => {
                for (k, v) in colors.iter() {
                    let key = k.get_ref().as_ref();
                    let Some(token) = token_named(key) else {
                        sink.warn(
                            k.span(),
                            format!("unknown colour token `{key}` (docs/themes.md lists them)"),
                        );
                        continue;
                    };
                    match colour(v.get_ref()) {
                        Some(c) => {
                            theme.colors.insert(token.to_owned(), ColorValue::Hex(c));
                        }
                        None => sink.warn(
                            v.span(),
                            format!(
                                "`color.{key}` isn't a colour (#rgb, #rrggbb or #rrggbbaa); \
                                 keeping {}'s",
                                base.id
                            ),
                        ),
                    }
                }
            }
            ("terminal", DeValue::Table(term)) => {
                parse_terminal(&mut sink, term, &mut theme.terminal, &base.id)
            }
            ("color" | "terminal", _) => {
                sink.warn(value.span(), format!("`{name}` must be a table"))
            }
            ("colors", _) => sink.warn(key.span(), "unknown section `colors` (it's `[color]`)"),
            (other, _) => sink.warn(key.span(), format!("unknown setting `{other}`")),
        }
    }
    Some(theme)
}

fn colour(value: &DeValue<'_>) -> Option<Rgba> {
    match value {
        DeValue::String(s) => Rgba::parse(s),
        _ => None,
    }
}

fn parse_terminal(sink: &mut Sink<'_>, table: &DeTable<'_>, out: &mut TerminalPalette, base: &str) {
    let bad = |sink: &mut Sink<'_>, span, key: &str| {
        sink.warn(
            span,
            format!("`terminal.{key}` isn't a colour (#rgb or #rrggbb); keeping {base}'s"),
        )
    };
    for (k, v) in table.iter() {
        let key = k.get_ref().as_ref();
        match key {
            "palette" => {
                parse_palette(sink, v, &mut out.palette, base);
                continue;
            }
            "background"
            | "foreground"
            | "cursor"
            | "cursor_text"
            | "selection_background"
            | "selection_foreground" => {}
            other => {
                sink.warn(k.span(), format!("unknown setting `terminal.{other}`"));
                continue;
            }
        }
        // "" clears the optional ones (selection_foreground: cells keep their colour).
        let cleared = matches!(v.get_ref(), DeValue::String(s) if s.trim().is_empty());
        let colour = colour(v.get_ref());
        match (key, colour) {
            ("cursor", _) if cleared => out.cursor = None,
            ("cursor_text", _) if cleared => out.cursor_text = None,
            ("selection_background", _) if cleared => out.selection_background = None,
            ("selection_foreground", _) if cleared => out.selection_foreground = None,
            (_, None) => bad(sink, v.span(), key),
            ("background", Some(c)) => out.background = c,
            ("foreground", Some(c)) => out.foreground = c,
            ("cursor", Some(c)) => out.cursor = Some(c),
            ("cursor_text", Some(c)) => out.cursor_text = Some(c),
            ("selection_background", Some(c)) => out.selection_background = Some(c),
            (_, Some(c)) => out.selection_foreground = Some(c),
        }
    }
}

fn parse_palette(
    sink: &mut Sink<'_>,
    value: &toml::Spanned<DeValue<'_>>,
    out: &mut [Rgba; 16],
    base: &str,
) {
    match value.get_ref() {
        DeValue::Array(items) => {
            if items.len() > 16 {
                sink.warn(
                    value.span(),
                    "`terminal.palette` has more than 16 colours; the rest are ignored",
                );
            }
            for (i, item) in items.iter().take(16).enumerate() {
                match colour(item.get_ref()) {
                    Some(c) => out[i] = c,
                    None => sink.warn(
                        item.span(),
                        format!("`terminal.palette` {i} isn't a colour; keeping {base}'s"),
                    ),
                }
            }
        }
        DeValue::Table(entries) => {
            for (k, v) in entries.iter() {
                let index = k.get_ref().parse::<usize>().ok().filter(|i| *i < 16);
                match (index, colour(v.get_ref())) {
                    (Some(i), Some(c)) => out[i] = c,
                    (None, _) => sink.warn(
                        k.span(),
                        format!("`terminal.palette` keys are 0 to 15, not `{}`", k.get_ref()),
                    ),
                    (Some(i), None) => sink.warn(
                        v.span(),
                        format!("`terminal.palette` {i} isn't a colour; keeping {base}'s"),
                    ),
                }
            }
        }
        _ => sink.warn(
            value.span(),
            "`terminal.palette` must be a list of 16 colours or a table like { 1 = \"#e06c75\" }",
        ),
    }
}

fn quoted<'a>(names: impl Iterator<Item = &'a str>) -> String {
    names
        .map(|n| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

// MARK: [appearance]

/// Which colours terminals use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalColors {
    /// The theme's terminal palette, over the user's Ghostty config.
    Theme,
    /// The user's Ghostty config alone.
    Ghostty,
}

impl TerminalColors {
    pub const NAMES: &[&str] = &["theme", "ghostty"];

    pub fn name(self) -> &'static str {
        match self {
            TerminalColors::Theme => "theme",
            TerminalColors::Ghostty => "ghostty",
        }
    }

    pub fn from_name(name: &str) -> Option<TerminalColors> {
        match name {
            "theme" => Some(TerminalColors::Theme),
            "ghostty" => Some(TerminalColors::Ghostty),
            _ => None,
        }
    }
}

/// Which icon the Dock shows while Azulejo runs. Finder, Launchpad, Spotlight
/// and the Dock before launch always show the bundle's light icon: without an
/// Xcode-compiled `Assets.car`, macOS has no dark variant to pick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AppIcon {
    /// The bundle's light icon, always.
    #[default]
    Light,
    /// The dark icon, always.
    Dark,
    /// The light or the dark icon, following the system appearance.
    Auto,
}

impl AppIcon {
    pub const NAMES: &[&str] = &["light", "dark", "auto"];

    pub fn name(self) -> &'static str {
        match self {
            AppIcon::Light => "light",
            AppIcon::Dark => "dark",
            AppIcon::Auto => "auto",
        }
    }

    pub fn from_name(name: &str) -> Option<AppIcon> {
        match name {
            "light" => Some(AppIcon::Light),
            "dark" => Some(AppIcon::Dark),
            "auto" => Some(AppIcon::Auto),
            _ => None,
        }
    }
}

/// `[appearance]` in config.toml.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppearanceConfig {
    /// A theme id, or [`AUTO`].
    pub theme: String,
    /// With `theme = "auto"`: the theme for the system's light and dark
    /// appearance.
    pub theme_light: String,
    pub theme_dark: String,
    /// `None`: "ghostty" when the user's Ghostty config sets colours, else
    /// "theme" (see [`AppearanceConfig::terminal_colors`]).
    pub terminal_colors: Option<TerminalColors>,
    /// The agent badge ("● claude · working") at the right end of terminal
    /// tile headers. Off by default: the sidebar, rail, tab strip, ⌘K and
    /// Dock show agent state, and a blocked tile keeps its own mark.
    pub tile_agent_badge: bool,
    /// The Dock icon while the app runs: light (the default), dark, or auto.
    pub app_icon: AppIcon,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        AppearanceConfig {
            theme: DEFAULT_THEME.into(),
            theme_light: DEFAULT_LIGHT_THEME.into(),
            theme_dark: DEFAULT_DARK_THEME.into(),
            terminal_colors: None,
            tile_agent_badge: false,
            app_icon: AppIcon::Light,
        }
    }
}

impl AppearanceConfig {
    pub fn is_auto(&self) -> bool {
        self.theme == AUTO
    }

    /// The theme for the system appearance `system` (only `auto` cares).
    pub fn theme_for(&self, system: Appearance) -> &str {
        match (self.is_auto(), system) {
            (false, _) => &self.theme,
            (true, Appearance::Light) => &self.theme_light,
            (true, Appearance::Dark) => &self.theme_dark,
        }
    }

    /// Whose colours terminals use. Set, the setting; unset, the user's
    /// Ghostty config wins whenever it sets `theme`, `background`,
    /// `foreground` or `palette`.
    pub fn terminal_colors(&self, ghostty: &GhosttyConfig) -> TerminalColors {
        self.terminal_colors.unwrap_or(if ghostty.sets_colors() {
            TerminalColors::Ghostty
        } else {
            TerminalColors::Theme
        })
    }
}

const APP_ICON_VALUES: &str = "`appearance.app_icon` must be \"light\", \"dark\" or \"auto\"";

pub(crate) fn parse_appearance(
    sink: &mut Sink<'_>,
    value: &toml::Spanned<DeValue<'_>>,
    out: &mut AppearanceConfig,
    spans: &mut AppearanceSpans,
) {
    let DeValue::Table(table) = value.get_ref() else {
        sink.error(value.span(), "`appearance` must be a table");
        return;
    };
    for (key, value) in table.iter() {
        let name = key.get_ref().as_ref();
        match (name, value.get_ref()) {
            ("theme" | "theme_light" | "theme_dark", DeValue::String(s)) => {
                let id = s.trim().to_owned();
                match name {
                    "theme" => {
                        out.theme = id;
                        spans.theme = Some(value.span());
                    }
                    "theme_light" => {
                        out.theme_light = id;
                        spans.theme_light = Some(value.span());
                    }
                    _ => {
                        out.theme_dark = id;
                        spans.theme_dark = Some(value.span());
                    }
                }
            }
            ("theme" | "theme_light" | "theme_dark", _) => sink.error(
                value.span(),
                format!("`appearance.{name}` must be a theme name"),
            ),
            ("terminal_colors", DeValue::String(s)) => match TerminalColors::from_name(s) {
                Some(c) => out.terminal_colors = Some(c),
                None => sink.error(
                    value.span(),
                    "`appearance.terminal_colors` must be \"theme\" or \"ghostty\"",
                ),
            },
            ("terminal_colors", _) => sink.error(
                value.span(),
                "`appearance.terminal_colors` must be \"theme\" or \"ghostty\"",
            ),
            ("tile_agent_badge", DeValue::Boolean(on)) => out.tile_agent_badge = *on,
            ("tile_agent_badge", _) => sink.error(
                value.span(),
                "`appearance.tile_agent_badge` must be true or false",
            ),
            ("app_icon", DeValue::String(s)) => match AppIcon::from_name(s) {
                Some(icon) => out.app_icon = icon,
                None => sink.error(value.span(), APP_ICON_VALUES),
            },
            ("app_icon", _) => sink.error(value.span(), APP_ICON_VALUES),
            (other, _) => sink.warn(key.span(), format!("unknown setting `appearance.{other}`")),
        }
    }
}

/// Where `[appearance]`'s theme names were, for diagnostics once the user
/// themes are loaded.
#[derive(Debug, Default)]
pub(crate) struct AppearanceSpans {
    pub theme: Option<std::ops::Range<usize>>,
    pub theme_light: Option<std::ops::Range<usize>>,
    pub theme_dark: Option<std::ops::Range<usize>>,
}

/// Checks `[appearance]`'s names against the themes there are. An unknown
/// name is an error (the config keeps its previous theme on reload; at
/// start, the default applies).
pub(crate) fn check_appearance(
    sink: &mut Sink<'_>,
    out: &mut AppearanceConfig,
    spans: &AppearanceSpans,
    themes: &Themes,
) {
    let known = || quoted(themes.iter().map(|t| t.id.as_str()));
    if out.theme != AUTO && themes.get(&out.theme).is_none() {
        let message = format!(
            "unknown theme `{}`; use \"auto\" or one of {}",
            out.theme,
            known()
        );
        match &spans.theme {
            Some(span) => sink.error(span.clone(), message),
            None => sink.push(Severity::Error, None, message),
        }
        out.theme = DEFAULT_THEME.into();
    }
    for (slot, span, wanted, default) in [
        (
            &mut out.theme_light,
            &spans.theme_light,
            Appearance::Light,
            DEFAULT_LIGHT_THEME,
        ),
        (
            &mut out.theme_dark,
            &spans.theme_dark,
            Appearance::Dark,
            DEFAULT_DARK_THEME,
        ),
    ] {
        let key = format!("theme_{}", wanted.name());
        match themes.get(slot) {
            None => {
                let message = format!("unknown theme `{slot}` for `{key}`; use one of {}", known());
                match span {
                    Some(span) => sink.error(span.clone(), message),
                    None => sink.push(Severity::Error, None, message),
                }
                *slot = default.into();
            }
            Some(theme) if theme.appearance != wanted => {
                if let Some(span) = span {
                    sink.warn(
                        span.clone(),
                        format!(
                            "`{key}` names `{}`, a {} theme",
                            theme.id,
                            theme.appearance.name()
                        ),
                    );
                }
            }
            Some(_) => {}
        }
    }
}

// MARK: Editing config.toml

/// `text` (config.toml) with `[appearance] theme` set to `theme`, touching
/// nothing else: the line is replaced, or added under `[appearance]`, or
/// the section is appended. `None` when the file sets it some other way
/// (an inline table, a dotted key) and a line edit wouldn't take.
pub fn set_theme_in(text: &str, theme: &str) -> Option<String> {
    let line_for = |indent: &str| format!("{indent}theme = \"{theme}\"");
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let header = |l: &str| {
        let t = l.trim();
        t.starts_with('[').then(|| {
            t.trim_start_matches('[')
                .split(']')
                .next()
                .unwrap_or("")
                .trim()
                .to_owned()
        })
    };
    let start = lines
        .iter()
        .position(|l| header(l).as_deref() == Some("appearance") && !l.trim().starts_with("[["));
    match start {
        Some(start) => {
            let end = lines[start + 1..]
                .iter()
                .position(|l| header(l).is_some())
                .map_or(lines.len(), |i| start + 1 + i);
            let existing = (start + 1..end).find(|&i| {
                let t = lines[i].trim_start();
                t.strip_prefix("theme")
                    .is_some_and(|rest| rest.trim_start().starts_with('='))
            });
            match existing {
                Some(i) => {
                    let indent: String =
                        lines[i].chars().take_while(|c| c.is_whitespace()).collect();
                    lines[i] = line_for(&indent);
                }
                None => lines.insert(start + 1, line_for("")),
            }
        }
        None => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[appearance]".into());
            lines.push(line_for(""));
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    // Only hand back an edit that says what we meant.
    let mut diagnostics = Vec::new();
    let mut check = AppearanceConfig::default();
    let mut sink = Sink {
        text: &out,
        source: Source::User(None),
        out: &mut diagnostics,
    };
    let table = DeTable::parse(&out).ok()?;
    let mut spans = AppearanceSpans::default();
    let mut seen = false;
    for (key, value) in table.get_ref().iter() {
        if key.get_ref().as_ref() == "appearance" {
            seen = true;
            parse_appearance(&mut sink, value, &mut check, &mut spans);
        }
    }
    (seen && check.theme == theme).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse_and_print() {
        assert_eq!(Rgba::parse("#1b1d23"), Some(Rgba::rgb(0x1b, 0x1d, 0x23)));
        assert_eq!(Rgba::parse("#fff"), Some(Rgba::rgb(255, 255, 255)));
        assert_eq!(
            Rgba::parse("#ffffff14").map(Rgba::hex).as_deref(),
            Some("#ffffff14")
        );
        assert_eq!(Rgba::parse("1b1d23"), None, "the # is required");
        assert_eq!(Rgba::parse("#12345"), None);
        assert_eq!(Rgba::parse("red"), None);
        let white = Rgba::rgb(255, 255, 255);
        assert!((white.contrast(Rgba::rgb(0, 0, 0)) - 21.0).abs() < 1e-9);
        assert_eq!(
            Rgba::parse("#00000080").unwrap().over(white),
            Rgba::rgb(127, 127, 127)
        );
    }

    #[test]
    fn set_theme_edits_one_line() {
        assert_eq!(
            set_theme_in("", "macos-dark").as_deref(),
            Some("[appearance]\ntheme = \"macos-dark\"\n")
        );
        let text = "# mine\n[herdr]\nsession = \"x\"\n";
        assert_eq!(
            set_theme_in(text, "auto").as_deref(),
            Some("# mine\n[herdr]\nsession = \"x\"\n\n[appearance]\ntheme = \"auto\"\n")
        );
        let text = "[appearance]\n  theme = \"atom-one-dark\"  # was\nterminal_colors = \"theme\"\n[layout]\ndefault = \"tile\"\n";
        assert_eq!(
            set_theme_in(text, "macos-light").as_deref(),
            Some(
                "[appearance]\n  theme = \"macos-light\"\nterminal_colors = \"theme\"\n[layout]\ndefault = \"tile\"\n"
            )
        );
        // theme_light isn't theme.
        let text = "[appearance]\ntheme_light = \"macos-light\"\n";
        assert_eq!(
            set_theme_in(text, "auto").as_deref(),
            Some("[appearance]\ntheme = \"auto\"\ntheme_light = \"macos-light\"\n")
        );
        // An inline table can't be line-edited.
        assert_eq!(
            set_theme_in("appearance = { theme = \"auto\" }\n", "macos-dark"),
            None
        );
    }
}
