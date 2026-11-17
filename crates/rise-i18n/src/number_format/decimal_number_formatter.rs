use super::percent_number_formatter::{LocalePercentData, locale_data};

/// decimal formatter for whole non negative counts
#[derive(Clone, Copy, Debug)]
pub struct DecimalNumberFormatter {
    data: &'static LocalePercentData,
}

impl DecimalNumberFormatter {
    pub fn new(locale_code: &str) -> Self {
        Self { data: locale_data(locale_code) }
    }

    /// formats with primary group at the right and secondary groups to its left
    /// no grouping while the number is shorter than primary plus the minimum grouping digits
    pub fn string(&self, value: u64) -> String {
        let data = self.data;
        let text = value.to_string();
        let len = text.len();
        let grouped = data.uses_grouping
            && data.primary_grouping > 0
            && len >= data.primary_grouping + data.minimum_grouping_digits;
        let mut out = String::with_capacity(len * 4);
        for (index, byte) in text.bytes().enumerate() {
            let left = len - index;
            if grouped && index > 0 && Self::group_starts(left, data) {
                out.push_str(data.grouping_separator);
            }
            let digit = usize::from(byte - b'0');
            out.push(data.digits[digit]);
        }
        out
    }

    fn group_starts(digits_left: usize, data: &LocalePercentData) -> bool {
        if digits_left == data.primary_grouping {
            return true;
        }
        digits_left > data.primary_grouping
            && (digits_left - data.primary_grouping).is_multiple_of(data.secondary_grouping.max(1))
    }
}
