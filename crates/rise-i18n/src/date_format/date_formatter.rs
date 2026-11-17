use std::sync::Mutex;

use icu_calendar::Gregorian;
use icu_datetime::fieldsets::builder::{DateFields, FieldSetBuilder};
use icu_datetime::fieldsets::enums::CompositeDateTimeFieldSet;
use icu_datetime::input::{Date as IcuDate, DateTime as IcuDateTime, Time as IcuTime};
use icu_datetime::options::{Length, TimePrecision};
use icu_datetime::pattern::{DateTimePattern, FixedCalendarDateTimeNames};
use icu_datetime::preferences::HourCycle;
use icu_datetime::{DateTimeFormatterPreferences, FixedCalendarDateTimeFormatter};
use icu_locale_core::Locale as IcuLocale;
use writeable::{TryWriteable, Writeable};

use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent, DateComponents, TimeZone};
use rise_core::foundation::foundation_date::Date;

use super::date_format_pattern::{PatternToken, parse_numeric, render_numeric, tokenize};

/// date or time style
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Style {
    #[default]
    None,
    Short,
    Medium,
    Long,
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Source {
    Pattern(String),
    Styles,
    Template(String),
}

enum Engine {
    Pattern { names: Box<FixedCalendarDateTimeNames<Gregorian, CompositeDateTimeFieldSet>>, pattern: DateTimePattern },
    Fields(Box<FixedCalendarDateTimeFormatter<Gregorian, CompositeDateTimeFieldSet>>),
    Numeric,
    Empty,
}

pub struct DateFormatter {
    source: Source,
    date_style: Style,
    time_style: Style,
    locale: String,
    calendar: Calendar,
    time_zone: TimeZone,
    engine: Mutex<Option<Engine>>,
}

impl Default for DateFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for DateFormatter {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            date_style: self.date_style,
            time_style: self.time_style,
            locale: self.locale.clone(),
            calendar: self.calendar.clone(),
            time_zone: self.time_zone.clone(),
            engine: Mutex::new(None),
        }
    }
}

impl DateFormatter {
    /// new formatter, empty pattern and system zone
    /// app language as locale since desktop has no separate region locale
    pub fn new() -> Self {
        Self {
            source: Source::Pattern(String::new()),
            date_style: Style::None,
            time_style: Style::None,
            locale: crate::i18n::I18n::shared().code().to_owned(),
            calendar: Calendar::gregorian(),
            time_zone: TimeZone::current(),
            engine: Mutex::new(None),
        }
    }

    /// localized pattern for a skeleton, eg hmma gives h:mm a in english
    /// j is the locale own hour symbol, options is reserved and ignored
    pub fn date_format_from_template(template: &str, _options: usize, locale: &str) -> Option<String> {
        let (builder, hour_cycle) = template_builder(&tokenize(template));
        let formatter = fields_formatter(preferences_for(locale, hour_cycle), builder)?;
        resolved_pattern(&formatter)
    }

    /// the fixed pattern, or the localized one a style or template resolved to
    pub fn date_format(&self) -> String {
        if let Source::Pattern(pattern) = &self.source {
            return pattern.clone();
        }
        let Ok(mut slot) = self.engine.lock() else {
            return String::new();
        };
        match slot.get_or_insert_with(|| self.build_engine()) {
            Engine::Fields(formatter) => resolved_pattern(formatter).unwrap_or_default(),
            Engine::Pattern { .. } | Engine::Numeric | Engine::Empty => String::new(),
        }
    }

    pub fn set_date_format(&mut self, pattern: impl Into<String>) {
        self.source = Source::Pattern(pattern.into());
        self.invalidate();
    }

    pub fn date_style(&self) -> Style {
        self.date_style
    }

    pub fn set_date_style(&mut self, style: Style) {
        self.date_style = style;
        self.source = Source::Styles;
        self.invalidate();
    }

    pub fn time_style(&self) -> Style {
        self.time_style
    }

    pub fn set_time_style(&mut self, style: Style) {
        self.time_style = style;
        self.source = Source::Styles;
        self.invalidate();
    }

    pub fn set_localized_date_format_from_template(&mut self, template: impl Into<String>) {
        self.source = Source::Template(template.into());
        self.invalidate();
    }

    pub fn locale(&self) -> &str {
        &self.locale
    }

    pub fn set_locale(&mut self, identifier: impl Into<String>) {
        self.locale = identifier.into();
        self.invalidate();
    }

    /// the time zone
    pub fn calendar(&self) -> &Calendar {
        &self.calendar
    }

