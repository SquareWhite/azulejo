//! Key chords: modifiers plus one physical key.
//!
//! Chords name the physical key, not the character it types: `ctrl+shift+minus`
//! stays `minus`, never `underscore`. A few shifted characters (`?`, `+`, …) are
//! accepted as shorthand and expand to shift plus their base key on a US layout.
//!
//! `cmd` and `super` are the same modifier: the Command key on macOS, the
//! Super (logo) key on Linux. Config written for one platform reads the same on
//! the other.

use std::fmt;

/// The platform a keymap is built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    Macos,
    Linux,
}

impl Platform {
    pub const ALL: [Platform; 2] = [Platform::Macos, Platform::Linux];

    /// The platform this binary runs on. Anything that isn't macOS uses the
    /// Linux keymap.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::Macos
        } else {
            Platform::Linux
        }
    }

    /// Section name in config files.
    pub fn name(self) -> &'static str {
        match self {
            Platform::Macos => "macos",
            Platform::Linux => "linux",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Platform::ALL.into_iter().find(|p| p.name() == name)
    }
}

/// Modifier set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Command on macOS, Super on Linux.
    pub cmd: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: false,
        cmd: false,
    };

    pub fn is_empty(self) -> bool {
        self == Mods::NONE
    }
}

/// A physical key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    /// `a`..=`z`, stored lowercase.
    Letter(char),
    /// `0`..=`9`.
    Digit(u8),
    /// `f1`..=`f24`.
    F(u8),
    Enter,
    Tab,
    Escape,
    Space,
    Backspace,
    Delete,
    Insert,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Backslash,
    Semicolon,
    Quote,
    Grave,
    Comma,
    Period,
    Slash,
}

/// Named keys: (canonical name, aliases, key).
const NAMED: &[(&str, &[&str], Key)] = &[
    ("enter", &["return"], Key::Enter),
    ("tab", &[], Key::Tab),
    ("escape", &["esc"], Key::Escape),
    ("space", &[], Key::Space),
    ("backspace", &[], Key::Backspace),
    ("delete", &["del"], Key::Delete),
    ("insert", &["ins"], Key::Insert),
    ("up", &[], Key::Up),
    ("down", &[], Key::Down),
    ("left", &[], Key::Left),
    ("right", &[], Key::Right),
    ("home", &[], Key::Home),
    ("end", &[], Key::End),
    ("pageup", &["page_up"], Key::PageUp),
    ("pagedown", &["page_down"], Key::PageDown),
    ("minus", &["-"], Key::Minus),
    ("equal", &["="], Key::Equal),
    ("bracketleft", &["["], Key::BracketLeft),
    ("bracketright", &["]"], Key::BracketRight),
    ("backslash", &["\\"], Key::Backslash),
    ("semicolon", &[";"], Key::Semicolon),
    ("quote", &["apostrophe", "'"], Key::Quote),
    ("grave", &["backtick", "`"], Key::Grave),
    ("comma", &[","], Key::Comma),
    ("period", &["."], Key::Period),
    ("slash", &["/"], Key::Slash),
];

/// Shifted characters accepted as shorthand for shift + base key (US layout).
const SHIFTED: &[(&str, Key)] = &[
    ("?", Key::Slash),
    ("plus", Key::Equal),
    ("+", Key::Equal),
    ("_", Key::Minus),
    ("{", Key::BracketLeft),
    ("}", Key::BracketRight),
    ("|", Key::Backslash),
    (":", Key::Semicolon),
    ("\"", Key::Quote),
    ("~", Key::Grave),
    ("<", Key::Comma),
    (">", Key::Period),
];

impl Key {
    /// Parses a key name. Returns the key and whether the name implies shift
    /// (`?` is shift+slash).
    pub fn parse(name: &str) -> Option<(Key, bool)> {
        let lower = name.to_ascii_lowercase();
        let mut chars = lower.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if c.is_ascii_lowercase() {
                return Some((Key::Letter(c), false));
            }
            if let Some(d) = c.to_digit(10) {
                return Some((Key::Digit(d as u8), false));
            }
        }
        if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok())
            && (1..=24).contains(&n)
        {
            return Some((Key::F(n), false));
        }
        for (canonical, aliases, key) in NAMED {
            if *canonical == lower || aliases.contains(&lower.as_str()) {
                return Some((*key, false));
            }
        }
        SHIFTED
            .iter()
            .find(|(s, _)| *s == lower)
            .map(|(_, key)| (*key, true))
    }

    pub fn name(self) -> String {
        match self {
            Key::Letter(c) => c.to_string(),
            Key::Digit(d) => d.to_string(),
            Key::F(n) => format!("f{n}"),
            other => NAMED
                .iter()
                .find(|(_, _, k)| *k == other)
                .map(|(name, _, _)| (*name).to_string())
                .unwrap_or_default(),
        }
    }
}

/// Modifiers plus a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chord {
    pub mods: Mods,
    pub key: Key,
}

impl Chord {
    pub const fn new(mods: Mods, key: Key) -> Self {
        Chord { mods, key }
    }

    /// `ctrl+b`: backward-char in shells, and the herdr attach client's prefix
    /// (`ctrl+b q` detaches).
    pub const CTRL_B: Chord = Chord {
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
            cmd: false,
        },
        key: Key::Letter('b'),
    };

    /// Parses `"ctrl+alt+h"`. See [`ChordPattern::parse`] for ranges.
    pub fn parse(text: &str) -> Result<Chord, String> {
        match ChordPattern::parse(text)? {
            ChordPattern::Single(chord) => Ok(chord),
            ChordPattern::Digits { .. } => Err(format!("`{text}` is a range, not one chord")),
        }
    }

    /// How the chord is written on `platform` (`cmd` on macOS, `super` on Linux).
    pub fn display(&self, platform: Platform) -> String {
        let mut out = String::new();
        let m = self.mods;
        for (on, name) in [
            (m.ctrl, "ctrl"),
            (m.alt, "alt"),
            (
                m.cmd,
                match platform {
                    Platform::Macos => "cmd",
                    Platform::Linux => "super",
                },
            ),
            (m.shift, "shift"),
        ] {
            if on {
                out.push_str(name);
                out.push('+');
            }
        }
        out.push_str(&self.key.name());
        out
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display(Platform::Macos))
    }
}

