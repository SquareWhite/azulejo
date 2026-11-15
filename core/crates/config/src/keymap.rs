//! The keymap: chords → actions, per platform and per context (the focused
//! tile kind, or a mode that has the keyboard), plus the optional prefix mode.
//!
//! Layers, lowest precedence first:
//! 1. bundled defaults, `[keys]` then `[keys.<platform>]`
//! 2. the user's config, `[keys]` then `[keys.<platform>]`
//!
//! A layer that names an action replaces that action's chords; `""` or
//! `"none"` unbinds it. Context sections (`[keys.terminal]`,
//! `[keys.macos.palette]`, …) replace an action's chords only in that context,
//! and win over the global bindings there. Actions that only work in one
//! context (`palette_next`, `browser_back`) go to that context wherever
//! they're written.
//!
//! `send_text` lines turn that round: the chord is the key and the value says
//! what to type (`"shift+enter" = { send_text = "\n" }`). They become
//! [`Action::SendText`] bindings in the terminal context, one entry per
//! chord, so a layer replaces a chord's text without touching the others.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use azulejo_protocol::TileKind;
use toml::Spanned;
use toml::de::{DeTable, DeValue};

use crate::action::{Action, NameSpec, SEND_TEXT, intern, names};
use crate::chord::{Chord, ChordPattern, Key, Mods, Platform};
use crate::diag::{Diagnostic, Location, Severity, Sink, Source};

/// Action names that changed, old → new. Configs using an old name keep
/// working, with a warning. `rotate_clockwise` went when `rotate_layout`
/// (`alt+o`) became the clockwise quarter turn (Nov 4, 2026).
const RENAMED: &[(&str, &str)] = &[("rotate_clockwise", "rotate_layout")];

/// Whether the shell has seen the prefix key and is waiting for the next chord.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PrefixState {
    #[default]
    Idle,
    Armed,
}

/// Where a binding applies beyond the global keymap: while a tile kind has
/// focus, or while a mode has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KeyContext {
    Terminal,
    Browser,
    Preview,
    /// Files tiles and the sidebar's Files view, while the list has the
    /// keyboard (its filter field is `Find`).
    Files,
    /// Quick open (the ⌘K menu).
    Palette,
    /// A find bar, or the files filter, while its field has the keyboard.
    Find,
    /// Resize mode (`resize_mode`): only its own bindings apply.
    Resize,
}

impl KeyContext {
    pub const ALL: [KeyContext; 7] = [
        KeyContext::Terminal,
        KeyContext::Browser,
        KeyContext::Preview,
        KeyContext::Files,
        KeyContext::Palette,
        KeyContext::Find,
        KeyContext::Resize,
    ];

    /// The section name in config files (`[keys.macos.<name>]`).
    pub fn name(self) -> &'static str {
        match self {
            KeyContext::Terminal => "terminal",
            KeyContext::Browser => "browser",
            KeyContext::Preview => "preview",
            KeyContext::Files => "files",
            KeyContext::Palette => "palette",
            KeyContext::Find => "find_bar",
            KeyContext::Resize => "resize",
        }
    }

    pub fn from_name(name: &str) -> Option<KeyContext> {
        KeyContext::ALL.into_iter().find(|c| c.name() == name)
    }

    /// The tile kind this context stands for, if it's one.
    pub fn tile(self) -> Option<TileKind> {
        match self {
            KeyContext::Terminal => Some(TileKind::Terminal),
            KeyContext::Browser => Some(TileKind::Browser),
            KeyContext::Preview => Some(TileKind::Preview),
            KeyContext::Files => Some(TileKind::Files),
            _ => None,
        }
    }

    /// "in browser tiles", for messages.
    pub fn describe(self) -> &'static str {
        match self {
            KeyContext::Terminal => "in terminal tiles",
            KeyContext::Browser => "in browser tiles",
            KeyContext::Preview => "in preview tiles",
            KeyContext::Files => "in files tiles and the sidebar's Files view",
            KeyContext::Palette => "in quick open",
            KeyContext::Find => "in find bars",
            KeyContext::Resize => "in resize mode",
        }
    }

    /// Whether global bindings apply here too, under the context's own.
    /// Resize mode swallows every chord it doesn't bind.
    pub fn inherits_global(self) -> bool {
        self != KeyContext::Resize
    }
}

impl From<TileKind> for KeyContext {
    fn from(kind: TileKind) -> Self {
        match kind {
            TileKind::Terminal => KeyContext::Terminal,
            TileKind::Browser => KeyContext::Browser,
            TileKind::Preview => KeyContext::Preview,
            TileKind::Files => KeyContext::Files,
        }
    }
}

/// What the key router knows when a chord arrives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Context {
    /// The focused tile kind or the mode with the keyboard; `None` when
    /// nothing context-specific has focus.
    pub scope: Option<KeyContext>,
    pub prefix: PrefixState,
}

