use crate::foundation::foundation_text::{
    character_boundaries, character_count, character_prefix, character_suffix, characters, drop_first_characters,
    drop_last_characters, is_newline, is_whitespace, is_whitespace_or_newline, trimming_newlines, trimming_whitespaces,
    trimming_whitespaces_and_newlines,
};

const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
const FLAG_RU: &str = "\u{1F1F7}\u{1F1FA}";
const FLAG_US: &str = "\u{1F1FA}\u{1F1F8}";

#[test]
fn zwj_family_is_one_character() {
    assert_eq!(character_count(FAMILY), 1);
    assert_eq!(FAMILY.chars().count(), 7);
}

#[test]
fn flags_count_one_each() {
    assert_eq!(character_count(FLAG_RU), 1);
    let two = format!("{FLAG_RU}{FLAG_US}");
    assert_eq!(character_count(&two), 2);
    assert_eq!(character_prefix(&two, 1), FLAG_RU);
    assert_eq!(character_suffix(&two, 1), FLAG_US);
}

#[test]
fn combining_mark_and_crlf_are_one_character() {
    assert_eq!(character_count("e\u{0301}"), 1);
    assert_eq!(character_count("a\r\nb"), 3);
    assert_eq!(character_count(""), 0);
}

#[test]
fn prefix_and_suffix_clamp_to_whole_string() {
    assert_eq!(character_prefix("abc", 0), "");
    assert_eq!(character_prefix("abc", 2), "ab");
    assert_eq!(character_prefix("abc", 10), "abc");
    assert_eq!(character_suffix("abc", 0), "");
    assert_eq!(character_suffix("abc", 2), "bc");
    assert_eq!(character_suffix("abc", 10), "abc");
}

#[test]
fn drop_first_and_last_by_characters() {
    let s = format!("{FAMILY}ab{FLAG_RU}");
    assert_eq!(drop_first_characters(&s, 1), format!("ab{FLAG_RU}"));
    assert_eq!(drop_last_characters(&s, 1), format!("{FAMILY}ab"));
    assert_eq!(drop_first_characters(&s, 0), s);
    assert_eq!(drop_last_characters(&s, 0), s);
    assert_eq!(drop_first_characters(&s, 4), "");
    assert_eq!(drop_last_characters(&s, 9), "");
}

#[test]
fn characters_and_boundaries_follow_graphemes() {
    let s = format!("a{FLAG_RU}e\u{0301}");
    let parts: Vec<&str> = characters(&s).collect();
    assert_eq!(parts, vec!["a", FLAG_RU, "e\u{0301}"]);
    assert_eq!(character_boundaries(&s), vec![0, 1, 9, 12]);
}

#[test]
fn trimming_whitespaces_and_newlines_takes_nel_and_line_separators() {
    assert_eq!(trimming_whitespaces_and_newlines("\u{0085} a b \u{2028}"), "a b");
    assert_eq!(trimming_whitespaces_and_newlines("\u{2029}\t\u{00A0}x\u{3000}\n"), "x");
    assert_eq!(trimming_whitespaces_and_newlines(" \r\n "), "");
}

#[test]
fn trimming_whitespaces_keeps_line_breaks() {
    assert_eq!(trimming_whitespaces("\u{2028} a \u{0085}"), "\u{2028} a \u{0085}");
    assert_eq!(trimming_whitespaces("\t a \u{202F}"), "a");
    assert_eq!(trimming_newlines("\n a \u{2028}"), " a ");
}

#[test]
fn zero_width_space_is_not_whitespace() {
    assert!(!is_whitespace('\u{200B}'));
    assert_eq!(trimming_whitespaces_and_newlines("\u{200B}a"), "\u{200B}a");
}

#[test]
fn character_sets_match_foundation() {
    assert!(is_whitespace('\t'));
    assert!(!is_whitespace('\n'));
    assert!(is_newline('\u{000B}'));
    assert!(is_newline('\u{0085}'));
    assert!(is_whitespace_or_newline('\u{2029}'));
    assert!(!is_whitespace_or_newline('a'));
}
