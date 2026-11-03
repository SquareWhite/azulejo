//! The HTML page around a preview body, built from `shared/webview/`.

use std::sync::OnceLock;

use crate::Kind;
use crate::code::syntax_css;

/// `shared/webview/preview.html`: the page skeleton, with `{{title}}`,
/// `{{css}}`, `{{kind}}`, `{{body}}` and `{{script}}` slots.
pub const PAGE_TEMPLATE: &str = include_str!("../../../../shared/webview/preview.html");
/// `shared/webview/preview.css`.
pub const PAGE_CSS: &str = include_str!("../../../../shared/webview/preview.css");
/// `shared/webview/preview.js`: the live-reload hook (`azulejoPreview.swap`).
pub const PAGE_JS: &str = include_str!("../../../../shared/webview/preview.js");

enum Segment {
    Text(&'static str),
    Slot(&'static str),
}

/// The template split into text and slots once, so slot values are inserted
/// in a single pass: a document that contains `{{script}}` stays literal.
fn segments() -> &'static [Segment] {
    static SEGMENTS: OnceLock<Vec<Segment>> = OnceLock::new();
    SEGMENTS.get_or_init(|| {
        let mut out = Vec::new();
        let mut rest = PAGE_TEMPLATE;
        while let Some(start) = rest.find("{{") {
            let Some(len) = rest[start..].find("}}") else {
                break;
            };
            out.push(Segment::Text(&rest[..start]));
            out.push(Segment::Slot(&rest[start + 2..start + len]));
            rest = &rest[start + len + 2..];
        }
        out.push(Segment::Text(rest));
        out
    })
}

fn css() -> &'static str {
    static CSS: OnceLock<String> = OnceLock::new();
    CSS.get_or_init(|| format!("{PAGE_CSS}\n{}", syntax_css()))
}

/// A complete document around `body` (already HTML).
pub fn document(title: &str, kind: Kind, body: &str) -> String {
    let title = escape(title);
    let mut out = String::with_capacity(body.len() + css().len() + PAGE_JS.len() + 1024);
    for segment in segments() {
        match segment {
            Segment::Text(text) => out.push_str(text),
            Segment::Slot("title") => out.push_str(&title),
            Segment::Slot("css") => out.push_str(css()),
            Segment::Slot("kind") => out.push_str(kind.as_str()),
            Segment::Slot("body") => out.push_str(body),
            Segment::Slot("script") => out.push_str(PAGE_JS),
            Segment::Slot(other) => {
                out.push_str("{{");
                out.push_str(other);
                out.push_str("}}");
            }
        }
    }
    out
}

/// Escapes text for HTML element content and quoted attribute values.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    escape_into(&mut out, text);
    out
}

pub fn escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_filled_once() {
        let doc = document("a <b>", Kind::Markdown, "<p>{{script}}</p>");
        assert!(doc.contains("<title>a &lt;b&gt;</title>"));
        assert!(doc.contains("<p>{{script}}</p>"), "body stays literal");
        assert!(doc.contains("azulejoPreview"));
        assert!(doc.contains("class=\"azj-kind-markdown\""));
        assert!(doc.contains(".syn-"), "syntax colours are inlined");
        assert!(!doc.contains("{{css}}"));
    }

    #[test]
    fn escapes_everything_that_matters() {
        assert_eq!(
            escape("<a href=\"x\">&'"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
    }
}