impl Context {
    pub fn tile(tile: TileKind) -> Self {
        Context::scope(tile.into())
    }

    pub fn scope(scope: KeyContext) -> Self {
        Context {
            scope: Some(scope),
            prefix: PrefixState::Idle,
        }
    }
}

fn context_names() -> String {
    KeyContext::ALL
        .iter()
        .map(|c| c.name())
        .collect::<Vec<_>>()
        .join(", ")
}

/// All lookup contexts: global, then each context.
fn all_contexts() -> impl Iterator<Item = Option<KeyContext>> {
    std::iter::once(None).chain(KeyContext::ALL.into_iter().map(Some))
}

/// Where a binding was written.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Origin {
    layer: usize,
    span: Range<usize>,
}

#[derive(Debug, Clone)]
struct Entry {
    platform: Option<Platform>,
    context: Option<KeyContext>,
    name: String,
    /// Position in the action table, for deterministic tie-breaks.
    order: usize,
    bindings: Vec<(Chord, Action)>,
    origin: Origin,
}

/// Everything one file says about keys.
#[derive(Debug, Default)]
pub(crate) struct Layer {
    entries: Vec<Entry>,
    prefix_entries: Vec<Entry>,
    prefix_enabled: Option<bool>,
    prefix_key: Vec<(Option<Platform>, Chord, Origin)>,
}

/// Text and source of each layer, for turning spans into diagnostics.
pub(crate) struct LayerInfo<'a> {
    pub text: &'a str,
    pub source: Source,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

pub(crate) struct LayerParser<'s, 'a> {
    pub layer: usize,
    pub sink: &'s mut Sink<'a>,
    names: Vec<(String, NameSpec)>,
}

impl<'s, 'a> LayerParser<'s, 'a> {
    pub fn new(layer: usize, sink: &'s mut Sink<'a>) -> Self {
        LayerParser {
            layer,
            sink,
            names: names(),
        }
    }

    fn origin(&self, span: Range<usize>) -> Origin {
        Origin {
            layer: self.layer,
            span,
        }
    }

    /// Parses the `[keys]` table.
    pub fn keys(&mut self, table: &DeTable<'_>, out: &mut Layer) {
        for (key, value) in table.iter() {
            let name = key.get_ref().as_ref();
            match value.get_ref() {
                DeValue::Table(sub) => {
                    if let Some(platform) = Platform::from_name(name) {
                        self.platform_section(platform, sub, out);
                    } else if let Some(kind) = KeyContext::from_name(name) {
                        self.action_section(None, Some(kind), sub, out);
                    } else if is_chord(name) {
                        if let Some(entry) = self.text_entry(key, value, None, None) {
                            out.entries.push(entry);
                        }
                    } else {
                        self.sink.warn(
                            key.span(),
                            format!(
                                "unknown section `[keys.{name}]`; expected a platform (macos, linux) or a context ({})",
                                context_names()
                            ),
                        );
                    }
                }
                _ => {
                    if let Some(entry) = self.entry(key, value, None, None, false) {
                        out.entries.push(entry);
                    }
                }
            }
        }
    }

    fn platform_section(&mut self, platform: Platform, table: &DeTable<'_>, out: &mut Layer) {
        for (key, value) in table.iter() {
            let name = key.get_ref().as_ref();
            match value.get_ref() {
                DeValue::Table(sub) => match KeyContext::from_name(name) {
                    Some(kind) => self.action_section(Some(platform), Some(kind), sub, out),
                    None if is_chord(name) => {
                        if let Some(entry) = self.text_entry(key, value, Some(platform), None) {
                            out.entries.push(entry);
                        }
                    }
                    None => self.sink.warn(
                        key.span(),
                        format!(
                            "unknown section `[keys.{}.{name}]`; expected a context ({})",
                            platform.name(),
                            context_names()
                        ),
                    ),
                },
                _ => {
                    if let Some(entry) = self.entry(key, value, Some(platform), None, false) {
                        out.entries.push(entry);
                    }
                }
            }
        }
    }

    fn action_section(
        &mut self,
        platform: Option<Platform>,
        context: Option<KeyContext>,
        table: &DeTable<'_>,
        out: &mut Layer,
    ) {
        for (key, value) in table.iter() {
            if let DeValue::Table(_) = value.get_ref() {
                if is_chord(key.get_ref()) {
                    if let Some(entry) = self.text_entry(key, value, platform, context) {
                        out.entries.push(entry);
                    }
                    continue;
                }
                self.sink.warn(
                    key.span(),
                    format!("unexpected table `{}` here", key.get_ref()),
                );
                continue;
            }
            if let Some(entry) = self.entry(key, value, platform, context, false) {
                out.entries.push(entry);
            }
        }
    }

