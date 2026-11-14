//! Natural, case-insensitive name order: `file2` before `file10`.

use std::cmp::Ordering;

/// Compares names the way people read them: runs of digits by value, other
/// characters case-insensitively. Ties fall back to plain byte order so the
/// order is total and stable.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.char_indices().peekable(), b.char_indices().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some((i, c)), Some((j, d))) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let n = digits(a, i);
                let m = digits(b, j);
                let order = cmp_numbers(&a[i..i + n], &b[j..j + m]);
                if order != Ordering::Equal {
                    return order;
                }
                for _ in 0..n {
                    x.next();
                }
                for _ in 0..m {
                    y.next();
                }
            }
            (Some((_, c)), Some((_, d))) => {
                let order = c.to_lowercase().cmp(d.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// Length in bytes of the ASCII digit run starting at `at`.
fn digits(s: &str, at: usize) -> usize {
    s[at..].bytes().take_while(u8::is_ascii_digit).count()
}

/// Compares two digit runs by value without parsing (they may be long).
fn cmp_numbers(a: &str, b: &str) -> Ordering {
    let a = a.trim_start_matches('0');
    let b = b.trim_start_matches('0');
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::natural_cmp;

    fn sorted(mut v: Vec<&str>) -> Vec<&str> {
        v.sort_by(|a, b| natural_cmp(a, b));
        v
    }

    #[test]
    fn numbers_by_value_and_case_insensitive() {
        assert_eq!(
            sorted(vec!["file10", "File2", "file1", "a", "B", "file02"]),
            vec!["a", "B", "file1", "File2", "file02", "file10"]
        );
    }

    #[test]
    fn total_order_on_ties() {
        assert_eq!(sorted(vec!["b", "B"]), vec!["B", "b"]);
        assert_eq!(sorted(vec!["x01", "x1"]), vec!["x01", "x1"]);
        assert_eq!(
            sorted(vec!["n99999999999999999999", "n100000000000000000000"]),
            vec!["n99999999999999999999", "n100000000000000000000"]
        );
    }
}