    /// sets the calendar, components are laid out by it in the formatter own zone
    /// the zone is kept separate from the calendar
    pub fn set_calendar(&mut self, calendar: Calendar) {
        self.calendar = calendar;
        self.invalidate();
    }

    pub fn time_zone(&self) -> &TimeZone {
        &self.time_zone
    }

    pub fn set_time_zone(&mut self, time_zone: TimeZone) {
        self.time_zone = time_zone;
    }

    pub fn string_from(&self, date: &Date) -> String {
        let components = self.local_components(date);
        let Some(input) = icu_input(&components) else {
            return String::new();
        };
        let Ok(mut slot) = self.engine.lock() else {
            return String::new();
        };
        let engine = slot.get_or_insert_with(|| self.build_engine());
        match engine {
            Engine::Pattern { names, pattern } => {
                match names.with_pattern_unchecked(pattern).format(&input).try_write_to_string() {
                    Ok(text) => text.into_owned(),
                    Err((_, lossy)) => lossy.into_owned(),
                }
            }
            Engine::Fields(formatter) => formatter.format(&input).write_to_string().into_owned(),
            Engine::Numeric => render_numeric(&tokenize(self.fixed_pattern()), &components).unwrap_or_default(),
            Engine::Empty => String::new(),
        }
    }

    /// parses a date, numeric patterns only
    /// thats all that is parsed with this formatter
    pub fn date_from(&self, text: &str) -> Option<Date> {
        let Source::Pattern(pattern) = &self.source else {
            return None;
        };
        parse_numeric(&tokenize(pattern), text, &self.time_zone)
    }

    fn fixed_pattern(&self) -> &str {
        match &self.source {
            Source::Pattern(pattern) => pattern,
            Source::Styles | Source::Template(_) => "",
        }
    }

    fn invalidate(&mut self) {
        self.engine = Mutex::new(None);
    }

    fn local_components(&self, date: &Date) -> DateComponents {
        let mut calendar = self.calendar.clone();
        calendar.time_zone = self.time_zone.clone();
        calendar.date_components(
            &[
                CalendarComponent::Year,
                CalendarComponent::Month,
                CalendarComponent::Day,
                CalendarComponent::Hour,
                CalendarComponent::Minute,
                CalendarComponent::Second,
                CalendarComponent::Nanosecond,
            ],
            date,
        )
    }

    fn preferences(&self) -> DateTimeFormatterPreferences {
        preferences_for(&self.locale, None)
    }

    fn build_engine(&self) -> Engine {
        match &self.source {
            Source::Pattern(pattern) if pattern.is_empty() => Engine::Empty,
            Source::Pattern(pattern) => self.pattern_engine(pattern),
            Source::Styles => self.fields_engine(self.preferences(), style_builder(self.date_style, self.time_style)),
            Source::Template(template) => {
                let (builder, hour_cycle) = template_builder(&tokenize(template));
                self.fields_engine(preferences_for(&self.locale, hour_cycle), builder)
            }
        }
    }

    fn pattern_engine(&self, pattern: &str) -> Engine {
        let Ok(parsed) = pattern.parse::<DateTimePattern>() else {
            return Engine::Numeric;
        };
        let Ok(mut names) = FixedCalendarDateTimeNames::<Gregorian, CompositeDateTimeFieldSet>::try_new(self.preferences())
        else {
            return Engine::Numeric;
        };
        if names.include_for_pattern(&parsed).is_err() {
            return Engine::Numeric;
        }
        Engine::Pattern { names: Box::new(names), pattern: parsed }
    }

    fn fields_engine(&self, preferences: DateTimeFormatterPreferences, builder: Option<FieldSetBuilder>) -> Engine {
        fields_formatter(preferences, builder).map_or(Engine::Empty, |formatter| Engine::Fields(Box::new(formatter)))
    }
}

type FieldsFormatter = FixedCalendarDateTimeFormatter<Gregorian, CompositeDateTimeFieldSet>;

fn preferences_for(locale: &str, hour_cycle: Option<HourCycle>) -> DateTimeFormatterPreferences {
    let identifier = locale.replace('_', "-");
    let locale = IcuLocale::try_from_str(&identifier).unwrap_or(IcuLocale::UNKNOWN);
    let mut preferences = DateTimeFormatterPreferences::from(&locale);
    if hour_cycle.is_some() {
        preferences.hour_cycle = hour_cycle;
    }
    preferences
}

fn fields_formatter(preferences: DateTimeFormatterPreferences, builder: Option<FieldSetBuilder>) -> Option<FieldsFormatter> {
    let field_set = builder?.build_composite_datetime().ok()?;
    FieldsFormatter::try_new(preferences, field_set).ok()
}

