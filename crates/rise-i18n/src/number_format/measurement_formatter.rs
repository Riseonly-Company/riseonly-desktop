use std::str::FromStr;

use rust_decimal::{Decimal, RoundingStrategy};

use super::measurement::{Dimension, Measurement};
use super::measurement_format_data::{
    CATEGORY_COUNT, LOCALES, PATTERN_ROWS, PATTERNS, PLACEHOLDER, UK_SYSTEM_REGIONS, US_SYSTEM_REGIONS,
};
use crate::language_store::Locale;

/// metric or us or uk measurement system
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LocaleMeasurementSystem {
    Metric,
    Us,
    Uk,
}

impl Locale {
    /// app language with the machine region
    /// eg a ru app on a us machine is ru_US
    pub fn current() -> Locale {
        let app = Locale::app_locale();
        match rise_platform::device_locale::current().region {
            Some(region) => Locale::new(format!("{}_{}", language_and_script(app.identifier()), region)),
            None => app,
        }
    }

    /// measurement system by the region of the identifier
    pub fn measurement_system(&self) -> LocaleMeasurementSystem {
        let Some(region) = region_code(self.identifier()) else {
            let language = self.identifier().split(['_', '-']).next().unwrap_or_default();
            return if language.is_empty() || language == "en" {
                LocaleMeasurementSystem::Us
            } else {
                LocaleMeasurementSystem::Metric
            };
        };
        if US_SYSTEM_REGIONS.contains(&region) {
            LocaleMeasurementSystem::Us
        } else if UK_SYSTEM_REGIONS.contains(&region) {
            LocaleMeasurementSystem::Uk
        } else {
            LocaleMeasurementSystem::Metric
        }
    }
}

/// trailing region subtag, two letters or three digits after language and script
fn region_code(identifier: &str) -> Option<&str> {
    let mut parts = identifier.split(['_', '-']);
    parts.next()?;
    parts.find(|part| {
        (part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_uppercase()))
            || (part.len() == 3 && part.bytes().all(|byte| byte.is_ascii_digit()))
    })
}

fn language_and_script(identifier: &str) -> &str {
    match region_code(identifier) {
        Some(region) => identifier
            .rfind(region)
            .map(|at| identifier[..at].trim_end_matches(['_', '-']))
            .unwrap_or(identifier),
        None => identifier,
    }
}

/// unit style, long or medium or short
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FormatterUnitStyle {
    Short,
    #[default]
    Medium,
    Long,
}

impl FormatterUnitStyle {
    const COUNT: usize = 3;

    fn format_index(self) -> usize {
        match self {
            FormatterUnitStyle::Short => 0,
            FormatterUnitStyle::Medium => 1,
            FormatterUnitStyle::Long => 2,
        }
    }
}

/// measurement unit options, empty by default
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MeasurementFormatterUnitOptions {
    bits: u8,
}

impl MeasurementFormatterUnitOptions {
    pub const PROVIDED_UNIT: Self = Self { bits: 1 };
    pub const NATURAL_SCALE: Self = Self { bits: 1 << 1 };

    pub const fn empty() -> Self {
        Self { bits: 0 }
    }

    pub const fn contains(self, other: Self) -> bool {
        self.bits & other.bits == other.bits
    }

    pub const fn union(self, other: Self) -> Self {
        Self { bits: self.bits | other.bits }
    }
}

/// number formatter part that the measurement formatter is configured through
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NumberFormatter {
    pub maximum_fraction_digits: u32,
}

