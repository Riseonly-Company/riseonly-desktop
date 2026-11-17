use std::sync::LazyLock;

use parking_lot::Mutex;
use rise_canon::RiseTimingCanon;
use rise_i18n::language_store::LanguageStore;
use rise_i18n::number_format::measurement::{Measurement, UnitSpeed};
use rise_i18n::number_format::measurement_formatter::{
    FormatterUnitStyle, MeasurementFormatter, MeasurementFormatterUnitOptions,
};
use rise_i18n::{FormatArg, format_printf, tr};

use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::modules::map::recorder::map_self_location_feed::MapSelfLocation;
use crate::modules::map::stores::map_actions::map_types::{MapFix, MapFriendPosition, MapLiveFriend, MapMotion};

/// tile at a pin's shoulder, emoji and headline and caption below
/// any part may be missing
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MapPinTile {
    pub emoji: Option<String>,
    pub system_image: Option<String>,
    pub headline: Option<String>,
    pub caption: Option<String>,
}

impl MapPinTile {
    pub fn new(
        emoji: Option<String>,
        system_image: Option<String>,
        text: Option<(String, Option<String>)>,
    ) -> Option<Self> {
        if emoji.is_none() && system_image.is_none() && text.is_none() {
            return None;
        }
        let (headline, caption) = match text {
            Some((headline, caption)) => (Some(headline), caption),
            None => (None, None),
        };
        Some(Self { emoji, system_image, headline, caption })
    }

