use super::percent_format_data::LOCALES;

/// percent number formatter for a locale code
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PercentNumberFormatter {
    pub locale_code: String,
    pub minimum_fraction_digits: usize,
    pub maximum_fraction_digits: usize,
    pub uses_significant_digits: bool,
    pub minimum_significant_digits: usize,
    pub maximum_significant_digits: usize,
}

impl PercentNumberFormatter {
    /// new percent formatter, no fraction digits and significant digits off (1 to 6 when on)
    pub fn new(locale_code: &str) -> Self {
        Self {
            locale_code: locale_code.to_owned(),
            minimum_fraction_digits: 0,
            maximum_fraction_digits: 0,
            uses_significant_digits: false,
            minimum_significant_digits: 1,
            maximum_significant_digits: 6,
        }
    }

    /// share times 100 rounded half to even
    /// with the language digits and separators and percent sign
    pub fn string(&self, value: f64) -> Option<String> {
        let data = locale_data(&self.locale_code);
        if value.is_nan() {
            return Some(data.not_a_number.to_owned());
        }
        if value.is_infinite() {
            let symbol = if value < 0.0 { data.negative_infinity } else { data.positive_infinity };
            return Some(symbol.to_owned());
        }

        let mut plain = PlainDecimal::shortest(value.abs());
        if !plain.digits.is_empty() {
            plain.point += 2;
        }
        let mut minimum_fraction = 0_usize;
        let mut significant_pad = 0_usize;
        if self.uses_significant_digits {
            let minimum = self.minimum_significant_digits.max(1);
            let maximum = self.maximum_significant_digits.max(minimum);
            plain.round_half_even(to_i64(maximum));
            let shown = if plain.digits.is_empty() { 1 } else { to_i64(plain.digits.len()).max(plain.point) };
            significant_pad = usize::try_from(to_i64(minimum) - shown).unwrap_or(0);
        } else {
            let maximum = self.maximum_fraction_digits.max(self.minimum_fraction_digits);
            plain.round_half_even(plain.point + to_i64(maximum));
            minimum_fraction = self.minimum_fraction_digits;
        }

        let (integer, mut fraction) = plain.split();
        minimum_fraction = minimum_fraction.max(fraction.len() + significant_pad);
        fraction.resize(minimum_fraction.max(fraction.len()), 0);

        let negative = value.is_sign_negative();
        let (prefix, suffix) = if negative {
            (data.negative_prefix, data.negative_suffix)
        } else {
            (data.positive_prefix, data.positive_suffix)
        };
        let mut out = String::with_capacity(prefix.len() + suffix.len() + (integer.len() + fraction.len()) * 3);
        out.push_str(prefix);
        push_grouped(&mut out, &integer, data);
        if !fraction.is_empty() {
            out.push_str(data.decimal_separator);
            push_digits(&mut out, &fraction, data);
        }
        out.push_str(suffix);
        Some(out)
    }
}

/// number symbols and percent affixes of one app language
#[derive(Debug)]
pub(super) struct LocalePercentData {
    pub(super) code: &'static str,
    pub(super) digits: [char; 10],
    pub(super) decimal_separator: &'static str,
    pub(super) grouping_separator: &'static str,
    pub(super) uses_grouping: bool,
    pub(super) primary_grouping: usize,
    pub(super) secondary_grouping: usize,
    pub(super) minimum_grouping_digits: usize,
    pub(super) positive_prefix: &'static str,
    pub(super) positive_suffix: &'static str,
    pub(super) negative_prefix: &'static str,
    pub(super) negative_suffix: &'static str,
    pub(super) positive_infinity: &'static str,
    pub(super) negative_infinity: &'static str,
    pub(super) not_a_number: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PlainDecimal {
    digits: Vec<u8>,
    point: i64,
}

impl PlainDecimal {
    /// shortest decimal text that reads back as the same double
    fn shortest(magnitude: f64) -> Self {
        let text = format!("{magnitude:e}");
        let (mantissa, exponent) = text.split_once('e').unwrap_or((text.as_str(), "0"));
        let exponent = exponent.parse::<i64>().unwrap_or(0);
        let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let mut digits: Vec<u8> = integer
            .bytes()
            .chain(fraction.bytes())
            .filter(u8::is_ascii_digit)
            .map(|byte| byte - b'0')
            .collect();
        let mut point = to_i64(integer.len()) + exponent;
        let leading = digits.iter().take_while(|digit| **digit == 0).count();
        digits.drain(..leading);
        point -= to_i64(leading);
        let mut plain = Self { digits, point };
        plain.drop_trailing_zeros();
        plain
    }

