//! Diagnostics for config problems. Loading never fails: a bad file falls back
//! to the defaults and the shell shows these in a non-blocking banner.

use std::fmt;
use std::ops::Range;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Something was ignored or overridden; the config still works as written
    /// elsewhere.
    Warning,
    /// A value was rejected; the default applies instead.
    Error,
}

/// Which file a diagnostic points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The bundled `shared/config/keymap.default.toml`.
    DefaultKeymap,
    /// The user's `config.toml`, or text passed in directly (no path).
    User(Option<PathBuf>),
    /// A Ghostty config file.
    Ghostty(PathBuf),
    /// A user theme, `<config dir>/themes/<id>.toml`.
    Theme(PathBuf),
}

/// 1-based line and column (column counts characters, not bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

impl Location {
    /// Location of byte offset `offset` in `text`.
    pub fn of_offset(text: &str, offset: usize) -> Self {
        let offset = offset.min(text.len());
        let before = &text[..floor_char_boundary(text, offset)];
        let line = before.matches('\n').count() + 1;
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        let column = before[line_start..].chars().count() + 1;
        Location { line, column }
    }
}

fn floor_char_boundary(text: &str, mut i: usize) -> usize {
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub source: Source,
    pub location: Option<Location>,
    pub message: String,
}

impl Diagnostic {
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Warning => "warning",
            Severity::Error => "error",
        };
        match &self.source {
            Source::DefaultKeymap => write!(f, "keymap.default.toml")?,
            Source::User(Some(path)) | Source::Ghostty(path) | Source::Theme(path) => {
                write!(f, "{}", path.display())?
            }
            Source::User(None) => write!(f, "config")?,
        }
        if let Some(loc) = self.location {
            write!(f, ":{}:{}", loc.line, loc.column)?;
        }
        write!(f, ": {severity}: {}", self.message)
    }
}

/// Collects diagnostics for one source text.
pub(crate) struct Sink<'a> {
    pub text: &'a str,
    pub source: Source,
    pub out: &'a mut Vec<Diagnostic>,
}

impl Sink<'_> {
    pub fn push(&mut self, severity: Severity, span: Option<Range<usize>>, message: String) {
        self.out.push(Diagnostic {
            severity,
            source: self.source.clone(),
            location: span.map(|s| Location::of_offset(self.text, s.start)),
            message,
        });
    }

    pub fn error(&mut self, span: Range<usize>, message: impl Into<String>) {
        self.push(Severity::Error, Some(span), message.into());
    }

    pub fn warn(&mut self, span: Range<usize>, message: impl Into<String>) {
        self.push(Severity::Warning, Some(span), message.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations_are_one_based_and_count_chars() {
        let text = "a = 1\nbé = 2\n";
        assert_eq!(
            Location::of_offset(text, 0),
            Location { line: 1, column: 1 }
        );
        let offset = text.find('=').unwrap();
        assert_eq!(
            Location::of_offset(text, offset),
            Location { line: 1, column: 3 }
        );
        let offset = text.rfind('=').unwrap();
        assert_eq!(
            Location::of_offset(text, offset),
            Location { line: 2, column: 4 }
        );
    }
}
