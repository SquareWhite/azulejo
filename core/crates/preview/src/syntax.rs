//! Syntax colours per theme (docs/themes.md). Each Azulejo theme names one
//! of these (tokens.json `themes.<id>.syntax`): One Dark and One Light for
//! the Atom themes, GitHub dark and light for the macOS ones. They're built
//! here from scope tables rather than bundled `.tmTheme` files.
//!
//! The page carries all of them, each scoped to
//! `html[data-azj-syntax="<name>"]`; the shell sets that attribute to the
//! theme's syntax, so switching theme recolours code without a re-render.
//! A page without the attribute keeps syntect's InspiredGitHub (light) and
//! base16-ocean (dark, by `prefers-color-scheme`).

use std::str::FromStr;

use syntect::highlighting::{
    Color, FontStyle, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings,
};

/// The syntax names a theme can pick, as in azulejo-config's `SYNTAXES`.
pub const NAMES: &[&str] = &["one-dark", "one-light", "github-dark", "github-light"];

struct Palette {
    background: &'static str,
    foreground: &'static str,
    /// (scope selectors, colour, italic)
    scopes: &'static [(&'static str, &'static str, bool)],
}

/// Atom's one-dark-syntax.
const ONE_DARK: Palette = Palette {
    background: "#282c34",
    foreground: "#abb2bf",
    scopes: &[
        ("comment, punctuation.definition.comment", "#5c6370", true),
        ("keyword, storage, keyword.control", "#c678dd", false),
        ("keyword.operator", "#56b6c2", false),
        ("string, punctuation.definition.string", "#98c379", false),
        ("constant.character.escape, string.regexp", "#56b6c2", false),
        (
            "constant.numeric, constant.language, constant.other",
            "#d19a66",
            false,
        ),
        (
            "variable.other.constant, support.constant",
            "#d19a66",
            false,
        ),
        (
            "entity.name.function, support.function, meta.function-call",
            "#61afef",
            false,
        ),
        (
            "entity.name.type, entity.name.class, entity.other.inherited-class, support.type, support.class, storage.type",
            "#e5c07b",
            false,
        ),
        (
            "variable, entity.name.tag, variable.other.member",
            "#e06c75",
            false,
        ),
        ("variable.parameter", "#abb2bf", false),
        ("entity.other.attribute-name", "#d19a66", false),
        ("punctuation, meta.brace", "#abb2bf", false),
        ("markup.heading", "#e06c75", false),
        ("markup.bold", "#d19a66", false),
        ("markup.italic", "#c678dd", true),
        ("markup.raw, markup.inline.raw", "#98c379", false),
        ("markup.underline.link, string.other.link", "#61afef", false),
        ("markup.inserted", "#98c379", false),
        ("markup.deleted, invalid", "#e06c75", false),
    ],
};

/// Atom's one-light-syntax.
const ONE_LIGHT: Palette = Palette {
    background: "#fafafa",
    foreground: "#383a42",
    scopes: &[
        ("comment, punctuation.definition.comment", "#a0a1a7", true),
        ("keyword, storage, keyword.control", "#a626a4", false),
        ("keyword.operator", "#0184bc", false),
        ("string, punctuation.definition.string", "#50a14f", false),
        ("constant.character.escape, string.regexp", "#0184bc", false),
        (
            "constant.numeric, constant.language, constant.other",
            "#986801",
            false,
        ),
        (
            "variable.other.constant, support.constant",
            "#986801",
            false,
        ),
        (
            "entity.name.function, support.function, meta.function-call",
            "#4078f2",
            false,
        ),
        (
            "entity.name.type, entity.name.class, entity.other.inherited-class, support.type, support.class, storage.type",
            "#c18401",
            false,
        ),
        (
            "variable, entity.name.tag, variable.other.member",
            "#e45649",
            false,
        ),
        ("variable.parameter", "#383a42", false),
        ("entity.other.attribute-name", "#986801", false),
        ("punctuation, meta.brace", "#383a42", false),
        ("markup.heading", "#e45649", false),
        ("markup.bold", "#986801", false),
        ("markup.italic", "#a626a4", true),
        ("markup.raw, markup.inline.raw", "#50a14f", false),
        ("markup.underline.link, string.other.link", "#4078f2", false),
        ("markup.inserted", "#50a14f", false),
        ("markup.deleted, invalid", "#e45649", false),
    ],
};

