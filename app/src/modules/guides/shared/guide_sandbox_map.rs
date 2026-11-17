use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::Duration;

use gpui::{App, AsyncApp, Context, WeakEntity};
use rise_canon::RiseGuideSignal;
use rise_canon::RiseLimits;
use rise_canon::rise_wire_methods::map as methods;
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_media::map_view::LatLng;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::widgets::notifier::notifier_store::{NotifierPosition, NotifierSendOptions, NotifierStore, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::guides::engine::rise_guides_models::RiseGuide;
use crate::modules::guides::shared::guide_sandbox::{GuideSandbox, SCENE_PARAM};
use crate::modules::map::components::globe::map_globe_layer::MapCameraRequest;
use crate::modules::map::components::tracks::map_tracks_panel::MapPanelMode;
use crate::modules::map::engine::map_track_erasure_geometry::{
    MapEraseStroke, MapErasureMask, MapErasureMaskPreparedStroke,
};
use crate::modules::map::engine::rise_map_engine_models::{
    RiseMapFriendsSnapshot, RiseMapLiveSnapshot, RiseMapLoadState, RiseMapStateSnapshot,
};
use crate::modules::map::engine::rise_map_presentation::RiseMapPresentation;
use crate::modules::map::engine::rise_map_profiles_repository::{MapProfile, MapProfileSnapshot, MapProfileTab};
use crate::modules::map::engine::rise_map_tracks_repository::RiseMapTracksSnapshot;
use crate::modules::map::recorder::map_self_location_feed::{MapSelfLocation, MapSelfLocationFeed};
use crate::modules::map::recorder::map_track_codec::MapTrackGeometry;
use crate::modules::map::stores::map_actions::map_photos_types::{MapPhoto, MapPhotoGrid};
use crate::modules::map::stores::map_actions::map_tracks_types::{
    MapPlace, MapPlaceKind, MapTrackPeriod, MapTrackPeriodStats, MapTrackSummary,
};
use crate::modules::map::stores::map_actions::map_types::{
    MapFix, MapFriend, MapFriendPosition, MapFriendRequest, MapLiveFriend, MapMotion, MapRequestStatus, MapSettings,
    MapSettingsPatch, MapState, MapUser, MapVenueKind,
};
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::post::stores::post_actions::post_types::PostCollaboratorUser;

/// whether a second person is part of the lesson and how
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuideSandboxMapScenePandaRole {
    Absent,
    Stranger,
    Friend,
}

/// fake half of the map for one guide, the map is real
/// a reader not in maps gets a member answer
/// panda friend and place never reach the server
#[derive(Clone, Debug, PartialEq)]
pub struct GuideSandboxMapScene {
    pub panda: GuideSandboxMapScenePandaRole,
    pub request_sent_at_ms: Option<i64>,
    pub panda_fix: Option<MapFix>,
    pub home: Option<MapPlace>,
    pub settings: MapSettingsPatch,
    pub shows_lines: Option<bool>,
    pub places: Vec<MapPlace>,
    pub demo_tail: Vec<Vec<LatLng>>,
    pub wants_demo_tail: bool,
    pub shows_panda_routes: bool,
    pub seeds_grid: bool,
    pub photo_grids: Vec<MapPhotoGrid>,
    pub photos_by_grid: HashMap<String, Vec<MapPhoto>>,
    pub faked_self: Option<MapSelfLocation>,
    pub revision: u64,
}

impl GuideSandboxMapScene {
    pub fn new(panda: GuideSandboxMapScenePandaRole, wants_demo_tail: bool) -> Self {
        Self {
            panda,
            request_sent_at_ms: None,
            panda_fix: None,
            home: None,
            settings: MapSettingsPatch::default(),
            shows_lines: None,
            places: Vec::new(),
            demo_tail: Vec::new(),
            wants_demo_tail,
            shows_panda_routes: false,
            seeds_grid: false,
            photo_grids: Vec::new(),
            photos_by_grid: HashMap::new(),
            faked_self: None,
            revision: 1,
        }
    }
}

static MAP_REFUSED_METHODS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    [
        methods::JOIN,
        methods::UPDATE_SETTINGS,
        methods::SET_PLACE,
        methods::DELETE_PLACE,
        methods::SEND_REACTION,
        methods::SEND_FRIEND_REQUEST,
        methods::RESPOND_FRIEND_REQUEST,
        methods::CANCEL_FRIEND_REQUEST,
        methods::REMOVE_FRIEND,
        methods::ERASE_TRACKS,
        methods::DELETE_ALL_TRACKS,
        methods::RESET_INVITE,
        methods::UPDATE_PRIVACY,
        methods::SET_PRIVACY_EXCEPTIONS,
        methods::LEAVE,
        methods::CREATE_PHOTO,
        methods::UPDATE_PHOTO_GRID,
        methods::DELETE_PHOTO,
        methods::DELETE_PHOTO_GRID,
        methods::PREPARE_JOURNEY,
        methods::REVOKE_JOURNEY,
    ]
    .into_iter()
    .map(|method| format!("{}.{method}", methods::SERVICE))
    .collect()
});

