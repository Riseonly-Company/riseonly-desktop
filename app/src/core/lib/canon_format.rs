use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent, CalendarIdentifier, TimeZone};
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_format::{Argument, ScaleBand};
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::language_store::LanguageStore;
use rise_i18n::{FormatArg, format_printf, tr};
use rise_i18n_catalog::{
    RELATIVE_TIME_BANDS, RELATIVE_TIME_CLAMP_FUTURE_TO_ZERO, RELATIVE_TIME_SEPARATOR, RelativeTimeBand,
};

/// canon presentation rules, one implementation for the whole app
/// the compaction rule had nine rewrites with four different answers
pub enum RiseFormat {}

impl RiseFormat {
    /// fraction separator of the app language
    /// 2,3 MB in russian and 2.3 MB in english
    pub fn decimal_separator() -> &'static str {
        rise_format::decimal_separator(rise_i18n::I18n::shared().code())
    }

    /// counter compaction
    /// eg compact_count(1500) gives 1.5K and 2500000 gives 2.5M, below a thousand the bare number
    pub fn compact_count(value: i64) -> String {
        let magnitude = value.unsigned_abs();
        if magnitude < rise_format::COMPACT_PLAIN_BELOW {
            return value.to_string();
        }

        let negative = value < 0 && rise_format::COMPACT_SIGN == Self::SIGN_PRESERVE;
        let Some(mut band) = rise_format::COMPACT_BANDS
            .iter()
            .rev()
            .find(|band| magnitude >= band.threshold)
        else {
            return value.to_string();
        };

        let mut scaled = Self::scale(magnitude, band);

        if rise_format::COMPACT_PROMOTE_ON_CARRY
            && let Some(next) = rise_format::COMPACT_BANDS
                .iter()
                .find(|next| next.threshold > band.threshold)
            && scaled >= (next.divisor / band.divisor) as f64
        {
            band = next;
            scaled = Self::scale(magnitude, band);
        }

        let body = Self::decimal(
            scaled,
            band.fraction_digits as usize,
            rise_format::COMPACT_TRIM_TRAILING_ZERO_FRACTION,
            Self::decimal_separator(),
        );
        let shown = if negative { format!("-{body}") } else { body };
        format_printf(&tr(band.i18n_key), &[FormatArg::from(shown)])
    }

    /// now and 5 min ago and 3 d ago, bands come from canon languages
    /// translate resolves a band key to its label
    /// off main thread callers pass labels captured up front
    pub fn elapsed_translating(since_ms: i64, now_ms: i64, translate: &dyn Fn(&str) -> String) -> String {
        let mut delta = now_ms - since_ms;
        if delta < 0 && RELATIVE_TIME_CLAMP_FUTURE_TO_ZERO {
            delta = 0;
        }

        let band = Self::relative_band(delta);
        if !band.shows_count {
            return translate(band.key);
        }

        let count = (delta / band.divisor_ms.max(1)).max(0);
        if let Some(template_key) = Self::relative_time_template_key(&band) {
            let template = translate(template_key);
            if template != template_key {
                return format_printf(&template, &[FormatArg::Int(count)]);
            }
        }
        format!("{count}{RELATIVE_TIME_SEPARATOR}{}", translate(band.key))
    }

    /// elapsed with the live dictionary
    pub fn elapsed(since_ms: i64, now_ms: i64) -> String {
        Self::elapsed_translating(since_ms, now_ms, &|key| tr(key))
    }

    /// the count and ago as one phrase
    /// a label glued after the number cant do german vor 5 Min or french il y a 5 min
    pub fn relative_time_template_key(band: &RelativeTimeBand) -> Option<&'static str> {
        match band.name {
            Self::BAND_MINUTES => Some("story_time_minutes_ago"),
            Self::BAND_HOURS => Some("story_time_hours_ago"),
            Self::BAND_DAYS => Some("story_time_days_ago"),
            _ => None,
        }
    }

    pub fn elapsed_since_date(date: &Date, now: &Date) -> String {
        Self::elapsed(
            (date.time_interval_since_1970() * 1000.0) as i64,
            (now.time_interval_since_1970() * 1000.0) as i64,
        )
    }

    pub fn elapsed_duration(since_ms: i64, now_ms: i64) -> String {
        let delta = (now_ms - since_ms).max(0);
        let band = Self::relative_band(delta);
        let key = match band.name {
            Self::BAND_MINUTES => "duration_minutes_abbr",
            Self::BAND_HOURS => "duration_hours_abbr",
            Self::BAND_DAYS => "notify_time_day",
            _ => return tr(band.key),
        };
        format_printf(&tr(key), &[FormatArg::Int(delta / band.divisor_ms.max(1))])
    }

    fn relative_band(delta: i64) -> RelativeTimeBand {
        RELATIVE_TIME_BANDS
            .iter()
            .copied()
            .find(|band| band.upper_bound_ms.is_none_or(|bound| delta < bound))
            .unwrap_or(RELATIVE_TIME_BANDS[RELATIVE_TIME_BANDS.len() - 1])
    }

    /// number with only trailing digits visible, eg *******3272
    /// for the password recovery screen since someone may be streaming
    pub fn masked_phone(raw: &str) -> String {
        rise_format::masked_phone(raw)
    }

    /// byte size per canon, base 1024
    /// b and kb as integers and one fraction digit above
    /// trailing zero stays so 1.0 GB and 1.5 GB line up
    pub fn file_size(bytes: i64) -> String {
        let magnitude = Self::file_size_magnitude(bytes);
        Self::render_file_size(magnitude, Self::file_size_unit(magnitude))
    }

    /// size in another numbers unit, eg 3.2 / 10.0 MB
    /// the total dictates the unit or half a megabyte prints as 512 KB / 10.0 MB
    pub fn file_size_unit_of(bytes: i64, reference: i64) -> String {
        let reference_magnitude = Self::file_size_magnitude(reference);
        Self::render_file_size(
            Self::file_size_magnitude(bytes),
            Self::file_size_unit(reference_magnitude),
        )
    }

    fn file_size_magnitude(bytes: i64) -> u64 {
        let clamped = if rise_format::FILE_SIZE_NEGATIVE == Self::CLAMP_TO_ZERO {
            bytes.max(0)
        } else {
            bytes
        };
        clamped.unsigned_abs()
    }

    fn file_size_unit(magnitude: u64) -> &'static ScaleBand {
        let units = &rise_format::FILE_SIZE_UNITS;
        let Some(mut unit) = units.iter().rev().find(|unit| magnitude >= unit.threshold) else {
            return &units[0];
        };

        if rise_format::FILE_SIZE_PROMOTE_ON_CARRY
            && let Some(next) = units.iter().find(|next| next.threshold > unit.threshold)
            && Self::scale(magnitude, unit) >= (next.divisor / unit.divisor) as f64
        {
            unit = next;
        }
        unit
    }

    fn render_file_size(magnitude: u64, unit: &ScaleBand) -> String {
        let scaled = Self::scale(magnitude, unit);
        match unit.argument {
            Argument::Integer => format_printf(&tr(unit.i18n_key), &[FormatArg::Int(scaled as i64)]),
            Argument::Decimal => {
                let body = Self::decimal(
                    scaled,
                    unit.fraction_digits as usize,
                    rise_format::FILE_SIZE_TRIM_TRAILING_ZERO_FRACTION,
                    Self::decimal_separator(),
                );
                format_printf(&tr(unit.i18n_key), &[FormatArg::from(body)])
            }
        }
    }

    /// eg 5:03 and from one hour 1:05:03, leading field isnt zero padded
    pub fn duration(total_seconds: i64) -> String {
        let total = if rise_format::DURATION_NEGATIVE == Self::CLAMP_TO_ZERO {
            total_seconds.max(0)
        } else {
            total_seconds
        };
        let leading = rise_format::DURATION_LEADING_DIGITS as usize;
        let trailing = rise_format::DURATION_TRAILING_DIGITS as usize;
        let separator = rise_format::DURATION_SEPARATOR;

        let seconds = total % 60;
        let minutes = (total / 60) % 60;

        if total < rise_format::DURATION_HOURS_THRESHOLD_SECONDS as i64 {
            return format!(
                "{}{separator}{}",
                Self::pad(minutes, leading),
                Self::pad(seconds, trailing)
            );
        }

        format!(
            "{}{separator}{}{separator}{}",
            Self::pad(total / 3600, leading),
            Self::pad(minutes, trailing),
            Self::pad(seconds, trailing)
        )
    }

    /// seconds are truncated and not rounded, a player mustnt show a time it hasnt reached
    pub fn duration_time_interval(seconds: TimeInterval) -> String {
        if !seconds.is_finite() {
            return Self::duration(0);
        }
        let floored = if rise_format::DURATION_SECONDS_ROUNDING == Self::ROUNDING_FLOOR {
            seconds.floor()
        } else {
            seconds.round()
        };
        let bounded = floored.clamp(-Self::DURATION_BOUND_SECONDS, Self::DURATION_BOUND_SECONDS);
        Self::duration(bounded as i64)
    }

    /// durations come from the wire in milliseconds so the division lives here and not at every call site
    pub fn duration_milliseconds(milliseconds: i64) -> String {
        let clamped = if rise_format::DURATION_NEGATIVE == Self::CLAMP_TO_ZERO {
            milliseconds.max(0)
        } else {
            milliseconds
        };
        let per_second = rise_format::DURATION_SOURCE_PER_SECOND as i64;
        let bound = Self::DURATION_BOUND_SECONDS as i64;
        Self::duration((clamped / per_second).clamp(-bound, bound))
    }

    pub fn spelled_duration(milliseconds: i64) -> String {
        let clamped = if rise_format::DURATION_NEGATIVE == Self::CLAMP_TO_ZERO {
            milliseconds.max(0)
        } else {
            milliseconds
        };
        let per_second = rise_format::DURATION_SOURCE_PER_SECOND as i64;
        let total = (clamped / per_second).clamp(0, i64::from(i32::MAX));
        let hours = total / 3600;
        let minutes = (total % 3600) / 60;
        let seconds = total % 60;
        if hours > 0 {
            return format_printf(
                &tr("duration_hours_minutes_abbr"),
                &[FormatArg::Int(hours), FormatArg::Int(minutes)],
            );
        }
        if minutes > 0 {
            return if seconds == 0 {
                format_printf(&tr("duration_minutes_abbr"), &[FormatArg::Int(minutes)])
            } else {
                format_printf(
                    &tr("duration_minutes_seconds_abbr"),
                    &[FormatArg::Int(minutes), FormatArg::Int(seconds)],
                )
            };
        }
        format_printf(&tr("duration_seconds_abbr"), &[FormatArg::Int(seconds)])
    }

    /// time of day by the device 12/24 hour setting, 15:07 or 3:07 PM
    /// canon rule device follows the system, a fixed cycle shows 15:07 everywhere
    pub fn clock(hour: i64, minute: i64) -> String {
        format!(
            "{}{}{}",
            Self::pad(hour, rise_format::CLOCK_HOUR_DIGITS as usize),
            rise_format::CLOCK_SEPARATOR,
            Self::pad(minute, rise_format::CLOCK_MINUTE_DIGITS as usize)
        )
    }

    pub fn clock_date(date: &Date, calendar: &Calendar, twelve_hour: bool) -> String {
        if twelve_hour {
            let locale = Self::app_locale();
            let pattern = RiseDeviceClockCycle::shared().twelve_hour_pattern(&locale);
            return Self::date_formatter(&pattern, &locale, calendar, None).string_from(date);
        }
        let components = calendar.date_components(&[CalendarComponent::Hour, CalendarComponent::Minute], date);
        Self::clock(
            components.value(CalendarComponent::Hour).unwrap_or(0),
            components.value(CalendarComponent::Minute).unwrap_or(0),
        )
    }

    /// clock for background bubble preparation
    /// the calendar comes from a cache and preparation runs over the whole visible history at once
    pub fn clock_in_time_zone(date: &Date, time_zone_identifier: &str, twelve_hour: bool) -> String {
        Self::clock_date(
            date,
            &RiseFormatCalendarCache::shared().calendar(time_zone_identifier),
            twelve_hour,
        )
    }

    /// true when canon leaves the cycle to the device and the system is on 12 hour time
    pub fn uses_twelve_hour_clock() -> bool {
        rise_format::CLOCK_HOUR_CYCLE == Self::HOUR_CYCLE_DEVICE && RiseDeviceClockCycle::shared().twelve_hour()
    }

    pub fn refresh_device_clock_cycle() -> bool {
        RiseDeviceClockCycle::shared().refresh()
    }

    /// fraction of one as 42%
    /// clamping is needed, the server once sent 1.0000001
    pub fn percent(fraction: f64) -> String {
        let source = if fraction.is_nan() { rise_format::PERCENT_CLAMP_LOW } else { fraction };
        let clamped = source.clamp(rise_format::PERCENT_CLAMP_LOW, rise_format::PERCENT_CLAMP_HIGH);
        let digits = rise_format::PERCENT_FRACTION_DIGITS as usize;
        let power = 10f64.powi(digits as i32);
        let scaled = ((clamped * rise_format::PERCENT_SCALE as f64) * power).round() / power;
        let body = Self::decimal(scaled, digits, false, Self::decimal_separator());
        format!("{body}{}", rise_format::PERCENT_SUFFIX)
    }

    pub fn calendar_band(date: &Date, now: &Date, calendar: &Calendar) -> CalendarBand {
        let day = calendar.start_of_day(date);
        let today = calendar.start_of_day(now);

        if day == today {
            return CalendarBand::Today;
        }
        if day > today && rise_format::CALENDAR_FUTURE_COLLAPSES_TO_TODAY {
            return CalendarBand::Today;
        }

        if calendar.date_by_adding(CalendarComponent::Day, -1, &today) == Some(day) {
            return CalendarBand::Yesterday;
        }
        if calendar.is_date_equal_to(date, now, CalendarComponent::Year) {
            return CalendarBand::ThisYear;
        }
        CalendarBand::Older
    }

    /// dictionary key for the band word form (Today) when it has one
    pub fn calendar_word_key(band: CalendarBand, short: bool) -> Option<&'static str> {
        let canon = Self::canon_calendar_band(band)?;
        if short { canon.short_word_i18n_key } else { canon.word_i18n_key }
    }

    /// date only pattern key when the band prints as a date
    pub fn calendar_date_key(band: CalendarBand) -> Option<&'static str> {
        Self::canon_calendar_band(band)?.date_i18n_key
    }

    /// short date pattern key (11 Aug.) when the band prints as a date
    pub fn calendar_short_date_key(band: CalendarBand) -> Option<&'static str> {
        Self::canon_calendar_band(band)?.short_date_i18n_key
    }

    /// date and time pattern key, every band has one
    pub fn calendar_date_time_key(band: CalendarBand) -> Option<&'static str> {
        Self::canon_calendar_band(band).map(|canon| canon.date_time_i18n_key)
    }

    /// day label, today and yesterday as words and older days as a date in the locale pattern
    pub fn day_label(date: &Date, now: &Date, calendar: &Calendar, locale: &str, short: bool) -> String {
        let band = Self::calendar_band(date, now, calendar);
        if let Some(key) = Self::calendar_word_key(band, short) {
            return tr(key);
        }
        let Some(key) = Self::calendar_date_key(band).or_else(|| Self::calendar_date_time_key(band)) else {
            return String::new();
        };
        Self::date_formatter(&tr(key), locale, calendar, None).string_from(date)
    }

    pub fn compact_day_label(date: &Date, now: &Date, calendar: &Calendar, locale: &str) -> String {
        let band = Self::calendar_band(date, now, calendar);
        if let Some(key) = Self::calendar_word_key(band, true) {
            return tr(key);
        }
        let Some(key) = Self::calendar_short_date_key(band)
            .or_else(|| Self::calendar_date_key(band))
            .or_else(|| Self::calendar_date_time_key(band))
        else {
            return String::new();
        };
        Self::date_formatter(&tr(key), locale, calendar, None).string_from(date)
    }

    /// date with time, eg Today at 15:07 or 3 February 2019 at 15:07
    pub fn date_time_label(date: &Date, now: &Date, calendar: &Calendar, locale: &str) -> String {
        let band = Self::calendar_band(date, now, calendar);
        let Some(key) = Self::calendar_date_time_key(band) else {
            return String::new();
        };
        Self::date_formatter(&tr(key), locale, calendar, None).string_from(date)
    }

    /// shared date formatter cache
    /// a formatter costs an icu lookup and in a thirty row list that shows in the frame
    pub fn date_formatter(
        pattern: &str,
        locale: &str,
        calendar: &Calendar,
        time_zone: Option<&TimeZone>,
    ) -> Arc<DateFormatter> {
        RiseFormatDateFormatterCache::shared().formatter(
            pattern,
            locale,
            calendar,
            time_zone.unwrap_or(&calendar.time_zone),
        )
    }

    pub fn app_locale() -> String {
        LanguageStore::current_locale().identifier().to_string()
    }

    fn canon_calendar_band(band: CalendarBand) -> Option<&'static rise_format::CalendarBand> {
        static BY_NAME: LazyLock<HashMap<&'static str, &'static rise_format::CalendarBand>> = LazyLock::new(|| {
            let mut map = HashMap::new();
            for band in &rise_format::CALENDAR_BANDS {
                map.entry(band.name).or_insert(band);
            }
            map
        });
        BY_NAME.get(band.raw_value()).copied()
    }

    fn scale(magnitude: u64, band: &ScaleBand) -> f64 {
        let raw = magnitude as f64 / band.divisor as f64;
        let power = 10f64.powi(band.fraction_digits as i32);
        (raw * power).round() / power
    }

    fn decimal(value: f64, digits: usize, trim_zero_fraction: bool, separator: &str) -> String {
        let mut text = format!("{value:.digits$}");
        if trim_zero_fraction && digits > 0 {
            let zeros = format!(".{}", "0".repeat(digits));
            if text.ends_with(&zeros) {
                text.truncate(text.len() - zeros.len());
            }
        }
        if separator != "." {
            text = text.replace('.', separator);
        }
        text
    }

    /// manual zero padding
    /// a format call on every seek bar frame costs more than joining two short strings
    fn pad(value: i64, digits: usize) -> String {
        let text = value.to_string();
        if text.len() >= digits {
            return text;
        }
        format!("{}{text}", "0".repeat(digits - text.len()))
    }

    const SIGN_PRESERVE: &'static str = "preserve";
    const CLAMP_TO_ZERO: &'static str = "clamp_to_zero";
    const ROUNDING_FLOOR: &'static str = "floor";
    const HOUR_CYCLE_DEVICE: &'static str = "device";
    const BAND_MINUTES: &'static str = "minutes";
    const BAND_HOURS: &'static str = "hours";
    const BAND_DAYS: &'static str = "days";
    const DURATION_BOUND_SECONDS: f64 = 1e9;
}

