//! A small Rust file for golden tests.

use std::collections::HashMap;

/// Counts words; `<` and `&` must be escaped.
pub fn count(text: &str) -> HashMap<&str, usize> {
    let mut out = HashMap::new();
    for word in text.split_whitespace() {
        *out.entry(word).or_insert(0) += 1;
    }
    out
}
