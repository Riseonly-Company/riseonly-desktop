use crate::foundation::ns_range::{
    byte_to_utf16_offset, byte_to_utf16_range, length_utf16, range_of, substring, utf16_to_byte_offset,
    utf16_to_byte_range, NSRange,
};

const GRIN: &str = "\u{1F600}";

#[test]
fn length_counts_surrogate_pairs_as_two() {
    assert_eq!(length_utf16(GRIN), 2);
    assert_eq!(length_utf16("a\u{1F600}b"), 4);
    assert_eq!(length_utf16("\u{0451}"), 1);
    assert_eq!(length_utf16(""), 0);
}

#[test]
fn utf16_range_maps_to_bytes_around_surrogates() {
    let text = "a\u{1F600}b";
    assert_eq!(utf16_to_byte_range(text, NSRange::new(1, 2)), Some(1..5));
    assert_eq!(utf16_to_byte_range(text, NSRange::new(3, 1)), Some(5..6));
    assert_eq!(substring(text, NSRange::new(1, 2)), Some(GRIN));
}

#[test]
fn half_a_surrogate_pair_is_not_a_range() {
    let text = "a\u{1F600}b";
    assert_eq!(utf16_to_byte_range(text, NSRange::new(2, 1)), None);
    assert_eq!(utf16_to_byte_range(text, NSRange::new(1, 1)), None);
    assert_eq!(utf16_to_byte_offset(text, 2), None);
}

#[test]
fn out_of_bounds_is_none() {
    assert_eq!(utf16_to_byte_range("abc", NSRange::new(2, 5)), None);
    assert_eq!(utf16_to_byte_range("abc", NSRange::new(usize::MAX, 1)), None);
    assert_eq!(utf16_to_byte_range("abc", NSRange::new(3, 0)), Some(3..3));
}

#[test]
fn byte_range_round_trips() {
    let text = "\u{0451}\u{1F600}x";
    let r = byte_to_utf16_range(text, 2..6);
    assert_eq!(r, NSRange::new(1, 2));
    assert_eq!(utf16_to_byte_range(text, r), Some(2..6));
    assert_eq!(NSRange::full(text), NSRange::new(0, 4));
}

#[test]
fn byte_offset_inside_scalar_rounds_down() {
    assert_eq!(byte_to_utf16_offset("a\u{1F600}", 3), 1);
    assert_eq!(byte_to_utf16_offset("a\u{1F600}", 99), 3);
}

#[test]
fn range_arithmetic_matches_foundation() {
    let a = NSRange::new(2, 4);
    let b = NSRange::new(5, 3);
    assert_eq!(a.upper_bound(), 6);
    assert!(a.contains(5));
    assert!(!a.contains(6));
    assert_eq!(a.intersection(b), Some(NSRange::new(5, 1)));
    assert_eq!(a.intersection(NSRange::new(6, 2)), None);
    assert_eq!(a.union(b), NSRange::new(2, 6));
    assert_eq!(NSRange::from(3..7), NSRange::new(3, 4));
}

#[test]
fn range_of_counts_in_utf16_and_misses_empty() {
    let text = "\u{1F600}@rise";
    assert_eq!(range_of(text, "@rise"), Some(NSRange::new(2, 5)));
    assert_eq!(range_of(text, ""), None);
    assert_eq!(range_of(text, "zz"), None);
}

#[test]
fn range_of_is_canonical_not_literal() {
    let text = "cafe\u{301} bar";
    assert_eq!(range_of(text, "caf\u{e9}"), Some(NSRange::new(0, 5)));
    assert_eq!(range_of("caf\u{e9}", "cafe\u{301}"), Some(NSRange::new(0, 4)));
}