    /// Parses the `[prefix]` table.
    pub fn prefix(&mut self, table: &DeTable<'_>, out: &mut Layer) {
        for (key, value) in table.iter() {
            match key.get_ref().as_ref() {
                "enabled" => match value.get_ref() {
                    DeValue::Boolean(b) => out.prefix_enabled = Some(*b),
                    _ => self
                        .sink
                        .error(value.span(), "`prefix.enabled` must be true or false"),
                },
                "key" => match value.get_ref() {
                    DeValue::String(s) => self.prefix_key(None, s, value.span(), out),
                    DeValue::Table(per_platform) => {
                        for (pkey, pvalue) in per_platform.iter() {
                            let Some(platform) = Platform::from_name(pkey.get_ref()) else {
                                self.sink.warn(
                                    pkey.span(),
                                    format!("unknown platform `{}`", pkey.get_ref()),
                                );
                                continue;
                            };
                            match pvalue.get_ref() {
                                DeValue::String(s) => {
                                    self.prefix_key(Some(platform), s, pvalue.span(), out)
                                }
                                _ => self
                                    .sink
                                    .error(pvalue.span(), "the prefix key must be a chord string"),
                            }
                        }
                    }
                    _ => self.sink.error(
                        value.span(),
                        "`prefix.key` must be a chord, or a table of chords per platform",
                    ),
                },
                "keys" => match value.get_ref() {
                    DeValue::Table(keys) => self.prefix_keys(keys, out),
                    _ => self
                        .sink
                        .error(value.span(), "`prefix.keys` must be a table"),
                },
                other => self
                    .sink
                    .warn(key.span(), format!("unknown setting `prefix.{other}`")),
            }
        }
    }

    fn prefix_key(
        &mut self,
        platform: Option<Platform>,
        text: &str,
        span: Range<usize>,
        out: &mut Layer,
    ) {
        match Chord::parse(text) {
            Ok(chord) if chord == Chord::CTRL_B => self.sink.error(
                span,
                "the prefix can't be ctrl+b: shells need it, and the herdr attach client treats ctrl+b q as detach",
            ),
            Ok(chord) => {
                let origin = self.origin(span);
                out.prefix_key.push((platform, chord, origin));
            }
            Err(message) => self.sink.error(span, message),
        }
    }

    fn prefix_keys(&mut self, table: &DeTable<'_>, out: &mut Layer) {
        for (key, value) in table.iter() {
            match value.get_ref() {
                DeValue::Table(sub) => {
                    let Some(platform) = Platform::from_name(key.get_ref()) else {
                        self.sink.warn(
                            key.span(),
                            format!(
                                "unknown section `[prefix.keys.{}]`; expected macos or linux",
                                key.get_ref()
                            ),
                        );
                        continue;
                    };
                    for (k, v) in sub.iter() {
                        if let Some(entry) = self.entry(k, v, Some(platform), None, true) {
                            out.prefix_entries.push(entry);
                        }
                    }
                }
                _ => {
                    if let Some(entry) = self.entry(key, value, None, None, true) {
                        out.prefix_entries.push(entry);
                    }
                }
            }
        }
    }

