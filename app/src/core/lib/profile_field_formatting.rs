use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::LazyLock;

use rise_canon::RiseLimits;
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent};
use rise_core::foundation::foundation_character::characters;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::Locale;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::date_format::iso8601_date_formatter::ISO8601DateFormatter;
use rise_i18n::i18n::{FormatArg, format_printf};
use rise_i18n::{plural_word, tr};
use rise_regions::REGIONS;
use rise_ui::phone_country_catalog::PhoneCountryCatalog;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;

pub enum PhoneNumberFormatting {}

static CALLING_CODES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let mut out: HashSet<String> = HashSet::new();
    for region in REGIONS.iter() {
        out.insert(region.calling_code.to_owned());
        out.extend(region.additional_calling_codes.iter().map(|code| (*code).to_owned()));
    }
    let max_code_length = *MAX_CODE_LENGTH;
    out.retain(|code| char_count(code) <= max_code_length);
    out
});

static MAX_CODE_LENGTH: LazyLock<usize> = LazyLock::new(|| {
    usize::try_from(RiseLimits::client_value("phone_number_formatting_max_code_length")).unwrap_or(0)
});

impl PhoneNumberFormatting {
    pub fn format(raw: &str) -> String {
        let trimmed = trimming_whitespaces_and_newlines(raw);
        if trimmed.is_empty() {
            return raw.to_owned();
        }
        let digits = only_numbers(trimmed);
        if digits.is_empty() {
            return trimmed.to_owned();
        }
        if let Some(patterned) = Self::patterned(&digits) {
            return patterned;
        }

        let digit_count = char_count(&digits);
        let mut code = String::new();
        for len in (1..=*MAX_CODE_LENGTH).rev() {
            if digit_count <= len {
                continue;
            }
            let prefix = prefix_chars(&digits, len);
            if CALLING_CODES.contains(&prefix) {
                code = prefix;
                break;
            }
        }

        let national = drop_chars(&digits, char_count(&code));
        if national.is_empty() {
            return format!("+{digits}");
        }
        let grouped = Self::group(&national);
        if code.is_empty() {
            format!("+{grouped}")
        } else {
            format!("+{code} {grouped}")
        }
    }

    /// the territory own grouping from canon like +998 90 123 45 67
    /// when the national part fills its pattern exactly, else generic grouping
    fn patterned(digits: &str) -> Option<String> {
        let country = PhoneCountryCatalog::country_matching_international_number(&format!("+{digits}"))?;
        let code = only_numbers(&country.calling_code);
        let pattern = &country.format_pattern;
        let code_count = char_count(&code);
        if code_count > *MAX_CODE_LENGTH
            || !digits.starts_with(code.as_str())
            || pattern.is_empty()
            || characters(pattern).filter(|slot| slot.as_str() == "X").count() != char_count(digits) - code_count
        {
            return None;
        }

        let national_digits = drop_chars(digits, code_count);
        let mut national = characters(&national_digits);
        let mut out = String::new();
        for slot in characters(pattern) {
            if slot.as_str() == "X" {
                out.push_str(national.next()?.as_str());
            } else {
                out.push_str(slot.as_str());
            }
        }
        Some(format!("+{code} {out}"))
    }

    fn group(s: &str) -> String {
        let chars: Vec<&str> = characters(s).map(|character| character.as_str()).collect();
        let sizes = Self::sizes(chars.len());
        let mut out: Vec<String> = Vec::new();
        let mut i = 0;
        for size in sizes {
            let end = (i + size).min(chars.len());
            out.push(chars[i..end].concat());
            i = end;
            if i >= chars.len() {
                break;
            }
        }
        if i < chars.len() {
            out.push(chars[i..].concat());
        }
        out.join(" ")
    }

    fn sizes(count: usize) -> Vec<usize> {
        match count {
            0..=4 => vec![count],
            5..=7 => vec![3, count - 3],
            8 => vec![3, 3, 2],
            9 => vec![3, 3, 3],
            10 => vec![3, 3, 2, 2],
            _ => {
                let mut sizes = vec![3, 3];
                let mut remaining = count - 6;
                while remaining > 4 {
                    sizes.push(3);
                    remaining -= 3;
                }
                if remaining == 4 {
                    sizes.extend([2, 2]);
                } else if remaining > 0 {
                    sizes.push(remaining);
                }
                sizes
            }
        }
    }
}

pub enum BirthdayFormatting {}

impl BirthdayFormatting {
    /// the localized date and the age when it makes sense, returned as a pair
    pub fn parse(raw: &str) -> Option<(String, Option<String>)> {
        let date = Self::date(raw)?;

        let mut out = DateFormatter::new();
        out.set_locale(Locale::app_locale().identifier());
        out.set_localized_date_format_from_template("d MMMM yyyy");
        let date_str = out.string_from(&date);

        let years = Calendar::current()
            .date_components_between(&[CalendarComponent::Year], &date, &RiseClock::now())
            .value(CalendarComponent::Year)
            .unwrap_or(-1);
        let age_str = if (1..150).contains(&years) {
            Some(Self::age_string(years))
        } else {
            None
        };
        Some((date_str, age_str))
    }

    fn age_string(years: i64) -> String {
        let one = tr("age_year_one");
        let few = tr("age_year_few");
        let many = tr("age_year_many");
        let template = plural_word(years, &one, &few, &many);
        format_printf(template, &[FormatArg::Text(Cow::Owned(years.to_string()))])
    }

    fn date(raw: &str) -> Option<Date> {
        let trimmed = trimming_whitespaces_and_newlines(raw);
        if trimmed.is_empty() {
            return None;
        }
        if let Some(date) = ISO8601DateFormatter::new().date_from(trimmed) {
            return Some(date);
        }
        let mut formatter = DateFormatter::new();
        formatter.set_locale("en_US_POSIX");
        for format in ["yyyy-MM-dd", "yyyy-MM-dd'T'HH:mm:ss", "yyyy/MM/dd", "dd.MM.yyyy"] {
            formatter.set_date_format(format);
            if let Some(date) = formatter.date_from(trimmed) {
                return Some(date);
            }
        }
        None
    }
}

/// keeps only the digits
fn only_numbers(s: &str) -> String {
    characters(s).filter(|character| character.is_number()).map(|character| character.as_str()).collect()
}

fn char_count(s: &str) -> usize {
    characters(s).count()
}

fn prefix_chars(s: &str, count: usize) -> String {
    characters(s).take(count).map(|character| character.as_str()).collect()
}

fn drop_chars(s: &str, count: usize) -> String {
    characters(s).skip(count).map(|character| character.as_str()).collect()
}
