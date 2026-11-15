//! The keymap for people: `config.keys` (every action, its context, default
//! and current chords, and who set it), `config.defaults` (a commented
//! `[keys]` template to paste) and the reference table in
//! `docs/keybindings.md` (generated; `just lint` checks it's current).

use std::fmt::Write as _;

use azulejo_protocol::KeyBinding;

use crate::action::{NameSpec, names};
use crate::catalogue::describe;
use crate::keymap::Written;
use crate::{Action, Chord, Config, Key, KeyContext, Platform};

/// The context column: `global` for the platform-wide bindings.
pub const GLOBAL: &str = "global";

/// Chords in config syntax, as written: indexed actions fold their digits
/// back into ranges (`ctrl+1..8`).
pub fn chord_patterns(bindings: &[(Chord, Action)], platform: Platform) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // Digit chords of indexed actions, grouped by modifiers in first-seen order.
    let mut digits: Vec<(crate::Mods, Vec<u8>)> = Vec::new();
    for (chord, action) in bindings {
        match (action.index(), chord.key) {
            (Some(_), Key::Digit(d)) => match digits.iter_mut().find(|(m, _)| *m == chord.mods) {
                Some((_, ds)) => ds.push(d),
                None => digits.push((chord.mods, vec![d])),
            },
            _ => {
                let text = chord.display(platform);
                if !out.contains(&text) {
                    out.push(text);
                }
            }
        }
    }
    for (mods, mut ds) in digits {
        ds.sort_unstable();
        ds.dedup();
        let mut i = 0;
        while i < ds.len() {
            let mut j = i;
            while j + 1 < ds.len() && ds[j + 1] == ds[j] + 1 {
                j += 1;
            }
            let first = Chord::new(mods, Key::Digit(ds[i])).display(platform);
            out.push(if j > i {
                format!("{first}..{}", ds[j])
            } else {
                first
            });
            i = j + 1;
        }
    }
    out
}

fn context_label(context: Option<KeyContext>) -> &'static str {
    context.map_or(GLOBAL, KeyContext::name)
}

fn find<'a>(list: &'a [Written], context: Option<KeyContext>, name: &str) -> Option<&'a Written> {
    list.iter().find(|w| w.context == context && w.name == name)
}

/// The sections an action shows up in: wherever either list writes it, else
/// its home context (or global).
fn sections(lists: &[&[Written]], name: &str, spec: &NameSpec) -> Vec<Option<KeyContext>> {
    let mut out: Vec<Option<KeyContext>> = std::iter::once(None)
        .chain(KeyContext::ALL.into_iter().map(Some))
        .filter(|ctx| lists.iter().any(|l| find(l, *ctx, name).is_some()))
        .collect();
    if out.is_empty() {
        out.push(match spec {
            NameSpec::Plain(action) => action.context(),
            NameSpec::Indexed(_) => None,
        });
    }
    out
}

/// Every action with its context, default chords, current chords and
/// source (`config.keys`). Prefix bindings are listed under `prefix` while
/// prefix mode is on.
pub fn key_listing(config: &Config) -> Vec<KeyBinding> {
    let platform = config.keymap.platform();
    let defaults = Config::defaults(platform);
    let cur = config.keymap.written();
    let def = defaults.keymap.written();
    let mut out = Vec::new();
    for (name, spec) in names() {
        for ctx in sections(&[cur, def], &name, &spec) {
            let c = find(cur, ctx, &name);
            let d = find(def, ctx, &name);
            out.push(KeyBinding {
                action: name.clone(),
                context: context_label(ctx).to_owned(),
                title: describe(&name, &spec),
                default: d.map_or_else(Vec::new, |w| chord_patterns(&w.bindings, platform)),
                current: c.map_or_else(Vec::new, |w| chord_patterns(&w.bindings, platform)),
                source: if c.is_some_and(|w| w.user) {
                    "user"
                } else {
                    "default"
                }
                .to_owned(),
            });
        }
    }
    if config.keymap.prefix_enabled() {
        let cur = config.keymap.prefix_written();
        let def = defaults.keymap.prefix_written();
        for (name, spec) in names() {
            let c = find(cur, None, &name);
            let d = find(def, None, &name);
            if c.is_none_or(|w| w.bindings.is_empty()) && d.is_none_or(|w| w.bindings.is_empty()) {
                continue;
            }
            out.push(KeyBinding {
                action: name.clone(),
                context: "prefix".to_owned(),
                title: describe(&name, &spec),
                default: d.map_or_else(Vec::new, |w| chord_patterns(&w.bindings, platform)),
                current: c.map_or_else(Vec::new, |w| chord_patterns(&w.bindings, platform)),
                source: if c.is_some_and(|w| w.user) {
                    "user"
                } else {
                    "default"
                }
                .to_owned(),
            });
        }
    }
    out
}