    /// splits number from unit at the first space
    /// a language that writes them joined stays on one line
    pub fn split(text: &str) -> (String, Option<String>) {
        let Some((gap, separator)) = text
            .char_indices()
            .find(|(_, character)| matches!(character, ' ' | '\u{00A0}' | '\u{202F}'))
        else {
            return (text.to_owned(), None);
        };
        let caption = &text[gap + separator.len_utf8()..];
        (text[..gap].to_owned(), (!caption.is_empty()).then(|| caption.to_owned()))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MapFriendStatusKind {
    Unknown,
    Home,
    Staying,
    Moving,
    Stale,
}

/// friend status derived from the last fix and the clock
/// a value and not a view so the pin layer and the friend card print the same words
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MapFriendStatus {
    pub kind: MapFriendStatusKind,
    pub text: String,
    pub tile: Option<MapPinTile>,
}

impl MapFriendStatus {
    pub const HOME_EMOJI: &'static str = "\u{1F3E0}";
    pub const UNAVAILABLE_SYMBOL: &'static str = "location.slash";
    pub const STALE_SYMBOL: &'static str = "clock";

    pub fn new(kind: MapFriendStatusKind, text: String, tile: Option<MapPinTile>) -> Self {
        Self { kind, text, tile }
    }

    /// how long a fix counts as live
    /// past it the pin shows when the friend was last seen
    pub fn live_window_ms() -> i64 {
        RiseTimingCanon::freshness_ms("map_friend_position")
    }

    /// a walker emits a fix every few steps and a stopped person emits none
    /// so a moving fix older than this no longer means movement
    pub fn motion_window_ms() -> i64 {
        RiseTimingCanon::freshness_ms("map_friend_motion")
    }

    pub fn is_live(&self) -> bool {
        matches!(self.kind, MapFriendStatusKind::Home | MapFriendStatusKind::Staying | MapFriendStatusKind::Moving)
    }

    pub fn location_update_text(recorded_at: Option<i64>, now_ms: i64) -> String {
        let Some(recorded_at) = recorded_at.filter(|recorded_at| *recorded_at > 0) else {
            return tr("map_status_unknown");
        };
        if now_ms - recorded_at < 60_000 {
            return tr("map_location_updated_now");
        }
        format_printf(
            &tr("map_location_updated_ago"),
            &[FormatArg::from(RiseFormat::elapsed_duration(recorded_at, now_ms))],
        )
    }

    /// explicit loss of location access reported by the owner's device
    pub fn inactive(since_ms: i64, now_ms: i64) -> MapFriendStatus {
        let elapsed = (now_ms - since_ms).max(0);
        MapFriendStatus::new(
            MapFriendStatusKind::Stale,
            tr("map_location_unavailable"),
            MapPinTile::new(
                None,
                Some(Self::UNAVAILABLE_SYMBOL.to_owned()),
                (elapsed >= 60_000).then(|| Self::duration_parts(elapsed)),
            ),
        )
    }

    pub fn resolve_self(location: Option<&MapSelfLocation>, now_ms: i64) -> MapFriendStatus {
        let Some(location) = location else {
            return MapFriendStatus::new(MapFriendStatusKind::Unknown, tr("map_status_unknown"), None);
        };
        if let Some(since) = location.unavailable_since_ms {
            return Self::inactive(since, now_ms);
        }
        if !location.is_at_home
            && location.stay_since_ms.is_none()
            && location.speed_meters_per_second > 0.5
            && now_ms - location.time_ms <= Self::motion_window_ms()
        {
            return MapFriendStatus::new(
                MapFriendStatusKind::Moving,
                tr("map_status_moving"),
                MapPinTile::new(None, None, MapSpeedFormat::parts(location.speed_meters_per_second)),
            );
        }
        if location.is_at_home {
            let elapsed = location.stay_since_ms.map(|since| (now_ms - since).max(0)).unwrap_or(0);
            let text = if elapsed >= 60_000 {
                format_printf(&tr("map_status_home_for"), &[FormatArg::from(Self::duration(elapsed))])
            } else {
                tr("map_status_home")
            };
            return MapFriendStatus::new(
                MapFriendStatusKind::Home,
                text,
                MapPinTile::new(
                    Some(Self::HOME_EMOJI.to_owned()),
                    None,
                    (elapsed >= 60_000).then(|| Self::duration_parts(elapsed)),
                ),
            );
        }
        let fix = MapFix {
            latitude: location.latitude,
            longitude: location.longitude,
            accuracy_meters: location.accuracy_meters,
            speed_meters_per_second: location.speed_meters_per_second,
            heading_degrees: location.course_degrees,
            battery_level: -1,
            is_charging: false,
            motion: MapMotion::Still,
            recorded_at: location.time_ms,
            stay_since: location.stay_since_ms.unwrap_or(0),
            place_kind: location.place_kind,
        };
        let position = MapFriendPosition::new(String::new(), fix, location.is_at_home);
        Self::resolve(Some(&position), now_ms)
    }

    pub fn resolve(position: Option<&MapFriendPosition>, now_ms: i64) -> MapFriendStatus {
        let Some(position) = position else {
            return MapFriendStatus::new(MapFriendStatusKind::Unknown, tr("map_status_unknown"), None);
        };
        let fix = &position.fix;
        if let Some(since) = position.unavailable_since_ms() {
            return Self::inactive(since, now_ms);
        }
        if now_ms - fix.recorded_at > Self::live_window_ms() {
            return MapFriendStatus::new(
                MapFriendStatusKind::Stale,
                Self::location_update_text(Some(fix.recorded_at), now_ms),
                MapPinTile::new(
                    None,
                    Some(Self::STALE_SYMBOL.to_owned()),
                    Some(Self::duration_parts((now_ms - fix.recorded_at).max(0))),
                ),
            );
        }

        if fix.motion == MapMotion::Unknown
            && fix.speed_meters_per_second < 0.0
            && fix.stay_since == 0
            && !position.at_home
        {
            return MapFriendStatus::new(MapFriendStatusKind::Unknown, tr("map_status_unknown"), None);
        }
        let stayed_ms = if fix.stay_since > 0 { (now_ms - fix.stay_since).max(0) } else { 0 };
        if position.at_home {
            let text = if stayed_ms >= 60_000 {
                format_printf(&tr("map_status_home_for"), &[FormatArg::from(Self::duration(stayed_ms))])
            } else {
                tr("map_status_home")
            };
            return MapFriendStatus::new(
                MapFriendStatusKind::Home,
                text,
                MapPinTile::new(
                    Some(Self::HOME_EMOJI.to_owned()),
                    None,
                    (stayed_ms >= 60_000).then(|| Self::duration_parts(stayed_ms)),
                ),
            );
        }
        if fix.stay_since > 0 && !fix.motion.is_moving() {
            let text = if stayed_ms >= 60_000 {
                format_printf(&tr("map_status_staying_for"), &[FormatArg::from(Self::duration(stayed_ms))])
            } else {
                tr("map_status_here")
            };
            return MapFriendStatus::new(
                MapFriendStatusKind::Staying,
                text,
                MapPinTile::new(
                    fix.place_kind.map(|kind| kind.emoji().to_owned()),
                    None,
                    (stayed_ms >= 60_000).then(|| Self::duration_parts(stayed_ms)),
                ),
            );
        }
        if fix.motion.is_moving() && now_ms - fix.recorded_at <= Self::motion_window_ms() {
            return MapFriendStatus::new(
                MapFriendStatusKind::Moving,
                tr(Self::moving_key(fix.motion)),
                MapPinTile::new(None, None, MapSpeedFormat::parts(fix.speed_meters_per_second)),
            );
        }
        MapFriendStatus::new(MapFriendStatusKind::Staying, tr("map_status_here"), None)
    }

    /// duration split for the tile
    /// hours over minutes when both are there
    /// otherwise the number over its unit
    pub fn duration_parts(milliseconds: i64) -> (String, Option<String>) {
        let minutes = milliseconds.max(0) / 60_000;
        let hours = minutes / 60;
        if minutes >= 60 && hours < 24 && minutes % 60 != 0 {
            return (
                format_printf(&tr("duration_hours_abbr"), &[FormatArg::Int(hours)]),
                Some(format_printf(&tr("duration_minutes_abbr"), &[FormatArg::Int(minutes % 60)])),
            );
        }
        MapPinTile::split(&Self::duration(milliseconds))
    }

    /// same abbreviated units the app prints for live locations in chats
    pub fn duration(milliseconds: i64) -> String {
        let minutes = milliseconds / 60_000;
        if minutes < 60 {
            return format_printf(&tr("duration_minutes_abbr"), &[FormatArg::Int(minutes.max(1))]);
        }
        let hours = minutes / 60;
        if hours >= 24 {
            return format_printf(&tr("notify_time_day"), &[FormatArg::Int(hours / 24)]);
        }
        let rest = minutes % 60;
        if rest == 0 {
            format_printf(&tr("duration_hours_abbr"), &[FormatArg::Int(hours)])
        } else {
            format_printf(&tr("duration_hours_minutes_abbr"), &[FormatArg::Int(hours), FormatArg::Int(rest)])
        }
    }

    fn moving_key(motion: MapMotion) -> &'static str {
        match motion {
            MapMotion::Walk => "map_status_walking",
            MapMotion::Run => "map_status_running",
            MapMotion::Cycle => "map_status_cycling",
            MapMotion::Drive => "map_status_driving",
            MapMotion::Fly => "map_status_flying",
            MapMotion::Unknown | MapMotion::Still => "map_status_moving",
        }
    }
}

/// everything one pin draws
/// built only when the map asks for an on screen view
/// so three hundred friends cost what the dozen visible ones cost
#[derive(Clone, PartialEq, Debug)]
pub struct MapPinModel {
    pub id: String,
    pub name: String,
    pub logo: String,
    pub status: MapFriendStatus,
    pub battery_level: Option<i64>,
    pub is_charging: bool,
    pub trail: Option<MapPinTrail>,
}

impl MapPinModel {
    pub fn new(friend: &MapLiveFriend, now_ms: i64) -> Option<Self> {
        let position = friend.position.as_ref()?;
        let fix = &position.fix;
        let status = MapFriendStatus::resolve(Some(position), now_ms);
        let trail = if status.kind == MapFriendStatusKind::Moving {
            MapPinTrail::new(fix.heading_degrees, fix.speed_meters_per_second)
        } else {
            None
        };
        Some(Self {
            id: friend.id().to_owned(),
            name: friend.user.display_name().to_owned(),
            logo: friend.user.logo.clone(),
            status,
            battery_level: fix.has_battery().then_some(fix.battery_level),
            is_charging: fix.is_charging,
            trail,
        })
    }
}

/// wake behind a moving pin, points away from the direction of travel
/// length grows with speed, long at walking pace and barely there at motorway speed
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapPinTrail {
    pub bearing_degrees: f64,
    pub length: f64,
}

impl MapPinTrail {
    pub fn new(course_degrees: f64, meters_per_second: f64) -> Option<Self> {
        if !(course_degrees >= 0.0
            && course_degrees.is_finite()
            && meters_per_second.is_finite()
            && meters_per_second >= MapSpeedFormat::MOVING_METERS_PER_SECOND)
        {
            return None;
        }
        Some(Self {
            bearing_degrees: (course_degrees + 180.0) % 360.0,
            length: (54.0 + 20.0 * (1.0 + meters_per_second).ln()).round(),
        })
    }
}

struct MapSpeedFormatCache {
    formatter: Option<MeasurementFormatter>,
    formatter_locale: String,
}

static MAP_SPEED_FORMAT_CACHE: LazyLock<Mutex<MapSpeedFormatCache>> = LazyLock::new(|| {
    Mutex::new(MapSpeedFormatCache { formatter: None, formatter_locale: String::new() })
});

/// speed text
/// the formatter picks the unit by region and words it in the app language, no manual conversion
pub enum MapSpeedFormat {}

impl MapSpeedFormat {
    pub const MOVING_METERS_PER_SECOND: f64 = 0.4;

    pub fn parts(meters_per_second: f64) -> Option<(String, Option<String>)> {
        if !(meters_per_second.is_finite() && meters_per_second >= Self::MOVING_METERS_PER_SECOND) {
            return None;
        }
        Some(MapPinTile::split(&Self::text(meters_per_second)))
    }

    pub fn text(meters_per_second: f64) -> String {
        let locale = LanguageStore::current_locale();
        let mut cache = MAP_SPEED_FORMAT_CACHE.lock();
        if cache.formatter.is_none() || cache.formatter_locale != locale.identifier() {
            let mut next = MeasurementFormatter::new();
            next.locale = locale.clone();
            next.unit_options = MeasurementFormatterUnitOptions::NATURAL_SCALE;
            next.unit_style = FormatterUnitStyle::Short;
            next.number_formatter.maximum_fraction_digits = 0;
            cache.formatter = Some(next);
            cache.formatter_locale = locale.identifier().to_owned();
        }
        let measurement = Measurement::new((meters_per_second * 3.6).max(1.0), UnitSpeed::KilometersPerHour);
        cache.formatter.as_ref().map(|formatter| formatter.string_from(&measurement)).unwrap_or_default()
    }
}
