//! The small part of the user's Ghostty config Azulejo reads itself: fonts
//! and colours for the Linux VTE tile (step 3.2), and how unfocused splits
//! dim, which both shells use for unfocused tiles. Read-only. On macOS
//! libghostty reads the full config for the terminals themselves.
//!
//! Format: `key = value` lines, `#` comments, repeated keys allowed. An empty
//! value resets a key to its default. `config-file = path` includes another
//! file (`?path` if it may be missing), resolved relative to the including
//! file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::chord::Platform;
use crate::diag::{Diagnostic, Location, Severity, Source};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    /// `#rrggbb`, `rrggbb`, `#rgb` or `rgb`.
    pub fn parse(text: &str) -> Option<Rgb> {
        let hex = text.trim().trim_start_matches('#');
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |s: &str| u8::from_str_radix(s, 16).ok();
        match hex.len() {
            6 => Some(Rgb {
                r: channel(&hex[0..2])?,
                g: channel(&hex[2..4])?,
                b: channel(&hex[4..6])?,
            }),
            3 => {
                let double = |i: usize| channel(&hex[i..=i]).map(|v| v * 17);
                Some(Rgb {
                    r: double(0)?,
                    g: double(1)?,
                    b: double(2)?,
                })
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GhosttyConfig {
    /// In priority order; empty means Ghostty's default.
    pub font_family: Vec<String>,
    /// Points.
    pub font_size: Option<f32>,
    /// Theme name as written, e.g. `catppuccin-mocha` or
    /// `light:Rose Pine Dawn,dark:Rose Pine`. Theme files aren't loaded.
    pub theme: Option<String>,
    pub background: Option<Rgb>,
    pub foreground: Option<Rgb>,
    pub cursor_color: Option<Rgb>,
    pub selection_background: Option<Rgb>,
    pub selection_foreground: Option<Rgb>,
    /// ANSI palette overrides, index 0–255.
    pub palette: BTreeMap<u8, Rgb>,
    /// `unfocused-split-opacity`, clamped to 0.15–1 as Ghostty does; `None`
    /// when the config doesn't set it. 1 turns dimming off.
    pub unfocused_split_opacity: Option<f64>,
    /// `unfocused-split-fill`: the colour unfocused splits fade towards.
    /// Hex only; Ghostty's X11 colour names are ignored with a warning.
    pub unfocused_split_fill: Option<Rgb>,
    /// Files read, in order.
    pub files: Vec<PathBuf>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Ghostty's default config files that exist, in the order Ghostty loads them
/// (later files override earlier ones): the XDG file, then on macOS the
/// Application Support file. In each place `config.ghostty` is preferred over
/// the legacy `config`.
pub fn default_paths(platform: Platform) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(xdg) = crate::xdg_config_home() {
        dirs.push(xdg.join("ghostty"));
    }
    if platform == Platform::Macos
        && let Some(home) = crate::home_dir()
    {
        dirs.push(home.join("Library/Application Support/com.mitchellh.ghostty"));
    }
    dirs.into_iter()
        .filter_map(|dir| {
            ["config.ghostty", "config"]
                .into_iter()
                .map(|name| dir.join(name))
                .find(|p| p.is_file())
        })
        .collect()
}

/// Reads the user's Ghostty config from its default locations.
pub fn load(platform: Platform) -> GhosttyConfig {
    load_from(&default_paths(platform))
}

/// Reads `paths` in order, as Ghostty reads its default files. A path that
/// doesn't exist is skipped quietly (a dev profile's optional config).
pub fn load_from(paths: &[PathBuf]) -> GhosttyConfig {
    let mut config = GhosttyConfig::default();
    for path in paths {
        config.read_file(path, true, 0);
    }
    config
}

/// Ghostty's lower bound for `unfocused-split-opacity`: 0 would hide a split
/// altogether, so it clamps to 0.15.
pub const UNFOCUSED_SPLIT_OPACITY_MIN: f64 = 0.15;

/// Includes nest at most this deep, which also stops include cycles.
const MAX_DEPTH: usize = 8;

impl GhosttyConfig {
    /// Parses config text as if read from `path` (used for includes and
    /// diagnostics).
    pub fn parse(text: &str, path: &Path) -> GhosttyConfig {
        let mut config = GhosttyConfig::default();
        config.apply(text, path, 0);
        config
    }

    fn read_file(&mut self, path: &Path, optional: bool, depth: usize) {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.files.push(path.to_path_buf());
                self.apply(&text, path, depth);
            }
            Err(e) if optional && e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => self.diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                source: Source::Ghostty(path.to_path_buf()),
                location: None,
                message: format!("can't read: {e}"),
            }),
        }
    }

    fn apply(&mut self, text: &str, path: &Path, depth: usize) {
        let mut includes = Vec::new();
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            let line_offset = offset;
            offset += line.len();
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = unquote(value.trim());
            let warn = |this: &mut Self, message: String| {
                this.diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    source: Source::Ghostty(path.to_path_buf()),
                    location: Some(Location::of_offset(text, line_offset)),
                    message,
                });
            };
            match key {
                "font-family" => {
                    if value.is_empty() {
                        self.font_family.clear();
                    } else {
                        self.font_family.push(value.to_string());
                    }
                }
                "font-size" => {
                    if value.is_empty() {
                        self.font_size = None;
                    } else {
                        match value.parse::<f32>() {
                            Ok(size) if size > 0.0 => self.font_size = Some(size),
                            _ => warn(self, format!("font-size `{value}` isn't a positive number")),
                        }
                    }
                }
                "theme" => self.theme = (!value.is_empty()).then(|| value.to_string()),
                "unfocused-split-opacity" => {
                    if value.is_empty() {
                        self.unfocused_split_opacity = None;
                    } else {
                        match value.parse::<f64>() {
                            Ok(v) if v.is_finite() => {
                                self.unfocused_split_opacity =
                                    Some(v.clamp(UNFOCUSED_SPLIT_OPACITY_MIN, 1.0))
                            }
                            _ => warn(
                                self,
                                format!("unfocused-split-opacity `{value}` isn't a number"),
                            ),
                        }
                    }
                }
                "background"
                | "foreground"
                | "cursor-color"
                | "selection-background"
                | "selection-foreground"
                | "unfocused-split-fill" => {
                    let slot = match key {
                        "background" => &mut self.background,
                        "foreground" => &mut self.foreground,
                        "cursor-color" => &mut self.cursor_color,
                        "selection-background" => &mut self.selection_background,
                        "unfocused-split-fill" => &mut self.unfocused_split_fill,
                        _ => &mut self.selection_foreground,
                    };
                    if value.is_empty() {
                        *slot = None;
                    } else if let Some(rgb) = Rgb::parse(value) {
                        *slot = Some(rgb);
                    } else {
                        warn(self, format!("{key} `{value}` isn't a hex colour; ignored"));
                    }
                }
                "palette" => {
                    if value.is_empty() {
                        self.palette.clear();
                        continue;
                    }
                    let parsed = value
                        .split_once('=')
                        .and_then(|(i, c)| Some((i.trim().parse::<u8>().ok()?, Rgb::parse(c)?)));
                    match parsed {
                        Some((index, rgb)) => {
                            self.palette.insert(index, rgb);
                        }
                        None => warn(
                            self,
                            format!("palette `{value}` should look like `N=#rrggbb`; ignored"),
                        ),
                    }
                }
                "config-file" if !value.is_empty() => {
                    let (optional, file) = match value.strip_prefix('?') {
                        Some(rest) => (true, rest),
                        None => (false, value),
                    };
                    includes.push((optional, file.to_string(), line_offset));
                }
                _ => {}
            }
        }
        // Ghostty applies includes after the file that names them.
        for (optional, file, line_offset) in includes {
            if depth + 1 >= MAX_DEPTH {
                self.diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    source: Source::Ghostty(path.to_path_buf()),
                    location: Some(Location::of_offset(text, line_offset)),
                    message: format!(
                        "config-file includes nest deeper than {MAX_DEPTH}; `{file}` skipped"
                    ),
                });
                continue;
            }
            let target = resolve_include(path, &file);
            self.read_file(&target, optional, depth + 1);
        }
    }
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value)
}

