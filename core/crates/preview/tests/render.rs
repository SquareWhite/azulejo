//! Golden-file and classification tests. Regenerate goldens with
//! `AZJ_UPDATE_GOLDEN=1 cargo test -p azulejo-preview`.

use std::path::{Path, PathBuf};

use azulejo_preview::{
    Kind, MAX_TEXT_BYTES, classify, code_body, markdown_body, render, render_source,
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    if std::env::var_os("AZJ_UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing golden {}; set AZJ_UPDATE_GOLDEN=1", path.display()));
    assert_eq!(
        actual, expected,
        "{name} differs from its golden file; set AZJ_UPDATE_GOLDEN=1 to accept"
    );
}

#[test]
fn markdown_golden() {
    let text = std::fs::read_to_string(fixture("doc.md")).unwrap();
    // A fixed path, so resolved links don't depend on the checkout location.
    let html = markdown_body(&text, Path::new("/fixtures/plans/doc.md"));
    golden("doc.md.html", &html);

    // The features the golden file must show, spelled out.
    for needle in [
        "<h1 id=\"plan-preview-tiles\">",
        "<del>struck</del>",
        "<a href=\"https://example.com/docs\">",
        "type=\"checkbox\"",
        "<table>",
        "class=\"footnotes\"",
        "src=\"file:///fixtures/plans/img/flow%20chart.png\"",
        "href=\"file:///fixtures/docs/PRD.md#goals\"",
        "href=\"#tasks\"",
        "href=\"mailto:me@example.com\"",
        "src=\"https://example.com/logo.png\"",
        "<pre class=\"syn-code",
        "syn-rust",
        "plain &amp; escaped &lt;text&gt;",
    ] {
        assert!(html.contains(needle), "missing {needle:?} in:\n{html}");
    }
    for banned in [
        "<script",
        "<b>",
        "javascript:",
        "Front matter is not rendered",
    ] {
        assert!(!html.contains(banned), "found {banned:?} in:\n{html}");
    }
}

#[test]
fn code_golden() {
    let classified = classify(&fixture("sample.rs"));
    assert_eq!(classified.kind, Kind::Code);
    assert_eq!(classified.language.as_deref(), Some("Rust"));
    let html = code_body(classified.text.as_deref().unwrap(), Some("Rust"));
    golden("sample.rs.html", &html);
    assert!(html.contains("class=\"azj-gutter\""));
    assert!(html.contains("&lt;"), "escaped");
    assert!(html.contains("12\n</pre>"), "12 line numbers");
}

#[test]
fn classification() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    };
    let kind = |path: &Path| classify(path).kind;

    assert_eq!(kind(&fixture("doc.md")), Kind::Markdown);
    assert_eq!(kind(&fixture("notes.txt")), Kind::Text);
    let script = classify(&fixture("script"));
    assert_eq!(script.kind, Kind::Code, "shebang");
    assert_eq!(script.language.as_deref(), Some("Python"));
    assert_eq!(kind(&write("Makefile", b"all:\n\techo hi\n")), Kind::Code);
    assert_eq!(kind(&write("README.MARKDOWN", b"# hi\n")), Kind::Markdown);

    // Native kinds go by extension and are never read.
    assert_eq!(kind(&write("a.PNG", b"not really a png")), Kind::Image);
    assert_eq!(kind(&write("a.mov", b"")), Kind::Video);
    assert_eq!(kind(&write("a.mp3", b"")), Kind::Audio);
    assert_eq!(kind(&write("a.pdf", b"%PDF-1.7")), Kind::Pdf);

    assert_eq!(kind(&write("blob.bin", &[0, 1, 2, 3, 255])), Kind::Binary);
    assert_eq!(
        kind(&write("latin1.txt", &[0x63, 0x61, 0x66, 0xe9, 0x20, 0x21])),
        Kind::Binary
    );
    assert_eq!(kind(&write("empty.txt", b"")), Kind::Text);
    assert_eq!(kind(dir.path()), Kind::Directory);
    assert_eq!(kind(&dir.path().join("nope.md")), Kind::Missing);
}

#[test]
fn native_kinds_have_no_html_and_messages_do() {
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("a.png");
    std::fs::write(&png, b"x").unwrap();
    let r = render(&png);
    assert_eq!(r.kind, Kind::Image);
    assert!(r.kind.is_native());
    assert_eq!(r.html, None);

    let missing = render(&dir.path().join("gone.md"));
    assert_eq!(missing.kind, Kind::Missing);
    let html = missing.html.unwrap();
    assert!(html.contains("File not found"));
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("Content-Security-Policy"));

    let bin = dir.path().join("a.bin");
    std::fs::write(&bin, [0u8, 159, 146, 150]).unwrap();
    assert!(render(&bin).html.unwrap().contains("No preview"));
}

#[test]
fn full_page_wraps_the_body() {
    let r = render(&fixture("doc.md"));
    assert_eq!(r.kind, Kind::Markdown);
    let html = r.html.unwrap();
    assert!(html.contains("<title>doc.md</title>"));
    assert!(html.contains("class=\"azj-kind-markdown\""));
    assert!(html.contains("<main id=\"azj-content\"><h1"));
    assert!(html.contains("azulejoPreview"));
    assert!(!r.truncated);
}

#[test]
fn source_mode_shows_markdown_as_highlighted_text() {
    let r = render_source(&fixture("doc.md"));
    assert_eq!(r.kind, Kind::Markdown);
    assert!(r.source);
    assert_eq!(r.language.as_deref(), Some("Markdown"));
    let html = r.html.unwrap();
    // Laid out as code, with a gutter, not as a document.
    assert!(html.contains("class=\"azj-kind-code\""), "{html}");
    assert!(html.contains("data-language=\"Markdown\""));
    assert!(!html.contains("<h1 id=\"plan-preview-tiles\">"));
    // The markdown syntax highlighter marked headings up.
    assert!(
        html.contains("syn-markdown") || html.contains("syn-heading"),
        "{html}"
    );

    // Code is the same either way; images have no source view.
    let code = render_source(&fixture("sample.rs"));
    assert!(code.source);
    assert_eq!(code.kind, Kind::Code);
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("a.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\n").unwrap();
    let image = render_source(&png);
    assert!(!image.source);
    assert!(image.html.is_none());

    // The rendered view says it isn't source.
    assert!(!render(&fixture("doc.md")).source);
}

#[test]
fn huge_text_is_truncated_with_a_notice() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.log");
    let line = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n";
    let lines = MAX_TEXT_BYTES / line.len() + 100;
    std::fs::write(&path, line.repeat(lines)).unwrap();
    let r = render(&path);
    assert!(r.truncated);
    let html = r.html.unwrap();
    assert!(html.contains("Showing the first 5 MB"));
    assert!(html.len() < MAX_TEXT_BYTES * 3);
}
