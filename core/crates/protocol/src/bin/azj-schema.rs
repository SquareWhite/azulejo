//! Writes the API schema: `azj-schema [path]` (stdout when no path is given).

fn main() {
    let text = azulejo_protocol::api_schema_text();
    match std::env::args().nth(1) {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, text) {
                eprintln!("azj-schema: can't write {path}: {e}");
                std::process::exit(1);
            }
        }
        None => print!("{text}"),
    }
}