    /// One `action = "chord"` line. `None` when it's rejected, so the layer
    /// below keeps its binding.
    fn entry(
        &mut self,
        key: &Spanned<std::borrow::Cow<'_, str>>,
        value: &Spanned<DeValue<'_>>,
        platform: Option<Platform>,
        context: Option<KeyContext>,
        prefix: bool,
    ) -> Option<Entry> {
        let mut name = key.get_ref().as_ref();
        if let Some((old, new)) = RENAMED.iter().find(|(old, _)| *old == name) {
            self.sink.warn(
                key.span(),
                format!("`{old}` is now `{new}`; it still works, but rename it"),
            );
            name = new;
        }
        let Some(order) = self.names.iter().position(|(n, _)| n == name) else {
            if !prefix && is_chord(name) {
                self.sink.error(
                    value.span(),
                    format!(
                        "to type text on `{name}`, write `\"{name}\" = {{ {SEND_TEXT} = \"…\" }}`"
                    ),
                );
            } else {
                self.sink
                    .warn(key.span(), format!("unknown action `{name}`"));
            }
            return None;
        };
        // Actions that only work in one context go there from a global
        // section; anywhere else they'd never fire.
        let home = match &self.names[order].1 {
            NameSpec::Plain(action) => action.context(),
            NameSpec::Indexed(_) => None,
        };
        let context = match (home, context) {
            (Some(home), _) if prefix => {
                self.sink.warn(
                    key.span(),
                    format!(
                        "`{name}` only works {}, not after the prefix; ignored",
                        home.describe()
                    ),
                );
                return None;
            }
            (Some(home), None) => Some(home),
            (Some(home), Some(other)) if home != other => {
                self.sink.warn(
                    key.span(),
                    format!(
                        "`{name}` only works {}, so it does nothing under `{}`; ignored",
                        home.describe(),
                        other.name(),
                    ),
                );
                return None;
            }
            (_, context) => context,
        };

        let texts: Vec<&str> = match value.get_ref() {
            DeValue::String(s) if is_unbound(s) => Vec::new(),
            DeValue::String(s) => vec![s.as_ref()],
            DeValue::Array(items) => {
                let mut texts = Vec::new();
                for item in items.iter() {
                    match item.get_ref() {
                        DeValue::String(s) if !is_unbound(s) => texts.push(s.as_ref()),
                        _ => {
                            self.sink.error(
                                item.span(),
                                format!(
                                    "`{name}`: each chord in the list must be a non-empty string"
                                ),
                            );
                            return None;
                        }
                    }
                }
                texts
            }
            _ => {
                self.sink.error(
                    value.span(),
                    format!("`{name}` must be a chord string, a list of chords, or \"\" to unbind"),
                );
                return None;
            }
        };

        let mut bindings = Vec::new();
        for text in texts {
            let text = if prefix {
                strip_prefix_token(text)
            } else {
                text
            };
            let pattern = match ChordPattern::parse(text) {
                Ok(p) => p,
                Err(message) => {
                    self.sink
                        .error(value.span(), format!("`{name}`: {message}"));
                    return None;
                }
            };
            for (chord, index) in pattern.expand() {
                let action = match &self.names[order].1 {
                    NameSpec::Plain(action) => {
                        if index.is_some() {
                            self.sink.error(
                                value.span(),
                                format!(
                                    "`{name}` takes no index; digit ranges like `1..8` are for switch_workspace and switch_tab"
                                ),
                            );
                            return None;
                        }
                        *action
                    }
                    NameSpec::Indexed(make) => {
                        let index = index.or(match chord.key {
                            Key::Digit(d @ 1..=9) => Some(d),
                            _ => None,
                        });
                        let Some(index) = index else {
                            self.sink.error(
                                value.span(),
                                format!("`{name}` needs a digit key or range, like `cmd+1..8`"),
                            );
                            return None;
                        };
                        make(index)
                    }
                };
                bindings.push((chord, action));
            }
        }

        Some(Entry {
            platform,
            context,
            name: name.to_string(),
            order,
            bindings,
            origin: self.origin(key.span().start..value.span().end),
        })
    }
}

impl LayerParser<'_, '_> {
    /// One `"chord" = { send_text = "…" }` line: typing text into terminal
    /// tiles on a chord, like kitty's `map … send_text` and Ghostty's
    /// `keybind = …=text:…`. It lives in the terminal context wherever it's
    /// written ([keys], [keys.<platform>] or their `terminal` sections), and
    /// replaces the layer below's text on the same chord. An empty text
    /// unbinds it, so the chord reaches the terminal as a key again.
    fn text_entry(
        &mut self,
        key: &Spanned<std::borrow::Cow<'_, str>>,
        value: &Spanned<DeValue<'_>>,
        platform: Option<Platform>,
        context: Option<KeyContext>,
    ) -> Option<Entry> {
        let written = key.get_ref().as_ref();
        let chord = match Chord::parse(written) {
            Ok(chord) => chord,
            Err(message) => {
                self.sink
                    .error(key.span(), format!("`{written}`: {message}"));
                return None;
            }
        };
        if let Some(other) = context.filter(|c| *c != KeyContext::Terminal) {
            self.sink.warn(
                key.span(),
                format!(
                    "`{SEND_TEXT}` only works {}, so `{written}` does nothing under `{}`; ignored",
                    KeyContext::Terminal.describe(),
                    other.name()
                ),
            );
            return None;
        }
        let DeValue::Table(table) = value.get_ref() else {
            self.sink.error(
                value.span(),
                format!("`{written}` must be `{{ {SEND_TEXT} = \"…\" }}`"),
            );
            return None;
        };
        let mut text = None;
        for (k, v) in table.iter() {
            match (k.get_ref().as_ref(), v.get_ref()) {
                (SEND_TEXT, DeValue::String(s)) => text = Some(s.to_string()),
                (SEND_TEXT, _) => {
                    self.sink.error(
                        v.span(),
                        format!("`{written}`: `{SEND_TEXT}` must be a string (\"\" unbinds it)"),
                    );
                    return None;
                }
                (other, _) => self.sink.warn(
                    k.span(),
                    format!("`{written}`: unknown setting `{other}`; expected `{SEND_TEXT}`"),
                ),
            }
        }
        let Some(text) = text else {
            self.sink.error(
                value.span(),
                format!("`{written}` needs `{SEND_TEXT} = \"…\"`"),
            );
            return None;
        };
        let bindings = if text.is_empty() {
            Vec::new()
        } else {
            vec![(chord, Action::SendText(intern(&text)))]
        };
        Some(Entry {
            platform,
            context: Some(KeyContext::Terminal),
            name: text_entry_name(chord),
            // After every named action: one of those wins a tie in the same
            // layer and context.
            order: self.names.len(),
            bindings,
            origin: self.origin(key.span().start..value.span().end),
        })
    }
}

