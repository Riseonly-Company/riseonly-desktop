use std::sync::OnceLock;

use jiff::civil::DateTime;
use jiff::tz::{Offset, TimeZone as ZoneRules};
use jiff::{Span, Timestamp, Zoned};
use parking_lot::RwLock;

use super::foundation_date::Date;

const MAX_SECONDS_FROM_GMT: u32 = 18 * 3_600;
const SYSTEM_ZONE_LINK: &str = "/etc/localtime";
const ZONEINFO_MARKER: &str = "zoneinfo/";

static SYSTEM_ZONE_RULES: RwLock<Option<ZoneRules>> = RwLock::new(None);
static SYSTEM_ZONE_NAME_SOURCE: OnceLock<fn() -> Option<String>> = OnceLock::new();

/// iana name from a zone link target, everything after the last zoneinfo/
pub(super) fn zone_name_in_link_target(target: &str) -> Option<&str> {
    let (_, name) = target.rsplit_once(ZONEINFO_MARKER)?;
    if name.is_empty() { None } else { Some(name) }
}

fn read_system_zone_name() -> Option<String> {
    if let Some(name) = SYSTEM_ZONE_NAME_SOURCE.get().and_then(|source| source()) {
        return Some(name);
    }
    let target = std::fs::read_link(SYSTEM_ZONE_LINK).ok()?;
    zone_name_in_link_target(target.to_str()?).map(str::to_owned)
}

fn read_system_zone_rules() -> ZoneRules {
    if std::env::var_os("TZ").is_some() {
        return ZoneRules::system();
    }
    read_system_zone_name()
        .and_then(|name| ZoneRules::get(&name).ok())
        .unwrap_or_else(ZoneRules::system)
}

#[derive(Clone, Debug)]
pub struct TimeZone {
    rules: ZoneRules,
}

impl TimeZone {
    /// the system zone, utc when the system names none
    pub fn current() -> Self {
        if let Some(rules) = SYSTEM_ZONE_RULES.read().as_ref() {
            return Self { rules: rules.clone() };
        }
        let mut cached = SYSTEM_ZONE_RULES.write();
        let rules = cached.get_or_insert_with(read_system_zone_rules).clone();
        Self { rules }
    }

    /// resets the cached system zone so the next current reads it again
    pub fn reset_system() {
        *SYSTEM_ZONE_RULES.write() = None;
    }

    pub fn set_system_zone_name_source(source: fn() -> Option<String>) {
        if SYSTEM_ZONE_NAME_SOURCE.set(source).is_ok() {
            Self::reset_system();
        }
    }

    /// the utc zone
    pub fn gmt() -> Self {
        Self { rules: ZoneRules::UTC }
    }

    /// zone from an iana name, none when unknown
    pub fn with_identifier(identifier: &str) -> Option<Self> {
        ZoneRules::get(identifier).ok().map(|rules| Self { rules })
    }

    /// zone from an offset in seconds, none outside the 18 hour range
    pub fn with_seconds_from_gmt(seconds: i32) -> Option<Self> {
        if seconds.unsigned_abs() > MAX_SECONDS_FROM_GMT {
            return None;
        }
        Offset::from_seconds(seconds).ok().map(|offset| Self { rules: ZoneRules::fixed(offset) })
    }

    /// iana name or a GMT+hhmm style name for a fixed offset
    pub fn identifier(&self) -> String {
        if let Some(name) = self.rules.iana_name() {
            return name.to_owned();
        }
        let seconds = self.seconds_from_gmt(&Date::from_ms(0));
        if seconds == 0 {
            return "GMT".to_owned();
        }
        let sign = if seconds < 0 { '-' } else { '+' };
        let magnitude = seconds.unsigned_abs();
        format!("GMT{sign}{:02}{:02}", magnitude / 3_600, magnitude % 3_600 / 60)
    }

    pub fn seconds_from_gmt(&self, date: &Date) -> i32 {
        match Timestamp::from_millisecond(date.ms()) {
            Ok(instant) => self.rules.to_offset(instant).seconds(),
            Err(_) => 0,
        }
    }

