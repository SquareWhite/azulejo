//! What a file is, by extension first and content second.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::Kind;
use crate::code;

/// Text files are rendered up to this size; the rest is cut with a notice.
pub const MAX_TEXT_BYTES: usize = 5 * 1024 * 1024;

/// How much of the start of a file decides whether it's binary.
const SNIFF_BYTES: usize = 8 * 1024;

const MARKDOWN: &[&str] = &["md", "markdown", "mdown", "mkd", "mkdn", "mdx"];
const IMAGE: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "tif", "tiff", "bmp", "ico", "svg", "avif",
];
const VIDEO: &[&str] = &["mp4", "m4v", "mov", "webm", "mkv", "avi", "mpg", "mpeg"];
const AUDIO: &[&str] = &["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", "aiff"];

/// A classified file, with its text when it has some.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    pub kind: Kind,
    /// Syntax name for [`Kind::Code`].
    pub language: Option<String>,
    /// The (possibly truncated) text, for markdown, code and plain text.
    pub text: Option<String>,
    pub truncated: bool,
    /// Why the file couldn't be read, for [`Kind::Unreadable`].
    pub error: Option<String>,
}

impl Classified {
    fn only(kind: Kind) -> Self {
        Classified {
            kind,
            language: None,
            text: None,
            truncated: false,
            error: None,
        }
    }
}

/// Sorts `path` into a [`Kind`], reading at most [`MAX_TEXT_BYTES`] + 1 bytes.
pub fn classify(path: &Path) -> Classified {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Classified::only(Kind::Missing),
        Err(e) => return unreadable(e),
    };
    if meta.is_dir() {
        return Classified::only(Kind::Directory);
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let is = |list: &[&str]| list.contains(&ext.as_str());
    if is(IMAGE) {
        return Classified::only(Kind::Image);
    }
    if is(VIDEO) {
        return Classified::only(Kind::Video);
    }
    if is(AUDIO) {
        return Classified::only(Kind::Audio);
    }
    if ext == "pdf" {
        return Classified::only(Kind::Pdf);
    }

    let (bytes, truncated) = match read_prefix(path, MAX_TEXT_BYTES) {
        Ok(read) => read,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Classified::only(Kind::Missing),
        Err(e) => return unreadable(e),
    };
    let Some(text) = decode(&bytes, truncated) else {
        return Classified::only(Kind::Binary);
    };

    let (kind, language) = if is(MARKDOWN) {
        (Kind::Markdown, None)
    } else {
        let first_line = text.lines().next().unwrap_or("");
        match code::detect(path, first_line) {
            Some(syntax) if syntax.name == "Markdown" => (Kind::Markdown, None),
            Some(syntax) => (Kind::Code, Some(syntax.name.clone())),
            None => (Kind::Text, None),
        }
    };
    Classified {
        kind,
        language,
        text: Some(text),
        truncated,
        error: None,
    }
}

fn unreadable(e: io::Error) -> Classified {
    Classified {
        error: Some(e.to_string()),
        ..Classified::only(Kind::Unreadable)
    }
}

/// Up to `limit` bytes, and whether the file was longer.
fn read_prefix(path: &Path, limit: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() > limit;
    bytes.truncate(limit);
    Ok((bytes, truncated))
}

/// The text in `bytes`, or `None` when they look binary: a NUL byte or
/// invalid UTF-8 near the start. A cut file ends at its last whole line.
fn decode(bytes: &[u8], truncated: bool) -> Option<String> {
    let head = &bytes[..bytes.len().min(SNIFF_BYTES)];
    if head.contains(&0) {
        return None;
    }
    match std::str::from_utf8(head) {
        Ok(_) => {}
        // A multi-byte character cut off at the sniff boundary is fine.
        Err(e) if e.error_len().is_none() => {}
        Err(_) => return None,
    }
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    if truncated && let Some(end) = text.rfind('\n') {
        text.truncate(end + 1);
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_sniffing() {
        assert_eq!(decode(b"hello\n", false).as_deref(), Some("hello\n"));
        assert_eq!(decode(b"a\0b", false), None);
        assert_eq!(decode(&[0xff, 0xfe, 0x41], false), None);
        // "é" cut in half at the end of the sniffed prefix.
        let mut cut = vec![b'a'; SNIFF_BYTES - 1];
        cut.push(0xc3);
        assert!(decode(&cut, true).is_some());
    }

    #[test]
    fn truncated_text_ends_on_a_line() {
        assert_eq!(
            decode(b"one\ntwo\nthr", true).as_deref(),
            Some("one\ntwo\n")
        );
        assert_eq!(
            decode(b"one\ntwo\nthr", false).as_deref(),
            Some("one\ntwo\nthr")
        );
    }
}
