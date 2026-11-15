//! Regenerates the keybinding reference in `docs/keybindings.md` from the
//! default keymap: `azj-keys-doc [--check] [path]` (default
//! `docs/keybindings.md`). With `--check` it changes nothing and fails when
//! the file is out of date (`just lint`).

use std::process::exit;

fn main() {
    let mut check = false;
    let mut path = "docs/keybindings.md".to_owned();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--check" => check = true,
            other => path = other.to_owned(),
        }
    }
    let doc = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("azj-keys-doc: can't read {path}: {e}");
        exit(1)
    });
    let Some(fresh) = azulejo_config::splice_reference(&doc) else {
        eprintln!(
            "azj-keys-doc: {path} needs the markers\n  {}\n  {}",
            azulejo_config::DOC_BEGIN,
            azulejo_config::DOC_END
        );
        exit(1)
    };
    if fresh == doc {
        return;
    }
    if check {
        eprintln!(
            "azj-keys-doc: {path} is out of date with shared/config/keymap.default.toml; run `just keys-doc`"
        );
        exit(1);
    }
    if let Err(e) = std::fs::write(&path, fresh) {
        eprintln!("azj-keys-doc: can't write {path}: {e}");
        exit(1);
    }
}