    fn zoned(&self, date: &Date) -> Option<Zoned> {
        Timestamp::from_millisecond(date.ms()).ok().map(|instant| instant.to_zoned(self.rules.clone()))
    }
}

impl PartialEq for TimeZone {
    fn eq(&self, other: &Self) -> bool {
        let epoch = Date::from_ms(0);
        self.rules.iana_name() == other.rules.iana_name()
            && self.seconds_from_gmt(&epoch) == other.seconds_from_gmt(&epoch)
    }
}

/// calendar kinds, the app only builds gregorian ones
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CalendarIdentifier {
    Gregorian,
}

/// calendar components like year,
/// month,
/// day
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CalendarComponent {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    Nanosecond,
    Weekday,
    WeekOfYear,
}

/// date parts, every field optional
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DateComponents {
    pub year: Option<i64>,
    pub month: Option<i64>,
    pub day: Option<i64>,
    pub hour: Option<i64>,
    pub minute: Option<i64>,
    pub second: Option<i64>,
    pub nanosecond: Option<i64>,
    pub weekday: Option<i64>,
    pub week_of_year: Option<i64>,
    pub time_zone: Option<TimeZone>,
}

impl DateComponents {
    /// date parts from year, month and day
    pub fn ymd(year: i64, month: i64, day: i64) -> Self {
        Self { year: Some(year), month: Some(month), day: Some(day), ..Self::default() }
    }

    pub fn value(&self, component: CalendarComponent) -> Option<i64> {
        match component {
            CalendarComponent::Year => self.year,
            CalendarComponent::Month => self.month,
            CalendarComponent::Day => self.day,
            CalendarComponent::Hour => self.hour,
            CalendarComponent::Minute => self.minute,
            CalendarComponent::Second => self.second,
            CalendarComponent::Nanosecond => self.nanosecond,
            CalendarComponent::Weekday => self.weekday,
            CalendarComponent::WeekOfYear => self.week_of_year,
        }
    }

    fn set_value(&mut self, component: CalendarComponent, value: i64) {
        let slot = match component {
            CalendarComponent::Year => &mut self.year,
            CalendarComponent::Month => &mut self.month,
            CalendarComponent::Day => &mut self.day,
            CalendarComponent::Hour => &mut self.hour,
            CalendarComponent::Minute => &mut self.minute,
            CalendarComponent::Second => &mut self.second,
            CalendarComponent::Nanosecond => &mut self.nanosecond,
            CalendarComponent::Weekday => &mut self.weekday,
            CalendarComponent::WeekOfYear => &mut self.week_of_year,
        };
        *slot = Some(value);
    }
}

/// span between two dates, start and end both count as inside
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DateInterval {
    pub start: Date,
    pub end: Date,
}

impl DateInterval {
    /// length in seconds
    pub fn duration(&self) -> f64 {
        self.end.time_interval_since(&self.start)
    }

    /// true if the date is inside, the end counts too
    pub fn contains(&self, date: &Date) -> bool {
        *date >= self.start && *date <= self.end
    }
}

const DEFAULT_FIRST_WEEKDAY: u8 = 1;

const DEFAULT_MINIMUM_DAYS_IN_FIRST_WEEK: u8 = 1;

static CURRENT_FIRST_WEEKDAY: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(DEFAULT_FIRST_WEEKDAY);

static CURRENT_MINIMUM_DAYS_IN_FIRST_WEEK: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(DEFAULT_MINIMUM_DAYS_IN_FIRST_WEEK);

/// gregorian calendar with one zone and week rules,
/// first weekday 1 is sunday to 7 and minimum days in first week 1 to 7
#[derive(Clone, Debug, PartialEq)]
pub struct Calendar {
    pub identifier: CalendarIdentifier,
    pub time_zone: TimeZone,
    pub first_weekday: u8,
    pub minimum_days_in_first_week: u8,
}