impl Default for NumberFormatter {
    fn default() -> Self {
        Self { maximum_fraction_digits: 3 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeasurementFormatter {
    pub locale: Locale,
    pub unit_options: MeasurementFormatterUnitOptions,
    pub unit_style: FormatterUnitStyle,
    pub number_formatter: NumberFormatter,
}

impl Default for MeasurementFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl MeasurementFormatter {
    /// new formatter, current locale and no unit options and medium style
    pub fn new() -> Self {
        Self {
            locale: Locale::current(),
            unit_options: MeasurementFormatterUnitOptions::empty(),
            unit_style: FormatterUnitStyle::default(),
            number_formatter: NumberFormatter::default(),
        }
    }

    pub fn string_from<U: Dimension>(&self, measurement: &Measurement<U>) -> String {
        let shown = if self.unit_options.contains(MeasurementFormatterUnitOptions::PROVIDED_UNIT) {
            *measurement
        } else {
            match U::preferred_unit(self.locale.measurement_system()) {
                Some(unit) => measurement.converted(unit),
                None => *measurement,
            }
        };
        let data = locale_data(self.locale.identifier());
        let plain = rounded_plain(shown.value, self.number_formatter.maximum_fraction_digits);
        let number = format_number(&plain, data);
        let row = shown.unit.format_index() * FormatterUnitStyle::COUNT + self.unit_style.format_index();
        let category = plural_category(data.identifier, &plain);
        let pattern = data
            .patterns
            .get(row)
            .and_then(|patterns| patterns.get(category))
            .and_then(|index| PATTERNS.get(usize::from(*index)))
            .copied()
            .unwrap_or(PLACEHOLDER);
        pattern.replacen(PLACEHOLDER, &number, 1)
    }
}

/// number symbols and unit patterns of one locale
/// patterns rows are unit by unit style, each has a pattern index per plural category
#[derive(Debug)]
pub(super) struct LocaleMeasurementData {
    pub(super) identifier: &'static str,
    pub(super) digits: [char; 10],
    pub(super) decimal_separator: &'static str,
    pub(super) grouping_separator: &'static str,
    pub(super) primary_grouping: usize,
    pub(super) secondary_grouping: usize,
    pub(super) minimum_grouping_digits: usize,
    pub(super) minus_sign: &'static str,
    pub(super) patterns: [[u16; CATEGORY_COUNT]; PATTERN_ROWS],
}

fn locale_data(identifier: &str) -> &'static LocaleMeasurementData {
    let normalized = identifier.replace('-', "_");
    if let Some(found) = LOCALES.iter().find(|data| data.identifier == normalized) {
        return found;
    }
    let language = normalized.split('_').next().unwrap_or_default();
    if !language.is_empty() {
        let same_language = LOCALES.iter().find(|data| data.identifier.split('_').next() == Some(language));
        if let Some(found) = same_language {
            return found;
        }
    }
    &LOCALES[0]
}

/// the number as shown before digits are localized, ascii integer and fraction digits
#[derive(Debug, PartialEq, Eq)]
struct PlainNumber {
    negative: bool,
    integer: String,
    fraction: String,
}

fn rounded_plain(value: f64, fraction_digits: u32) -> PlainNumber {
    let source = if value.is_finite() {
        Decimal::from_str(&format!("{value}")).unwrap_or(if value < 0.0 { Decimal::MIN } else { Decimal::MAX })
    } else {
        Decimal::ZERO
    };
    let rounded = source.round_dp_with_strategy(fraction_digits, RoundingStrategy::MidpointNearestEven);
    let negative = rounded.is_sign_negative() && !rounded.is_zero();
    let plain = rounded.abs().normalize().to_string();
    let (integer, fraction) = plain.split_once('.').unwrap_or((plain.as_str(), ""));
    PlainNumber {
        negative,
        integer: integer.to_owned(),
        fraction: fraction.trim_end_matches('0').to_owned(),
    }
}

fn push_digits(out: &mut String, ascii_digits: &str, data: &LocaleMeasurementData) {
    for byte in ascii_digits.bytes() {
        let index = usize::from(byte.wrapping_sub(b'0'));
        out.push(data.digits.get(index).copied().unwrap_or(char::from(byte)));
    }
}

fn format_number(plain: &PlainNumber, data: &LocaleMeasurementData) -> String {
    let integer = plain.integer.as_str();
    let len = integer.len();
    let mut out = String::with_capacity(len * 3 + plain.fraction.len() * 3 + data.minus_sign.len());
    if plain.negative {
        out.push_str(data.minus_sign);
    }
    if data.primary_grouping == 0 || len < data.primary_grouping + data.minimum_grouping_digits {
        push_digits(&mut out, integer, data);
    } else {
        let mut cuts = Vec::with_capacity(len / data.primary_grouping + 1);
        let mut end = len - data.primary_grouping;
        cuts.push(end);
        let secondary = data.secondary_grouping.max(1);
        while end > secondary {
            end -= secondary;
            cuts.push(end);
        }
        cuts.reverse();
        let mut start = 0;
        for cut in cuts {
            push_digits(&mut out, &integer[start..cut], data);
            out.push_str(data.grouping_separator);
            start = cut;
        }
        push_digits(&mut out, &integer[start..], data);
    }
    if !plain.fraction.is_empty() {
        out.push_str(data.decimal_separator);
        push_digits(&mut out, &plain.fraction, data);
    }
    out
}

const ZERO: usize = 0;
const ONE: usize = 1;
const TWO: usize = 2;
const FEW: usize = 3;
const MANY: usize = 4;
const OTHER: usize = 5;

fn plural_category(identifier: &str, plain: &PlainNumber) -> usize {
    let v = plain.fraction.len();
    let integer_tail = &plain.integer[plain.integer.len().saturating_sub(18)..];
    let i: i64 = integer_tail.parse().unwrap_or(0);
    let f: i64 = if v == 0 { 0 } else { plain.fraction.parse().unwrap_or(0) };
    let whole = v == 0;
    let n = |k: i64| whole && i == k;
    let (i10, i100, f10, f100) = (i % 10, i % 100, f % 10, f % 100);
    let million = whole && i != 0 && i % 1_000_000 == 0;
    let language = identifier.split('_').next().unwrap_or_default();
    match language {
        "ar" => {
            if n(0) {
                ZERO
            } else if n(1) {
                ONE
            } else if n(2) {
                TWO
            } else if whole && (3..=10).contains(&i100) {
                FEW
            } else if whole && (11..=99).contains(&i100) {
                MANY
            } else {
                OTHER
            }
        }
        "bn" | "fa" | "hi" => {
            if i == 0 || n(1) { ONE } else { OTHER }
        }
        "cs" => {
            if i == 1 && v == 0 {
                ONE
            } else if (2..=4).contains(&i) && v == 0 {
                FEW
            } else if v != 0 {
                MANY
            } else {
                OTHER
            }
        }
        "da" => {
            if n(1) || (f != 0 && (i == 0 || i == 1)) { ONE } else { OTHER }
        }
        "es" | "it" => {
            let is_one = if language == "es" { n(1) } else { i == 1 && v == 0 };
            if is_one {
                ONE
            } else if million {
                MANY
            } else {
                OTHER
            }
        }
        "fr" => {
            if i == 0 || i == 1 {
                ONE
            } else if million {
                MANY
            } else {
                OTHER
            }
        }
        "pt" => {
            let is_one = if identifier == "pt_PT" { i == 1 && v == 0 } else { i == 0 || i == 1 };
            if is_one {
                ONE
            } else if million {
                MANY
            } else {
                OTHER
            }
        }
        "fil" => {
            let is_one = (v == 0 && (1..=3).contains(&i))
                || (v == 0 && ![4, 6, 9].contains(&i10))
                || (v != 0 && ![4, 6, 9].contains(&f10));
            if is_one { ONE } else { OTHER }
        }
        "he" => {
            if (i == 1 && v == 0) || (i == 0 && v != 0) {
                ONE
            } else if i == 2 && v == 0 {
                TWO
            } else {
                OTHER
            }
        }
        "pl" => {
            if i == 1 && v == 0 {
                ONE
            } else if v == 0 && (2..=4).contains(&i10) && !(12..=14).contains(&i100) {
                FEW
            } else if v == 0
                && ((i != 1 && (0..=1).contains(&i10)) || (5..=9).contains(&i10) || (12..=14).contains(&i100))
            {
                MANY
            } else {
                OTHER
            }
        }
        "ro" => {
            if i == 1 && v == 0 {
                ONE
            } else if v != 0 || n(0) || (whole && i != 1 && (1..=19).contains(&i100)) {
                FEW
            } else {
                OTHER
            }
        }
        "ru" | "uk" => {
            if v == 0 && i10 == 1 && i100 != 11 {
                ONE
            } else if v == 0 && (2..=4).contains(&i10) && !(12..=14).contains(&i100) {
                FEW
            } else if v == 0 && (i10 == 0 || (5..=9).contains(&i10) || (11..=14).contains(&i100)) {
                MANY
            } else {
                OTHER
            }
        }
        "sr" => {
            if (v == 0 && i10 == 1 && i100 != 11) || (f10 == 1 && f100 != 11) {
                ONE
            } else if (v == 0 && (2..=4).contains(&i10) && !(12..=14).contains(&i100))
                || ((2..=4).contains(&f10) && !(12..=14).contains(&f100))
            {
                FEW
            } else {
                OTHER
            }
        }
        "de" | "en" | "fi" | "nl" | "sv" | "sw" | "ur" => {
            if i == 1 && v == 0 { ONE } else { OTHER }
        }
        "el" | "hu" | "ka" | "kk" | "nb" | "ta" | "tr" | "uz" => {
            if n(1) { ONE } else { OTHER }
        }
        _ => OTHER,
    }
}