    /// keeps the first keep digits, rounds half to even on the exact decimal
    fn round_half_even(&mut self, keep: i64) {
        let Ok(keep) = usize::try_from(keep) else {
            self.digits.clear();
            self.point = 0;
            return;
        };
        if keep >= self.digits.len() {
            return;
        }
        let first_dropped = self.digits[keep];
        let rest_non_zero = self.digits[keep + 1..].iter().any(|digit| *digit != 0);
        let last_kept_odd = keep > 0 && self.digits[keep - 1] % 2 == 1;
        let up = first_dropped > 5 || (first_dropped == 5 && (rest_non_zero || last_kept_odd));
        self.digits.truncate(keep);
        if up {
            let mut carry = true;
            for digit in self.digits.iter_mut().rev() {
                if *digit == 9 {
                    *digit = 0;
                } else {
                    *digit += 1;
                    carry = false;
                    break;
                }
            }
            if carry {
                self.digits.insert(0, 1);
                self.point += 1;
            }
        }
        self.drop_trailing_zeros();
    }

    fn drop_trailing_zeros(&mut self) {
        while self.digits.last() == Some(&0) {
            self.digits.pop();
        }
        if self.digits.is_empty() {
            self.point = 0;
        }
    }

    /// integer digits (at least one) and fraction digits as shown
    fn split(&self) -> (Vec<u8>, Vec<u8>) {
        let len = self.digits.len();
        let integer = match usize::try_from(self.point) {
            Ok(point) if point > 0 => {
                let mut integer: Vec<u8> = self.digits.iter().take(point).copied().collect();
                integer.resize(point, 0);
                integer
            }
            _ => vec![0],
        };
        let fraction = match usize::try_from(self.point) {
            Ok(point) if point < len => self.digits[point..].to_vec(),
            Ok(_) => Vec::new(),
            Err(_) => {
                let zeros = usize::try_from(-self.point).unwrap_or(0);
                let mut fraction = vec![0; zeros];
                fraction.extend_from_slice(&self.digits);
                fraction
            }
        };
        (integer, fraction)
    }
}

fn to_i64(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

pub(super) fn locale_data(code: &str) -> &'static LocalePercentData {
    let normalized = code.replace('_', "-");
    if let Some(found) = LOCALES.iter().find(|data| data.code == normalized) {
        return found;
    }
    let language = normalized.split('-').next().unwrap_or_default();
    if !language.is_empty() {
        let same_language = LOCALES.iter().find(|data| data.code.split('-').next() == Some(language));
        if let Some(found) = same_language {
            return found;
        }
    }
    &LOCALES[0]
}

fn push_digits(out: &mut String, digits: &[u8], data: &LocalePercentData) {
    for digit in digits {
        out.push(data.digits.get(usize::from(*digit)).copied().unwrap_or('0'));
    }
}

fn push_grouped(out: &mut String, integer: &[u8], data: &LocalePercentData) {
    let len = integer.len();
    if !data.uses_grouping || data.primary_grouping == 0 || len < data.primary_grouping + data.minimum_grouping_digits {
        push_digits(out, integer, data);
        return;
    }
    let mut cuts = Vec::with_capacity(len / data.primary_grouping + 1);
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
