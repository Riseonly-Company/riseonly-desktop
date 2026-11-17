use std::collections::HashMap;

use gpui::{App, Context, Entity};
use rise_canon::rise_key;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::lib::device_info::DeviceInfo;
use crate::core::rise_engine_bridge::rise_canon_events::RiseAnalyticsEvents;
use crate::modules::analytics::shared::engagement_feature_gate::{
    EngagementFeatureGate, EngagementFeatureGateChannel,
};
use crate::modules::analytics::stores::engagement::engagement_actions::engagement_types::{
    EngagementEvent, EngagementSurface, EngagementValue,
};
use crate::modules::analytics::stores::engagement::engagement_services::engagement_services::EngagementServicesStore;
use crate::modules::notify::engine::core::rise_release_notes_seen_store::RiseReleaseNotesDeviceContext;

pub struct EngagementInteractionsStore {}

impl EngagementInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    fn service(cx: &mut App) -> Entity<EngagementServicesStore> {
        EngagementServicesStore::shared(cx)
    }

    pub fn start_session(&self, cx: &mut App) {
        Self::service(cx).update(cx, |service, cx| service.start(cx));
        self.enqueue(
            rise_events::analytics::SESSION_START,
            HashMap::from([
                (
                    rise_key::report::PLATFORM.to_owned(),
                    EngagementValue::String(RiseReleaseNotesDeviceContext::platform().to_owned()),
                ),
                (
                    rise_key::report::APP_VERSION.to_owned(),
                    EngagementValue::String(RiseReleaseNotesDeviceContext::app_version()),
                ),
                (
                    rise_key::report::BUILD_NUMBER.to_owned(),
                    EngagementValue::String(RiseReleaseNotesDeviceContext::build_number()),
                ),
                (
                    rise_key::events::DEVICE_TYPE.to_owned(),
                    EngagementValue::String(DeviceInfo::device_type().to_owned()),
                ),
            ]),
            None,
            cx,
        );
    }

    pub fn onboarding_complete(&self, steps_shown: i64, cx: &mut App) {
        self.enqueue(
            rise_events::analytics::ONBOARDING_COMPLETE,
            HashMap::from([
                (rise_key::engagement::STEPS_SHOWN.to_owned(), EngagementValue::Int(steps_shown)),
                (
                    rise_key::report::PLATFORM.to_owned(),
                    EngagementValue::String(RiseReleaseNotesDeviceContext::platform().to_owned()),
                ),
                (
                    rise_key::report::APP_VERSION.to_owned(),
                    EngagementValue::String(RiseReleaseNotesDeviceContext::app_version()),
                ),
            ]),
            None,
            cx,
        );
    }

    pub fn flush_now(&self, cx: &mut App) {
        Self::service(cx).update(cx, |service, cx| service.flush(cx));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn impression(
        &self,
        post_id: &str,
        author_id: &str,
        surface: EngagementSurface,
        position: i64,
        is_short: bool,
        visible_ms: i64,
        cx: &mut App,
    ) {
        if !EngagementFeatureGate::allows(EngagementFeatureGateChannel::Attention) {
            return;
        }
        let Ok(pid) = post_id.parse::<i64>() else {
            return;
        };
        let mut payload = Self::base(pid, author_id, surface, position, is_short);
        payload.insert(
            rise_key::engagement::VISIBLE_MS.to_owned(),
            EngagementValue::Int(visible_ms.max(0)),
        );
        self.enqueue(
            RiseAnalyticsEvents::post_impression(),
            payload,
            Some(format!("{post_id}:{}", surface.slug())),
            cx,
        );
    }

    pub fn dwell(&self, post_id: &str, author_id: Option<&str>, dwell_ms: i64, cx: &mut App) {
        if !EngagementFeatureGate::allows(EngagementFeatureGateChannel::Attention) {
            return;
        }
        let Ok(pid) = post_id.parse::<i64>() else {
            return;
        };
        if dwell_ms <= 0 {
            return;
        }
        let mut payload: HashMap<String, EngagementValue> = HashMap::from([
            (rise_key::report::POST_ID.to_owned(), EngagementValue::Int(pid)),
            (rise_key::engagement::DWELL_MS.to_owned(), EngagementValue::Int(dwell_ms)),
        ]);
        if let Some(author_id) = author_id.filter(|id| !id.is_empty()) {
            payload.insert(
                rise_key::report::AUTHOR_ID.to_owned(),
                EngagementValue::String(author_id.to_owned()),
            );
        }
        self.enqueue(RiseAnalyticsEvents::post_dwell(), payload, None, cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn video_watch(
        &self,
        post_id: &str,
        author_id: &str,
        surface: EngagementSurface,
        position: i64,
        watched_ms: i64,
        duration_ms: i64,
        replays: i64,
        cx: &mut App,
    ) {
        if !EngagementFeatureGate::allows(EngagementFeatureGateChannel::Attention) {
            return;
        }
        let Ok(pid) = post_id.parse::<i64>() else {
            return;
        };
        if watched_ms <= 0 {
            return;
        }
        let mut payload = Self::base(pid, author_id, surface, position, true);
        let completion = if duration_ms > 0 {
            (watched_ms as f64 / duration_ms as f64).min(1.5)
        } else {
            0.0
        };
        payload.insert(rise_key::engagement::WATCHED_MS.to_owned(), EngagementValue::Int(watched_ms));
        payload.insert(rise_key::post::DURATION_MS.to_owned(), EngagementValue::Int(duration_ms.max(0)));
        payload.insert(
            rise_key::engagement::COMPLETION_PCT.to_owned(),
            EngagementValue::Double((completion * 1000.0).round() / 1000.0),
        );
        payload.insert(rise_key::engagement::REPLAYS.to_owned(), EngagementValue::Int(replays.max(0)));
        self.enqueue(RiseAnalyticsEvents::video_watch(), payload, None, cx);
    }

    pub fn skip(
        &self,
        post_id: &str,
        author_id: Option<&str>,
        surface: EngagementSurface,
        position: i64,
        visible_ms: i64,
        cx: &mut App,
    ) {
        if !EngagementFeatureGate::allows(EngagementFeatureGateChannel::Attention) {
            return;
        }
        let Ok(pid) = post_id.parse::<i64>() else {
            return;
        };
        let mut payload = Self::base(pid, author_id.unwrap_or(""), surface, position, false);
        payload.insert(
            rise_key::engagement::VISIBLE_MS.to_owned(),
            EngagementValue::Int(visible_ms.max(0)),
        );
        self.enqueue(RiseAnalyticsEvents::post_skip(), payload, None, cx);
    }

    pub fn comments_opened(
        &self,
        post_id: &str,
        author_id: Option<&str>,
        surface: EngagementSurface,
        cx: &mut App,
    ) {
        if !EngagementFeatureGate::allows(EngagementFeatureGateChannel::Action) {
            return;
        }
        let Ok(pid) = post_id.parse::<i64>() else {
            return;
        };
        let mut payload: HashMap<String, EngagementValue> = HashMap::from([
            (rise_key::report::POST_ID.to_owned(), EngagementValue::Int(pid)),
            (
                rise_key::report::SURFACE.to_owned(),
                EngagementValue::String(surface.slug().to_owned()),
            ),
        ]);
        if let Some(author_id) = author_id.filter(|id| !id.is_empty()) {
            payload.insert(
                rise_key::report::AUTHOR_ID.to_owned(),
                EngagementValue::String(author_id.to_owned()),
            );
        }
        self.enqueue(
            RiseAnalyticsEvents::comments_opened(),
            payload,
            Some(format!("comments:{post_id}:{}", surface.slug())),
            cx,
        );
    }

    pub fn share(
        &self,
        post_id: &str,
        author_id: &str,
        surface: EngagementSurface,
        is_short: bool,
        cx: &mut App,
    ) {
        if !EngagementFeatureGate::allows(EngagementFeatureGateChannel::Action) {
            return;
        }
        let Ok(pid) = post_id.parse::<i64>() else {
            return;
        };
        self.enqueue(
            RiseAnalyticsEvents::post_share(),
            Self::base(pid, author_id, surface, -1, is_short),
            None,
            cx,
        );
    }

    pub fn follow(&self, followee_id: &str, post_id: Option<&str>, cx: &mut App) {
        if !EngagementFeatureGate::allows(EngagementFeatureGateChannel::Action) || followee_id.is_empty() {
            return;
        }
        let mut payload: HashMap<String, EngagementValue> = HashMap::from([(
            rise_key::engagement::FOLLOWEE_ID.to_owned(),
            EngagementValue::String(followee_id.to_owned()),
        )]);
        if let Some(pid) = post_id.and_then(|id| id.parse::<i64>().ok()) {
            payload.insert(rise_key::report::POST_ID.to_owned(), EngagementValue::Int(pid));
        }
        self.enqueue(RiseAnalyticsEvents::follow_user(), payload, None, cx);
    }

    pub fn profile_view(&self, profile_user_id: &str, post_id: Option<&str>, cx: &mut App) {
        if !EngagementFeatureGate::allows(EngagementFeatureGateChannel::Action) || profile_user_id.is_empty() {
            return;
        }
        let mut payload: HashMap<String, EngagementValue> = HashMap::from([(
            rise_key::user::PROFILE_USER_ID.to_owned(),
            EngagementValue::String(profile_user_id.to_owned()),
        )]);
        if let Some(pid) = post_id.and_then(|id| id.parse::<i64>().ok()) {
            payload.insert(rise_key::report::POST_ID.to_owned(), EngagementValue::Int(pid));
        }
        self.enqueue(RiseAnalyticsEvents::profile_view(), payload, None, cx);
    }

    fn base(
        pid: i64,
        author_id: &str,
        surface: EngagementSurface,
        position: i64,
        is_short: bool,
    ) -> HashMap<String, EngagementValue> {
        let mut p: HashMap<String, EngagementValue> = HashMap::from([
            (rise_key::report::POST_ID.to_owned(), EngagementValue::Int(pid)),
            (
                rise_key::report::SURFACE.to_owned(),
                EngagementValue::String(surface.slug().to_owned()),
            ),
            (rise_key::events::POSITION.to_owned(), EngagementValue::Int(position)),
            (rise_key::post::IS_SHORT.to_owned(), EngagementValue::Bool(is_short)),
        ]);
        if !author_id.is_empty() {
            p.insert(
                rise_key::report::AUTHOR_ID.to_owned(),
                EngagementValue::String(author_id.to_owned()),
            );
        }
        p
    }

    fn enqueue(
        &self,
        r#type: &str,
        payload: HashMap<String, EngagementValue>,
        dedup_key: Option<String>,
        cx: &mut App,
    ) {
        let event = EngagementEvent {
            id: Uuid::new_v4().uuid_string(),
            event_type: r#type.to_owned(),
            timestamp_ms: (RiseClock::now().time_interval_since_1970() * 1000.0) as i64,
            payload,
        };
        Self::service(cx).update(cx, |service, cx| service.enqueue(event, dedup_key, cx));
    }
}
