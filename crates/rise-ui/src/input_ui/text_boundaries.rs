use std::ops::Range;

const ZERO_WIDTH_JOINER: char = '\u{200D}';

/// The nearest char boundary at or before `offset`, clamped to the string.
///
/// Every offset that crosses the boundary of this module — the public API, the
/// IME handler, the element — passes through here first. Slicing a `String` off
/// a char boundary panics, and Cyrillic, CJK and emoji are where that happens.
pub fn clamp_to_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

pub fn next_grapheme_boundary(text: &str, offset: usize) -> usize {
    let offset = clamp_to_char_boundary(text, offset);
    if offset >= text.len() {
        return text.len();
    }

    let rest = &text[offset..];
    let first = rest.chars().next().expect("offset < len");
    let mut cursor = first.len_utf8();

    if first == '\r' && rest[cursor..].starts_with('\n') {
        return offset + cursor + 1;
    }

    if is_regional_indicator(first)
        && let Some(next) = rest[cursor..].chars().next()
        && is_regional_indicator(next)
    {
        cursor += next.len_utf8();
    }

    while let Some(next) = rest[cursor..].chars().next() {
        if is_extending(next) {
            cursor += next.len_utf8();
            continue;
        }
        if next == ZERO_WIDTH_JOINER {
            let after_joiner = cursor + next.len_utf8();
            let Some(joined) = rest[after_joiner..].chars().next() else {
                break;
            };
            cursor = after_joiner + joined.len_utf8();
            continue;
        }
        break;
    }

    offset + cursor
}

pub fn previous_grapheme_boundary(text: &str, offset: usize) -> usize {
    let offset = clamp_to_char_boundary(text, offset);
    let mut cursor = offset;

    loop {
        let Some((start, preceding)) = char_before(text, cursor) else {
            return 0;
        };

        if is_extending(preceding) || preceding == ZERO_WIDTH_JOINER {
            cursor = start;
            continue;
        }

        // A joiner sits *before* its right-hand base, so a base is only the head
        // of its cluster once we have looked past it. Without this the walk
        // stops inside a family emoji.
        if let Some((_, before_base)) = char_before(text, start)
            && before_base == ZERO_WIDTH_JOINER
        {
            cursor = start;
            continue;
        }

        if preceding == '\n'
            && let Some((carriage_return_start, '\r')) = char_before(text, start)
        {
            return carriage_return_start;
        }

        if is_regional_indicator(preceding) {
            let mut run = 0usize;
            let mut scan = start;
            while let Some((preceding_start, earlier)) = char_before(text, scan) {
                if !is_regional_indicator(earlier) {
                    break;
                }
                run += 1;
                scan = preceding_start;
            }
            if run % 2 == 1 {
                return char_before(text, start).map_or(0, |(pair_start, _)| pair_start);
            }
        }

        return start;
    }
}

pub fn grapheme_count(text: &str) -> usize {
    graphemes(text).count()
}

pub fn graphemes(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut cursor = 0usize;
    std::iter::from_fn(move || {
        if cursor >= text.len() {
            return None;
        }
        let start = cursor;
        cursor = next_grapheme_boundary(text, cursor);
        Some((start, &text[start..cursor]))
    })
}

/// The byte offset of the `index`-th grapheme, or `text.len()` past the end.
pub fn grapheme_offset(text: &str, index: usize) -> usize {
    graphemes(text)
        .nth(index)
        .map_or(text.len(), |(start, _)| start)
}

