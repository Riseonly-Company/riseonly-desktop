use rust_decimal::{Decimal, RoundingStrategy};

use super::currency_format_data::{AFFIXES, LOCALES, TEMPLATE_CODE};

/// the two currency presentations the app formats with
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CurrencyFormatStyleConfigurationPresentation {
    Narrow,
    #[default]
    Standard,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DecimalFormatStyleCurrency {
    code: String,
    presentation: CurrencyFormatStyleConfigurationPresentation,
    locale: String,
}

impl DecimalFormatStyleCurrency {
    /// currency format by code, starts with the root locale until locale is called
    pub fn currency(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            presentation: CurrencyFormatStyleConfigurationPresentation::default(),
            locale: String::new(),
        }
    }

    pub fn presentation(mut self, presentation: CurrencyFormatStyleConfigurationPresentation) -> Self {
        self.presentation = presentation;
        self
    }

    /// sets the locale by identifier (eg ru_RU)
    pub fn locale(mut self, identifier: impl Into<String>) -> Self {
        self.locale = identifier.into();
        self
    }

    pub fn format(&self, value: &Decimal) -> String {
        let data = locale_data(&self.locale);
        let affixes = affixes(data, &self.code, self.presentation);

        let rounded = value.round_dp_with_strategy(u32::from(affixes.fraction_digits), RoundingStrategy::MidpointNearestEven);
        let negative = value.is_sign_negative() && !value.is_zero();
        let plain = rounded.abs().to_string();
        let (integer, fraction) = plain.split_once('.').unwrap_or((plain.as_str(), ""));

        let mut number = String::with_capacity(plain.len() * 3);
        push_grouped(&mut number, integer, data);
        if affixes.fraction_digits > 0 {
            number.push_str(data.decimal_separator);
            push_digits(&mut number, fraction, data);
            for _ in fraction.len()..usize::from(affixes.fraction_digits) {
                number.push(data.digits[0]);
            }
        }

        let mut out = String::with_capacity(number.len() + affixes.prefix.len() + affixes.suffix.len() + data.minus_sign.len());
        if negative && data.minus_before_prefix {
            out.push_str(data.minus_sign);
        }
        out.push_str(&affixes.prefix);
        if negative && !data.minus_before_prefix {
            out.push_str(data.minus_sign);
        }
        out.push_str(&number);
        out.push_str(&affixes.suffix);
        out
    }
}

/// affixes of one presentation printed around the number
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CurrencyAffixes {
    pub(super) fraction_digits: u8,
    pub(super) prefix: &'static str,
    pub(super) suffix: &'static str,
}

/// one currency of one locale, the four affixes are indices into AFFIXES
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CurrencyRow {
    code: &'static str,
    fraction_digits: u8,
    narrow_prefix: u16,
    narrow_suffix: u16,
    standard_prefix: u16,
    standard_suffix: u16,
}

pub(super) const fn row(code: &'static str, fraction_digits: u8, narrow_prefix: u16, narrow_suffix: u16, standard_prefix: u16, standard_suffix: u16) -> CurrencyRow {
    CurrencyRow { code, fraction_digits, narrow_prefix, narrow_suffix, standard_prefix, standard_suffix }
}

/// number symbols and currency affixes of one locale, currencies sorted by code
#[derive(Debug)]
pub(super) struct LocaleCurrencyData {
    pub(super) identifier: &'static str,
    pub(super) digits: [char; 10],
    pub(super) decimal_separator: &'static str,
    pub(super) grouping_separator: &'static str,
    pub(super) primary_grouping: usize,
    pub(super) secondary_grouping: usize,
    pub(super) minimum_grouping_digits: usize,
    pub(super) minus_sign: &'static str,
    pub(super) minus_before_prefix: bool,
    pub(super) template_narrow: CurrencyAffixes,
    pub(super) template_standard: CurrencyAffixes,
    pub(super) currencies: &'static [CurrencyRow],
}

struct ResolvedAffixes {
    fraction_digits: u8,
    prefix: String,
    suffix: String,
}

fn locale_data(identifier: &str) -> &'static LocaleCurrencyData {
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

fn affix(index: u16) -> &'static str {
    AFFIXES.get(usize::from(index)).copied().unwrap_or_default()
}

fn affixes(data: &LocaleCurrencyData, code: &str, presentation: CurrencyFormatStyleConfigurationPresentation) -> ResolvedAffixes {
    let code = code.to_ascii_uppercase();
    if let Ok(position) = data.currencies.binary_search_by(|row| row.code.cmp(code.as_str())) {
        let row = &data.currencies[position];
        let (prefix, suffix) = match presentation {
            CurrencyFormatStyleConfigurationPresentation::Narrow => (row.narrow_prefix, row.narrow_suffix),
            CurrencyFormatStyleConfigurationPresentation::Standard => (row.standard_prefix, row.standard_suffix),
        };
        return ResolvedAffixes {
            fraction_digits: row.fraction_digits,
            prefix: affix(prefix).to_owned(),
            suffix: affix(suffix).to_owned(),
        };
    }
    let template = match presentation {
        CurrencyFormatStyleConfigurationPresentation::Narrow => data.template_narrow,
        CurrencyFormatStyleConfigurationPresentation::Standard => data.template_standard,
    };
    ResolvedAffixes {
        fraction_digits: template.fraction_digits,
        prefix: template.prefix.replace(TEMPLATE_CODE, &code),
        suffix: template.suffix.replace(TEMPLATE_CODE, &code),
    }
}

fn push_digits(out: &mut String, ascii_digits: &str, data: &LocaleCurrencyData) {
    for byte in ascii_digits.bytes() {
        let index = usize::from(byte.wrapping_sub(b'0'));
        out.push(data.digits.get(index).copied().unwrap_or(char::from(byte)));
    }
}

fn push_grouped(out: &mut String, integer: &str, data: &LocaleCurrencyData) {
    let len = integer.len();
    if data.primary_grouping == 0 || len < data.primary_grouping + data.minimum_grouping_digits {
        push_digits(out, integer, data);
        return;
    }
    let mut cuts = Vec::with_capacity(len / data.primary_grouping.max(1) + 1);
    let mut end = len - data.primary_grouping;
    cuts.push(end);
    let secondary = data.secondary_grouping.max(1);
    while end > secondary {
        end -= secondary;
        cuts.push(end);
    }
    let mut start = 0;
    for cut in cuts.into_iter().rev() {
        push_digits(out, &integer[start..cut], data);
        out.push_str(data.grouping_separator);
        start = cut;
    }
    push_digits(out, &integer[start..], data);
}
