//! Character classes and widths the detector needs, matching what the
//! macOS shell's Foundation used before the port (`CharacterSet.whitespaces`,
//! `CharacterSet.controlCharacters`, `Unicode.Scalar.Properties
//! .isEmojiPresentation`, grapheme clusters for `String.count`), so a Linux
//! shell finds the same items.

use unicode_segmentation::UnicodeSegmentation;

/// Horizontal whitespace: Unicode `Zs` plus tab and U+200B
/// (Foundation's `CharacterSet.whitespaces`).
pub fn is_hspace(c: char) -> bool {
    matches!(
        c as u32,
        0x09 | 0x20 | 0xA0 | 0x1680 | 0x2000..=0x200B | 0x202F | 0x205F | 0x3000
    )
}

/// `Cc` and `Cf`: characters that must never reach a shell as input.
pub fn is_control(c: char) -> bool {
    matches!(c as u32,
        0x00..=0x1F | 0x7F..=0x9F | 0xAD | 0x600..=0x605 | 0x61C | 0x6DD | 0x70F
        | 0x890..=0x891 | 0x8E2 | 0x180E | 0x200B..=0x200F | 0x202A..=0x202E
        | 0x2060..=0x2064 | 0x2066..=0x206F | 0xFEFF | 0xFFF9..=0xFFFB | 0x110BD
        | 0x110CD | 0x13430..=0x1343F | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A
        | 0xE0001 | 0xE0020..=0xE007F)
}

