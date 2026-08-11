use super::core::rise_session_rpc::UserSessionDto;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SessionEntry {
    pub id: String,
    pub device: String,
    pub location: String,
    pub ip: String,
    pub is_current: bool,
    pub last_active_ms: i64,
    pub first_active_ms: Option<i64>,
}

impl SessionEntry {
    pub fn from_dto(dto: &UserSessionDto, current_session_id: &str) -> Self {
        let device = trimmed(dto.device_info.as_deref());
        let ip = trimmed(dto.ip_address.as_deref());
        let location = trimmed(dto.location.as_deref());
        let current = current_session_id.trim();

        Self {
            id: dto.id.clone(),
            device: if device.is_empty() {
                "Unknown device".to_owned()
            } else {
                device
            },
            location: if location.is_empty() {
                ip.clone()
            } else {
                location
            },
            ip,
            is_current: dto.is_current == Some(true) || (!current.is_empty() && dto.id == current),
            last_active_ms: parse_timestamp(dto.last_accessed_at.as_deref())
                .or_else(|| parse_timestamp(dto.created_at.as_deref()))
                .unwrap_or_default(),
            first_active_ms: parse_timestamp(dto.created_at.as_deref()),
        }
    }
}

fn trimmed(value: Option<&str>) -> String {
    value.unwrap_or_default().trim().to_owned()
}

pub fn parse_timestamp(raw: Option<&str>) -> Option<i64> {
    let text = raw?.trim();
    if text.len() < 19 {
        return None;
    }
    let bytes = text.as_bytes();
    if bytes[4] != b'-' || bytes[7] != b'-' || (bytes[10] != b'T' && bytes[10] != b' ') {
        return None;
    }

    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let hour = number(11..13)?;
    let minute = number(14..16)?;
    let second = number(17..19)?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let millis = text
        .get(20..23)
        .filter(|_| bytes.get(19) == Some(&b'.'))
        .and_then(|fraction| fraction.parse::<i64>().ok())
        .unwrap_or(0);

    Some(
        (days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second) * 1_000
            + millis,
    )
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub fn normalize(mut rows: Vec<SessionEntry>) -> Vec<SessionEntry> {
    rows.sort_by(|left, right| {
        right
            .is_current
            .cmp(&left.is_current)
            .then_with(|| right.last_active_ms.cmp(&left.last_active_ms))
            .then_with(|| left.id.cmp(&right.id))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dto(id: &str, last: Option<&str>) -> UserSessionDto {
        UserSessionDto {
            id: id.to_owned(),
            last_accessed_at: last.map(str::to_owned),
            ..UserSessionDto::default()
        }
    }

    #[test]
    fn the_epoch_itself_parses_to_zero() {
        assert_eq!(parse_timestamp(Some("1970-01-01T00:00:00Z")), Some(0));
    }

    #[test]
    fn a_known_instant_parses_to_its_known_value() {
        assert_eq!(
            parse_timestamp(Some("2024-03-01T12:34:56Z")),
            Some(1_709_296_496_000),
        );
        assert_eq!(
            parse_timestamp(Some("2024-03-01T12:34:56.789Z")),
            Some(1_709_296_496_789),
        );
    }

    #[test]
    fn a_leap_day_and_a_century_boundary_are_not_off_by_one() {
        assert_eq!(
            parse_timestamp(Some("2000-02-29T00:00:00Z")),
            Some(951_782_400_000)
        );
        assert_eq!(
            parse_timestamp(Some("2024-02-29T00:00:00Z")),
            Some(1_709_164_800_000)
        );
    }

    #[test]
    fn a_missing_zone_or_a_space_separator_still_parses() {
        assert_eq!(
            parse_timestamp(Some("2024-03-01T12:34:56")),
            Some(1_709_296_496_000)
        );
        assert_eq!(
            parse_timestamp(Some("2024-03-01 12:34:56")),
            Some(1_709_296_496_000)
        );
    }

    #[test]
    fn something_that_is_not_a_timestamp_is_none_rather_than_a_wrong_instant() {
        assert_eq!(parse_timestamp(None), None);
        assert_eq!(parse_timestamp(Some("")), None);
        assert_eq!(parse_timestamp(Some("yesterday")), None);
        assert_eq!(parse_timestamp(Some("2024-13-01T00:00:00Z")), None);
        assert_eq!(parse_timestamp(Some("2024/03/01T00:00:00Z")), None);
    }

    #[test]
    fn a_row_falls_back_to_its_creation_time_when_it_has_never_been_used_since() {
        let entry = SessionEntry::from_dto(
            &UserSessionDto {
                id: "s1".into(),
                created_at: Some("2024-03-01T12:34:56Z".into()),
                ..UserSessionDto::default()
            },
            "",
        );
        assert_eq!(entry.last_active_ms, 1_709_296_496_000);
    }

    #[test]
    fn the_current_session_is_recognised_by_the_flag_or_by_its_id() {
        let flagged = SessionEntry::from_dto(
            &UserSessionDto {
                id: "s1".into(),
                is_current: Some(true),
                ..UserSessionDto::default()
            },
            "",
        );
        assert!(flagged.is_current);

        let by_id = SessionEntry::from_dto(&dto("s2", None), "  s2  ");
        assert!(
            by_id.is_current,
            "the id comparison has to survive whitespace"
        );

        assert!(!SessionEntry::from_dto(&dto("s3", None), "").is_current);
    }

    #[test]
    fn a_row_with_no_device_string_is_still_a_row_the_user_can_end() {
        let entry = SessionEntry::from_dto(&dto("s1", None), "");
        assert_eq!(entry.device, "Unknown device");
    }

    #[test]
    fn a_row_with_no_location_falls_back_to_its_address() {
        let entry = SessionEntry::from_dto(
            &UserSessionDto {
                id: "s1".into(),
                ip_address: Some("203.0.113.7".into()),
                ..UserSessionDto::default()
            },
            "",
        );
        assert_eq!(entry.location, "203.0.113.7");
    }

    #[test]
    fn the_current_session_sorts_first_and_the_rest_by_recency() {
        let rows = normalize(vec![
            SessionEntry::from_dto(&dto("a", Some("2024-01-01T00:00:00Z")), ""),
            SessionEntry::from_dto(&dto("b", Some("2024-03-01T00:00:00Z")), ""),
            SessionEntry::from_dto(&dto("c", Some("2024-02-01T00:00:00Z")), "c"),
        ]);

        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            vec!["c", "b", "a"]
        );
    }

    #[test]
    fn rows_with_the_same_instant_keep_a_stable_order() {
        let rows: Vec<SessionEntry> = vec![
            SessionEntry::from_dto(&dto("b", Some("2024-01-01T00:00:00Z")), ""),
            SessionEntry::from_dto(&dto("a", Some("2024-01-01T00:00:00Z")), ""),
        ];
        assert_eq!(normalize(rows.clone()), normalize(rows.clone()));
        assert_eq!(normalize(rows)[0].id, "a");
    }
}