/// pattern a field set resolves to
/// cldr patterns vary with the value only for day periods and eras so one sample date is enough
fn resolved_pattern(formatter: &FieldsFormatter) -> Option<String> {
    let date = IcuDate::try_new_gregorian(PATTERN_SAMPLE_YEAR, 1, 1).ok()?;
    let time = IcuTime::try_new(0, 0, 0, 0).ok()?;
    let input = IcuDateTime { date, time };
    Some(formatter.format(&input).pattern().write_to_string().into_owned())
}

const PATTERN_SAMPLE_YEAR: i32 = 2000;

fn icu_input(components: &DateComponents) -> Option<IcuDateTime<Gregorian>> {
    let narrow = |value: Option<i64>| value.and_then(|value| u8::try_from(value).ok());
    let year = i32::try_from(components.year?).ok()?;
    let date = IcuDate::try_new_gregorian(year, narrow(components.month)?, narrow(components.day)?).ok()?;
    let nanosecond = u32::try_from(components.nanosecond.unwrap_or(0)).ok()?;
    let time = IcuTime::try_new(
        narrow(components.hour)?,
        narrow(components.minute)?,
        narrow(components.second)?,
        nanosecond,
    )
    .ok()?;
    Some(IcuDateTime { date, time })
}

fn style_builder(date_style: Style, time_style: Style) -> Option<FieldSetBuilder> {
    if date_style == Style::None && time_style == Style::None {
        return None;
    }
    let mut builder = FieldSetBuilder::new();
    let length_of = |style: Style| match style {
        Style::Short => Length::Short,
        Style::Medium | Style::None => Length::Medium,
        Style::Long | Style::Full => Length::Long,
    };
    builder.length = Some(length_of(if date_style == Style::None { time_style } else { date_style }));
    builder.date_fields = match date_style {
        Style::None => None,
        Style::Full => Some(DateFields::YMDE),
        _ => Some(DateFields::YMD),
    };
    builder.time_precision = match time_style {
        Style::None => None,
        Style::Short => Some(TimePrecision::Minute),
        _ => Some(TimePrecision::Second),
    };
    Some(builder)
}

/// a template names fields not their order, eg d MMMM yyyy is year and month and day in long form
/// hour letter picks the clock like a cldr skeleton
/// h is 1-12 and K is 0-11 and H and k are 24 hour and j is the locale pick
fn template_builder(tokens: &[PatternToken]) -> (Option<FieldSetBuilder>, Option<HourCycle>) {
    let mut year = false;
    let mut month_width = 0;
    let mut day = false;
    let mut weekday = false;
    let mut precision: Option<TimePrecision> = None;
    let mut hour_cycle: Option<HourCycle> = None;
    for token in tokens {
        let PatternToken::Field { symbol, width } = token else {
            continue;
        };
        match symbol {
            'y' => year = true,
            'M' | 'L' => month_width = month_width.max(*width),
            'd' => day = true,
            'E' | 'c' | 'e' => weekday = true,
            'H' | 'h' | 'K' | 'k' | 'j' => {
                precision = precision.or(Some(TimePrecision::Hour));
                hour_cycle = match symbol {
                    'h' => Some(HourCycle::H12),
                    'K' => Some(HourCycle::H11),
                    'H' | 'k' => Some(HourCycle::H23),
                    _ => hour_cycle,
                };
            }
            'm' => precision = Some(TimePrecision::Minute),
            's' => precision = Some(TimePrecision::Second),
            _ => {}
        }
    }
    let month = month_width > 0;
    let mut builder = FieldSetBuilder::new();
    builder.date_fields = match (year, month, day, weekday) {
        (true, true, true, false) => Some(DateFields::YMD),
        (true, true, true, true) => Some(DateFields::YMDE),
        (false, true, true, false) => Some(DateFields::MD),
        (false, true, true, true) => Some(DateFields::MDE),
        (true, true, false, _) => Some(DateFields::YM),
        (false, true, false, _) => Some(DateFields::M),
        (true, false, false, _) => Some(DateFields::Y),
        (false, false, true, false) => Some(DateFields::D),
        (false, false, true, true) => Some(DateFields::DE),
        (false, false, false, true) => Some(DateFields::E),
        _ => None,
    };
    builder.time_precision = precision;
    builder.length = Some(match month_width {
        4.. => Length::Long,
        3 => Length::Medium,
        _ => Length::Short,
    });
    (Some(builder), hour_cycle)
}