/// calendar band, the boundary follows the viewers calendar day and not elapsed milliseconds
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CalendarBand {
    Today,
    Yesterday,
    ThisYear,
    Older,
}

impl CalendarBand {
    /// canon band name
    pub fn raw_value(self) -> &'static str {
        match self {
            CalendarBand::Today => "today",
            CalendarBand::Yesterday => "yesterday",
            CalendarBand::ThisYear => "this_year",
            CalendarBand::Older => "older",
        }
    }
}

/// device 12/24 hour setting, read once and refreshed on locale change or window activation
/// behind a lock since background bubble preparation reads it
struct RiseDeviceClockCycle {
    state: Mutex<RiseDeviceClockCycleState>,
}

struct RiseDeviceClockCycleState {
    cached: bool,
    patterns: HashMap<String, String>,
}

impl RiseDeviceClockCycle {
    fn shared() -> &'static RiseDeviceClockCycle {
        static SHARED: LazyLock<RiseDeviceClockCycle> = LazyLock::new(|| RiseDeviceClockCycle {
            state: Mutex::new(RiseDeviceClockCycleState {
                cached: RiseDeviceClockCycle::read_device(),
                patterns: HashMap::new(),
            }),
        });
        &SHARED
    }

    fn twelve_hour(&self) -> bool {
        self.state.lock().cached
    }

    fn refresh(&self) -> bool {
        let current = Self::read_device();
        let mut state = self.state.lock();
        if current == state.cached {
            return false;
        }
        state.cached = current;
        true
    }

    fn twelve_hour_pattern(&self, locale: &str) -> String {
        let mut state = self.state.lock();
        if let Some(cached) = state.patterns.get(locale) {
            return cached.clone();
        }
        let pattern = DateFormatter::date_format_from_template(Self::TWELVE_HOUR_TEMPLATE, 0, locale)
            .unwrap_or_else(|| Self::TWELVE_HOUR_FALLBACK.to_owned());
        if state.patterns.len() >= Self::PATTERN_LIMIT {
            state.patterns.clear();
        }
        state.patterns.insert(locale.to_owned(), pattern.clone());
        pattern
    }

    fn read_device() -> bool {
        rise_platform::device_hour_cycle::prefers_twelve_hour()
    }

    const TWELVE_HOUR_TEMPLATE: &'static str = "hmma";
    const TWELVE_HOUR_FALLBACK: &'static str = "h:mm a";
    const PATTERN_LIMIT: usize = 8;
}

