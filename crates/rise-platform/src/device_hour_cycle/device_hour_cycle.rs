use crate::host_os::HostOs;

/// pattern a platform hands back for its preferred time of day, in that platform syntax
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HourPattern {
    Icu(String),
    WindowsTimeFormat(String),
    PosixTimeFormat(String),
}

/// why the device clock cycle couldnt be read, callers treat it as 24 hour time
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HourCycleUnavailable {
    BranchNotWritten { host: HostOs },
    NoPattern { host: HostOs },
}

impl std::fmt::Display for HourCycleUnavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BranchNotWritten { host } => {
                write!(formatter, "the {host:?} layer does not read the hour cycle yet")
            }
            Self::NoPattern { host } => {
                write!(formatter, "{host:?} returned no time pattern for the current locale")
            }
        }
    }
}

impl std::error::Error for HourCycleUnavailable {}

impl HourPattern {
    /// true when the pattern writes the hour on a 12 hour clock
    pub fn is_twelve_hour(&self) -> bool {
        match self {
            Self::Icu(pattern) => pattern.contains('a'),
            Self::WindowsTimeFormat(format) => windows_format_is_twelve_hour(format),
            Self::PosixTimeFormat(format) => posix_format_is_twelve_hour(format),
        }
    }
}

fn windows_format_is_twelve_hour(format: &str) -> bool {
    let mut quoted = false;
    for character in format.chars() {
        match character {
            '\'' => quoted = !quoted,
            'h' | 't' if !quoted => return true,
            _ => {}
        }
    }
    false
}

fn posix_format_is_twelve_hour(format: &str) -> bool {
    let mut characters = format.chars();
    while let Some(character) = characters.next() {
        if character != '%' {
            continue;
        }
        for conversion in characters.by_ref() {
            match conversion {
                '_' | '-' | '0' | '^' | '#' | 'E' | 'O' => continue,
                'I' | 'l' | 'r' | 'p' | 'P' => return true,
                _ => break,
            }
        }
    }
    false
}

/// the device own 12 or 24 hour setting, read again on locale change and on activation
/// not cached here
pub fn device_hour_cycle() -> Result<HourPattern, HourCycleUnavailable> {
    super::os::native_hour_pattern(HostOs::current())
}

/// true when the device is set to 12 hour time, a failed read counts as 24 hour
pub fn prefers_twelve_hour() -> bool {
    match device_hour_cycle() {
        Ok(pattern) => pattern.is_twelve_hour(),
        Err(reason) => {
            tracing::debug!(%reason, "device hour cycle unreadable, using 24 hour time");
            false
        }
    }
}
