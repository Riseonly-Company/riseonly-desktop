use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::date_format::iso8601_date_formatter::{ISO8601DateFormatter, ISO8601DateFormatterOptions};
use rise_i18n::{Locale, tr};

use super::session_wire_types::UserSessionWireDTO;

const APP_LINE: &str = "Riseonly";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionEntry {
    pub id: String,
    pub device: String,
    pub app_line: String,
    pub location: String,
    pub ip: String,
    pub is_current: bool,
    pub last_active: Date,
    pub first_active: Option<Date>,
}

impl SessionEntry {
    pub fn from(dto: &UserSessionWireDTO, current_session_id: &str) -> SessionEntry {
        let device = trimming_whitespaces_and_newlines(&dto.device_info);
        let ip = trimming_whitespaces_and_newlines(&dto.ip_address);
        let loc = Self::shown_location(dto);
        let last = Self::parse_iso(Some(&dto.last_accessed_at))
            .or_else(|| Self::parse_iso(Some(&dto.created_at)))
            .unwrap_or_else(Date::now);
        let trimmed_current = trimming_whitespaces_and_newlines(current_session_id);
        let is_current = !trimmed_current.is_empty() && dto.id == trimmed_current;
        SessionEntry {
            id: dto.id.clone(),
            device: if device.is_empty() {
                tr("session_info_device_unknown")
            } else {
                device.to_owned()
            },
            app_line: APP_LINE.to_owned(),
            location: loc,
            ip: ip.to_owned(),
            is_current,
            last_active: last,
            first_active: Self::parse_iso(Some(&dto.created_at)),
        }
    }

    fn shown_location(dto: &UserSessionWireDTO) -> String {
        if dto.location_country.is_empty() {
            return trimming_whitespaces_and_newlines(&dto.location).to_owned();
        }
        let country = Locale::app_locale()
            .localized_string_for_region_code(&dto.location_country)
            .unwrap_or_else(|| dto.location_country.clone());
        if dto.location_city.is_empty() {
            country
        } else {
            format!("{}, {}", dto.location_city, country)
        }
    }

    fn parse_iso(raw: Option<&str>) -> Option<Date> {
        let s = trimming_whitespaces_and_newlines(raw?);
        if s.is_empty() {
            return None;
        }
        let mut f = ISO8601DateFormatter::new();
        f.set_format_options(
            ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME
                | ISO8601DateFormatterOptions::WITH_FRACTIONAL_SECONDS,
        );
        if let Some(d) = f.date_from(s) {
            return Some(d);
        }
        f.set_format_options(ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME);
        if let Some(d) = f.date_from(s) {
            return Some(d);
        }
        let mut alt = DateFormatter::new();
        alt.set_locale("en_US_POSIX");
        alt.set_date_format("yyyy-MM-dd'T'HH:mm:ss");
        alt.date_from(s)
    }
}