struct RiseFormatCalendarCache {
    storage: Mutex<HashMap<String, Calendar>>,
}

impl RiseFormatCalendarCache {
    fn shared() -> &'static RiseFormatCalendarCache {
        static SHARED: LazyLock<RiseFormatCalendarCache> = LazyLock::new(|| RiseFormatCalendarCache {
            storage: Mutex::new(HashMap::new()),
        });
        &SHARED
    }

    fn calendar(&self, time_zone_identifier: &str) -> Calendar {
        let mut storage = self.storage.lock();
        if let Some(cached) = storage.get(time_zone_identifier) {
            return cached.clone();
        }

        let mut calendar = Calendar::gregorian();
        calendar.time_zone = TimeZone::with_identifier(time_zone_identifier).unwrap_or_else(TimeZone::gmt);
        if storage.len() >= Self::LIMIT {
            storage.clear();
        }
        storage.insert(time_zone_identifier.to_owned(), calendar.clone());
        calendar
    }

    const LIMIT: usize = 8;
}

struct RiseFormatDateFormatterCache {
    state: Mutex<RiseFormatDateFormatterCacheState>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct RiseFormatDateFormatterKey {
    pattern: String,
    locale: String,
    calendar: CalendarIdentifier,
    time_zone: String,
}

#[derive(Default)]
struct RiseFormatDateFormatterCacheState {
    storage: HashMap<RiseFormatDateFormatterKey, Arc<DateFormatter>>,
    order: VecDeque<RiseFormatDateFormatterKey>,
}

impl RiseFormatDateFormatterCache {
    fn shared() -> &'static RiseFormatDateFormatterCache {
        static SHARED: LazyLock<RiseFormatDateFormatterCache> = LazyLock::new(|| RiseFormatDateFormatterCache {
            state: Mutex::new(RiseFormatDateFormatterCacheState::default()),
        });
        &SHARED
    }

    fn formatter(&self, pattern: &str, locale: &str, calendar: &Calendar, time_zone: &TimeZone) -> Arc<DateFormatter> {
        let key = RiseFormatDateFormatterKey {
            pattern: pattern.to_owned(),
            locale: locale.to_owned(),
            calendar: calendar.identifier,
            time_zone: time_zone.identifier(),
        };

        let mut state = self.state.lock();
        if let Some(cached) = state.storage.get(&key) {
            return cached.clone();
        }

        let mut formatter = DateFormatter::new();
        formatter.set_locale(locale);
        formatter.set_calendar(calendar.clone());
        formatter.set_time_zone(time_zone.clone());
        formatter.set_date_format(pattern);
        let formatter = Arc::new(formatter);

        if state.order.len() >= Self::LIMIT
            && let Some(evicted) = state.order.pop_front()
        {
            state.storage.remove(&evicted);
        }
        state.order.push_back(key.clone());
        state.storage.insert(key, formatter.clone());
        formatter
    }

    const LIMIT: usize = 16;
}
