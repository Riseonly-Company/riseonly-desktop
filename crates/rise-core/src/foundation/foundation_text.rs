use unicode_segmentation::UnicodeSegmentation;

/// number of grapheme clusters
pub fn character_count(s: &str) -> usize {
    s.graphemes(true).count()
}

/// first n characters or the whole string if shorter
pub fn character_prefix(s: &str, n: usize) -> &str {
    if n == 0 {
        return "";
    }
    match s.grapheme_indices(true).nth(n) {
        Some((end, _)) => &s[..end],
        None => s,
    }
}

/// last n characters or the whole string if shorter
pub fn character_suffix(s: &str, n: usize) -> &str {
    if n == 0 {
        return "";
    }
    match s.grapheme_indices(true).rev().nth(n - 1) {
        Some((start, _)) => &s[start..],
        None => s,
    }
}

/// everything after the first n characters, empty if shorter
pub fn drop_first_characters(s: &str, n: usize) -> &str {
    if n == 0 {
        return s;
    }
    match s.grapheme_indices(true).nth(n) {
        Some((start, _)) => &s[start..],
        None => "",
    }
}

/// everything before the last n characters, empty if shorter
pub fn drop_last_characters(s: &str, n: usize) -> &str {
    if n == 0 {
        return s;
    }
    match s.grapheme_indices(true).rev().nth(n - 1) {
        Some((start, _)) => &s[..start],
        None => "",
    }
}

/// the characters of s in order, each as a string slice
pub fn characters(s: &str) -> impl DoubleEndedIterator<Item = &str> + '_ {
    s.graphemes(true)
}

/// byte offsets of every character boundary, 0 and s.len() included
pub fn character_boundaries(s: &str) -> Vec<usize> {
    let mut out: Vec<usize> = s.grapheme_indices(true).map(|(i, _)| i).collect();
    out.push(s.len());
    out
}

/// true for a space separator char or tab
pub fn is_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

/// true for line break chars, U+000A to U+000D and U+0085 and U+2028 and U+2029
pub fn is_newline(c: char) -> bool {
    matches!(c, '\u{000A}'..='\u{000D}' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

pub fn is_whitespace_or_newline(c: char) -> bool {
    is_whitespace(c) || is_newline(c)
}

/// trims chars matching the predicate from both ends
pub fn trimming_characters(s: &str, in_set: impl Fn(char) -> bool) -> &str {
    s.trim_matches(in_set)
}

/// trims whitespace and newlines from both ends
pub fn trimming_whitespaces_and_newlines(s: &str) -> &str {
    trimming_characters(s, is_whitespace_or_newline)
}

/// trims whitespace from both ends, line breaks stay
pub fn trimming_whitespaces(s: &str) -> &str {
    trimming_characters(s, is_whitespace)
}

/// trims newlines from both ends
pub fn trimming_newlines(s: &str) -> &str {
    trimming_characters(s, is_newline)
}
