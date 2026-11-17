use super::decimal_number_formatter::DecimalNumberFormatter;

#[test]
fn short_numbers_have_no_separator() {
    assert_eq!(DecimalNumberFormatter::new("en").string(0), "0");
    assert_eq!(DecimalNumberFormatter::new("en").string(999), "999");
}

#[test]
fn english_groups_by_three() {
    assert_eq!(DecimalNumberFormatter::new("en").string(1_000), "1,000");
    assert_eq!(DecimalNumberFormatter::new("en").string(1_234_567), "1,234,567");
}
