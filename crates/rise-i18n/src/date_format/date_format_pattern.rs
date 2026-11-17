use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent, DateComponents, TimeZone};
use rise_core::foundation::foundation_date::Date;

const DEFAULT_YEAR: i64 = 2000;
const TWO_DIGIT_PIVOT: i64 = 50;
const NANOS_PER_SECOND: i64 = 1_000_000_000;

/// one run of the same pattern letter, or quoted literal text
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternToken {
    Field { symbol: char, width: usize },
    Literal(String),
}

impl PatternToken {
    fn is_numeric_field(&self) -> bool {
        match self {
            PatternToken::Field { symbol, width } => numeric_symbol(*symbol, *width),
            PatternToken::Literal(_) => false,
        }
    }
}

fn numeric_symbol(symbol: char, width: usize) -> bool {
    match symbol {
        'y' | 'd' | 'H' | 'h' | 'K' | 'k' | 'm' | 's' | 'S' => true,
        'M' | 'L' => width <= 2,
        _ => false,
    }
}

/// splits a pattern like icu does, ascii letters are fields and quoted text is literal
/// two quotes make one quote
pub fn tokenize(pattern: &str) -> Vec<PatternToken> {
    let mut tokens = Vec::new();
    let mut literal = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                literal.push('\'');
                continue;
            }
            for quoted in chars.by_ref() {
                if quoted == '\'' {
                    break;
                }
                literal.push(quoted);
            }
            continue;
        }
        if c.is_ascii_alphabetic() {
            if !literal.is_empty() {
                tokens.push(PatternToken::Literal(std::mem::take(&mut literal)));
            }
            let mut width = 1;
            while chars.peek() == Some(&c) {
                chars.next();
                width += 1;
            }
            tokens.push(PatternToken::Field { symbol: c, width });
            continue;
        }
        literal.push(c);
    }
    if !literal.is_empty() {
        tokens.push(PatternToken::Literal(literal));
    }
    tokens
}

/// renders numeric fields and literals only, for when the pattern engine rejects one
/// none if a name field (month or weekday text or era or zone) is there
pub fn render_numeric(tokens: &[PatternToken], components: &DateComponents) -> Option<String> {
    let mut out = String::new();
    for token in tokens {
        match token {
            PatternToken::Literal(text) => out.push_str(text),
            PatternToken::Field { symbol, width } => {
                if !numeric_symbol(*symbol, *width) {
                    return None;
                }
                out.push_str(&render_field(*symbol, *width, components));
            }
        }
    }
    Some(out)
}

fn render_field(symbol: char, width: usize, components: &DateComponents) -> String {
    let hour = components.hour.unwrap_or(0);
    let value = match symbol {
        'y' => {
            let year = components.year.unwrap_or(0);
            if width == 2 {
                return format!("{:02}", year.rem_euclid(100));
            }
            year
        }
        'M' | 'L' => components.month.unwrap_or(1),
        'd' => components.day.unwrap_or(1),
        'H' => hour,
        'k' => if hour == 0 { 24 } else { hour },
        'K' => hour % 12,
        'h' => if hour % 12 == 0 { 12 } else { hour % 12 },
        'm' => components.minute.unwrap_or(0),
        's' => components.second.unwrap_or(0),
        _ => return fraction(components.nanosecond.unwrap_or(0), width),
    };
    format!("{value:0width$}")
}

fn fraction(nanosecond: i64, width: usize) -> String {
    let digits = format!("{:09}", nanosecond.clamp(0, NANOS_PER_SECOND - 1));
    let mut out: String = digits.chars().take(width).collect();
    while out.len() < width {
        out.push('0');
    }
    out
}

#[derive(Default)]
struct Parsed {
    year: Option<i64>,
    month: Option<i64>,
    day: Option<i64>,
    hour: Option<i64>,
    pm: Option<bool>,
    minute: Option<i64>,
    second: Option<i64>,
    nanosecond: Option<i64>,
}

