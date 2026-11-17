use std::panic::Location;

use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::date_format::date_formatter::DateFormatter;

use super::sai_log_kind::SaiLogKind;

const UNNAMED: &str = "Unnamed";
const TIME_FORMAT: &str = "HH:mm:ss.SSS";

thread_local! {
    static TIME_FORMATTER: DateFormatter = {
        let mut formatter = DateFormatter::new();
        formatter.set_date_format(TIME_FORMAT);
        formatter
    };
}

#[derive(Clone, Debug, PartialEq)]
pub struct SaiLogEntry {
    pub id: Uuid,
    pub timestamp: Date,
    pub name: String,
    pub kind: SaiLogKind,
    pub message: String,
    pub file: String,
    pub line: u64,
}

impl SaiLogEntry {
    /// builds an entry with a fresh id and timestamp
    /// file and line come from the callers location
    pub fn new(name: &str, kind: SaiLogKind, message: String, location: &Location<'_>) -> Self {
        Self::with_identity(Uuid::new_v4(), RiseClock::now(), name, kind, message, location)
    }

    pub fn with_identity(
        id: Uuid,
        timestamp: Date,
        name: &str,
        kind: SaiLogKind,
        message: String,
        location: &Location<'_>,
    ) -> Self {
        let raw_file = location.file();
        let file = raw_file
            .rsplit(['/', '\\'])
            .next()
            .filter(|last| !last.is_empty())
            .unwrap_or(raw_file)
            .to_owned();
        Self {
            id,
            timestamp,
            name: name.to_owned(),
            kind,
            message,
            file,
            line: u64::from(location.line()),
        }
    }

    pub fn console_line(&self) -> String {
        let t = TIME_FORMATTER.with(|formatter| formatter.string_from(&self.timestamp));
        let head = if self.name == UNNAMED {
            format!("[{UNNAMED}]")
        } else {
            format!("[{}]", self.name)
        };
        format!("{t} {head} ({}) {}", self.kind.short_label(), self.message)
    }
}
