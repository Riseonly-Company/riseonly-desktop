use std::ops::{BitOr, BitOrAssign};

use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::Date;

const MS_PER_SECOND: i64 = 1_000;
const MS_PER_MINUTE: i64 = 60 * MS_PER_SECOND;
const MS_PER_HOUR: i64 = 60 * MS_PER_MINUTE;
const MS_PER_DAY: i64 = 24 * MS_PER_HOUR;
const SECONDS_PER_MINUTE: i64 = 60;
const SECONDS_PER_HOUR: i64 = 3_600;
const MAX_ZONE_HOURS: i64 = 18;
const DAYS_PER_WEEK: i64 = 7;
const UNIX_EPOCH_YEAR: i64 = 1_970;
const FRACTION_DIGITS: usize = 3;

/// iso8601 formatter options, same raw bits as foundation
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ISO8601DateFormatterOptions(u64);

impl ISO8601DateFormatterOptions {
    pub const WITH_YEAR: Self = Self(1);
    pub const WITH_MONTH: Self = Self(1 << 1);
    pub const WITH_WEEK_OF_YEAR: Self = Self(1 << 2);
    pub const WITH_DAY: Self = Self(1 << 4);
    pub const WITH_TIME: Self = Self(1 << 5);
    pub const WITH_TIME_ZONE: Self = Self(1 << 6);
    pub const WITH_SPACE_BETWEEN_DATE_AND_TIME: Self = Self(1 << 7);
    pub const WITH_DASH_SEPARATOR_IN_DATE: Self = Self(1 << 8);
    pub const WITH_COLON_SEPARATOR_IN_TIME: Self = Self(1 << 9);
    pub const WITH_COLON_SEPARATOR_IN_TIME_ZONE: Self = Self(1 << 10);
    pub const WITH_FRACTIONAL_SECONDS: Self = Self(1 << 11);
    pub const WITH_FULL_DATE: Self =
        Self(Self::WITH_YEAR.0 | Self::WITH_MONTH.0 | Self::WITH_DAY.0 | Self::WITH_DASH_SEPARATOR_IN_DATE.0);
    pub const WITH_FULL_TIME: Self = Self(
        Self::WITH_TIME.0
            | Self::WITH_TIME_ZONE.0
            | Self::WITH_COLON_SEPARATOR_IN_TIME.0
            | Self::WITH_COLON_SEPARATOR_IN_TIME_ZONE.0,
    );
    pub const WITH_INTERNET_DATE_TIME: Self = Self(Self::WITH_FULL_DATE.0 | Self::WITH_FULL_TIME.0);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// clears the bits of other
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}

impl BitOr for ISO8601DateFormatterOptions {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOrAssign for ISO8601DateFormatterOptions {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

#[derive(Clone, Debug)]
pub struct ISO8601DateFormatter {
    format_options: ISO8601DateFormatterOptions,
    time_zone: TimeZone,
}

impl Default for ISO8601DateFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl ISO8601DateFormatter {
    /// new formatter, internet date time in gmt
    pub fn new() -> Self {
        Self { format_options: ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME, time_zone: TimeZone::gmt() }
    }

    pub fn format_options(&self) -> ISO8601DateFormatterOptions {
        self.format_options
    }

    pub fn set_format_options(&mut self, format_options: ISO8601DateFormatterOptions) {
        self.format_options = format_options;
    }

    pub fn time_zone(&self) -> &TimeZone {
        &self.time_zone
    }

    pub fn set_time_zone(&mut self, time_zone: TimeZone) {
        self.time_zone = time_zone;
    }

    pub fn string_from(&self, date: &Date) -> String {
        let offset_seconds = i64::from(self.time_zone.seconds_from_gmt(date));
        let local = date.ms().saturating_add(offset_seconds * MS_PER_SECOND);
        let day = local.div_euclid(MS_PER_DAY);
        let time = local.rem_euclid(MS_PER_DAY);
        let (year, month, month_day) = civil_from_days(day);
        let (week_year, week) = iso_week(day);
        let mut out = String::new();
        for field in fields(self.format_options) {
            match field {
                Field::Literal(byte) => out.push(char::from(byte)),
                Field::Year { week_based } => {
                    out.push_str(&format!("{:04}", if week_based { week_year } else { year }));
                }
                Field::Month => out.push_str(&format!("{month:02}")),
                Field::Week => out.push_str(&format!("W{week:02}")),
                Field::WeekDay => out.push_str(&format!("{:02}", iso_weekday(day))),
                Field::MonthDay => out.push_str(&format!("{month_day:02}")),
                Field::OrdinalDay => out.push_str(&format!("{:03}", day - days_from_civil(year, 1, 1) + 1)),
                Field::Hour => out.push_str(&format!("{:02}", time / MS_PER_HOUR)),
                Field::Minute => out.push_str(&format!("{:02}", time % MS_PER_HOUR / MS_PER_MINUTE)),
                Field::Second => out.push_str(&format!("{:02}", time % MS_PER_MINUTE / MS_PER_SECOND)),
                Field::Fraction => out.push_str(&format!("{:03}", time % MS_PER_SECOND)),
                Field::Zone { colon } => push_zone(&mut out, offset_seconds, colon),
            }
        }
        out
    }

