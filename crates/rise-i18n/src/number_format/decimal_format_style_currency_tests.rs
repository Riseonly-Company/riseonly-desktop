use rust_decimal::Decimal;

use super::decimal_format_style_currency::{CurrencyFormatStyleConfigurationPresentation as Presentation, DecimalFormatStyleCurrency};

fn amount(text: &str) -> Decimal {
    text.parse().expect("test amount literal")
}

fn narrow(locale: &str, code: &str, value: &str) -> String {
    DecimalFormatStyleCurrency::currency(code).presentation(Presentation::Narrow).locale(locale).format(&amount(value))
}

fn standard(locale: &str, code: &str, value: &str) -> String {
    DecimalFormatStyleCurrency::currency(code).presentation(Presentation::Standard).locale(locale).format(&amount(value))
}

#[test]
fn russian_narrow_matches_foundation() {
    let cases = [
        ("USD", "0.5", "0,50\u{a0}$"),
        ("USD", "2.99", "2,99\u{a0}$"),
        ("USD", "1490", "1\u{a0}490,00\u{a0}$"),
        ("USD", "12345", "12\u{a0}345,00\u{a0}$"),
        ("USD", "1234567.895", "1\u{a0}234\u{a0}567,90\u{a0}$"),
        ("USD", "-2.99", "-2,99\u{a0}$"),
        ("KZT", "2.99", "2,99\u{a0}\u{20b8}"),
        ("KZT", "1490", "1\u{a0}490,00\u{a0}\u{20b8}"),
        ("KZT", "-2.99", "-2,99\u{a0}\u{20b8}"),
        ("EUR", "2.99", "2,99\u{a0}\u{20ac}"),
        ("EUR", "1234567.895", "1\u{a0}234\u{a0}567,90\u{a0}\u{20ac}"),
    ];
    for (code, value, expected) in cases {
        assert_eq!(narrow("ru_RU", code, value), expected, "ru_RU {code} {value}");
    }
}

#[test]
fn english_narrow_matches_foundation() {
    let cases = [
        ("USD", "0.5", "$0.50"),
        ("USD", "2.99", "$2.99"),
        ("USD", "1490", "$1,490.00"),
        ("USD", "1234567.895", "$1,234,567.90"),
        ("USD", "-2.99", "-$2.99"),
        ("KZT", "2.99", "\u{20b8}2.99"),
        ("KZT", "12345", "\u{20b8}12,345.00"),
        ("KZT", "-2.99", "-\u{20b8}2.99"),
        ("EUR", "1490", "\u{20ac}1,490.00"),
        ("EUR", "-2.99", "-\u{20ac}2.99"),
    ];
    for (code, value, expected) in cases {
        assert_eq!(narrow("en_US", code, value), expected, "en_US {code} {value}");
    }
}

#[test]
fn kazakh_narrow_matches_foundation() {
    let cases = [
        ("USD", "2.99", "2,99\u{a0}$"),
        ("USD", "1490", "1\u{a0}490,00\u{a0}$"),
        ("KZT", "0.5", "0,50\u{a0}\u{20b8}"),
        ("KZT", "1490", "1\u{a0}490,00\u{a0}\u{20b8}"),
        ("KZT", "1234567.895", "1\u{a0}234\u{a0}567,90\u{a0}\u{20b8}"),
        ("EUR", "12345", "12\u{a0}345,00\u{a0}\u{20ac}"),
        ("EUR", "-2.99", "-2,99\u{a0}\u{20ac}"),
    ];
    for (code, value, expected) in cases {
        assert_eq!(narrow("kk_KZ", code, value), expected, "kk_KZ {code} {value}");
    }
}

#[test]
fn standard_presentation_keeps_the_code_where_foundation_does() {
    assert_eq!(standard("ru_RU", "KZT", "1490"), "1\u{a0}490,00\u{a0}KZT");
    assert_eq!(standard("en_US", "KZT", "2.99"), "KZT\u{a0}2.99");
    assert_eq!(standard("en_US", "KZT", "-2.99"), "-KZT\u{a0}2.99");
    assert_eq!(standard("kk_KZ", "KZT", "2.99"), "2,99\u{a0}\u{20b8}");
    assert_eq!(standard("en_US", "USD", "2.99"), "$2.99");
}

#[test]
fn default_presentation_is_standard() {
    let style = DecimalFormatStyleCurrency::currency("KZT").locale("en_US");
    assert_eq!(style.format(&amount("2.99")), "KZT\u{a0}2.99");
}

#[test]
fn minimum_grouping_digits_and_indian_grouping() {
    assert_eq!(narrow("es_ES", "EUR", "1490"), "1490,00\u{a0}\u{20ac}");
    assert_eq!(narrow("es_ES", "EUR", "12345"), "12.345,00\u{a0}\u{20ac}");
    assert_eq!(narrow("hi_IN", "INR", "1234567.895"), "\u{20b9}12,34,567.90");
}

#[test]
fn native_digits_and_bidi_marks() {
    assert_eq!(narrow("ar_SA", "USD", "2.99"), "\u{200f}\u{662}\u{66b}\u{669}\u{669}\u{a0}US$");
}

#[test]
fn rounding_is_half_even_and_keeps_negative_zero() {
    assert_eq!(narrow("en_US", "JPY", "2.5"), "\u{a5}2");
    assert_eq!(narrow("en_US", "JPY", "3.5"), "\u{a5}4");
    assert_eq!(narrow("en_US", "JPY", "-0.5"), "-\u{a5}0");
}

#[test]
fn unknown_codes_print_the_code_like_foundation() {
    assert_eq!(narrow("ru_RU", "XAU", "2.99"), "2,99\u{a0}XAU");
    assert_eq!(narrow("ru_RU", "ZZZ", "2.99"), "2,99\u{a0}ZZZ");
}

#[test]
fn bcp47_identifier_and_bare_language_resolve_to_the_canon_locale() {
    assert_eq!(narrow("ru-RU", "USD", "2.99"), "2,99\u{a0}$");
    assert_eq!(narrow("ru", "USD", "2.99"), "2,99\u{a0}$");
}