fn resolve_include(from: &Path, file: &str) -> PathBuf {
    let expanded = match file.strip_prefix("~/") {
        Some(rest) => crate::home_dir().map_or_else(|| PathBuf::from(file), |h| h.join(rest)),
        None => PathBuf::from(file),
    };
    if expanded.is_absolute() {
        expanded
    } else {
        from.parent().unwrap_or(Path::new(".")).join(expanded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours() {
        assert_eq!(
            Rgb::parse("#1e1e2e"),
            Some(Rgb {
                r: 0x1e,
                g: 0x1e,
                b: 0x2e
            })
        );
        assert_eq!(
            Rgb::parse("fff"),
            Some(Rgb {
                r: 255,
                g: 255,
                b: 255
            })
        );
        assert_eq!(Rgb::parse("red"), None);
        assert_eq!(Rgb::parse("#12345"), None);
    }

    #[test]
    fn parses_the_subset() {
        let text = "\
# comment
font-family = \"JetBrains Mono\"
font-family = Symbols Nerd Font
font-size = 13.5
theme = light:Rose Pine Dawn,dark:Rose Pine
background = #191724
foreground = e0def4
palette = 0=#26233a
palette = 15 = #ffffff
keybind = cmd+d=new_split:right
cursor-color = nonsense
";
        let config = GhosttyConfig::parse(text, Path::new("/tmp/ghostty/config"));
        assert_eq!(config.font_family, ["JetBrains Mono", "Symbols Nerd Font"]);
        assert_eq!(config.font_size, Some(13.5));
        assert_eq!(
            config.theme.as_deref(),
            Some("light:Rose Pine Dawn,dark:Rose Pine")
        );
        assert_eq!(config.background, Rgb::parse("191724"));
        assert_eq!(config.foreground, Rgb::parse("e0def4"));
        assert_eq!(config.palette.get(&0), Rgb::parse("26233a").as_ref());
        assert_eq!(config.palette.get(&15), Rgb::parse("ffffff").as_ref());
        assert_eq!(config.cursor_color, None);
        assert_eq!(config.diagnostics.len(), 1);
        assert_eq!(config.diagnostics[0].location.unwrap().line, 11);
    }

    #[test]
    fn unfocused_split_dimming() {
        let path = Path::new("/tmp/config");
        let config = GhosttyConfig::parse(
            "unfocused-split-opacity = 0.6\nunfocused-split-fill = #101010\n",
            path,
        );
        assert_eq!(config.unfocused_split_opacity, Some(0.6));
        assert_eq!(config.unfocused_split_fill, Rgb::parse("101010"));
        assert!(config.diagnostics.is_empty());

        // Clamped as Ghostty does: 0.15 to 1.
        let low = GhosttyConfig::parse("unfocused-split-opacity = 0\n", path);
        assert_eq!(low.unfocused_split_opacity, Some(0.15));
        let high = GhosttyConfig::parse("unfocused-split-opacity = 3\n", path);
        assert_eq!(high.unfocused_split_opacity, Some(1.0));

        // Unset, reset, junk.
        let none = GhosttyConfig::parse("font-size = 12\n", path);
        assert_eq!(none.unfocused_split_opacity, None);
        let reset = GhosttyConfig::parse(
            "unfocused-split-opacity = 0.5\nunfocused-split-opacity =\n",
            path,
        );
        assert_eq!(reset.unfocused_split_opacity, None);
        let junk = GhosttyConfig::parse(
            "unfocused-split-opacity = dim\nunfocused-split-fill = red\n",
            path,
        );
        assert_eq!(junk.unfocused_split_opacity, None);
        assert_eq!(junk.unfocused_split_fill, None);
        assert_eq!(junk.diagnostics.len(), 2);
    }

    #[test]
    fn load_from_skips_missing_files() {
        let dir = std::env::temp_dir().join(format!("azj-ghostty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config");
        std::fs::write(&file, "unfocused-split-opacity = 0.8\n").unwrap();
        let config = load_from(&[dir.join("missing"), file.clone()]);
        assert_eq!(config.unfocused_split_opacity, Some(0.8));
        assert!(config.diagnostics.is_empty());
        assert_eq!(config.files, [file]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_value_resets() {
        let text = "font-family = A\nfont-family =\nfont-family = B\nfont-size = 12\nfont-size =\n";
        let config = GhosttyConfig::parse(text, Path::new("/tmp/config"));
        assert_eq!(config.font_family, ["B"]);
        assert_eq!(config.font_size, None);
    }
}