impl Calendar {
    /// gregorian calendar in the system zone, sunday first
    pub fn gregorian() -> Self {
        Self {
            identifier: CalendarIdentifier::Gregorian,
            time_zone: TimeZone::current(),
            first_weekday: DEFAULT_FIRST_WEEKDAY,
            minimum_days_in_first_week: DEFAULT_MINIMUM_DAYS_IN_FIRST_WEEK,
        }
    }

    /// calendar in the system zone with the week rules of the users region
    pub fn current() -> Self {
        Self {
            first_weekday: CURRENT_FIRST_WEEKDAY.load(std::sync::atomic::Ordering::Relaxed),
            minimum_days_in_first_week: CURRENT_MINIMUM_DAYS_IN_FIRST_WEEK
                .load(std::sync::atomic::Ordering::Relaxed),
            ..Self::gregorian()
        }
    }

    pub fn install_current_first_weekday(first_weekday: u8) {
        let value = if (1..=7).contains(&first_weekday) { first_weekday } else { DEFAULT_FIRST_WEEKDAY };
        CURRENT_FIRST_WEEKDAY.store(value, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn install_current_minimum_days_in_first_week(minimum_days: u8) {
        let value =
            if (1..=7).contains(&minimum_days) { minimum_days } else { DEFAULT_MINIMUM_DAYS_IN_FIRST_WEEK };
        CURRENT_MINIMUM_DAYS_IN_FIRST_WEEK.store(value, std::sync::atomic::Ordering::Relaxed);
    }

    /// first instant of the local day, not always midnight
    pub fn start_of_day(&self, date: &Date) -> Date {
        self.time_zone
            .zoned(date)
            .and_then(|zoned| zoned.start_of_day().ok())
            .map_or(*date, |start| zoned_date(&start))
    }

    pub fn component(&self, component: CalendarComponent, date: &Date) -> i64 {
        self.time_zone.zoned(date).map_or(0, |zoned| self.read_component(component, &zoned))
    }

    pub fn date_components(&self, components: &[CalendarComponent], date: &Date) -> DateComponents {
        let mut out = DateComponents::default();
        if let Some(zoned) = self.time_zone.zoned(date) {
            for &component in components {
                out.set_value(component, self.read_component(component, &zoned));
            }
        }
        out
    }

    /// whole units from start to end, larger units first
    pub fn date_components_between(
        &self,
        components: &[CalendarComponent],
        start: &Date,
        end: &Date,
    ) -> DateComponents {
        let mut out = DateComponents::default();
        let (Some(mut cursor), Some(target)) = (self.time_zone.zoned(start), self.time_zone.zoned(end)) else {
            return out;
        };
        const ORDER: [CalendarComponent; 7] = [
            CalendarComponent::Year,
            CalendarComponent::Month,
            CalendarComponent::Day,
            CalendarComponent::Hour,
            CalendarComponent::Minute,
            CalendarComponent::Second,
            CalendarComponent::Nanosecond,
        ];
        for unit in ORDER {
            if !components.contains(&unit) {
                continue;
            }
            let count = whole_units_between(&cursor, &target, unit);
            if let Some(next) = add_units(&cursor, unit, count) {
                cursor = next;
            }
            out.set_value(unit, count);
        }
        out
    }

    /// date from components, lenient so month 13 is january of the next year
    pub fn date_from_components(&self, components: &DateComponents) -> Option<Date> {
        let zone = components.time_zone.as_ref().unwrap_or(&self.time_zone);
        let year = i16::try_from(components.year.unwrap_or(1)).ok()?;
        let base = DateTime::new(year, 1, 1, 0, 0, 0, 0).ok()?;
        let steps = [
            Span::new().try_months(components.month.unwrap_or(1) - 1).ok()?,
            Span::new().try_days(components.day.unwrap_or(1) - 1).ok()?,
            Span::new().try_hours(components.hour.unwrap_or(0)).ok()?,
            Span::new().try_minutes(components.minute.unwrap_or(0)).ok()?,
            Span::new().try_seconds(components.second.unwrap_or(0)).ok()?,
            Span::new().try_nanoseconds(components.nanosecond.unwrap_or(0)).ok()?,
        ];
        let mut local = base;
        for step in steps {
            local = local.checked_add(step).ok()?;
        }
        let zoned = local.to_zoned(zone.rules.clone()).ok()?;
        Some(zoned_date(&zoned))
    }

    /// adds a value of a component to a date, calendar math in the local zone
    pub fn date_by_adding(&self, component: CalendarComponent, value: i64, date: &Date) -> Option<Date> {
        let zoned = self.time_zone.zoned(date)?;
        let unit = match component {
            CalendarComponent::Weekday => CalendarComponent::Day,
            CalendarComponent::WeekOfYear => return add_weeks(&zoned, value).map(|next| zoned_date(&next)),
            other => other,
        };
        add_units(&zoned, unit, value).map(|next| zoned_date(&next))
    }

    /// true if both dates fall on the same day
    pub fn is_date_in_same_day_as(&self, date: &Date, other: &Date) -> bool {
        self.start_of_day(date) == self.start_of_day(other)
    }

    pub fn is_date_in_today(&self, date: &Date) -> bool {
        self.is_date_in_same_day_as(date, &Date::now())
    }

    pub fn is_date_in_yesterday(&self, date: &Date) -> bool {
        self.date_by_adding(CalendarComponent::Day, -1, &Date::now())
            .is_some_and(|yesterday| self.is_date_in_same_day_as(date, &yesterday))
    }

    /// true if both dates are equal down to the granularity
    pub fn is_date_equal_to(&self, date: &Date, other: &Date, granularity: CalendarComponent) -> bool {
        match (self.date_interval(granularity, date), self.date_interval(granularity, other)) {
            (Some(left), Some(right)) => left.start == right.start,
            _ => false,
        }
    }

    /// the local unit that contains the date, eg the whole week or month
    pub fn date_interval(&self, component: CalendarComponent, date: &Date) -> Option<DateInterval> {
        let zoned = self.time_zone.zoned(date)?;
        let day_start = zoned.start_of_day().ok()?;
        let start = match component {
            CalendarComponent::Year => day_start.with().month(1).day(1).build().ok()?.start_of_day().ok()?,
            CalendarComponent::Month => day_start.with().day(1).build().ok()?.start_of_day().ok()?,
            CalendarComponent::Day | CalendarComponent::Weekday => day_start,
            CalendarComponent::WeekOfYear => {
                let weekday = i64::from(day_start.weekday().to_sunday_one_offset());
                let back = (weekday - i64::from(self.first_weekday) + 7) % 7;
                add_units(&day_start, CalendarComponent::Day, -back)?.start_of_day().ok()?
            }
            CalendarComponent::Hour => zoned.with().minute(0).second(0).subsec_nanosecond(0).build().ok()?,
            CalendarComponent::Minute => zoned.with().second(0).subsec_nanosecond(0).build().ok()?,
            CalendarComponent::Second => zoned.with().subsec_nanosecond(0).build().ok()?,
            CalendarComponent::Nanosecond => zoned.clone(),
        };
        let end = match component {
            CalendarComponent::WeekOfYear => add_weeks(&start, 1)?,
            CalendarComponent::Weekday => add_units(&start, CalendarComponent::Day, 1)?,
            other => add_units(&start, other, 1)?,
        };
        let end = match component {
            CalendarComponent::Year
            | CalendarComponent::Month
            | CalendarComponent::Day
            | CalendarComponent::Weekday
            | CalendarComponent::WeekOfYear => end.start_of_day().ok()?,
            _ => end,
        };
        Some(DateInterval { start: zoned_date(&start), end: zoned_date(&end) })
    }

    fn read_component(&self, component: CalendarComponent, zoned: &Zoned) -> i64 {
        match component {
            CalendarComponent::Year => i64::from(zoned.year()),
            CalendarComponent::Month => i64::from(zoned.month()),
            CalendarComponent::Day => i64::from(zoned.day()),
            CalendarComponent::Hour => i64::from(zoned.hour()),
            CalendarComponent::Minute => i64::from(zoned.minute()),
            CalendarComponent::Second => i64::from(zoned.second()),
            CalendarComponent::Nanosecond => i64::from(zoned.subsec_nanosecond()),
            CalendarComponent::Weekday => i64::from(zoned.weekday().to_sunday_one_offset()),
            CalendarComponent::WeekOfYear => self.week_of_year(zoned),
        }
    }

    /// week 1 is the first week with at least minimum_days_in_first_week days
    /// early january may sit in last years week and late december in next years
    fn week_of_year(&self, zoned: &Zoned) -> i64 {
        let year = i64::from(zoned.year());
        let weekday = i64::from(zoned.weekday().to_sunday_one_offset());
        let day_index = i64::from(zoned.day_of_year()) - 1;
        let this_start = self.first_week_start(day_index, weekday);
        let days_in_year = i64::from(zoned.days_in_year());
        let next_start = days_in_year + self.first_week_start(day_index - days_in_year, weekday);
        if day_index >= next_start {
            return 1;
        }
        if day_index >= this_start {
            return (day_index - this_start) / 7 + 1;
        }
        let previous_days = days_in_gregorian_year(year - 1);
        let previous_index = day_index + previous_days;
        let previous_start = self.first_week_start(previous_index, weekday);
        (previous_index - previous_start) / 7 + 1
    }

    /// day index where week 1 begins, counted from january 1 and may be negative
    fn first_week_start(&self, day_index: i64, weekday: i64) -> i64 {
        let january_first_weekday = (weekday - 1 - day_index).rem_euclid(7) + 1;
        let back = (january_first_weekday - i64::from(self.first_weekday)).rem_euclid(7);
        if 7 - back >= i64::from(self.minimum_days_in_first_week) {
            -back
        } else {
            7 - back
        }
    }
}

fn days_in_gregorian_year(year: i64) -> i64 {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    if leap { 366 } else { 365 }
}

fn zoned_date(zoned: &Zoned) -> Date {
    Date::from_ms(zoned.timestamp().as_millisecond())
}

fn add_units(zoned: &Zoned, unit: CalendarComponent, value: i64) -> Option<Zoned> {
    let span = match unit {
        CalendarComponent::Year => Span::new().try_years(value).ok()?,
        CalendarComponent::Month => Span::new().try_months(value).ok()?,
        CalendarComponent::Day | CalendarComponent::Weekday => Span::new().try_days(value).ok()?,
        CalendarComponent::WeekOfYear => Span::new().try_weeks(value).ok()?,
        CalendarComponent::Hour => Span::new().try_hours(value).ok()?,
        CalendarComponent::Minute => Span::new().try_minutes(value).ok()?,
        CalendarComponent::Second => Span::new().try_seconds(value).ok()?,
        CalendarComponent::Nanosecond => Span::new().try_nanoseconds(value).ok()?,
    };
    zoned.checked_add(span).ok()
}

fn add_weeks(zoned: &Zoned, value: i64) -> Option<Zoned> {
    add_units(zoned, CalendarComponent::WeekOfYear, value)
}

fn whole_units_between(start: &Zoned, end: &Zoned, unit: CalendarComponent) -> i64 {
    let target = end.timestamp();
    let forward = target >= start.timestamp();
    let passes = |count: i64| match add_units(start, unit, count) {
        Some(moved) if forward => moved.timestamp() > target,
        Some(moved) => moved.timestamp() < target,
        None => true,
    };
    let step: i64 = if forward { 1 } else { -1 };
    let mut low = 0i64;
    let mut high = step;
    while !passes(high) {
        low = high;
        match high.checked_mul(2) {
            Some(next) => high = next,
            None => return low,
        }
    }
    while (high - low).abs() > 1 {
        let middle = low + (high - low) / 2;
        if passes(middle) {
            high = middle;
        } else {
            low = middle;
        }
    }
    low
}
