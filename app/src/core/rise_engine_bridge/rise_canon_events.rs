use std::collections::{BTreeMap, HashSet};
use std::panic::Location;
use std::sync::LazyLock;

use gpui::App;
use rise_engine::RiseWireConfiguration;
use rise_events::{CLIENT_ALLOWED_ANALYTICS, FRAME_EXTRAS, WS_EVENTS, WsEvent};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_canon_events_parity_hooks::{
    RiseCanonEventsParityChatModelsHooks, RiseCanonEventsParityMessageRealtimeHooks,
};

/// wire names of gateway realtime events from the canon table
/// a renamed event breaks the build instead of silently losing its handler
pub enum RiseWsEvents {}

impl RiseWsEvents {
    pub fn canonical_names() -> HashSet<String> {
        INDEX.iter().map(|name| (*name).to_owned()).collect()
    }

    /// keys the gateway puts beside data and not inside it
    /// the flat envelope is decoded by these names and not by the nested dto
    pub fn frame_extras(event: &str) -> Vec<String> {
        FRAME_EXTRAS
            .iter()
            .find(|(name, _)| *name == event)
            .map(|(_, keys)| keys.iter().map(|key| (*key).to_owned()).collect())
            .unwrap_or_default()
    }

    pub fn report_registration_drift(registered: &HashSet<String>, cx: &mut App) {
        let mut unknown: Vec<&str> = registered
            .iter()
            .map(String::as_str)
            .filter(|name| !INDEX.contains(name))
            .collect();
        if unknown.is_empty() {
            return;
        }
        unknown.sort_unstable();
        let joined = unknown.join(", ");
        Sai::log(
            "Event",
            move || format!("events subscribed outside canon: {joined}"),
            SaiLogKind::Websocket,
            cx,
        );
    }
}

static INDEX: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| WS_EVENTS.iter().map(|event: &WsEvent| event.name).collect());

pub trait RiseWireConfigurationCanonical {
    /// the transport must not guess which keys sit beside data
    /// the list comes whole from canon so a new key arrives by regenerating canon
    fn canonical() -> RiseWireConfiguration;
}

impl RiseWireConfigurationCanonical for RiseWireConfiguration {
    fn canonical() -> RiseWireConfiguration {
        let frame_extras: BTreeMap<String, Vec<String>> = FRAME_EXTRAS
            .iter()
            .map(|(name, keys)| {
                ((*name).to_owned(), keys.iter().map(|key| (*key).to_owned()).collect())
            })
            .collect();
        RiseWireConfiguration { frame_extras, ..RiseWireConfiguration::default() }
    }
}

/// enums whose raw value must be a wire event name
/// rust cant tie a constant to a discriminant so the match against canon is checked once at launch
pub enum RiseCanonEventsParity {}

impl RiseCanonEventsParity {
    pub fn verify(cx: &mut App) {
        if !(cfg!(debug_assertions) || cfg!(feature = "sai_debug_tools")) {
            return;
        }
        let mut declared: HashSet<String> = HashSet::new();
        if let Some(hooks) = RiseCanonEventsParityMessageRealtimeHooks::installed() {
            declared.extend((hooks.all_raw_values)());
        }
        if let Some(hooks) = RiseCanonEventsParityChatModelsHooks::installed() {
            declared.extend((hooks.all_raw_values)());
        }
        let mut unknown: Vec<String> =
            declared.into_iter().filter(|name| !INDEX.contains(name.as_str())).collect();
        if unknown.is_empty() {
            return;
        }
        unknown.sort_unstable();
        let joined = unknown.join(", ");
        let logged = joined.clone();
        Sai::log(
            "Event",
            move || format!("raw value outside canon: {logged}"),
            SaiLogKind::Websocket,
            cx,
        );
        debug_assert!(false, "ws events [{joined}] are missing from the riseonly-tools canon");
    }
}

/// engagement analytics types and the surfaces the client sends them from
pub enum RiseAnalyticsEvents {}

impl RiseAnalyticsEvents {
    pub fn post_share() -> &'static str {
        Self::client_allowed(rise_events::analytics::POST_SHARE)
    }

    pub fn follow_user() -> &'static str {
        Self::client_allowed(rise_events::analytics::FOLLOW_USER)
    }

    pub fn video_watch() -> &'static str {
        Self::client_allowed(rise_events::analytics::VIDEO_WATCH)
    }

    pub fn profile_view() -> &'static str {
        Self::client_allowed(rise_events::analytics::PROFILE_VIEW)
    }

    pub fn post_impression() -> &'static str {
        Self::client_allowed(rise_events::analytics::POST_IMPRESSION)
    }

    pub fn post_dwell() -> &'static str {
        Self::client_allowed(rise_events::analytics::POST_DWELL)
    }

    pub fn post_skip() -> &'static str {
        Self::client_allowed(rise_events::analytics::POST_SKIP)
    }

    pub fn comments_opened() -> &'static str {
        Self::client_allowed(rise_events::analytics::COMMENTS_OPENED)
    }

    /// collector accepts only the types canon lists for clients and silently drops the rest
    #[track_caller]
    fn client_allowed(name: &'static str) -> &'static str {
        if !CLIENT_ALLOWED_ANALYTICS.contains(&name) {
            let location = Location::caller();
            debug_assert!(
                false,
                "analytics {name} is not accepted from a client by the collector ({location})"
            );
        }
        name
    }
}