/// Whether a config key names a chord (`"shift+enter"`) rather than an action.
fn is_chord(name: &str) -> bool {
    Chord::parse(name).is_ok()
}

/// The merge key of a `send_text` line: one per chord, so a later layer's
/// line replaces only that chord's text.
fn text_entry_name(chord: Chord) -> String {
    format!("{SEND_TEXT} {chord}")
}

fn is_unbound(s: &str) -> bool {
    let s = s.trim();
    s.is_empty() || s.eq_ignore_ascii_case("none")
}

/// `prefix+h` → `h`. The token is optional inside `[prefix.keys]`.
fn strip_prefix_token(text: &str) -> &str {
    let trimmed = text.trim();
    match trimmed.get(..7) {
        Some(head) if head.eq_ignore_ascii_case("prefix+") => &trimmed[7..],
        _ => trimmed,
    }
}

// ---------------------------------------------------------------------------
// Building
// ---------------------------------------------------------------------------

/// One chord's winner while assembling a map.
#[derive(Debug, Clone)]
struct Slot {
    action: Action,
    origin: Origin,
    context_specific: bool,
    order: usize,
}

impl Slot {
    /// Context bindings beat global ones, the user beats the defaults, and the
    /// action listed first wins a tie.
    fn beats(&self, other: &Slot) -> bool {
        (
            self.context_specific,
            self.origin.layer,
            std::cmp::Reverse(self.order),
        ) > (
            other.context_specific,
            other.origin.layer,
            std::cmp::Reverse(other.order),
        )
    }
}

#[derive(Debug, Clone)]
struct Duplicate {
    chord: Chord,
    winner: Slot,
    loser: Slot,
}

fn assemble<'e>(
    entries: impl Iterator<Item = (&'e Entry, bool)>,
) -> (HashMap<Chord, Slot>, Vec<Duplicate>) {
    let mut map: HashMap<Chord, Slot> = HashMap::new();
    let mut dups = Vec::new();
    let mut entries: Vec<_> = entries.collect();
    entries.sort_by_key(|(e, _)| e.order);
    for (entry, context_specific) in entries {
        for (chord, action) in &entry.bindings {
            let slot = Slot {
                action: *action,
                origin: entry.origin.clone(),
                context_specific,
                order: entry.order,
            };
            match map.get(chord) {
                None => {
                    map.insert(*chord, slot);
                }
                Some(existing) if existing.action == slot.action => {}
                Some(existing) => {
                    let existing = existing.clone();
                    let (winner, loser) = if slot.beats(&existing) {
                        (slot, existing)
                    } else {
                        (existing, slot)
                    };
                    map.insert(*chord, winner.clone());
                    dups.push(Duplicate {
                        chord: *chord,
                        winner,
                        loser,
                    });
                }
            }
        }
    }
    (map, dups)
}

/// Merges layers for `platform`: later layers and platform sections replace
/// earlier bindings for the same (context, action).
fn merge(
    platform: Platform,
    layers: &[Layer],
    pick: impl Fn(&Layer) -> &Vec<Entry>,
) -> HashMap<(Option<KeyContext>, String), Entry> {
    let mut merged = HashMap::new();
    for layer in layers {
        for wanted in [None, Some(platform)] {
            for entry in pick(layer).iter().filter(|e| e.platform == wanted) {
                merged.insert((entry.context, entry.name.clone()), entry.clone());
            }
        }
    }
    merged
}

/// Chords the OS or desktop takes before the app sees them.
fn reserved(platform: Platform) -> Vec<(Chord, &'static str)> {
    let list: &[(&str, &str)] = match platform {
        Platform::Macos => &[
            ("cmd+tab", "the app switcher"),
            ("cmd+grave", "window cycling"),
            ("cmd+space", "Spotlight"),
            ("cmd+alt+escape", "Force Quit"),
            ("ctrl+cmd+q", "Lock Screen"),
            ("cmd+shift+3", "screenshots"),
            ("cmd+shift+4", "screenshots"),
            ("cmd+shift+5", "screenshots"),
        ],
        Platform::Linux => &[
            ("alt+tab", "the window switcher"),
            ("alt+f4", "Close Window"),
            ("super+l", "Lock Screen"),
            ("super+tab", "the app switcher"),
            ("ctrl+alt+delete", "Log Out"),
            ("ctrl+alt+t", "Open Terminal on Ubuntu"),
            ("ctrl+alt+l", "Lock Screen on Ubuntu and KDE"),
        ],
    };
    list.iter()
        .map(|(text, what)| (Chord::parse(text).expect("reserved chords parse"), *what))
        .collect()
}

