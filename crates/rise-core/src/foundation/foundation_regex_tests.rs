use crate::foundation::foundation_regex::{
    escaping_stray_percent_signs, range_of_regular_expression, replacing_occurrences_regular_expression,
    NSMatchingOptions, NSRegularExpression, NSRegularExpressionOptions, HEX_32_TO_128, MENTION,
    NUMBER_IN_PATH, UUID_IN_PATH,
};
use crate::foundation::ns_range::NSRange;

fn re(pattern: &str) -> NSRegularExpression {
    NSRegularExpression::with_pattern(pattern).expect("valid pattern")
}

#[test]
fn mention_pattern_reports_utf16_ranges() {
    let mention = re(r"(?:^|[^\w@])@([A-Za-z0-9_]{1,32})");
    let text = "\u{1F600} @Rise and a@b";
    let found = mention.matches(text, NSMatchingOptions::NONE, NSRange::full(text));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].range(), NSRange::new(2, 6));
    assert_eq!(found[0].range_at(1), Some(NSRange::new(4, 4)));
    assert_eq!(found[0].number_of_ranges(), 2);
    assert_eq!(mention.number_of_capture_groups(), 1);
}

#[test]
fn unmatched_group_is_none() {
    let r = re(r"a(x)?b");
    let m = r.first_match("ab", NSMatchingOptions::NONE, NSRange::new(0, 2)).expect("match");
    assert_eq!(m.range_at(1), None);
    assert_eq!(m.range_at(9), None);
}

#[test]
fn anchored_matches_only_from_range_start() {
    let digits = re(r"\d+");
    assert!(digits.first_match("x12", NSMatchingOptions::ANCHORED, NSRange::new(0, 3)).is_none());
    assert_eq!(digits.range_of_first_match("x12", NSMatchingOptions::ANCHORED, NSRange::new(1, 2)), Some(NSRange::new(1, 2)));
    assert_eq!(re("ab").number_of_matches("ababxab", NSMatchingOptions::ANCHORED, NSRange::new(0, 7)), 2);
}

#[test]
fn range_bounds_are_opaque_with_anchors() {
    let start = re(r"^b");
    assert!(start.first_match("ab", NSMatchingOptions::NONE, NSRange::new(1, 1)).is_some());
    assert!(re("abc").first_match("abc", NSMatchingOptions::NONE, NSRange::new(0, 2)).is_none());
}

#[test]
fn invalid_range_or_pattern_fails_like_foundation() {
    assert!(re("a").first_match("a", NSMatchingOptions::NONE, NSRange::new(0, 5)).is_none());
    assert!(NSRegularExpression::with_pattern("(").is_err());
    assert!(NSRegularExpression::with_pattern(r"(?<=a)b").is_err());
}

#[test]
fn replacing_matches_expands_template() {
    let uuid = re("[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}");
    let path = "/v1/users/0f8fad5b-d9cb-469f-a165-70867728950e/posts";
    assert_eq!(uuid.string_by_replacing_matches(path, NSMatchingOptions::NONE, NSRange::full(path), ":id"), "/v1/users/:id/posts");
    let num = re(r"/\d{2,}");
    let p = "/a/12/b/3";
    assert_eq!(num.string_by_replacing_matches(p, NSMatchingOptions::NONE, NSRange::full(p), "/:id"), "/a/:id/b/3");
    let swap = re(r"(\w)(\w)");
    assert_eq!(swap.string_by_replacing_matches("ab", NSMatchingOptions::NONE, NSRange::new(0, 2), r"$2$1\$$12"), "ba$a2");
}

#[test]
fn options_map_to_flags() {
    let ci = NSRegularExpression::new("rise", NSRegularExpressionOptions::CASE_INSENSITIVE).expect("valid");
    assert!(ci.first_match("RISE", NSMatchingOptions::NONE, NSRange::new(0, 4)).is_some());
    let literal = NSRegularExpression::new(
        "a.b",
        NSRegularExpressionOptions { ignore_metacharacters: true, ..NSRegularExpressionOptions::NONE },
    )
    .expect("valid");
    assert!(literal.first_match("axb", NSMatchingOptions::NONE, NSRange::new(0, 3)).is_none());
    assert!(literal.first_match("a.b", NSMatchingOptions::NONE, NSRange::new(0, 3)).is_some());
    assert_eq!(NSRegularExpression::escaped_template("$1\\"), "\\$1\\\\");
}

fn fixed(regex: &'static std::sync::LazyLock<Option<NSRegularExpression>>) -> &'static NSRegularExpression {
    regex.as_ref().expect("fixed iOS pattern compiles")
}

#[test]
fn ready_mention_takes_the_canon_tag_ceiling() {
    let mention = fixed(&MENTION);
    let ceiling = rise_validate::rule_max::TAG as usize;
    assert!(mention.pattern().ends_with(&format!("{{1,{ceiling}}})")));
    let text = "\u{1F600} @Rise and a@b";
    let found = mention.matches(text, NSMatchingOptions::NONE, NSRange::full(text));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].range_at(1), Some(NSRange::new(4, 4)));
    let long = format!("@{}", "a".repeat(ceiling + 5));
    let hit = mention.first_match(&long, NSMatchingOptions::NONE, NSRange::full(&long)).expect("match");
    assert_eq!(hit.range_at(1), Some(NSRange::new(1, ceiling)));
}

#[test]
fn ready_hex_is_whole_string_only() {
    let hex = fixed(&HEX_32_TO_128);
    let ok = "a".repeat(32);
    assert_eq!(range_of_regular_expression(&ok, hex), Some(0..32));
    assert_eq!(range_of_regular_expression(&"F".repeat(128), hex), Some(0..128));
    assert_eq!(range_of_regular_expression(&"a".repeat(31), hex), None);
    assert_eq!(range_of_regular_expression(&"a".repeat(129), hex), None);
    assert_eq!(range_of_regular_expression(&format!("x{ok}"), hex), None);
}

#[test]
fn ready_path_patterns_normalise_like_sai_debugger() {
    let path = "/v1/users/123e4567-E89B-12d3-a456-426614174000/posts/42/7";
    let step = replacing_occurrences_regular_expression(path, fixed(&UUID_IN_PATH), ":id");
    assert_eq!(step, "/v1/users/:id/posts/42/7");
    let done = replacing_occurrences_regular_expression(&step, fixed(&NUMBER_IN_PATH), "/:id");
    assert_eq!(done, "/v1/users/:id/posts/:id/7");
}

#[test]
fn string_regex_range_is_byte_range_of_first_match() {
    let digits = re(r"\d+");
    let text = "\u{1F600}ab12cd345";
    assert_eq!(range_of_regular_expression(text, &digits), Some(6..8));
    assert_eq!(range_of_regular_expression("none", &digits), None);
    assert_eq!(replacing_occurrences_regular_expression(text, &digits, "<$0>"), "\u{1F600}ab<12>cd<345>");
}

#[test]
fn lookahead_pattern_does_not_compile_and_has_a_twin() {
    assert!(NSRegularExpression::with_pattern("%(?![0-9A-Fa-f]{2})").is_err());
    assert_eq!(escaping_stray_percent_signs("a=%41&b=%zz&c=100%"), "a=%41&b=%25zz&c=100%25");
    assert_eq!(escaping_stray_percent_signs("%%41"), "%25%41");
    assert_eq!(escaping_stray_percent_signs("%4"), "%254");
    assert_eq!(escaping_stray_percent_signs("%e2%80%94\u{442}%"), "%e2%80%94\u{442}%25");
    assert_eq!(escaping_stray_percent_signs(""), "");
}