impl GuideSandbox {
    const PANDA_ACCEPT_MS: u64 = 3_500;

    const FALLBACK_COORDINATE: LatLng = LatLng::new(43.2380, 76.9450);

    pub fn wants_map(guide: &RiseGuide) -> bool {
        guide.steps.iter().any(|step| {
            step.route_params
                .get(SCENE_PARAM)
                .is_some_and(|scene| *scene == Self::map_scene_id())
        })
    }

    pub fn arm_map(&mut self, guide: &RiseGuide, cx: &mut Context<Self>) {
        let signals: HashSet<RiseGuideSignal> = guide.steps.iter().filter_map(|step| step.advance_signal).collect();
        let teaches_adding = signals.contains(&RiseGuideSignal::MapFriendRequestSent);
        let needs_friend = [
            RiseGuideSignal::MapFriendCardOpen,
            RiseGuideSignal::MapReactionSent,
            RiseGuideSignal::MapReactionBurst,
            RiseGuideSignal::MapReactionPickerOpen,
            RiseGuideSignal::MapFriendProfileOpen,
        ]
        .iter()
        .any(|signal| signals.contains(signal));
        let wants_tail = [
            RiseGuideSignal::MapEraserHasStrokes,
            RiseGuideSignal::MapTracksErased,
            RiseGuideSignal::MapLinesToggled,
        ]
        .iter()
        .any(|signal| signals.contains(signal));
        let panda = if teaches_adding {
            GuideSandboxMapScenePandaRole::Stranger
        } else if needs_friend {
            GuideSandboxMapScenePandaRole::Friend
        } else {
            GuideSandboxMapScenePandaRole::Absent
        };
        let mut scene = GuideSandboxMapScene::new(panda, wants_tail);
        scene.seeds_grid = signals.contains(&RiseGuideSignal::MapJourneyOutlined);
        self.map = Some(scene);
        cx.notify();
        *Self::refused_methods().lock() = MAP_REFUSED_METHODS.clone();
        GuideFacts::set(RiseGuideSignal::MapMember, true, cx);
        let interactions = MapInteractionsStore::shared(cx);
        interactions.update(cx, |interactions, cx| interactions.set_panel_mode(MapPanelMode::Friends, cx));
        self.place_map_scene(cx);
    }

    pub fn withdraw_map(&mut self, cx: &mut Context<Self>) {
        self.map_tasks.clear();
        let had_scene = self.map.is_some();
        if had_scene {
            self.withdraw_map_photos(cx);
        }
        let feed = MapSelfLocationFeed::shared(cx);
        if let Some(faked) = self.map.as_ref().and_then(|scene| scene.faked_self.clone())
            && feed.read(cx).location() == Some(&faked)
        {
            feed.update(cx, |feed, cx| feed.reset(cx));
        }
        self.map = None;
        cx.notify();
        Self::refused_methods().lock().clear();
        if !had_scene {
            return;
        }
        let presentation = RiseMapPresentation::shared(cx);
        let is_member = presentation.read(cx).state.read(cx).snapshot().is_member();
        GuideFacts::set(RiseGuideSignal::MapMember, is_member, cx);
        GuideFacts::set(RiseGuideSignal::MapFriendCardOpen, false, cx);
        let panda_id = Self::panda_user_id();
        let place_id = Self::map_place_id();
        let interactions = MapInteractionsStore::shared(cx);
        interactions.update(cx, |interactions, cx| {
            if interactions.selected_friend_id() == Some(panda_id.as_str()) {
                interactions.select_friend(None, cx);
            }
            if interactions
                .place_sheet()
                .is_some_and(|sheet| sheet.draft.id == place_id)
            {
                interactions.set_place_sheet(None, cx);
            }
            if interactions.routes_owner().is_some_and(|owner| owner.id == panda_id) {
                interactions.show_own_routes(cx);
            }
        });
    }