/// What the config says about one action in one context, after layering:
/// for listings (`config.keys`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// `None` for the global bindings.
    pub context: Option<KeyContext>,
    /// The action's config name (`switch_tab`, not `switch_tab_3`).
    pub name: String,
    /// The chords as written, with the action each one stands for (indexed
    /// actions get one per digit). Empty when unbound.
    pub bindings: Vec<(Chord, Action)>,
    /// The user's config wrote it (otherwise the bundled defaults).
    pub user: bool,
}

impl Written {
    /// The chord of a `send_text` line (`"cmd+l" = { send_text = … }`),
    /// even when it's unbound; `None` for action lines.
    pub fn send_text_chord(&self) -> Option<Chord> {
        let rest = self.name.strip_prefix(SEND_TEXT)?.strip_prefix(' ')?;
        Chord::parse(rest).ok()
    }

    /// What a `send_text` line types; `None` when unbound or not one.
    pub fn send_text(&self) -> Option<&'static str> {
        self.bindings.first().and_then(|(_, action)| action.text())
    }
}

/// The resolved keymap for one platform.
#[derive(Debug, Clone)]
pub struct Keymap {
    platform: Platform,
    maps: HashMap<Option<KeyContext>, HashMap<Chord, Action>>,
    prefix_enabled: bool,
    prefix_key: Option<Chord>,
    prefix_map: HashMap<Chord, Action>,
    written: Vec<Written>,
    prefix_written: Vec<Written>,
}

fn written(merged: &HashMap<(Option<KeyContext>, String), Entry>) -> Vec<Written> {
    let mut out: Vec<(usize, Written)> = merged
        .iter()
        .map(|((context, name), e)| {
            (
                e.order,
                Written {
                    context: *context,
                    name: name.clone(),
                    bindings: e.bindings.clone(),
                    user: e.origin.layer > 0,
                },
            )
        })
        .collect();
    // `send_text` lines share one order; their names (one per chord) keep
    // them in a stable order.
    out.sort_by(|(a, x), (b, y)| (a, x.context, &x.name).cmp(&(b, y.context, &y.name)));
    out.into_iter().map(|(_, w)| w).collect()
}