    /// parses a date, none unless the whole string matches the options
    pub fn date_from(&self, string: &str) -> Option<Date> {
        let mut cursor = Cursor { bytes: string.as_bytes(), position: 0 };
        let mut parsed = Parsed::default();
        for field in fields(self.format_options) {
            match field {
                Field::Literal(byte) => cursor.literal(byte)?,
                Field::Year { .. } => parsed.year = Some(cursor.digits(4)?),
                Field::Month => parsed.month = Some(cursor.digits(2)?),
                Field::Week => {
                    cursor.literal(b'W')?;
                    parsed.week = Some(cursor.digits(2)?);
                }
                Field::WeekDay => parsed.weekday = Some(cursor.digits(2)?),
                Field::MonthDay => parsed.month_day = Some(cursor.digits(2)?),
                Field::OrdinalDay => parsed.ordinal_day = Some(cursor.digits(3)?),
                Field::Hour => parsed.hour = cursor.digits(2)?,
                Field::Minute => parsed.minute = cursor.digits(2)?,
                Field::Second => parsed.second = cursor.digits(2)?,
                Field::Fraction => parsed.millisecond = cursor.fraction()?,
                Field::Zone { .. } => parsed.offset_seconds = Some(cursor.zone()?),
            }
        }
        if !cursor.is_at_end() {
            return None;
        }
        parsed.resolve(&self.time_zone)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Literal(u8),
    Year { week_based: bool },
    Month,
    Week,
    WeekDay,
    MonthDay,
    OrdinalDay,
    Hour,
    Minute,
    Second,
    Fraction,
    Zone { colon: bool },
}

fn fields(options: ISO8601DateFormatterOptions) -> Vec<Field> {
    type Options = ISO8601DateFormatterOptions;
    let dash = options.contains(Options::WITH_DASH_SEPARATOR_IN_DATE);
    let week_based = options.contains(Options::WITH_WEEK_OF_YEAR);
    let mut fields = Vec::new();
    if options.contains(Options::WITH_YEAR) {
        push_date_field(&mut fields, dash, Field::Year { week_based });
    }
    if options.contains(Options::WITH_MONTH) {
        push_date_field(&mut fields, dash, Field::Month);
    }
    if week_based {
        push_date_field(&mut fields, dash, Field::Week);
    }
    if options.contains(Options::WITH_DAY) {
        let day = if week_based {
            Field::WeekDay
        } else if options.contains(Options::WITH_MONTH) {
            Field::MonthDay
        } else {
            Field::OrdinalDay
        };
        push_date_field(&mut fields, dash, day);
    }
    if options.contains(Options::WITH_TIME) {
        if !fields.is_empty() {
            let separator = if options.contains(Options::WITH_SPACE_BETWEEN_DATE_AND_TIME) { b' ' } else { b'T' };
            fields.push(Field::Literal(separator));
        }
        let colon = options.contains(Options::WITH_COLON_SEPARATOR_IN_TIME);
        fields.push(Field::Hour);
        if colon {
            fields.push(Field::Literal(b':'));
        }
        fields.push(Field::Minute);
        if colon {
            fields.push(Field::Literal(b':'));
        }
        fields.push(Field::Second);
        if options.contains(Options::WITH_FRACTIONAL_SECONDS) {
            fields.push(Field::Literal(b'.'));
            fields.push(Field::Fraction);
        }
    }
    if options.contains(Options::WITH_TIME_ZONE) {
        fields.push(Field::Zone { colon: options.contains(Options::WITH_COLON_SEPARATOR_IN_TIME_ZONE) });
    }
    fields
}

fn push_date_field(fields: &mut Vec<Field>, dash: bool, field: Field) {
    if dash && !fields.is_empty() {
        fields.push(Field::Literal(b'-'));
    }
    fields.push(field);
}

fn push_zone(out: &mut String, offset_seconds: i64, colon: bool) {
    if offset_seconds == 0 {
        out.push('Z');
        return;
    }
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let magnitude = offset_seconds.abs();
    let hours = magnitude / SECONDS_PER_HOUR;
    let minutes = magnitude % SECONDS_PER_HOUR / SECONDS_PER_MINUTE;
    let separator = if colon { ":" } else { "" };
    out.push_str(&format!("{sign}{hours:02}{separator}{minutes:02}"));
}

#[derive(Default)]
struct Parsed {
    year: Option<i64>,
    month: Option<i64>,
    week: Option<i64>,
    weekday: Option<i64>,
    month_day: Option<i64>,
    ordinal_day: Option<i64>,
    hour: i64,
    minute: i64,
    second: i64,
    millisecond: i64,
    offset_seconds: Option<i64>,
}

impl Parsed {
    fn resolve(&self, time_zone: &TimeZone) -> Option<Date> {
        let year = self.year.unwrap_or(UNIX_EPOCH_YEAR);
        let day = if let Some(week) = self.week {
            week_date_day(year, week, self.weekday.unwrap_or(1))?
        } else if let Some(ordinal_day) = self.ordinal_day {
            if !(1..=days_in_year(year)).contains(&ordinal_day) {
                return None;
            }
            days_from_civil(year, 1, 1) + ordinal_day - 1
        } else {
            let month = self.month.unwrap_or(1);
            let month_day = self.month_day.unwrap_or(1);
            if !(1..=12).contains(&month) || !(1..=days_in_month(year, month)).contains(&month_day) {
                return None;
            }
            days_from_civil(year, month, month_day)
        };
        if self.hour > 23 || self.minute > 59 || self.second > 59 {
            return None;
        }
        let time = self.hour * MS_PER_HOUR + self.minute * MS_PER_MINUTE + self.second * MS_PER_SECOND + self.millisecond;
        let local = day.checked_mul(MS_PER_DAY)?.checked_add(time)?;
        let utc = match self.offset_seconds {
            Some(offset_seconds) => local.checked_sub(offset_seconds * MS_PER_SECOND)?,
            None => local_to_utc(local, time_zone)?,
        };
        Some(Date::from_ms(utc))
    }
}

fn local_to_utc(local: i64, time_zone: &TimeZone) -> Option<i64> {
    let first = i64::from(time_zone.seconds_from_gmt(&Date::from_ms(local))) * MS_PER_SECOND;
    let guess = local.checked_sub(first)?;
    let second = i64::from(time_zone.seconds_from_gmt(&Date::from_ms(guess))) * MS_PER_SECOND;
    local.checked_sub(second)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Cursor<'_> {
    fn is_at_end(&self) -> bool {
        self.position == self.bytes.len()
    }

    fn skip(&mut self, byte: u8) -> bool {
        if self.bytes.get(self.position) != Some(&byte) {
            return false;
        }
        self.position += 1;
        true
    }

    fn literal(&mut self, byte: u8) -> Option<()> {
        self.skip(byte).then_some(())
    }

    fn digits(&mut self, count: usize) -> Option<i64> {
        let end = self.position.checked_add(count)?;
        let slice = self.bytes.get(self.position..end)?;
        let mut value = 0_i64;
        for byte in slice {
            if !byte.is_ascii_digit() {
                return None;
            }
            value = value * 10 + i64::from(byte - b'0');
        }
        self.position = end;
        Some(value)
    }

    fn fraction(&mut self) -> Option<i64> {
        let bytes = self.bytes;
        let start = self.position;
        let mut millisecond = 0_i64;
        while let Some(byte) = bytes.get(self.position).filter(|byte| byte.is_ascii_digit()) {
            if self.position - start < FRACTION_DIGITS {
                millisecond = millisecond * 10 + i64::from(byte - b'0');
            }
            self.position += 1;
        }
        let taken = self.position - start;
        if taken == 0 {
            return None;
        }
        for _ in taken..FRACTION_DIGITS {
            millisecond *= 10;
        }
        Some(millisecond)
    }

    fn zone(&mut self) -> Option<i64> {
        if self.skip(b'Z') {
            return Some(0);
        }
        let sign = match self.bytes.get(self.position) {
            Some(b'+') => 1,
            Some(b'-') => -1,
            _ => return None,
        };
        self.position += 1;
        let hours = self.digits(2)?;
        let colon = self.skip(b':');
        let minutes = if !colon && self.is_at_end() { 0 } else { self.digits(2)? };
        if hours > MAX_ZONE_HOURS || minutes >= SECONDS_PER_MINUTE {
            return None;
        }
        Some(sign * (hours * SECONDS_PER_HOUR + minutes * SECONDS_PER_MINUTE))
    }
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_year(year: i64) -> i64 {
    if is_leap_year(year) { 366 } else { 365 }
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 };
    let year = year_of_era + era * 400;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn iso_weekday(days: i64) -> i64 {
    (days + 3).rem_euclid(DAYS_PER_WEEK) + 1
}

fn iso_week(days: i64) -> (i64, i64) {
    let thursday = days - (iso_weekday(days) - 1) + 3;
    let (week_year, _, _) = civil_from_days(thursday);
    (week_year, (thursday - days_from_civil(week_year, 1, 1)) / DAYS_PER_WEEK + 1)
}

fn week_date_day(week_year: i64, week: i64, weekday: i64) -> Option<i64> {
    let (_, weeks_in_year) = iso_week(days_from_civil(week_year, 12, 28));
    if !(1..=weeks_in_year).contains(&week) || !(1..=DAYS_PER_WEEK).contains(&weekday) {
        return None;
    }
    let january_fourth = days_from_civil(week_year, 1, 4);
    let first_monday = january_fourth - (iso_weekday(january_fourth) - 1);
    Some(first_monday + (week - 1) * DAYS_PER_WEEK + weekday - 1)
}
