use super::percent_number_formatter::PercentNumberFormatter;

fn fixed_one(code: &str) -> PercentNumberFormatter {
    let mut formatter = PercentNumberFormatter::new(code);
    formatter.minimum_fraction_digits = 1;
    formatter.maximum_fraction_digits = 1;
    formatter
}

fn significant(code: &str) -> PercentNumberFormatter {
    let mut formatter = PercentNumberFormatter::new(code);
    formatter.uses_significant_digits = true;
    formatter.minimum_significant_digits = 1;
    formatter.maximum_significant_digits = 2;
    formatter
}

fn strings(formatter: &PercentNumberFormatter, values: &[f64]) -> Vec<String> {
    values.iter().map(|value| formatter.string(*value).unwrap_or_default()).collect()
}

#[test]
fn fixed_one_fraction_digit_matches_foundation() {
    let values = [0.003, 0.125, 0.0005, 12.3456];
    assert_eq!(strings(&fixed_one("ru"), &values), ["0,3\u{a0}%", "12,5\u{a0}%", "0,0\u{a0}%", "1\u{a0}234,6\u{a0}%"]);
    assert_eq!(strings(&fixed_one("en"), &values), ["0.3%", "12.5%", "0.0%", "1,234.6%"]);
    assert_eq!(strings(&fixed_one("tr"), &values), ["%0,3", "%12,5", "%0,0", "%1.234,6"]);
    assert_eq!(strings(&fixed_one("pt-BR"), &values), ["0,3%", "12,5%", "0,0%", "1.234,6%"]);
    assert_eq!(
        strings(&fixed_one("fa"), &values),
        [
            "\u{6f0}\u{66b}\u{6f3}\u{66a}",
            "\u{6f1}\u{6f2}\u{66b}\u{6f5}\u{66a}",
            "\u{6f0}\u{66b}\u{6f0}\u{66a}",
            "\u{6f1}\u{66c}\u{6f2}\u{6f3}\u{6f4}\u{66b}\u{6f6}\u{66a}",
        ]
    );
}

#[test]
fn significant_digits_match_foundation() {
    let values = [0.0035, 0.000049, 0.00995, -0.0];
    assert_eq!(strings(&significant("ru"), &values), ["0,35\u{a0}%", "0,0049\u{a0}%", "1\u{a0}%", "-0\u{a0}%"]);
    assert_eq!(strings(&significant("en"), &values), ["0.35%", "0.0049%", "1%", "-0%"]);
    assert_eq!(strings(&significant("tr"), &values), ["%0,35", "%0,0049", "%1", "-%0"]);
    assert_eq!(strings(&significant("fa"), &values)[3], "\u{200e}\u{2212}\u{6f0}\u{66a}");
}

#[test]
fn default_percent_rounds_half_to_even_and_groups() {
    let values = [0.125, 0.135, 1234.5];
    assert_eq!(strings(&PercentNumberFormatter::new("ru"), &values), ["12\u{a0}%", "14\u{a0}%", "123\u{a0}450\u{a0}%"]);
    assert_eq!(strings(&PercentNumberFormatter::new("hi"), &values), ["12%", "14%", "1,23,450%"]);
}

#[test]
fn region_identifier_falls_back_to_its_language() {
    let values = [0.003, 12.3456];
    assert_eq!(strings(&fixed_one("ru_RU"), &values), strings(&fixed_one("ru"), &values));
}

#[test]
fn non_finite_values_use_the_language_symbols() {
    let formatter = PercentNumberFormatter::new("en");
    assert_eq!(formatter.string(f64::NAN).as_deref(), Some("NaN"));
    assert_eq!(formatter.string(f64::INFINITY).as_deref(), Some("+\u{221e}"));
    assert_eq!(formatter.string(f64::NEG_INFINITY).as_deref(), Some("-\u{221e}"));
}