pub fn grapheme_index(text: &str, offset: usize) -> usize {
    let offset = clamp_to_char_boundary(text, offset);
    graphemes(text)
        .take_while(|(start, _)| *start < offset)
        .count()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CharClass {
    Whitespace,
    Word,
    Punctuation,
}

fn class_of(character: char) -> CharClass {
    if character.is_whitespace() {
        CharClass::Whitespace
    } else if character.is_alphanumeric() || character == '_' {
        CharClass::Word
    } else {
        CharClass::Punctuation
    }
}

pub fn next_word_boundary(text: &str, offset: usize) -> usize {
    let mut cursor = clamp_to_char_boundary(text, offset);
    while let Some(character) = text[cursor..].chars().next() {
        if class_of(character) != CharClass::Whitespace {
            break;
        }
        cursor += character.len_utf8();
    }
    let Some(first) = text[cursor..].chars().next() else {
        return cursor;
    };
    let class = class_of(first);
    while let Some(character) = text[cursor..].chars().next() {
        if class_of(character) != class {
            break;
        }
        cursor += character.len_utf8();
    }
    cursor
}

pub fn previous_word_boundary(text: &str, offset: usize) -> usize {
    let mut cursor = clamp_to_char_boundary(text, offset);
    while let Some((start, character)) = char_before(text, cursor) {
        if class_of(character) != CharClass::Whitespace {
            break;
        }
        cursor = start;
    }
    let Some((_, last)) = char_before(text, cursor) else {
        return cursor;
    };
    let class = class_of(last);
    while let Some((start, character)) = char_before(text, cursor) {
        if class_of(character) != class {
            break;
        }
        cursor = start;
    }
    cursor
}

pub fn line_start(text: &str, offset: usize) -> usize {
    let offset = clamp_to_char_boundary(text, offset);
    text[..offset].rfind('\n').map_or(0, |index| index + 1)
}

pub fn line_end(text: &str, offset: usize) -> usize {
    let offset = clamp_to_char_boundary(text, offset);
    text[offset..]
        .find('\n')
        .map_or(text.len(), |index| offset + index)
}

pub fn line_range_at(text: &str, offset: usize) -> Range<usize> {
    line_start(text, offset)..line_end(text, offset)
}

/// The run a double click selects: the same character class on both sides.
pub fn word_range_at(text: &str, offset: usize) -> Range<usize> {
    let offset = clamp_to_char_boundary(text, offset);
    let at = text[offset..].chars().next().map(class_of);
    let before = char_before(text, offset).map(|(_, character)| class_of(character));

    let class = match (at, before) {
        (Some(CharClass::Whitespace) | None, Some(class)) if class != CharClass::Whitespace => {
            class
        }
        (Some(class), _) => class,
        (None, Some(class)) => class,
        (None, None) => return offset..offset,
    };

    let mut start = offset;
    while let Some((preceding_start, character)) = char_before(text, start) {
        if class_of(character) != class {
            break;
        }
        start = preceding_start;
    }
    let mut end = offset;
    while let Some(character) = text[end..].chars().next() {
        if class_of(character) != class {
            break;
        }
        end += character.len_utf8();
    }
    start..end
}

pub fn char_count(text: &str) -> usize {
    text.chars().count()
}

/// The byte offset that leaves at most `limit` chars in `text`.
///
/// Chars, not graphemes and not bytes: `common::validation` on the backend
/// counts `value.chars().count()`, and a client limit that counts anything else
/// either rejects text the server would accept or accepts text it will reject.
pub fn truncate_to_char_limit(text: &str, limit: usize) -> usize {
    text.char_indices()
        .nth(limit)
        .map_or(text.len(), |(offset, _)| offset)
}

pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

pub fn utf16_offset_from_utf8(text: &str, offset: usize) -> usize {
    let offset = clamp_to_char_boundary(text, offset);
    text[..offset].chars().map(char::len_utf16).sum()
}

pub fn utf8_offset_from_utf16(text: &str, offset_utf16: usize) -> usize {
    let mut utf16_seen = 0usize;
    for (index, character) in text.char_indices() {
        if utf16_seen >= offset_utf16 {
            return index;
        }
        utf16_seen += character.len_utf16();
    }
    text.len()
}

pub fn utf16_range_from_utf8(text: &str, range: &Range<usize>) -> Range<usize> {
    utf16_offset_from_utf8(text, range.start)..utf16_offset_from_utf8(text, range.end)
}

pub fn utf8_range_from_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    let start = utf8_offset_from_utf16(text, range.start);
    let end = utf8_offset_from_utf16(text, range.end.max(range.start));
    start..end.max(start)
}

fn char_before(text: &str, offset: usize) -> Option<(usize, char)> {
    if offset == 0 {
        return None;
    }
    let offset = clamp_to_char_boundary(text, offset);
    if offset == 0 {
        return None;
    }
    let mut start = offset - 1;
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    text[start..offset].chars().next().map(|c| (start, c))
}

fn is_regional_indicator(character: char) -> bool {
    matches!(character as u32, 0x1F1E6..=0x1F1FF)
}