/// strict parse for numeric patterns, the whole string must match and every field be in range
pub fn parse_numeric(tokens: &[PatternToken], text: &str, time_zone: &TimeZone) -> Option<Date> {
    let mut parsed = Parsed::default();
    let mut rest = text;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            PatternToken::Literal(literal) => rest = rest.strip_prefix(literal.as_str())?,
            PatternToken::Field { symbol, width } => {
                if *symbol == 'a' {
                    rest = parse_day_period(rest, &mut parsed)?;
                    continue;
                }
                if !numeric_symbol(*symbol, *width) {
                    return None;
                }
                let abutting = tokens.get(index + 1).is_some_and(PatternToken::is_numeric_field);
                let (digits, tail) = take_digits(rest, if abutting { Some(*width) } else { None })?;
                rest = tail;
                store(&mut parsed, *symbol, *width, digits)?;
            }
        }
    }
    if !rest.is_empty() {
        return None;
    }
    build(parsed, time_zone)
}

fn take_digits(text: &str, exact: Option<usize>) -> Option<(&str, &str)> {
    let available = text.bytes().take_while(u8::is_ascii_digit).count();
    let count = match exact {
        Some(width) if available >= width => width,
        Some(_) => return None,
        None => available,
    };
    if count == 0 {
        return None;
    }
    Some(text.split_at(count))
}

fn parse_day_period<'a>(text: &'a str, parsed: &mut Parsed) -> Option<&'a str> {
    let head = text.get(..2)?;
    let pm = match head.to_ascii_uppercase().as_str() {
        "AM" => false,
        "PM" => true,
        _ => return None,
    };
    parsed.pm = Some(pm);
    text.get(2..)
}

fn store(parsed: &mut Parsed, symbol: char, width: usize, digits: &str) -> Option<()> {
    let value: i64 = digits.parse().ok()?;
    match symbol {
        'y' => {
            parsed.year = Some(if width == 2 && digits.len() == 2 { two_digit_year(value) } else { value });
        }
        'M' | 'L' => parsed.month = Some(value),
        'd' => parsed.day = Some(value),
        'H' => parsed.hour = Some(value),
        'k' => parsed.hour = Some(if value == 24 { 0 } else { value }),
        'K' | 'h' => parsed.hour = Some(if value == 12 && symbol == 'h' { 0 } else { value }),
        'm' => parsed.minute = Some(value),
        's' => parsed.second = Some(value),
        _ => {
            let scale = 10_i64.checked_pow(u32::try_from(9_usize.checked_sub(digits.len())?).ok()?)?;
            parsed.nanosecond = Some(value.checked_mul(scale)?);
        }
    }
    Some(())
}

fn two_digit_year(value: i64) -> i64 {
    if value < TWO_DIGIT_PIVOT { 2000 + value } else { 1900 + value }
}

fn build(parsed: Parsed, time_zone: &TimeZone) -> Option<Date> {
    let mut hour = parsed.hour.unwrap_or(0);
    if let Some(pm) = parsed.pm {
        if !(0..12).contains(&hour) {
            return None;
        }
        if pm {
            hour += 12;
        }
    }
    let components = DateComponents {
        year: Some(parsed.year.unwrap_or(DEFAULT_YEAR)),
        month: Some(parsed.month.unwrap_or(1)),
        day: Some(parsed.day.unwrap_or(1)),
        hour: Some(hour),
        minute: Some(parsed.minute.unwrap_or(0)),
        second: Some(parsed.second.unwrap_or(0)),
        nanosecond: Some(parsed.nanosecond.unwrap_or(0)),
        ..DateComponents::default()
    };
    let in_range = (1..=12).contains(&components.month.unwrap_or(0))
        && (1..=31).contains(&components.day.unwrap_or(0))
        && (0..24).contains(&hour)
        && (0..60).contains(&components.minute.unwrap_or(0))
        && (0..60).contains(&components.second.unwrap_or(0));
    if !in_range {
        return None;
    }
    let mut calendar = Calendar::gregorian();
    calendar.time_zone = time_zone.clone();
    let date = calendar.date_from_components(&components)?;
    let back = calendar.date_components(&[CalendarComponent::Month, CalendarComponent::Day], &date);
    (back.month == components.month && back.day == components.day).then_some(date)
}