/// Unicode `Emoji_Presentation`: drawn two cells wide by default.
fn emoji_presentation(v: u32) -> bool {
    const RANGES: &[(u32, u32)] = &[
        (0x231A, 0x231B),
        (0x23E9, 0x23EC),
        (0x23F0, 0x23F0),
        (0x23F3, 0x23F3),
        (0x25FD, 0x25FE),
        (0x2614, 0x2615),
        (0x2648, 0x2653),
        (0x267F, 0x267F),
        (0x2693, 0x2693),
        (0x26A1, 0x26A1),
        (0x26AA, 0x26AB),
        (0x26BD, 0x26BE),
        (0x26C4, 0x26C5),
        (0x26CE, 0x26CE),
        (0x26D4, 0x26D4),
        (0x26EA, 0x26EA),
        (0x26F2, 0x26F3),
        (0x26F5, 0x26F5),
        (0x26FA, 0x26FA),
        (0x26FD, 0x26FD),
        (0x2705, 0x2705),
        (0x270A, 0x270B),
        (0x2728, 0x2728),
        (0x274C, 0x274C),
        (0x274E, 0x274E),
        (0x2753, 0x2755),
        (0x2757, 0x2757),
        (0x2795, 0x2797),
        (0x27B0, 0x27B0),
        (0x27BF, 0x27BF),
        (0x2B1B, 0x2B1C),
        (0x2B50, 0x2B50),
        (0x2B55, 0x2B55),
        (0x1F004, 0x1F004),
        (0x1F0CF, 0x1F0CF),
        (0x1F18E, 0x1F18E),
        (0x1F191, 0x1F19A),
        (0x1F1E6, 0x1F1FF),
        (0x1F201, 0x1F201),
        (0x1F21A, 0x1F21A),
        (0x1F22F, 0x1F22F),
        (0x1F232, 0x1F236),
        (0x1F238, 0x1F23A),
        (0x1F250, 0x1F251),
        (0x1F300, 0x1F320),
        (0x1F32D, 0x1F335),
        (0x1F337, 0x1F37C),
        (0x1F37E, 0x1F393),
        (0x1F3A0, 0x1F3CA),
        (0x1F3CF, 0x1F3D3),
        (0x1F3E0, 0x1F3F0),
        (0x1F3F4, 0x1F3F4),
        (0x1F3F8, 0x1F43E),
        (0x1F440, 0x1F440),
        (0x1F442, 0x1F4FC),
        (0x1F4FF, 0x1F53D),
        (0x1F54B, 0x1F54E),
        (0x1F550, 0x1F567),
        (0x1F57A, 0x1F57A),
        (0x1F595, 0x1F596),
        (0x1F5A4, 0x1F5A4),
        (0x1F5FB, 0x1F64F),
        (0x1F680, 0x1F6C5),
        (0x1F6CC, 0x1F6CC),
        (0x1F6D0, 0x1F6D2),
        (0x1F6D5, 0x1F6D8),
        (0x1F6DC, 0x1F6DF),
        (0x1F6EB, 0x1F6EC),
        (0x1F6F4, 0x1F6FC),
        (0x1F7E0, 0x1F7EB),
        (0x1F7F0, 0x1F7F0),
        (0x1F90C, 0x1F93A),
        (0x1F93C, 0x1F945),
        (0x1F947, 0x1F9FF),
        (0x1FA70, 0x1FA7C),
        (0x1FA80, 0x1FA8A),
        (0x1FA8E, 0x1FAC6),
        (0x1FAC8, 0x1FAC8),
        (0x1FACD, 0x1FADC),
        (0x1FADF, 0x1FAEA),
        (0x1FAEF, 0x1FAF8),
    ];
    RANGES
        .binary_search_by(|&(lo, hi)| {
            if hi < v {
                std::cmp::Ordering::Less
            } else if lo > v {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Approximate terminal cell width, one grapheme at a time: wide East
/// Asian ranges and emoji count two. Good enough for wrap decisions; exact
/// overlay geometry still comes from the terminal.
pub fn cells(text: &str) -> usize {
    if text.is_ascii() {
        return text.len();
    }
    text.graphemes(true)
        .map(|g| {
            let v = g.chars().next().map_or(0, |c| c as u32);
            let wide = matches!(v,
                0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF
                | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x20000..=0x3FFFD)
                || emoji_presentation(v);
            if wide { 2 } else { 1 }
        })
        .sum()
}

/// User-perceived characters (Swift's `String.count`).
pub fn graphemes(text: &str) -> usize {
    if text.is_ascii() {
        text.len()
    } else {
        text.graphemes(true).count()
    }
}

/// Leading spaces and tabs, in bytes (= UTF-16 units: both are ASCII).
pub fn leading_ws(text: &str) -> usize {
    text.bytes()
        .take_while(|&b| b == b' ' || b == b'\t')
        .count()
}

/// Leading spaces and tabs, as an indent.
pub fn indentation(text: &str) -> usize {
    leading_ws(text)
}

/// Whether every character is whitespace (Swift's `Character.isWhitespace`).
pub fn is_blank(text: &str) -> bool {
    text.chars().all(char::is_whitespace)
}

/// Trims `CharacterSet.whitespaces` from both ends.
pub fn trim_hspace(text: &str) -> &str {
    text.trim_matches(is_hspace)
}

/// Words split on whitespace (Swift's `split(whereSeparator: \.isWhitespace)`).
pub fn words(text: &str) -> impl DoubleEndedIterator<Item = &str> {
    text.split(char::is_whitespace).filter(|w| !w.is_empty())
}

/// The character ending at byte `end`.
pub fn char_before(text: &str, end: usize) -> Option<char> {
    text[..end].chars().next_back()
}

/// The character starting at byte `at`.
pub fn char_at(text: &str, at: usize) -> Option<char> {
    text.get(at..).and_then(|s| s.chars().next())
}

/// UTF-16 length of a string.
pub fn utf16_len(text: &str) -> usize {
    if text.is_ascii() {
        text.len()
    } else {
        text.chars().map(char::len_utf16).sum()
    }
}

/// UTF-16 offset of a byte offset.
pub fn utf16_at(text: &str, byte: usize) -> usize {
    utf16_len(&text[..byte])
}

/// `NSString.pathExtension`: the last component's text after its last
/// dot, if the dot neither starts nor ends it.
pub fn path_extension(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    let last = trimmed.rsplit('/').next().unwrap_or("");
    match last.rfind('.') {
        Some(0) | None => "",
        Some(i) => &last[i + 1..],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths() {
        assert_eq!(cells("abc"), 3);
        assert_eq!(cells("界"), 2);
        assert_eq!(cells("👩‍💻"), 2);
        assert_eq!(cells("e\u{301}"), 1);
        assert_eq!(graphemes("界👩‍💻e\u{301}"), 3);
    }

    #[test]
    fn extensions() {
        for (path, ext) in [
            ("./build.sh", "sh"),
            (".bashrc", ""),
            ("foo.", ""),
            ("a/b.sh/", "sh"),
            ("x.tar.gz", "gz"),
            ("~/bin/tool", ""),
            ("a.b/c", ""),
            ("./.sh", ""),
            ("a.SH", "SH"),
        ] {
            assert_eq!(path_extension(path), ext, "{path}");
        }
    }
}
