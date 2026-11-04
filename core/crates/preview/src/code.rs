//! Source code and plain text as HTML, highlighted with syntect.
//!
//! Highlighting uses CSS classes (`syn-*`), not inline colours, so one page
//! follows the theme (`html[data-azj-syntax]`, see `syntax.rs`) or, without
//! one, the system's light or dark appearance, without re-rendering.

use std::path::Path;
use std::sync::OnceLock;

use syntect::highlighting::ThemeSet;
use syntect::html::{ClassStyle, ClassedHTMLGenerator, css_for_theme_with_class_style};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

use crate::page::{escape, escape_into};

const CLASS_STYLE: ClassStyle = ClassStyle::SpacedPrefixed { prefix: "syn-" };
const LIGHT_THEME: &str = "InspiredGitHub";
const DARK_THEME: &str = "base16-ocean.dark";

/// Beyond this, source is shown unhighlighted: syntect would take seconds.
pub(crate) const HIGHLIGHT_LIMIT_BYTES: usize = 2 * 1024 * 1024;

pub(crate) fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// The syntax colours for `syn-*` classes. Each theme's syntax
/// (`crate::syntax::NAMES`) applies under `html[data-azj-syntax="<name>"]`;
/// a page without the attribute is light by default and dark under
/// `prefers-color-scheme: dark`.
pub fn syntax_css() -> &'static str {
    static CSS: OnceLock<String> = OnceLock::new();
    CSS.get_or_init(|| {
        let themes = ThemeSet::load_defaults();
        let css = |theme: Option<&syntect::highlighting::Theme>| {
            theme
                .and_then(|theme| css_for_theme_with_class_style(theme, CLASS_STYLE).ok())
                .unwrap_or_default()
        };
        let unthemed = "html:not([data-azj-syntax])";
        let mut out = format!(
            "/* syntax: {LIGHT_THEME} */\n{}\n@media (prefers-color-scheme: dark) {{\n/* syntax: {DARK_THEME} */\n{}\n}}\n",
            crate::syntax::scoped(&css(themes.themes.get(LIGHT_THEME)), unthemed),
            crate::syntax::scoped(&css(themes.themes.get(DARK_THEME)), unthemed),
        );
        for name in crate::syntax::NAMES {
            let theme = crate::syntax::theme(name);
            let scope = format!("html[data-azj-syntax=\"{name}\"]");
            out.push_str(&format!(
                "/* syntax: {name} */\n{}\n",
                crate::syntax::scoped(&css(theme.as_ref()), &scope)
            ));
        }
        out
    })
}

/// The syntax for a file, from its name or extension, then its first line
/// (shebangs, modelines). `None` for plain text.
pub(crate) fn detect(path: &Path, first_line: &str) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let by_name = |name: &str| set.find_syntax_by_extension(name);
    let found = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(by_name)
        .or_else(|| {
            path.extension()
                .and_then(|e| e.to_str())
                .and_then(|e| by_name(&e.to_ascii_lowercase()).or_else(|| by_name(e)))
        })
        .or_else(|| set.find_syntax_by_first_line(first_line));
    found.filter(|s| s.name != "Plain Text")
}

/// A syntax by the name `detect` reported, or a markdown fence's info string
/// (`rust`, `py`, `TypeScript`).
fn lookup(name: &str) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let token = name
        .split(|c: char| c.is_whitespace() || c == ',' || c == '{')
        .next()
        .unwrap_or("")
        .trim();
    if token.is_empty() {
        return None;
    }
    set.find_syntax_by_name(token)
        .or_else(|| set.find_syntax_by_token(token))
        .or_else(|| set.find_syntax_by_extension(&token.to_ascii_lowercase()))
        .filter(|s| s.name != "Plain Text")
}

/// `code` as highlighted spans (no wrapping element), or escaped text when the
/// syntax is unknown, the code is huge, or highlighting fails.
pub(crate) fn highlight(code: &str, language: Option<&str>) -> String {
    let syntax = language.and_then(lookup);
    if let Some(syntax) = syntax
        && code.len() <= HIGHLIGHT_LIMIT_BYTES
    {
        let mut generator =
            ClassedHTMLGenerator::new_with_class_style(syntax, syntaxes(), CLASS_STYLE);
        let ok = LinesWithEndings::from(code).all(|line| {
            generator
                .parse_html_for_line_which_includes_newline(line)
                .is_ok()
        });
        if ok {
            return generator.finalize();
        }
    }
    escape(code)
}

/// A whole source file: a line-number gutter beside the code.
pub(crate) fn render_body(text: &str, language: Option<&str>) -> String {
    let lines = text.lines().count().max(1);
    let mut gutter = String::with_capacity(lines * 6);
    for n in 1..=lines {
        gutter.push_str(&n.to_string());
        gutter.push('\n');
    }
    let mut out = String::with_capacity(text.len() * 3 + gutter.len() + 128);
    out.push_str("<div class=\"azj-source\"><pre class=\"azj-gutter\" aria-hidden=\"true\">");
    out.push_str(&gutter);
    out.push_str("</pre><pre class=\"syn-code\"><code");
    if let Some(language) = language {
        out.push_str(" data-language=\"");
        escape_into(&mut out, language);
        out.push('"');
    }
    out.push('>');
    out.push_str(&highlight(text, language));
    out.push_str("</code></pre></div>\n");
    out
}