/// A TOML value for `chords`: `"x"`, `["x", "y"]`, or `""` when unbound.
fn toml_value(chords: &[String]) -> String {
    let quote = |c: &String| format!("\"{}\"", c.replace('\\', "\\\\").replace('"', "\\\""));
    match chords {
        [] => "\"\"".to_owned(),
        [one] => quote(one),
        many => format!(
            "[{}]",
            many.iter().map(quote).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Rows of one template section: (name, chords, title).
fn section_rows(
    def: &[Written],
    context: Option<KeyContext>,
    platform: Platform,
) -> Vec<(String, Vec<String>, String)> {
    let mut rows = Vec::new();
    for (name, spec) in names() {
        let home = match &spec {
            NameSpec::Plain(action) => action.context(),
            NameSpec::Indexed(_) => None,
        };
        let written = find(def, context, &name);
        // Global: every action that works anywhere. A context: what the
        // defaults bind there, plus every action that only works there.
        let listed = match context {
            None => home.is_none(),
            Some(_) => written.is_some() || home == context,
        };
        if !listed {
            continue;
        }
        let chords = written.map_or_else(Vec::new, |w| chord_patterns(&w.bindings, platform));
        rows.push((name.clone(), chords, describe(&name, &spec)));
    }
    rows
}

fn push_rows(out: &mut String, rows: &[(String, Vec<String>, String)]) {
    let lines: Vec<(String, &str, bool)> = rows
        .iter()
        .map(|(name, chords, title)| {
            (
                format!("# {name} = {}", toml_value(chords)),
                title.as_str(),
                chords.is_empty(),
            )
        })
        .collect();
    let width = lines
        .iter()
        .map(|(l, ..)| l.chars().count())
        .max()
        .unwrap_or(0);
    for (line, title, unbound) in lines {
        let pad = width - line.chars().count();
        let note = if unbound { " (unbound)" } else { "" };
        let _ = writeln!(out, "{line}{}  # {title}{note}", " ".repeat(pad));
    }
}

/// A commented `[keys]` template with every action and its default chords
/// on `platform` (`config.defaults`, and the file "Open config" creates).
pub fn keys_template(platform: Platform) -> String {
    let defaults = Config::defaults(platform);
    let def = defaults.keymap.written();
    let p = platform.name();
    let mut out = String::new();
    let _ = write!(
        out,
        "\
# Azulejo config (~/.config/azulejo/config.toml). It reloads on save.
#
# Every key binding, commented out with its default. Uncomment a line and
# change it to rebind: it replaces that action's default chords in its
# section. \"\" or \"none\" unbinds. A context section ([keys.{p}.browser], …)
# wins over [keys.{p}] while that context has the keyboard. `azj config keys`
# shows what's in effect; docs/keybindings.md has the reference.
#
# Chords name the physical key: ctrl, alt, shift and {cmd} plus a key such as
# a, 1, f5, enter, escape, tab, space, up, pagedown, minus, equal, [, ], /,
# period or comma. switch_workspace and switch_tab take digit ranges
# (\"{cmd}+1..8\").
",
        cmd = match platform {
            Platform::Macos => "cmd",
            Platform::Linux => "super",
        }
    );
    let _ = writeln!(out, "\n[keys.{p}]");
    push_rows(&mut out, &section_rows(def, None, platform));
    for context in KeyContext::ALL {
        let rows = section_rows(def, Some(context), platform);
        if rows.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\n[keys.{p}.{}]", context.name());
        push_rows(&mut out, &rows);
    }

    // Prefix mode, off by default.
    let prefix = defaults.keymap.prefix_chord().map(|c| c.display(platform));
    let _ = write!(
        out,
        "\n# Optional herdr-style prefix mode: the prefix key, then one of these.\n\
         # [prefix]\n\
         # enabled = true\n\
         # key = \"{}\"\n\
         # [prefix.keys]\n",
        prefix.unwrap_or_default()
    );
    let pdef = defaults.keymap.prefix_written();
    let rows: Vec<_> = names()
        .into_iter()
        .filter_map(|(name, spec)| {
            let w = find(pdef, None, &name)?;
            let chords: Vec<String> = chord_patterns(&w.bindings, platform)
                .into_iter()
                .map(|c| format!("prefix+{c}"))
                .collect();
            Some((name.clone(), chords, describe(&name, &spec)))
        })
        .collect();
    let mut block = String::new();
    push_rows(&mut block, &rows);
    // `# # name = …` reads badly; these lines are already comments.
    out.push_str(&block);
    out
}

fn md_chords(chords: &[String]) -> String {
    if chords.is_empty() {
        "unbound".to_owned()
    } else {
        chords
            .iter()
            .map(|c| format!("`{}`", c.replace('|', "\\|")))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The reference tables for `docs/keybindings.md`: every action per context
/// with its macOS and Linux defaults, then prefix mode.
pub fn reference_markdown() -> String {
    let mac = Config::defaults(Platform::Macos);
    let linux = Config::defaults(Platform::Linux);
    let (m, l) = (mac.keymap.written(), linux.keymap.written());
    let mut out = String::new();
    out.push_str("| Action | Context | macOS | Linux | What it does |\n");
    out.push_str("| --- | --- | --- | --- | --- |\n");
    for (name, spec) in names() {
        for ctx in sections(&[m, l], &name, &spec) {
            let chords = |list: &[Written], platform| {
                find(list, ctx, &name)
                    .map_or_else(Vec::new, |w| chord_patterns(&w.bindings, platform))
            };
            let _ = writeln!(
                out,
                "| `{name}` | {} | {} | {} | {} |",
                context_label(ctx),
                md_chords(&chords(m, Platform::Macos)),
                md_chords(&chords(l, Platform::Linux)),
                describe(&name, &spec).replace('|', "\\|"),
            );
        }
    }
    let (pm, pl) = (mac.keymap.prefix_written(), linux.keymap.prefix_written());
    let key = |c: &Config, p| {
        c.keymap
            .prefix_chord()
            .map_or_else(String::new, |k| k.display(p))
    };
    let _ = write!(
        out,
        "\nPrefix mode (`[prefix] enabled = true`): press the prefix (`{}` on macOS, `{}` on Linux), then:\n\n",
        key(&mac, Platform::Macos),
        key(&linux, Platform::Linux)
    );
    out.push_str("| Action | Then | What it does |\n| --- | --- | --- |\n");
    for (name, spec) in names() {
        let Some(w) = find(pm, None, &name).or_else(|| find(pl, None, &name)) else {
            continue;
        };
        let _ = writeln!(
            out,
            "| `{name}` | {} | {} |",
            md_chords(&chord_patterns(&w.bindings, Platform::Macos)),
            describe(&name, &spec).replace('|', "\\|")
        );
    }
    out
}

/// The markers around the generated part of `docs/keybindings.md`.
pub const DOC_BEGIN: &str = "<!-- keys:begin (generated by `just keys-doc` from shared/config/keymap.default.toml; don't edit by hand) -->";
pub const DOC_END: &str = "<!-- keys:end -->";

/// `doc` with the generated part replaced by the current reference. `None`
/// when the markers are missing.
pub fn splice_reference(doc: &str) -> Option<String> {
    let start = doc.find(DOC_BEGIN)? + DOC_BEGIN.len();
    let end = start + doc[start..].find(DOC_END)?;
    Some(format!(
        "{}\n\n{}\n{}",
        &doc[..start],
        reference_markdown(),
        &doc[end..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row<'a>(list: &'a [KeyBinding], action: &str, context: &str) -> &'a KeyBinding {
        list.iter()
            .find(|k| k.action == action && k.context == context)
            .unwrap_or_else(|| panic!("no {action} in {context}"))
    }

    #[test]
    fn listing_covers_every_action_and_marks_user_overrides() {
        let config = Config::from_toml(
            "[keys.macos.browser]\nbrowser_back = \"alt+left\"\n[keys]\ngoto = \"cmd+p\"\nquit = \"\"\n",
            None,
            Platform::Macos,
        );
        let list = key_listing(&config);
        for (name, _) in names() {
            assert!(list.iter().any(|k| k.action == name), "{name} missing");
        }
        let back = row(&list, "browser_back", "browser");
        assert_eq!(back.default, ["cmd+bracketleft"]);
        assert_eq!(back.current, ["alt+left"]);
        assert_eq!(back.source, "user");
        let goto = row(&list, "goto", "global");
        assert!(goto.default.is_empty());
        assert_eq!(goto.current, ["cmd+p"]);
        let quit = row(&list, "quit", "global");
        assert_eq!(quit.default, ["cmd+q"]);
        assert!(quit.current.is_empty(), "unbound by the user");
        assert_eq!(quit.source, "user");
        let tab = row(&list, "switch_tab", "global");
        assert_eq!(tab.current, ["ctrl+1..8"]);
        assert_eq!(tab.source, "default");
        assert_eq!(tab.title, "Switch to tab N");
        // Context-only actions sit in their context even when unbound.
        assert!(row(&list, "take_over", "terminal").current.is_empty());
        assert_eq!(row(&list, "palette_next", "palette").current.len(), 3);
        assert_eq!(
            row(&list, "resize_tile_left", "resize").current,
            ["h", "left"]
        );
        assert!(
            !list.iter().any(|k| k.context == "prefix"),
            "prefix mode is off"
        );
    }

    #[test]
    fn listing_shows_prefix_bindings_when_prefix_mode_is_on() {
        let config = Config::from_toml("[prefix]\nenabled = true\n", None, Platform::Linux);
        let list = key_listing(&config);
        assert_eq!(row(&list, "switch_tab", "prefix").current, ["1..9"]);
    }

    #[test]
    fn template_parses_as_defaults_and_lists_every_action() {
        for platform in Platform::ALL {
            let text = keys_template(platform);
            // Commented out, it's the defaults exactly.
            let config = Config::from_toml(&text, None, platform);
            assert!(config.diagnostics.is_empty(), "{:#?}", config.diagnostics);
            // Uncommented, too (the key sections; prefix mode stays off).
            let keys = &text[..text.find("\n# Optional herdr-style").unwrap()];
            let live: String = keys
                .lines()
                .map(|l| match l.strip_prefix("# ") {
                    Some(rest) if rest.contains(" = ") && !rest.starts_with('[') => rest.to_owned(),
                    _ => l.to_owned(),
                })
                .collect::<Vec<_>>()
                .join("\n");
            let uncommented = Config::from_toml(&live, None, platform);
            assert!(
                uncommented.diagnostics.is_empty(),
                "{platform:?}: {:#?}",
                uncommented.diagnostics
            );
            for (name, _) in names() {
                assert!(text.contains(&format!("# {name} = ")), "{name} missing");
            }
            let defaults = Config::defaults(platform);
            for ctx in std::iter::once(None).chain(KeyContext::ALL.into_iter().map(Some)) {
                let a: std::collections::HashSet<_> = uncommented.keymap.bindings(ctx).collect();
                let b: std::collections::HashSet<_> = defaults.keymap.bindings(ctx).collect();
                assert_eq!(a, b, "{platform:?} {ctx:?}");
            }
        }
    }

    /// The example in docs/keybindings.md ("Configuring") is a clean config.
    #[test]
    fn the_documented_example_is_clean() {
        let doc = include_str!("../../../../docs/keybindings.md");
        let start = doc.find("```toml\n").unwrap() + "```toml\n".len();
        let example = &doc[start..start + doc[start..].find("```").unwrap()];
        let config = Config::from_toml(example, None, Platform::Macos);
        assert!(config.diagnostics.is_empty(), "{:#?}", config.diagnostics);
        assert_eq!(
            config.keymap.chords_for(Action::Quit, None),
            Vec::<Chord>::new()
        );
    }

    #[test]
    fn reference_lists_both_platforms() {
        let md = reference_markdown();
        assert!(
            md.contains("| `browser_back` | browser | `cmd+bracketleft` | `alt+left` |"),
            "{md}"
        );
        assert!(md.contains("| `toggle_sidebar` | terminal | `cmd+a` | `super+a` |"));
        assert!(md.contains("| `last_tab` | global | unbound | unbound |"));
        let doc = format!("intro\n{DOC_BEGIN}\nold\n{DOC_END}\nrest\n");
        let spliced = splice_reference(&doc).unwrap();
        assert!(spliced.starts_with("intro\n"));
        assert!(spliced.ends_with(&format!("{DOC_END}\nrest\n")));
        assert!(!spliced.contains("\nold\n"));
        assert_eq!(splice_reference(&spliced).unwrap(), spliced, "stable");
    }
}