/// GitHub's Primer syntax colours, dark.
const GITHUB_DARK: Palette = Palette {
    background: "#0d1117",
    foreground: "#e6edf3",
    scopes: &[
        ("comment, punctuation.definition.comment", "#9198a1", false),
        (
            "keyword, storage, keyword.control, keyword.operator",
            "#ff7b72",
            false,
        ),
        (
            "string, punctuation.definition.string, string.regexp",
            "#a5d6ff",
            false,
        ),
        (
            "constant, variable.other.constant, support.constant",
            "#79c0ff",
            false,
        ),
        ("constant.character.escape", "#79c0ff", false),
        (
            "entity.name.function, support.function, meta.function-call",
            "#d2a8ff",
            false,
        ),
        (
            "entity.name.type, entity.name.class, entity.other.inherited-class, support.type, support.class",
            "#ffa657",
            false,
        ),
        ("storage.type", "#ff7b72", false),
        ("variable.parameter, variable.other", "#e6edf3", false),
        ("entity.name.tag", "#7ee787", false),
        ("entity.other.attribute-name", "#79c0ff", false),
        ("punctuation, meta.brace", "#e6edf3", false),
        ("markup.heading", "#79c0ff", false),
        ("markup.bold", "#e6edf3", false),
        ("markup.italic", "#e6edf3", true),
        ("markup.raw, markup.inline.raw", "#79c0ff", false),
        ("markup.underline.link, string.other.link", "#a5d6ff", false),
        ("markup.inserted", "#aff5b4", false),
        ("markup.deleted, invalid", "#ffdcd7", false),
    ],
};

/// GitHub's Primer syntax colours, light.
const GITHUB_LIGHT: Palette = Palette {
    background: "#ffffff",
    foreground: "#1f2328",
    scopes: &[
        ("comment, punctuation.definition.comment", "#59636e", false),
        (
            "keyword, storage, keyword.control, keyword.operator",
            "#cf222e",
            false,
        ),
        (
            "string, punctuation.definition.string, string.regexp",
            "#0a3069",
            false,
        ),
        (
            "constant, variable.other.constant, support.constant",
            "#0550ae",
            false,
        ),
        ("constant.character.escape", "#0550ae", false),
        (
            "entity.name.function, support.function, meta.function-call",
            "#6639ba",
            false,
        ),
        (
            "entity.name.type, entity.name.class, entity.other.inherited-class, support.type, support.class",
            "#953800",
            false,
        ),
        ("storage.type", "#cf222e", false),
        ("variable.parameter, variable.other", "#1f2328", false),
        ("entity.name.tag", "#116329", false),
        ("entity.other.attribute-name", "#0550ae", false),
        ("punctuation, meta.brace", "#1f2328", false),
        ("markup.heading", "#0550ae", false),
        ("markup.bold", "#1f2328", false),
        ("markup.italic", "#1f2328", true),
        ("markup.raw, markup.inline.raw", "#0550ae", false),
        ("markup.underline.link, string.other.link", "#0a3069", false),
        ("markup.inserted", "#116329", false),
        ("markup.deleted, invalid", "#82071e", false),
    ],
};

fn palette(name: &str) -> Option<&'static Palette> {
    match name {
        "one-dark" => Some(&ONE_DARK),
        "one-light" => Some(&ONE_LIGHT),
        "github-dark" => Some(&GITHUB_DARK),
        "github-light" => Some(&GITHUB_LIGHT),
        _ => None,
    }
}