    pub fn owns_map_scene(&self) -> bool {
        self.is_armed && self.map.is_some()
    }

    pub fn owns_map_user(&self, id: Option<&str>) -> bool {
        self.owns_map_scene() && id == Some(Self::panda_user_id().as_str())
    }

    /// panda and the practice line go where the reader is, last position the phone sent
    /// desktop has no location of its own so default place otherwise
    fn place_map_scene(&mut self, cx: &mut Context<Self>) {
        let centre = MapSelfLocationFeed::shared(cx)
            .read(cx)
            .location()
            .map(|here| LatLng::new(here.latitude, here.longitude))
            .unwrap_or(Self::FALLBACK_COORDINATE);
        self.place_map_scene_at(centre, cx);
    }

    fn place_map_scene_at(&mut self, centre: LatLng, cx: &mut Context<Self>) {
        let Some(mut scene) = self.map.clone() else {
            return;
        };
        let now = Self::now_milliseconds();
        let feed = MapSelfLocationFeed::shared(cx);
        if feed.read(cx).location().is_none() {
            let faked = MapSelfLocation {
                latitude: centre.latitude,
                longitude: centre.longitude,
                accuracy_meters: 10.0,
                speed_meters_per_second: 0.0,
                course_degrees: -1.0,
                time_ms: now,
                is_at_home: false,
                place_kind: None,
                stay_since_ms: None,
                unavailable_since_ms: None,
            };
            feed.update(cx, |feed, cx| feed.publish(faked, cx));
            scene.faked_self = feed.read(cx).location().cloned();
            let request = MapCameraRequest {
                id: Uuid::new_v4(),
                latitude: centre.latitude,
                longitude: centre.longitude,
                zoom_level: 15.0,
                is_animated: false,
            };
            MapInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.set_camera_request(Some(request), cx));
        }
        if scene.panda != GuideSandboxMapScenePandaRole::Absent {
            scene.panda_fix = Some(MapFix {
                latitude: centre.latitude + 0.0021,
                longitude: centre.longitude + 0.0017,
                accuracy_meters: 12.0,
                speed_meters_per_second: 0.0,
                heading_degrees: -1.0,
                battery_level: 76,
                is_charging: false,
                motion: MapMotion::Still,
                recorded_at: now,
                stay_since: now - 47 * 60_000,
                place_kind: Some(MapVenueKind::Cafe),
            });
        }
        if scene.wants_demo_tail || scene.seeds_grid {
            scene.demo_tail = if scene.wants_demo_tail {
                vec![Self::loop_around(centre, 80.0)]
            } else {
                Vec::new()
            };
            let request = MapCameraRequest {
                id: Uuid::new_v4(),
                latitude: centre.latitude,
                longitude: centre.longitude,
                zoom_level: 15.5,
                is_animated: false,
            };
            MapInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.set_camera_request(Some(request), cx));
        }
        scene.revision = scene.revision.wrapping_add(1);
        self.map = Some(scene);
        cx.notify();
        self.seed_practice_grid(centre, cx);
    }

    fn loop_around(centre: LatLng, radius_meters: f64) -> Vec<LatLng> {
        let metres_per_degree = 111_320.0;
        let latitude_scale = radius_meters / metres_per_degree;
        let longitude_scale =
            radius_meters / (metres_per_degree * (centre.latitude * std::f64::consts::PI / 180.0).cos().max(0.2));
        let steps: i32 = 28;
        (0..=steps)
            .map(|index| {
                let angle = f64::from(index) / f64::from(steps) * 2.0 * std::f64::consts::PI;
                let wobble = 1.0 + 0.18 * (angle * 3.0).sin() + 0.08 * (angle * 5.0).cos();
                LatLng::new(
                    centre.latitude + angle.sin() * latitude_scale * wobble,
                    centre.longitude + angle.cos() * longitude_scale * wobble * 1.3,
                )
            })
            .collect()
    }

    /// where the scene stands, beside panda or at the fallback if the device never said
    pub fn map_panda_coordinate(&self) -> LatLng {
        self.map
            .as_ref()
            .and_then(|scene| scene.panda_fix.as_ref())
            .map(MapFix::coordinate)
            .unwrap_or(Self::FALLBACK_COORDINATE)
    }

    fn now_milliseconds() -> i64 {
        (Date::now().time_interval_since_1970() * 1000.0) as i64
    }

    fn update_map(&mut self, change: impl FnOnce(&mut GuideSandboxMapScene), cx: &mut Context<Self>) {
        let Some(scene) = self.map.as_mut() else {
            return;
        };
        change(scene);
        scene.revision = scene.revision.wrapping_add(1);
        cx.notify();
    }

    fn today() -> i32 {
        let now = Date::now();
        let offset_minutes = i64::from(TimeZone::current().seconds_from_gmt(&now) / 60);
        MapTrackGeometry::local_day(Self::now_milliseconds(), offset_minutes)
    }

    pub fn map_panda(&self) -> MapUser {
        MapUser {
            id: Self::panda_user_id(),
            tag: tr("guide_scene_sender_tag"),
            name: tr("guide_scene_sender"),
            logo: String::new(),
            is_premium: false,
            is_official: false,
            name_color: None,
            premium_badge_emoji: None,
            is_deleted: false,
        }
    }

    fn map_viewer(&self, cx: &mut App) -> MapUser {
        let store = AuthServicesStore::shared(cx);
        let profile = store.read(cx).profile();
        MapUser {
            id: SaiAccountCacheScope::active_user_id().unwrap_or_default(),
            tag: profile.map(|profile| profile.tag.clone()).unwrap_or_default(),
            name: profile.map(|profile| profile.name.clone()).unwrap_or_default(),
            logo: profile
                .map(|profile| profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone()))
                .unwrap_or_default(),
            is_premium: profile.is_some_and(|profile| profile.more.is_premium),
            is_official: profile.is_some_and(|profile| profile.more.is_official),
            name_color: profile.and_then(|profile| profile.more.name_color.clone()),
            premium_badge_emoji: profile.and_then(|profile| profile.more.premium_badge_emoji.clone()),
            is_deleted: false,
        }
    }

    pub fn map_picker_suggestions(&self) -> Vec<PostCollaboratorUser> {
        let Some(scene) = self.map.as_ref() else {
            return Vec::new();
        };
        if !self.owns_map_scene()
            || scene.panda != GuideSandboxMapScenePandaRole::Stranger
            || scene.request_sent_at_ms.is_some()
        {
            return Vec::new();
        }
        vec![PostCollaboratorUser {
            id: Self::panda_user_id(),
            is_friend: false,
            is_official: false,
            is_premium: false,
            logo: String::new(),
            name: tr("guide_scene_sender"),
            name_color: None,
            premium_badge_emoji: None,
            since_ms: 0,
            tag: tr("guide_scene_sender_tag"),
        }]
    }

    /// reader asked panda to be friends on the map
    /// she answers later and the request sits in your requests until then
    pub fn request_map_friendship(&mut self, cx: &mut Context<Self>) {
        let pending_stranger = self.map.as_ref().is_some_and(|scene| {
            scene.panda == GuideSandboxMapScenePandaRole::Stranger && scene.request_sent_at_ms.is_none()
        });
        if !self.owns_map_scene() || !pending_stranger {
            return;
        }
        let now = Self::now_milliseconds();
        self.update_map(|scene| scene.request_sent_at_ms = Some(now), cx);
        GuideSignalBus::emit(RiseGuideSignal::MapFriendRequestSent);
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_millis(Self::PANDA_ACCEPT_MS))
                .await;
            this.update(cx, |sandbox, cx| {
                if !sandbox.owns_map_scene() {
                    return;
                }
                sandbox.update_map(
                    |scene| {
                        scene.panda = GuideSandboxMapScenePandaRole::Friend;
                        scene.request_sent_at_ms = None;
                    },
                    cx,
                );
                sandbox.place_map_scene(cx);
                let options = NotifierSendOptions {
                    r#type: NotifierType::Success,
                    position: NotifierPosition::Top,
                    ..NotifierSendOptions::default()
                };
                NotifierStore::shared(cx).update(cx, |notifier, cx| {
                    notifier.send(tr("map_friend_added"), options, cx);
                });
                GuideSignalBus::emit(RiseGuideSignal::MapFriendAdded);
            })
            .ok();
        });
        self.map_tasks.push(task);
    }

    /// cancelling the practice request takes it back without an answer
    pub fn cancel_map_friendship(&mut self, cx: &mut Context<Self>) {
        if !self.owns_map_scene() {
            return;
        }
        self.map_tasks.clear();
        self.update_map(|scene| scene.request_sent_at_ms = None, cx);
    }

    pub fn show_panda_routes(&mut self, shown: bool, cx: &mut Context<Self>) {
        if !self.owns_map_scene() || self.map.as_ref().is_some_and(|scene| scene.shows_panda_routes == shown) {
            return;
        }
        self.update_map(|scene| scene.shows_panda_routes = shown, cx);
    }

    /// panda's card as the profile endpoint would describe a friend who opened everything
    pub fn map_panda_profile(&self) -> Option<MapProfileSnapshot> {
        if !self.owns_map_scene() {
            return None;
        }
        let scene = self.map.as_ref();
        let profile = MapProfile {
            user: self.map_panda(),
            friends_count: 12,
            is_friend: scene.is_some_and(|scene| scene.panda == GuideSandboxMapScenePandaRole::Friend),
            outgoing_request_id: scene
                .and_then(|scene| scene.request_sent_at_ms)
                .map(|_| Self::map_request_id()),
            can_view_tracks: true,
            can_view_media: true,
            can_view_friends: false,
        };
        Some(MapProfileSnapshot {
            profile: Some(profile),
            grids: Vec::new(),
            friends: Vec::new(),
            has_more_grids: false,
            has_more_friends: false,
            loaded_tabs: MapProfileTab::ALL_CASES.into_iter().collect(),
            failed: false,
            page_revision: scene.map_or(0, |scene| scene.revision),
        })
    }

    fn panda_summary(&self) -> MapTrackSummary {
        let today = Self::today();
        let stats = |period: MapTrackPeriod, days: i32, km: f64, walk_km: f64, minutes: i64, active: i64| {
            MapTrackPeriodStats {
                period,
                since_day: today - days + 1,
                distance_meters: (km * 1000.0) as i64,
                duration_seconds: minutes * 60,
                active_days: active,
                walk_run_distance_meters: Some((walk_km * 1000.0) as i64),
            }
        };
        MapTrackSummary {
            revision: 1,
            periods: vec![
                stats(MapTrackPeriod::Day, 1, 3.4, 2.1, 52, 1),
                stats(MapTrackPeriod::Week, 7, 21.8, 11.6, 312, 6),
                stats(MapTrackPeriod::Month, 30, 94.5, 47.2, 1_380, 24),
                stats(MapTrackPeriod::HalfYear, 182, 611.0, 280.0, 8_640, 151),
                stats(MapTrackPeriod::Year, 365, 1_284.0, 566.0, 17_900, 297),
                stats(MapTrackPeriod::All, 540, 1_907.0, 812.0, 26_300, 431),
            ],
            bands: Vec::new(),
            first_day: today - 539,
            last_day: today,
        }
    }

    pub fn apply_map_settings(&mut self, patch: &MapSettingsPatch, cx: &mut Context<Self>) {
        if !self.owns_map_scene() {
            return;
        }
        self.update_map(|scene| scene.settings = scene.settings.merged(patch), cx);
        GuideSignalBus::emit(RiseGuideSignal::MapSettingsChanged);
    }

    pub fn set_map_home(&mut self, coordinate: Option<LatLng>, cx: &mut Context<Self>) {
        if !self.owns_map_scene() {
            return;
        }
        self.update_map(
            |scene| {
                scene.home = coordinate.map(|coordinate| {
                    MapPlace::new(
                        Self::map_home_id(),
                        MapPlaceKind::Home,
                        false,
                        String::new(),
                        coordinate.latitude,
                        coordinate.longitude,
                        RiseLimits::service_value("map_home_radius_m"),
                    )
                });
            },
            cx,
        );
    }

    pub fn toggle_map_lines(&mut self, current: bool, cx: &mut Context<Self>) -> bool {
        let next = !current;
        self.update_map(|scene| scene.shows_lines = Some(next), cx);
        GuideSignalBus::emit(RiseGuideSignal::MapLinesToggled);
        next
    }

    pub fn save_map_place(&mut self, draft: &MapPlace, cx: &mut Context<Self>) -> MapPlace {
        let mut saved = draft.clone();
        if !saved.id.starts_with(&Self::map_place_id()) {
            saved = MapPlace {
                id: Self::map_place_id(),
                kind: draft.kind,
                is_inferred: false,
                name: draft.name.clone(),
                latitude: draft.latitude,
                longitude: draft.longitude,
                radius_meters: draft.radius_meters,
                description: draft.description.clone(),
                emoji: draft.emoji.clone(),
                custom_emoji: draft.custom_emoji.clone(),
                record_tracks: draft.record_tracks,
                track_exclusion_radius_meters: draft.track_exclusion_radius_meters,
            };
        }
        let stored = saved.clone();
        self.update_map(
            move |scene| {
                scene.places.retain(|place| place.id != stored.id);
                scene.places.push(stored);
            },
            cx,
        );
        GuideSignalBus::emit(RiseGuideSignal::MapPlaceSaved);
        saved
    }

    pub fn delete_map_place(&mut self, id: &str, cx: &mut Context<Self>) {
        self.update_map(|scene| scene.places.retain(|place| place.id != id), cx);
    }

    pub fn owns_map_place(&self, id: &str) -> bool {
        self.owns_map_scene() && id == Self::map_place_id()
    }

    pub fn erase_map_tail(&mut self, strokes: &[MapEraseStroke], cx: &mut Context<Self>) {
        if !self.owns_map_scene() {
            return;
        }
        let mask = MapErasureMask::new(
            strokes
                .iter()
                .cloned()
                .map(MapErasureMaskPreparedStroke::new)
                .collect(),
        );
        let today = Self::today();
        self.update_map(
            |scene| {
                scene.demo_tail = scene
                    .demo_tail
                    .iter()
                    .flat_map(|line| mask.clip(line, today))
                    .filter(|line| line.len() >= 2)
                    .collect();
            },
            cx,
        );
        GuideSignalBus::emit(RiseGuideSignal::MapTracksErased);
    }

    pub fn map_state(&self, real: RiseMapStateSnapshot) -> RiseMapStateSnapshot {
        let Some(scene) = self.map.as_ref() else {
            return real;
        };
        if !self.is_armed {
            return real;
        }
        let state = real.state.clone().filter(|state| state.is_member).unwrap_or_else(|| MapState {
            is_member: true,
            joined_at: (Self::scene_stamp() * 1000.0) as i64,
            settings: MapSettings::initial(),
            friends_count: 0,
            incoming_requests_count: 0,
        });
        let overlaid = MapState {
            is_member: true,
            joined_at: state.joined_at,
            settings: scene.settings.applied(&state.settings),
            friends_count: state.friends_count
                + if scene.panda == GuideSandboxMapScenePandaRole::Friend { 1 } else { 0 },
            incoming_requests_count: state.incoming_requests_count,
        };
        RiseMapStateSnapshot {
            state: Some(overlaid),
            is_joining: false,
            load_state: RiseMapLoadState::Loaded,
            error_text: real.error_text,
            cache_source: real.cache_source,
            revision: real.revision.wrapping_add(scene.revision),
        }
    }

    pub fn map_friends(&self, real: RiseMapFriendsSnapshot, cx: &mut App) -> RiseMapFriendsSnapshot {
        let Some(scene) = self.map.as_ref() else {
            return real;
        };
        if !self.is_armed || scene.panda == GuideSandboxMapScenePandaRole::Absent {
            return real;
        }
        let mut friends = real.friends;
        let mut outgoing = real.outgoing;
        let now = Self::now_milliseconds();
        if scene.panda == GuideSandboxMapScenePandaRole::Friend {
            friends.insert(0, MapFriend { user: self.map_panda(), since: now });
        } else if let Some(sent_at) = scene.request_sent_at_ms {
            outgoing.insert(
                0,
                MapFriendRequest {
                    id: Self::map_request_id(),
                    from_user: self.map_viewer(cx),
                    to_user: self.map_panda(),
                    status: MapRequestStatus::Pending,
                    created_at: sent_at,
                    updated_at: sent_at,
                },
            );
        }
        RiseMapFriendsSnapshot {
            friends,
            incoming: real.incoming,
            outgoing,
            has_more_friends: real.has_more_friends,
            has_more_incoming: real.has_more_incoming,
            has_more_outgoing: real.has_more_outgoing,
            is_loading_more: real.is_loading_more,
            invite: real.invite,
            load_state: if real.load_state == RiseMapLoadState::Failed {
                real.load_state
            } else {
                RiseMapLoadState::Loaded
            },
            error_text: real.error_text,
            cache_source: real.cache_source,
            revision: real.revision.wrapping_add(scene.revision),
        }
    }

    pub fn map_live(&self, real: RiseMapLiveSnapshot) -> RiseMapLiveSnapshot {
        let Some(scene) = self.map.as_ref() else {
            return real;
        };
        let Some(fix) = scene.panda_fix.clone() else {
            return real;
        };
        if !self.is_armed || scene.panda != GuideSandboxMapScenePandaRole::Friend {
            return real;
        }
        let panda_id = Self::panda_user_id();
        let panda = MapLiveFriend {
            user: self.map_panda(),
            position: Some(MapFriendPosition::new(panda_id.clone(), fix, false)),
        };
        let mut friends = Vec::with_capacity(real.friends.len() + 1);
        friends.push(panda);
        friends.extend(real.friends.into_iter().filter(|friend| friend.user.id != panda_id));
        RiseMapLiveSnapshot {
            friends,
            is_session_open: real.is_session_open,
            cache_source: real.cache_source,
            revision: real.revision.wrapping_add(scene.revision.wrapping_mul(7_919)),
        }
    }

    pub fn map_tracks(&self, real: RiseMapTracksSnapshot) -> RiseMapTracksSnapshot {
        let Some(scene) = self.map.as_ref() else {
            return real;
        };
        if !self.is_armed {
            return real;
        }
        if scene.shows_panda_routes {
            return RiseMapTracksSnapshot {
                owner_id: Some(Self::panda_user_id()),
                summary: Some(self.panda_summary()),
                period: real.period,
                shows_lines: real.shows_lines,
                home: None,
                places: Vec::new(),
                load_state: RiseMapLoadState::Loaded,
                error_text: None,
                cache_source: real.cache_source,
                revision: real.revision.wrapping_add(scene.revision),
            };
        }
        if real.owner_id.is_some() {
            return real;
        }
        let mut places: Vec<MapPlace> = real
            .places
            .into_iter()
            .filter(|place| place.kind != MapPlaceKind::Home)
            .collect();
        places.extend(scene.places.iter().cloned());
        RiseMapTracksSnapshot {
            owner_id: None,
            summary: real.summary,
            period: real.period,
            shows_lines: scene.shows_lines.unwrap_or(real.shows_lines),
            home: scene.home.clone(),
            places,
            load_state: real.load_state,
            error_text: real.error_text,
            cache_source: real.cache_source,
            revision: real.revision.wrapping_add(scene.revision),
        }
    }

    pub fn map_demo_tail(&self) -> Vec<Vec<LatLng>> {
        if !self.is_armed {
            return Vec::new();
        }
        self.map.as_ref().map(|scene| scene.demo_tail.clone()).unwrap_or_default()
    }
}

impl MapSettingsPatch {
    /// later patch wins field by field, like two real saves in a row
    pub fn merged(&self, later: &MapSettingsPatch) -> MapSettingsPatch {
        MapSettingsPatch {
            sharing_enabled: later.sharing_enabled.or(self.sharing_enabled),
            tracks_enabled: later.tracks_enabled.or(self.tracks_enabled),
            battery_visible: later.battery_visible.or(self.battery_visible),
            frozen_until: later.frozen_until.or(self.frozen_until),
            track_cooling_days: later.track_cooling_days.or(self.track_cooling_days),
            track_heat_walks: later.track_heat_walks.or(self.track_heat_walks),
            home_track_exclusion_radius_m: later.home_track_exclusion_radius_m.or(self.home_track_exclusion_radius_m),
            track_merge_enabled: later.track_merge_enabled.or(self.track_merge_enabled),
            track_merge_radius_m: later.track_merge_radius_m.or(self.track_merge_radius_m),
            track_merge_lanes: later.track_merge_lanes.or(self.track_merge_lanes),
            track_precise_recording: later.track_precise_recording.or(self.track_precise_recording),
        }
    }
}