/// The approximation this module is honest about.
///
/// This is not UAX #29. It covers the combining marks a Latin, Cyrillic, Greek,
/// Arabic, Hebrew, Devanagari or Thai string actually produces, variation
/// selectors, emoji skin-tone modifiers, joined emoji sequences, flag pairs and
/// CRLF. It does not implement Hangul syllable composition (L/V/T jamo compose
/// into one cluster in UAX #29 and into several here), Indic conjunct clusters
/// past the Devanagari range below, or prepend characters. Those degrade to a
/// caret that steps through a syllable rather than over it — wrong, but never
/// unsafe: every offset it returns is a char boundary.
fn is_extending(character: char) -> bool {
    matches!(
        character as u32,
        0x0300..=0x036F
            | 0x0483..=0x0489
            | 0x0591..=0x05BD
            | 0x05BF
            | 0x05C1..=0x05C2
            | 0x05C4..=0x05C5
            | 0x05C7
            | 0x0610..=0x061A
            | 0x064B..=0x065F
            | 0x0670
            | 0x06D6..=0x06DC
            | 0x06DF..=0x06E4
            | 0x06E7..=0x06E8
            | 0x06EA..=0x06ED
            | 0x0711
            | 0x0730..=0x074A
            | 0x07A6..=0x07B0
            | 0x07EB..=0x07F3
            | 0x0816..=0x0819
            | 0x081B..=0x0823
            | 0x0825..=0x0827
            | 0x0829..=0x082D
            | 0x0900..=0x0903
            | 0x093A..=0x094F
            | 0x0951..=0x0957
            | 0x0962..=0x0963
            | 0x0E31
            | 0x0E34..=0x0E3A
            | 0x0E47..=0x0E4E
            | 0x0EB1
            | 0x0EB4..=0x0EBC
            | 0x0EC8..=0x0ECD
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20F0
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
            | 0x1F3FB..=0x1F3FF
            | 0xE0100..=0xE01EF
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CYRILLIC: &str = "Привет";
    const CJK: &str = "日本語";
    const SKIN_TONE: &str = "👍🏻";
    const FAMILY: &str = "👨‍👩‍👧";
    const FLAG: &str = "🇷🇺";
    const ACCENT: &str = "e\u{0301}";

    fn walk_forward(text: &str) -> Vec<usize> {
        let mut out = vec![0];
        let mut cursor = 0;
        while cursor < text.len() {
            cursor = next_grapheme_boundary(text, cursor);
            out.push(cursor);
        }
        out
    }

    fn walk_backward(text: &str) -> Vec<usize> {
        let mut out = vec![text.len()];
        let mut cursor = text.len();
        while cursor > 0 {
            cursor = previous_grapheme_boundary(text, cursor);
            out.push(cursor);
        }
        out.reverse();
        out
    }

    #[test]
    fn every_boundary_it_reports_is_a_char_boundary() {
        for text in [CYRILLIC, CJK, SKIN_TONE, FAMILY, FLAG, ACCENT, "a\r\nb", ""] {
            for offset in walk_forward(text) {
                assert!(text.is_char_boundary(offset), "{text:?} at {offset}");
            }
            for offset in walk_backward(text) {
                assert!(text.is_char_boundary(offset), "{text:?} at {offset}");
            }
        }
    }

    #[test]
    fn clamping_never_lands_mid_char() {
        for offset in 0..=CYRILLIC.len() + 4 {
            let clamped = clamp_to_char_boundary(CYRILLIC, offset);
            assert!(CYRILLIC.is_char_boundary(clamped));
            assert!(clamped <= CYRILLIC.len());
        }
    }

    #[test]
    fn forward_and_backward_walks_agree() {
        for text in [CYRILLIC, CJK, SKIN_TONE, FAMILY, FLAG, ACCENT, "a\r\nb"] {
            assert_eq!(walk_forward(text), walk_backward(text), "{text:?}");
        }
    }

    #[test]
    fn cyrillic_moves_one_letter_per_press() {
        assert_eq!(grapheme_count(CYRILLIC), 6);
        assert_eq!(next_grapheme_boundary(CYRILLIC, 0), 2);
        assert_eq!(previous_grapheme_boundary(CYRILLIC, CYRILLIC.len()), 10);
    }

    #[test]
    fn cjk_moves_one_ideograph_per_press() {
        assert_eq!(grapheme_count(CJK), 3);
        assert_eq!(next_grapheme_boundary(CJK, 0), 3);
        assert_eq!(previous_grapheme_boundary(CJK, CJK.len()), 6);
    }

    #[test]
    fn an_emoji_with_a_modifier_is_one_press_not_four() {
        assert_eq!(grapheme_count(SKIN_TONE), 1);
        assert_eq!(next_grapheme_boundary(SKIN_TONE, 0), SKIN_TONE.len());
        assert_eq!(previous_grapheme_boundary(SKIN_TONE, SKIN_TONE.len()), 0);
    }

    #[test]
    fn a_joined_family_is_one_cluster_from_either_side() {
        assert_eq!(grapheme_count(FAMILY), 1);
        assert_eq!(previous_grapheme_boundary(FAMILY, FAMILY.len()), 0);
    }

    #[test]
    fn a_flag_is_a_pair_of_regional_indicators() {
        assert_eq!(grapheme_count(FLAG), 1);
        assert_eq!(grapheme_count("🇷🇺🇰🇿"), 2);
        assert_eq!(previous_grapheme_boundary("🇷🇺🇰🇿", "🇷🇺🇰🇿".len()), 8);
        assert_eq!(grapheme_count("a🇷"), 2);
    }

    #[test]
    fn a_combining_accent_rides_with_its_base() {
        assert_eq!(grapheme_count(ACCENT), 1);
        assert_eq!(previous_grapheme_boundary(ACCENT, ACCENT.len()), 0);
    }

    #[test]
    fn crlf_is_one_cluster() {
        assert_eq!(next_grapheme_boundary("a\r\nb", 1), 3);
        assert_eq!(previous_grapheme_boundary("a\r\nb", 3), 1);
    }

    #[test]
    fn movement_at_the_ends_is_a_fixed_point() {
        assert_eq!(previous_grapheme_boundary(CYRILLIC, 0), 0);
        assert_eq!(
            next_grapheme_boundary(CYRILLIC, CYRILLIC.len()),
            CYRILLIC.len()
        );
        assert_eq!(next_grapheme_boundary("", 0), 0);
        assert_eq!(previous_grapheme_boundary("", 0), 0);
    }

    #[test]
    fn word_movement_crosses_words_not_letters() {
        let text = "Привет мир  hello";
        assert_eq!(next_word_boundary(text, 0), 12);
        assert_eq!(next_word_boundary(text, 12), 19);
        assert_eq!(previous_word_boundary(text, text.len()), 21);
        assert_eq!(previous_word_boundary(text, 19), 13);
    }

    #[test]
    fn word_movement_at_both_ends_is_a_fixed_point() {
        let text = "  word  ";
        assert_eq!(previous_word_boundary(text, 0), 0);
        assert_eq!(next_word_boundary(text, text.len()), text.len());
        assert_eq!(next_word_boundary("", 0), 0);
        assert_eq!(previous_word_boundary("", 0), 0);
        assert_eq!(next_word_boundary(text, 0), 6);
        assert_eq!(previous_word_boundary(text, text.len()), 2);
    }

    #[test]
    fn punctuation_is_its_own_class() {
        let text = "a.b";
        assert_eq!(next_word_boundary(text, 0), 1);
        assert_eq!(next_word_boundary(text, 1), 2);
    }

    #[test]
    fn a_double_click_selects_the_word_under_it() {
        let text = "Привет мир";
        assert_eq!(word_range_at(text, 4), 0..12);
        assert_eq!(word_range_at(text, 12), 0..12);
        assert_eq!(word_range_at(text, 13), 13..19);
    }

    #[test]
    fn a_triple_click_selects_the_line_under_it() {
        let text = "one\ntwo\nthree";
        assert_eq!(line_range_at(text, 0), 0..3);
        assert_eq!(line_range_at(text, 5), 4..7);
        assert_eq!(line_range_at(text, text.len()), 8..13);
    }

    #[test]
    fn utf16_round_trips_through_utf8_on_astral_text() {
        for text in [CYRILLIC, CJK, SKIN_TONE, FAMILY, FLAG, "a👍🏻b"] {
            for (offset, _) in text.char_indices().chain([(text.len(), ' ')]) {
                let utf16 = utf16_offset_from_utf8(text, offset);
                assert_eq!(utf8_offset_from_utf16(text, utf16), offset, "{text:?}");
            }
            assert_eq!(utf16_len(text), utf16_offset_from_utf8(text, text.len()));
        }
    }

    #[test]
    fn a_utf16_offset_inside_a_surrogate_pair_lands_on_a_char_boundary() {
        let text = "👍";
        let offset = utf8_offset_from_utf16(text, 1);
        assert!(text.is_char_boundary(offset));
    }

    #[test]
    fn the_char_limit_counts_what_the_backend_counts() {
        assert_eq!(truncate_to_char_limit("Привет", 3), 6);
        assert_eq!(truncate_to_char_limit("日本語", 2), 6);
        assert_eq!(truncate_to_char_limit("ab", 9), 2);
        assert_eq!(char_count(SKIN_TONE), 2);
    }
}
