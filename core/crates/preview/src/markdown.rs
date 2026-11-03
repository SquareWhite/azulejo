//! Markdown as HTML: GitHub-flavoured, raw HTML dropped, relative links and
//! images resolved against the file's folder.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use comrak::adapters::SyntaxHighlighterAdapter;
use comrak::options::{Plugins, URLRewriter};
use comrak::{Arena, Options, format_html_with_plugins, parse_document};

use crate::code;
use crate::page::escape_into;

fn options(dir: PathBuf) -> Options<'static> {
    let mut options = Options::default();
    let ext = &mut options.extension;
    ext.strikethrough = true;
    ext.table = true;
    ext.autolink = true;
    ext.tasklist = true;
    ext.footnotes = true;
    ext.header_id_prefix = Some(String::new());
    ext.front_matter_delimiter = Some("---".into());
    // Rewriters run after comrak's safety check on the original URL, so a
    // relative link may become `file://` while `javascript:` is still refused.
    let rewrite: Arc<dyn URLRewriter> =
        Arc::new(move |url: &str| resolve(url, &dir).unwrap_or_else(|| url.to_owned()));
    ext.link_url_rewriter = Some(rewrite.clone());
    ext.image_url_rewriter = Some(rewrite);
    // Raw HTML stays off: comrak replaces it with a comment and refuses
    // `javascript:` and similar link targets.
    options.render.r#unsafe = false;
    options.render.tasklist_classes = true;
    options
}

pub(crate) fn render_body(text: &str, path: &Path) -> String {
    let dir = path.parent().map(absolute).unwrap_or_default();
    let options = options(dir);
    let arena = Arena::new();
    let root = parse_document(&arena, text, &options);
    let highlighter = Highlighter;
    let mut plugins = Plugins::default();
    plugins.render.codefence_syntax_highlighter = Some(&highlighter);
    let mut out = String::with_capacity(text.len() * 2);
    // Writing to a String can't fail.
    let _ = format_html_with_plugins(root, &options, &mut out, &plugins);
    out
}

/// A `file://` URL for a relative or absolute path in the document; `None`
/// leaves the URL alone (other schemes, in-page anchors, empty).
fn resolve(url: &str, dir: &Path) -> Option<String> {
    if url.is_empty() || url.starts_with('#') || url.starts_with("//") || has_scheme(url) {
        return None;
    }
    let (path_part, suffix) = match url.find(['#', '?']) {
        Some(i) => (&url[..i], &url[i..]),
        None => (url, ""),
    };
    let decoded = percent_decode(path_part);
    let joined = if decoded.starts_with('/') {
        PathBuf::from(decoded.as_ref())
    } else {
        dir.join(decoded.as_ref())
    };
    let file = url::Url::from_file_path(normalize(&joined)).ok()?;
    Some(format!("{file}{suffix}"))
}

fn has_scheme(url: &str) -> bool {
    match url.find(':') {
        Some(i) if i > 1 => url[..i]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')),
        // `C:` style drive letters aren't schemes, but this is macOS and Linux.
        _ => false,
    }
}

fn percent_decode(text: &str) -> Cow<'_, str> {
    if !text.contains('%') {
        return Cow::Borrowed(text);
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(byte) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Cow::Owned(String::from_utf8_lossy(&out).into_owned())
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

/// Removes `.` and `..` without touching the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Fenced code blocks, highlighted like source files.
struct Highlighter;

impl SyntaxHighlighterAdapter for Highlighter {
    fn write_highlighted(
        &self,
        output: &mut dyn fmt::Write,
        lang: Option<&str>,
        code: &str,
    ) -> fmt::Result {
        output.write_str(&code::highlight(code, lang))
    }

    fn write_pre_tag(
        &self,
        output: &mut dyn fmt::Write,
        attributes: HashMap<&'static str, Cow<'_, str>>,
    ) -> fmt::Result {
        write_tag(output, "pre", &attributes, Some("syn-code"))
    }

    fn write_code_tag(
        &self,
        output: &mut dyn fmt::Write,
        attributes: HashMap<&'static str, Cow<'_, str>>,
    ) -> fmt::Result {
        write_tag(output, "code", &attributes, None)
    }
}

fn write_tag(
    output: &mut dyn fmt::Write,
    tag: &str,
    attributes: &HashMap<&'static str, Cow<'_, str>>,
    extra_class: Option<&str>,
) -> fmt::Result {
    let mut html = String::new();
    html.push('<');
    html.push_str(tag);
    // Sorted, so output is stable for golden tests.
    let mut keys: Vec<_> = attributes.keys().copied().collect();
    keys.sort_unstable();
    let mut wrote_class = false;
    for key in keys {
        let mut value = attributes[key].to_string();
        if key == "class"
            && let Some(extra) = extra_class
        {
            value = format!("{extra} {value}");
            wrote_class = true;
        }
        html.push(' ');
        html.push_str(key);
        html.push_str("=\"");
        escape_into(&mut html, &value);
        html.push('"');
    }
    if !wrote_class && let Some(extra) = extra_class {
        html.push_str(" class=\"");
        html.push_str(extra);
        html.push('"');
    }
    html.push('>');
    output.write_str(&html)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_paths_only() {
        let dir = Path::new("/notes/plans");
        assert_eq!(
            resolve("img/a b.png", dir).as_deref(),
            Some("file:///notes/plans/img/a%20b.png")
        );
        assert_eq!(
            resolve("../other.md#goals", dir).as_deref(),
            Some("file:///notes/other.md#goals")
        );
        assert_eq!(
            resolve("pic%20one.png", dir).as_deref(),
            Some("file:///notes/plans/pic%20one.png")
        );
        assert_eq!(
            resolve("/abs/x.png", dir).as_deref(),
            Some("file:///abs/x.png")
        );
        assert_eq!(resolve("https://example.com/x.png", dir), None);
        assert_eq!(resolve("mailto:me@example.com", dir), None);
        assert_eq!(resolve("#section", dir), None);
        assert_eq!(resolve("//cdn.example.com/x.png", dir), None);
        assert_eq!(resolve("", dir), None);
    }

    #[test]
    fn raw_html_and_script_links_are_dropped() {
        let html = render_body(
            "<script>alert(1)</script>\n\n[x](javascript:alert(1)) <b>bold</b>\n",
            Path::new("/tmp/a.md"),
        );
        assert!(!html.contains("<script>"), "{html}");
        assert!(!html.contains("<b>"), "{html}");
        assert!(!html.contains("javascript:"), "{html}");
    }
}
