use super::{first_strong_is_right_to_left, is_right_to_left_text};

#[test]
fn arabic_paragraph_is_right_to_left() {
    assert!(is_right_to_left_text("هذا نص تجريبي"));
}

#[test]
fn hebrew_after_digits_is_right_to_left() {
    assert!(is_right_to_left_text("2024 שלום"));
}

#[test]
fn latin_first_is_left_to_right() {
    assert!(!is_right_to_left_text("hello هذا"));
}

#[test]
fn no_letters_is_left_to_right() {
    assert!(!is_right_to_left_text("123 ..."));
}

#[test]
fn no_letters_has_no_direction() {
    assert_eq!(first_strong_is_right_to_left("12 ..."), None);
    assert_eq!(first_strong_is_right_to_left(" \u{FFFC} שלום"), Some(true));
}