impl Keymap {
    pub(crate) fn build(
        platform: Platform,
        layers: &[Layer],
        infos: &[LayerInfo<'_>],
        diags: &mut Vec<Diagnostic>,
    ) -> Keymap {
        let diag = |diags: &mut Vec<Diagnostic>, severity, origin: &Origin, message: String| {
            let info = &infos[origin.layer];
            diags.push(Diagnostic {
                severity,
                source: info.source.clone(),
                location: Some(Location::of_offset(info.text, origin.span.start)),
                message,
            });
        };
        let show = |chord: &Chord| chord.display(platform);

        // Direct bindings, per context.
        let merged = merge(platform, layers, |l| &l.entries);
        let mut maps = HashMap::new();
        let mut slot_maps: Vec<(Option<KeyContext>, HashMap<Chord, Slot>)> = Vec::new();
        let mut reported: HashSet<(Chord, Action, Action)> = HashSet::new();
        for context in all_contexts() {
            // Global entries, with this context's entries replacing them per
            // action. Resize mode only has its own.
            let inherit = context.is_none_or(KeyContext::inherits_global);
            let mut effective: HashMap<&str, (&Entry, bool)> = merged
                .iter()
                .filter(|((ctx, _), _)| inherit && ctx.is_none())
                .map(|((_, name), e)| (name.as_str(), (e, false)))
                .collect();
            if context.is_some() {
                for ((ctx, name), e) in &merged {
                    if *ctx == context {
                        effective.insert(name.as_str(), (e, true));
                    }
                }
            }
            let (slots, dups) = assemble(effective.into_values());
            for dup in dups {
                // Global clashes show up in every context; report them once.
                let relevant =
                    context.is_none() || dup.winner.context_specific || dup.loser.context_specific;
                if relevant && reported.insert((dup.chord, dup.winner.action, dup.loser.action)) {
                    let at = if dup.winner.origin.layer >= dup.loser.origin.layer {
                        &dup.winner.origin
                    } else {
                        &dup.loser.origin
                    };
                    diag(
                        diags,
                        Severity::Warning,
                        at,
                        format!(
                            "`{}` is bound to both `{}` and `{}`{}; `{}` wins. Unbind one of them to silence this.",
                            show(&dup.chord),
                            dup.winner.action.name(),
                            dup.loser.action.name(),
                            context
                                .map(|k| format!(" {}", k.describe()))
                                .unwrap_or_default(),
                            dup.winner.action.name(),
                        ),
                    );
                }
            }
            maps.insert(
                context,
                slots
                    .iter()
                    .map(|(c, s)| (*c, s.action))
                    .collect::<HashMap<_, _>>(),
            );
            slot_maps.push((context, slots));
        }

        // Prefix mode.
        let mut prefix_enabled = false;
        let mut prefix_key: Option<(Chord, Origin)> = None;
        for layer in layers {
            if let Some(enabled) = layer.prefix_enabled {
                prefix_enabled = enabled;
            }
            for wanted in [None, Some(platform)] {
                for (p, chord, origin) in &layer.prefix_key {
                    if *p == wanted {
                        prefix_key = Some((*chord, origin.clone()));
                    }
                }
            }
        }
        let merged_prefix = merge(platform, layers, |l| &l.prefix_entries);
        let (prefix_slots, prefix_dups) = assemble(merged_prefix.values().map(|e| (e, false)));
        for dup in prefix_dups {
            diag(
                diags,
                Severity::Warning,
                &dup.winner.origin,
                format!(
                    "prefix `{}` is bound to both `{}` and `{}`; `{}` wins",
                    show(&dup.chord),
                    dup.winner.action.name(),
                    dup.loser.action.name(),
                    dup.winner.action.name(),
                ),
            );
        }

        // Conflicts. Report each written binding once, wherever it applies.
        let mut seen: HashSet<(Chord, Origin)> = HashSet::new();
        let reserved = reserved(platform);
        for (context, slots) in &slot_maps {
            let mut chords: Vec<_> = slots.iter().collect();
            chords.sort_by_key(|(c, _)| **c);
            for (chord, slot) in chords {
                let terminal = *context == Some(KeyContext::Terminal)
                    // Typing ctrl+b literally is what that one is for.
                    && slot.action != Action::SendHerdrPrefix;
                let first_time = !seen.contains(&(*chord, slot.origin.clone()));
                if terminal && *chord == Chord::CTRL_B {
                    diag(
                        diags,
                        Severity::Warning,
                        &slot.origin,
                        format!(
                            "`{}` takes ctrl+b away from terminal tiles: shells use it (backward-char) and herdr's attach client needs it",
                            slot.action.name()
                        ),
                    );
                } else if terminal
                    // Typing something on a ctrl chord is what that line asks for.
                    && !matches!(slot.action, Action::SendText(_))
                    && chord.mods
                        == (Mods {
                            ctrl: true,
                            ..Mods::NONE
                        })
                    && matches!(chord.key, Key::Letter(_))
                {
                    diag(
                        diags,
                        Severity::Warning,
                        &slot.origin,
                        format!(
                            "`{}` takes `{}` away from terminal tiles, where shells and TUIs use it",
                            slot.action.name(),
                            show(chord)
                        ),
                    );
                }
                if first_time {
                    if let Some((_, what)) = reserved.iter().find(|(r, _)| r == chord) {
                        diag(
                            diags,
                            Severity::Warning,
                            &slot.origin,
                            format!(
                                "`{}` is taken by {what} on {}, so `{}` may never fire",
                                show(chord),
                                platform.name(),
                                slot.action.name()
                            ),
                        );
                    }
                    seen.insert((*chord, slot.origin.clone()));
                }
            }
        }
        if prefix_enabled && let Some((key, origin)) = &prefix_key {
            if let Some((_, what)) = reserved.iter().find(|(r, _)| r == key) {
                diag(
                    diags,
                    Severity::Warning,
                    origin,
                    format!(
                        "prefix `{}` is taken by {what} on {}",
                        show(key),
                        platform.name()
                    ),
                );
            }
            // The prefix wins over a direct binding of the same chord. The
            // defaults rely on that on Linux (toggle_sidebar moves to prefix+b),
            // so only the user's own clashes are worth a warning.
            let mut clashes: HashSet<Origin> = HashSet::new();
            for (context, slots) in &slot_maps {
                if let Some(slot) = slots.get(key)
                    && slot.origin.layer > 0
                    && clashes.insert(slot.origin.clone())
                {
                    diag(
                        diags,
                        Severity::Warning,
                        &slot.origin,
                        format!(
                            "`{}` is the prefix key, so `{}`{} never fires while prefix mode is on",
                            show(key),
                            slot.action.name(),
                            context
                                .map(|k| format!(" {}", k.describe()))
                                .unwrap_or_default(),
                        ),
                    );
                }
            }
        }

        Keymap {
            platform,
            maps,
            prefix_enabled,
            prefix_key: prefix_key.map(|(k, _)| k),
            prefix_map: prefix_slots.iter().map(|(c, s)| (*c, s.action)).collect(),
            written: written(&merged),
            prefix_written: written(&merged_prefix),
        }
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// Whether prefix mode is on.
    pub fn prefix_enabled(&self) -> bool {
        self.prefix_enabled
    }

    /// The prefix chord, when prefix mode is on.
    pub fn prefix_key(&self) -> Option<Chord> {
        self.prefix_key.filter(|_| self.prefix_enabled)
    }

    /// The configured prefix chord, whether or not prefix mode is on.
    pub fn prefix_chord(&self) -> Option<Chord> {
        self.prefix_key
    }

    /// The action for `chord`, or `None` to let the key through to the tile.
    ///
    /// While the prefix is armed only the prefix keymap applies; otherwise the
    /// prefix key (when enabled) arms it, and the focused tile kind's bindings
    /// apply.
    pub fn resolve(&self, chord: &Chord, context: &Context) -> Option<Action> {
        match context.prefix {
            PrefixState::Armed => self.prefix_map.get(chord).copied(),
            PrefixState::Idle => {
                if self.prefix_key() == Some(*chord) {
                    return Some(Action::EnterPrefix);
                }
                self.maps.get(&context.scope)?.get(chord).copied()
            }
        }
    }

    /// Direct chords for `action` in `context`, sorted. For menus and the
    /// palette.
    pub fn chords_for(&self, action: Action, context: Option<KeyContext>) -> Vec<Chord> {
        let mut chords: Vec<Chord> = self
            .maps
            .get(&context)
            .into_iter()
            .flatten()
            .filter(|(_, a)| **a == action)
            .map(|(c, _)| *c)
            .collect();
        chords.sort();
        chords
    }

    /// Chords that follow the prefix for `action`, sorted.
    pub fn prefix_chords_for(&self, action: Action) -> Vec<Chord> {
        let mut chords: Vec<Chord> = self
            .prefix_map
            .iter()
            .filter(|(_, a)| **a == action)
            .map(|(c, _)| *c)
            .collect();
        chords.sort();
        chords
    }

    /// Every direct binding that applies in `context`.
    pub fn bindings(
        &self,
        context: Option<KeyContext>,
    ) -> impl Iterator<Item = (Chord, Action)> + '_ {
        self.maps
            .get(&context)
            .into_iter()
            .flatten()
            .map(|(c, a)| (*c, *a))
    }

    /// Every prefix binding (whether or not prefix mode is on).
    pub fn prefix_bindings(&self) -> impl Iterator<Item = (Chord, Action)> + '_ {
        self.prefix_map.iter().map(|(c, a)| (*c, *a))
    }