/// What a binding string can say: one chord, or a digit range such as
/// `cmd+1..8` for indexed actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordPattern {
    Single(Chord),
    Digits { mods: Mods, first: u8, last: u8 },
}

impl ChordPattern {
    /// Parses `mod+mod+key`, where the key may be a digit range `N..M`.
    ///
    /// Modifiers: `ctrl`/`control`, `alt`/`opt`/`option`, `shift`,
    /// `cmd`/`command`/`super`. A literal `+` key is written `plus`.
    pub fn parse(text: &str) -> Result<ChordPattern, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("empty chord".into());
        }
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let (key_part, mod_parts) = parts.split_last().expect("split yields one part");
        if key_part.is_empty() {
            return Err(format!(
                "`{text}` has no key after the last `+` (write `plus` for the + key)"
            ));
        }
        let mut mods = Mods::NONE;
        for part in mod_parts {
            let flag = match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut mods.ctrl,
                "alt" | "opt" | "option" => &mut mods.alt,
                "shift" => &mut mods.shift,
                "cmd" | "command" | "super" => &mut mods.cmd,
                "" => return Err(format!("`{text}` has an empty modifier")),
                other => return Err(format!("unknown modifier `{other}` in `{text}`")),
            };
            if *flag {
                return Err(format!("modifier `{part}` repeated in `{text}`"));
            }
            *flag = true;
        }
        if let Some((a, b)) = key_part.split_once("..") {
            let first = parse_index_digit(a)
                .ok_or_else(|| format!("range start `{a}` in `{text}` must be a digit 1-9"))?;
            let last = parse_index_digit(b)
                .ok_or_else(|| format!("range end `{b}` in `{text}` must be a digit 1-9"))?;
            if first > last {
                return Err(format!("range `{key_part}` in `{text}` runs backwards"));
            }
            return Ok(ChordPattern::Digits { mods, first, last });
        }
        let (key, shifted) =
            Key::parse(key_part).ok_or_else(|| format!("unknown key `{key_part}` in `{text}`"))?;
        if shifted {
            if mods.shift {
                return Err(format!("`{key_part}` already implies shift in `{text}`"));
            }
            mods.shift = true;
        }
        Ok(ChordPattern::Single(Chord { mods, key }))
    }

    /// Every chord the pattern covers, with the index each one carries.
    pub fn expand(self) -> Vec<(Chord, Option<u8>)> {
        match self {
            ChordPattern::Single(chord) => vec![(chord, None)],
            ChordPattern::Digits { mods, first, last } => (first..=last)
                .map(|d| (Chord::new(mods, Key::Digit(d)), Some(d)))
                .collect(),
        }
    }
}

fn parse_index_digit(s: &str) -> Option<u8> {
    match s.trim().parse::<u8>() {
        Ok(d @ 1..=9) => Some(d),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(s: &str) -> Chord {
        Chord::parse(s).unwrap()
    }

    #[test]
    fn parses_modifiers_in_any_order() {
        assert_eq!(chord("ctrl+alt+shift+h"), chord("shift+alt+ctrl+H"));
        assert_eq!(chord("cmd+t"), chord("super+t"));
        assert_eq!(chord("command+t"), chord("cmd+t"));
        assert_eq!(chord("option+o"), chord("alt+o"));
    }

    #[test]
    fn punctuation_names_and_shorthands() {
        assert_eq!(chord("alt+]").key, Key::BracketRight);
        assert_eq!(chord("ctrl+shift+minus"), chord("ctrl+shift+-"));
        assert_eq!(chord("ctrl+comma"), chord("ctrl+,"));
        let q = chord("cmd+?");
        assert_eq!(q, chord("cmd+shift+slash"));
        assert!(Chord::parse("shift+?").is_err());
        assert_eq!(chord("ctrl+plus"), chord("ctrl+shift+equal"));
    }

    #[test]
    fn rejects_bad_chords() {
        for bad in [
            "",
            "cmd+",
            "hyper+a",
            "cmd+cmd+a",
            "cmd+nope",
            "cmd++",
            "f25",
            "cmd+0..3",
            "cmd+5..2",
        ] {
            assert!(ChordPattern::parse(bad).is_err(), "{bad} should fail");
        }
    }

    #[test]
    fn ranges_expand_with_indexes() {
        let pattern = ChordPattern::parse("cmd+1..3").unwrap();
        let chords = pattern.expand();
        assert_eq!(chords.len(), 3);
        assert_eq!(chords[0], (chord("cmd+1"), Some(1)));
        assert_eq!(chords[2], (chord("cmd+3"), Some(3)));
    }

    #[test]
    fn display_round_trips_per_platform() {
        for text in ["ctrl+alt+shift+h", "alt+]", "cmd+shift+p", "ctrl+tab", "f5"] {
            let c = chord(text);
            for p in Platform::ALL {
                assert_eq!(chord(&c.display(p)), c);
            }
        }
        assert_eq!(chord("cmd+t").display(Platform::Linux), "super+t");
        assert_eq!(chord("alt+]").display(Platform::Macos), "alt+bracketright");
    }
}