fn color(hex: &str) -> Color {
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
    Color {
        r: byte(1),
        g: byte(3),
        b: byte(5),
        a: 0xff,
    }
}

/// The syntect theme for a syntax name; `None` for an unknown name.
pub fn theme(name: &str) -> Option<Theme> {
    let p = palette(name)?;
    Some(Theme {
        name: Some(name.to_owned()),
        author: None,
        settings: ThemeSettings {
            foreground: Some(color(p.foreground)),
            background: Some(color(p.background)),
            ..ThemeSettings::default()
        },
        scopes: p
            .scopes
            .iter()
            .map(|(scope, hex, italic)| ThemeItem {
                scope: ScopeSelectors::from_str(scope).expect("valid scope selectors"),
                style: StyleModifier {
                    foreground: Some(color(hex)),
                    background: None,
                    font_style: italic.then_some(FontStyle::ITALIC),
                },
            })
            .collect(),
    })
}

/// Prefixes every selector in syntect's CSS with `scope`.
pub(crate) fn scoped(css: &str, scope: &str) -> String {
    let mut out = String::with_capacity(css.len() * 2);
    for block in css.split_inclusive('}') {
        let Some((head, body)) = block.split_once('{') else {
            out.push_str(block);
            continue;
        };
        // Keep comments before the selectors as they are.
        let (comment, selectors) = match head.rfind("*/") {
            Some(end) => head.split_at(end + 2),
            None => ("", head),
        };
        out.push_str(comment);
        // The page's own CSS owns a code block's text and fill (the theme's
        // `textSecondary` and `sunken`); scoped, the syntax theme's base
        // `.syn-code` rule would outrank it.
        if selectors.trim() == ".syn-code" {
            continue;
        }
        let list: Vec<String> = selectors
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("{scope} {s}"))
            .collect();
        out.push('\n');
        out.push_str(&list.join(",\n"));
        out.push_str(" {");
        out.push_str(body);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_builds() {
        for name in NAMES {
            let theme = theme(name).unwrap();
            assert!(theme.scopes.len() > 10, "{name}");
        }
        assert!(theme("monokai").is_none());
    }

    #[test]
    fn the_page_css_carries_every_theme() {
        let css = crate::code::syntax_css();
        for name in NAMES {
            assert!(
                css.contains(&format!("html[data-azj-syntax=\"{name}\"] .syn-")),
                "{name}"
            );
        }
        // One Dark comments are Atom's grey, in italics.
        let at = css
            .find("html[data-azj-syntax=\"one-dark\"] .syn-comment")
            .unwrap();
        let rule = &css[at..at + css[at..].find('}').unwrap()];
        assert!(
            rule.contains("#5c6370") && rule.contains("italic"),
            "{rule}"
        );
        // Pages without a theme keep the light/dark pair.
        assert!(css.contains("html:not([data-azj-syntax]) .syn-"));
    }

    #[test]
    fn scoping_prefixes_each_selector() {
        let css = "/*\n * theme\n */\n.syn-code {\n color: #fff;\n}\n\n.syn-a, .syn-b.syn-c {\n color: #000;\n}\n";
        let out = scoped(css, "html[data-azj-syntax=\"x\"]");
        assert!(out.contains("/*\n * theme\n */"), "{out}");
        assert!(
            !out.contains(".syn-code"),
            "the base rule is the page's: {out}"
        );
        assert!(
            out.contains(
                "html[data-azj-syntax=\"x\"] .syn-a,\nhtml[data-azj-syntax=\"x\"] .syn-b.syn-c {"
            ),
            "{out}"
        );
        assert_eq!(out.matches('{').count(), 1);
        let code = scoped(
            ".syn-code {\n color: #fff;\n background-color: #000;\n}\n.syn-a {\n color: #111;\n}\n",
            "html",
        );
        assert!(!code.contains("#fff") && !code.contains("#000"), "{code}");
        assert!(code.contains("html .syn-a {"), "{code}");
    }
}