    /// What the config files say per action and context, in action order
    /// (before conflicts are settled): for `config.keys`.
    pub fn written(&self) -> &[Written] {
        &self.written
    }

    /// The same for `[prefix.keys]`.
    pub fn prefix_written(&self) -> &[Written] {
        &self.prefix_written
    }
}

/// What the router did with a chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Run this action; the chord is consumed.
    Action(Action),
    /// The prefix key arrived; the next chord goes to the prefix keymap.
    PrefixArmed,
    /// The chord after the prefix matched nothing (or was escape); it's consumed.
    PrefixCancelled,
    /// Not a binding; pass the chord to the focused tile.
    Unhandled,
}

/// Tracks prefix state between chords. One per window.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyRouter {
    state: PrefixState,
}

impl KeyRouter {
    pub fn state(&self) -> PrefixState {
        self.state
    }

    /// Drops an armed prefix (focus moved, window lost key status, …).
    pub fn reset(&mut self) {
        self.state = PrefixState::Idle;
    }

    /// Routes `chord` in `scope` (the focused tile kind or the mode that has
    /// the keyboard).
    pub fn handle(
        &mut self,
        keymap: &Keymap,
        chord: Chord,
        scope: Option<KeyContext>,
    ) -> KeyOutcome {
        let context = Context {
            scope,
            prefix: self.state,
        };
        match self.state {
            PrefixState::Armed => {
                self.state = PrefixState::Idle;
                if chord.key == Key::Escape && chord.mods.is_empty() {
                    return KeyOutcome::PrefixCancelled;
                }
                match keymap.resolve(&chord, &context) {
                    Some(action) => KeyOutcome::Action(action),
                    None => KeyOutcome::PrefixCancelled,
                }
            }
            PrefixState::Idle => match keymap.resolve(&chord, &context) {
                Some(Action::EnterPrefix) => {
                    self.state = PrefixState::Armed;
                    KeyOutcome::PrefixArmed
                }
                Some(action) => KeyOutcome::Action(action),
                None => KeyOutcome::Unhandled,
            },
        }
    }
}
