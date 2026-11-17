use crate::foundation::foundation_character::{
    characters, double_from_string, int_from_string, int_from_string_radix,
    scalar_is_control_character, scalar_is_emoji, scalar_is_emoji_presentation, Character,
};

fn ch(s: &str) -> Character<'_> {
    Character::new(s).expect("one character")
}

#[test]
fn new_takes_exactly_one_character() {
    assert!(Character::new("").is_none());
    assert!(Character::new("ab").is_none());
    assert!(Character::new("\u{1F1F7}\u{1F1FA}").is_some());
    assert_eq!(characters("a\r\nb").count(), 3);
}

#[test]
fn is_number_is_unicode_numeric() {
    assert!(ch("7").is_number());
    assert!(ch("\u{0663}").is_number());
    assert!(ch("\u{00BD}").is_number());
    assert!(ch("\u{2163}").is_number());
    assert!(!ch("a").is_number());
    assert!(ch("\u{4E00}").is_number());
    assert!(ch("\u{842C}").is_number());
    assert!(ch("\u{20001}").is_number());
    assert!(!ch("\u{4E01}").is_number());
}

#[test]
fn letters_whitespace_newlines() {
    assert!(ch("\u{0451}").is_letter());
    assert!(ch("e\u{0301}").is_letter());
    assert!(!ch("1").is_letter());
    assert!(ch("\u{00A0}").is_whitespace());
    assert!(ch("\r\n").is_newline());
    assert!(ch("\u{2028}").is_newline());
    assert!(!ch(" ").is_newline());
}

#[test]
fn punctuation_and_symbols_by_general_category() {
    assert!(ch("!").is_punctuation());
    assert!(ch("\u{00AB}").is_punctuation());
    assert!(!ch("+").is_punctuation());
    assert!(ch("+").is_symbol());
    assert!(ch("+").is_math_symbol());
    assert!(ch("\u{20BD}").is_currency_symbol());
}

#[test]
fn ascii_value_reads_crlf_as_lf() {
    assert_eq!(ch("\r\n").ascii_value(), Some(b'\n'));
    assert_eq!(ch("A").ascii_value(), Some(b'A'));
    assert!(!ch("\u{00E9}").is_ascii());
    assert!(!ch("e\u{0301}").is_ascii());
}

#[test]
fn case_properties() {
    assert!(ch("A").is_uppercase());
    assert!(ch("\u{0401}").is_uppercase());
    assert!(!ch("a").is_uppercase());
    assert!(ch("a").is_lowercase());
    assert!(!ch("1").is_uppercase());
    assert!(!ch("1").is_lowercase());
    assert!(ch("E\u{0301}").is_uppercase());
}

#[test]
fn hex_digits_ascii_and_fullwidth() {
    assert_eq!(ch("f").hex_digit_value(), Some(15));
    assert_eq!(ch("B").hex_digit_value(), Some(11));
    assert_eq!(ch("\u{FF19}").hex_digit_value(), Some(9));
    assert!(!ch("g").is_hex_digit());
}

#[test]
fn emoji_scalar_properties() {
    assert!(scalar_is_emoji('\u{1F600}'));
    assert!(scalar_is_emoji('#'));
    assert!(!scalar_is_emoji_presentation('#'));
    assert!(scalar_is_emoji_presentation('\u{1F600}'));
    assert!(!scalar_is_emoji('a'));
}

#[test]
fn int_parses_like_swift() {
    assert_eq!(int_from_string("+5"), Some(5));
    assert_eq!(int_from_string("-12"), Some(-12));
    assert_eq!(int_from_string(" 5"), None);
    assert_eq!(int_from_string("5 "), None);
    assert_eq!(int_from_string(""), None);
    assert_eq!(int_from_string("+"), None);
    assert_eq!(int_from_string("\u{0663}"), None);
    assert_eq!(int_from_string("9223372036854775808"), None);
    assert_eq!(int_from_string_radix("ff", 16), Some(255));
    assert_eq!(int_from_string_radix("1", 37), None);
}

#[test]
fn double_parses_like_swift() {
    assert_eq!(double_from_string("1.5"), Some(1.5));
    assert_eq!(double_from_string("-.5"), Some(-0.5));
    assert_eq!(double_from_string("1e3"), Some(1000.0));
    assert_eq!(double_from_string("0x1.8p1"), Some(3.0));
    assert_eq!(double_from_string("-inf"), Some(f64::NEG_INFINITY));
    assert!(double_from_string("NaN").is_some_and(f64::is_nan));
    assert_eq!(double_from_string(" 1"), None);
    assert_eq!(double_from_string("1 "), None);
    assert_eq!(double_from_string("1e"), None);
    assert_eq!(double_from_string("+-1"), None);
    assert_eq!(double_from_string(""), None);
}

#[test]
fn control_characters_are_cc_and_cf() {
    assert!(scalar_is_control_character('\u{0000}'));
    assert!(scalar_is_control_character('\n'));
    assert!(scalar_is_control_character('\u{007F}'));
    assert!(scalar_is_control_character('\u{0085}'));
    assert!(scalar_is_control_character('\u{00AD}'));
    assert!(scalar_is_control_character('\u{200B}'));
    assert!(scalar_is_control_character('\u{202E}'));
    assert!(scalar_is_control_character('\u{FEFF}'));
    assert!(scalar_is_control_character('\u{E0001}'));
    assert!(!scalar_is_control_character('a'));
    assert!(!scalar_is_control_character(' '));
    assert!(!scalar_is_control_character('\u{00A0}'));
    assert!(!scalar_is_control_character('\u{2028}'));
}
