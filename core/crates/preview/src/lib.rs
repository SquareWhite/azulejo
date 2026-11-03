//! Preview tiles (F9): what a file is, how it looks as HTML, and when it
//! changes.
//!
//! - [`classify`] sorts a file into markdown, code, plain text, a native kind
//!   (image, video, audio, PDF) the shells show with QuickLook or GTK, or
//!   something we can only explain (binary, missing, a directory).
//! - [`render`] turns markdown and code into a self-contained HTML document
//!   styled by `shared/webview/preview.css`; native kinds get no HTML.
//! - [`Watcher`] reports changes to watched files, debounced, including
//!   editors' save-by-rename.
//!
//! Safety: all text is escaped, raw HTML in markdown is dropped, and the page's
//! content security policy blocks scripts other than our own inline hook and
//! every network request except images the document itself references.

mod classify;
mod code;
mod markdown;
mod page;
mod watch;

use std::path::{Path, PathBuf};

pub use classify::{Classified, MAX_TEXT_BYTES, classify};
pub use code::syntax_css;
pub use page::{PAGE_CSS, PAGE_JS, PAGE_TEMPLATE};
pub use watch::Watcher;

/// What kind of thing a preview shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Markdown,
    /// Source code with a known syntax.
    Code,
    /// Text without a syntax we know.
    Text,
    /// Shown natively (QuickLook on macOS, GTK on Linux); no HTML.
    Image,
    Video,
    Audio,
    Pdf,
    /// Not text and not a native kind.
    Binary,
    Directory,
    Missing,
    /// It exists but can't be read (permissions, I/O error).
    Unreadable,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Markdown => "markdown",
            Kind::Code => "code",
            Kind::Text => "text",
            Kind::Image => "image",
            Kind::Video => "video",
            Kind::Audio => "audio",
            Kind::Pdf => "pdf",
            Kind::Binary => "binary",
            Kind::Directory => "directory",
            Kind::Missing => "missing",
            Kind::Unreadable => "unreadable",
        }
    }

    /// Shown by the shell's native viewer from the path, not from HTML.
    pub fn is_native(self) -> bool {
        matches!(self, Kind::Image | Kind::Video | Kind::Audio | Kind::Pdf)
    }
}

/// A rendered preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub path: PathBuf,
    pub kind: Kind,
    /// The syntax name for code, e.g. `Rust`.
    pub language: Option<String>,
    /// A complete HTML document; `None` for native kinds.
    pub html: Option<String>,
    /// Only the first [`MAX_TEXT_BYTES`] were rendered.
    pub truncated: bool,
    /// The file as it was just before it was read; `None` when it couldn't be
    /// examined (missing, no permission).
    pub stamp: Option<Stamp>,
}

/// What a file looked like when it was rendered: enough to tell a real change
/// from a repeated notification. File watchers report events that happened
/// just before a watch started (FSEvents replays the tail of its latency
/// window), so the same unchanged file can be reported more than once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub len: u64,
    pub modified: Option<std::time::SystemTime>,
    pub is_dir: bool,
}

impl Stamp {
    /// Examines `path`, following symlinks.
    pub fn of(path: &Path) -> Option<Stamp> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Stamp {
            len: meta.len(),
            modified: meta.modified().ok(),
            is_dir: meta.is_dir(),
        })
    }
}

/// Classifies and renders `path`. Never fails: problems become a page that
/// explains them.
pub fn render(path: &Path) -> Rendered {
    // Taken before reading, so a write that races the read gives the next
    // render a different stamp and can't be mistaken for a repeat.
    let stamp = Stamp::of(path);
    let classified = classify(path);
    let path = path.to_path_buf();
    let title = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let notice = classified.truncated.then(|| {
        format!(
            "Showing the first {} MB of this file.",
            MAX_TEXT_BYTES / (1024 * 1024)
        )
    });
    let kind = classified.kind;
    let body = match kind {
        Kind::Markdown => Some(with_notice(
            notice.as_deref(),
            markdown::render_body(classified.text.as_deref().unwrap_or(""), &path),
        )),
        Kind::Code | Kind::Text => Some(with_notice(
            notice.as_deref(),
            code::render_body(
                classified.text.as_deref().unwrap_or(""),
                classified.language.as_deref(),
            ),
        )),
        Kind::Image | Kind::Video | Kind::Audio | Kind::Pdf => None,
        Kind::Binary => Some(message(
            "No preview for this file",
            "It isn't text, and it isn't an image, video, audio file or PDF.",
            &path,
        )),
        Kind::Directory => Some(message(
            "This is a folder",
            "Open a file inside it, or use a files tile to browse it.",
            &path,
        )),
        Kind::Missing => Some(message(
            "File not found",
            "It may have been moved or deleted. The preview updates if it comes back.",
            &path,
        )),
        Kind::Unreadable => Some(message(
            "Can't read this file",
            classified.error.as_deref().unwrap_or("Unknown error."),
            &path,
        )),
    };
    Rendered {
        html: body.map(|body| page::document(&title, kind, &body)),
        path,
        kind,
        language: classified.language,
        truncated: classified.truncated,
        stamp,
    }
}

fn with_notice(notice: Option<&str>, body: String) -> String {
    match notice {
        Some(text) => format!(
            "<div class=\"azj-notice\">{}</div>\n{body}",
            page::escape(text)
        ),
        None => body,
    }
}

fn message(title: &str, detail: &str, path: &Path) -> String {
    format!(
        "<div class=\"azj-message\"><h1>{}</h1><p>{}</p><p><code>{}</code></p></div>\n",
        page::escape(title),
        page::escape(detail),
        page::escape(&path.display().to_string()),
    )
}

/// The body of a markdown document as HTML, without the page around it.
pub fn markdown_body(text: &str, path: &Path) -> String {
    markdown::render_body(text, path)
}

/// The body of a source file as HTML, without the page around it.
pub fn code_body(text: &str, language: Option<&str>) -> String {
    code::render_body(text, language)
}
