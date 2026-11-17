use std::cmp::Ordering;

use crate::foundation::foundation_compare::{
    canonically_equal, compare_case_insensitive, contains_case_diacritic_insensitive, folding,
    localized_case_insensitive_compare, localized_case_insensitive_contains, localized_compare,
    localized_standard_contains, ns_range_of, range_of, range_of_in,
    CompareOptions,
};
use crate::foundation::ns_range::NSRange;

const CASE_DIACRITIC: CompareOptions = CompareOptions::CASE_INSENSITIVE.union(CompareOptions::DIACRITIC_INSENSITIVE);

#[test]
fn yo_matches_ye_without_diacritics() {
    assert!(contains_case_diacritic_insensitive("\u{0401}\u{043B}\u{043A}\u{0430}", "\u{0435}\u{043B}\u{043A}"));
    assert!(!localized_case_insensitive_contains("\u{0451}\u{043B}\u{043A}\u{0430}", "\u{0435}\u{043B}\u{043A}"));
    assert!(localized_case_insensitive_contains("\u{0401}\u{043B}\u{043A}\u{0430}", "\u{0451}\u{043B}"));
}

#[test]
fn e_acute_matches_e_without_diacritics() {
    assert!(contains_case_diacritic_insensitive("Caf\u{00E9}", "CAFE"));
    assert!(localized_standard_contains("Caf\u{00E9}", "cafe"));
    assert!(!localized_case_insensitive_contains("Caf\u{00E9}", "cafe"));
}

#[test]
fn precomposed_and_decomposed_are_equivalent() {
    assert!(localized_case_insensitive_contains("Caf\u{00E9}", "CAFE\u{0301}"));
    assert!(canonically_equal("\u{00E9}", "e\u{0301}"));
    assert!(!canonically_equal("\u{00E9}", "e"));
}

#[test]
fn no_match_inside_a_character() {
    assert_eq!(range_of("\u{00E9}", "e", CompareOptions::NONE), None);
    assert_eq!(range_of("e\u{0301}", "e", CompareOptions::NONE), None);
    assert_eq!(range_of("e\u{0301}", "e", CompareOptions::DIACRITIC_INSENSITIVE), Some(0..3));
}

#[test]
fn range_points_into_the_source() {
    let text = "x \u{00C9}t\u{00E9} y";
    assert_eq!(range_of(text, "ETE", CASE_DIACRITIC), Some(2..7));
    assert_eq!(range_of(text, "", CASE_DIACRITIC), None);
}

#[test]
fn anchored_and_backwards() {
    let title = "Post on Riseonly";
    let opts = CompareOptions::CASE_INSENSITIVE.union(CompareOptions::ANCHORED).union(CompareOptions::BACKWARDS);
    assert_eq!(range_of(title, " ON RISEONLY", opts), Some(4..16));
    assert_eq!(range_of("abcabc", "abc", CompareOptions::BACKWARDS), Some(3..6));
    assert_eq!(range_of("xabc", "abc", CompareOptions::ANCHORED), None);
    assert_eq!(range_of("abcx", "ABC", CompareOptions::ANCHORED.union(CompareOptions::CASE_INSENSITIVE)), Some(0..3));
}

#[test]
fn search_range_in_bytes_and_utf16() {
    let text = "\u{1F600}@Rise @rise";
    assert_eq!(range_of_in(text, "@RISE", CompareOptions::CASE_INSENSITIVE, 5..text.len()), Some(10..15));
    assert_eq!(ns_range_of(text, "@RISE", CompareOptions::CASE_INSENSITIVE, NSRange::new(3, 10)), Some(NSRange::new(8, 5)));
    assert_eq!(ns_range_of(text, "@RISE", CompareOptions::CASE_INSENSITIVE, NSRange::new(3, 9)), None);
}

#[test]
fn case_insensitive_compare_orders_folded_forms() {
    assert_eq!(compare_case_insensitive("Unknown", "unknown"), Ordering::Equal);
    assert_eq!(compare_case_insensitive("\u{0401}", "\u{0451}"), Ordering::Equal);
    assert_eq!(compare_case_insensitive("a", "B"), Ordering::Less);
    assert_eq!(compare_case_insensitive("\u{00E9}", "e"), Ordering::Greater);
}

#[test]
fn folding_strips_case_and_marks() {
    assert_eq!(folding("\u{0401}\u{00C9}", CASE_DIACRITIC), "\u{0435}e");
    assert_eq!(folding("\u{FF21}", CompareOptions::WIDTH_INSENSITIVE), "A");
}

fn sorted_in(locale: &str, names: &[&'static str]) -> Vec<&'static str> {
    let mut sorted = names.to_vec();
    sorted.sort_by(|a, b| localized_compare(a, b, locale));
    sorted
}

#[test]
fn accented_country_names_stay_among_their_letters() {
    let aland = "\u{C5}land Islands";
    let ivory = "C\u{F4}te d'Ivoire";
    assert_eq!(
        sorted_in("en", &["Zambia", ivory, "Cuba", "Albania", aland, "Colombia"]),
        vec![aland, "Albania", "Colombia", ivory, "Cuba", "Zambia"]
    );
}

#[test]
fn yo_sorts_with_ye_not_after_ya_in_russian() {
    let yozh = "\u{451}\u{436}";
    let zhuk = "\u{436}\u{443}\u{43A}";
    let yama = "\u{44F}\u{43C}\u{430}";
    assert_eq!(sorted_in("ru", &[yama, yozh, zhuk]), vec![yozh, zhuk, yama]);
}

#[test]
fn the_language_decides_where_a_letter_sorts() {
    let apple = "\u{E4}pple";
    assert_eq!(sorted_in("en", &["zebra", apple]), vec![apple, "zebra"]);
    assert_eq!(sorted_in("sv", &[apple, "zebra"]), vec!["zebra", apple]);
}

#[test]
fn region_identifiers_with_an_underscore_are_read_and_unknown_ones_use_the_root_order() {
    assert_eq!(localized_compare("a", "b", "pt_BR"), Ordering::Less);
    assert_eq!(localized_compare("\u{C5}", "B", "not a locale!"), Ordering::Less);
}

#[test]
fn localized_compare_counts_case_and_the_insensitive_one_only_diacritics() {
    assert_ne!(localized_compare("a", "A", "en"), Ordering::Equal);
    assert_eq!(localized_case_insensitive_compare("Apple", "aPPLE", "en"), Ordering::Equal);
    assert_eq!(localized_case_insensitive_compare("banana", "Cherry", "en"), Ordering::Less);
    assert_ne!(localized_case_insensitive_compare("e", "\u{E9}", "en"), Ordering::Equal);
}
