use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::future::BoxFuture;
use gpui::{App, Context, Entity, Task};
use rise_platform::process_activity::begin_process_activity;
use rise_canon::{RiseLimits, RiseTimingCanon};
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::rise_app_events::{RiseAppEvents, RiseNotificationName, RiseSubscription, UIApplication};
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_platform::location::{
    self, CircularRegion, DesiredAccuracy, LocationCommand, LocationEvent, LocationFailure, LocationFix,
    LocationHeading, LocationService, LocationSubscription, LocationVisit, RegionState, StationaryWatch,
    UpdateRequest,
};
use rise_platform::power::{self, BATTERY_LEVEL_DID_CHANGE, BATTERY_STATE_DID_CHANGE, BatteryReading, BatteryState};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::MAP_RECORDER_CONFIGURATION;
use serde::{Deserialize, Serialize};

use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::auth::services::account_state_reset_coordinator::{
    AccountResettable, AccountStateResetRegistry,
};
use crate::modules::map::engine::rise_map_live_repository::RiseMapLiveRepository;
use crate::modules::map::recorder::map_location_filter::MapLocationFilter;
use crate::modules::map::recorder::map_place_probe::MapPlaceProbe;
use crate::modules::map::recorder::map_recorder_journal::{MapRecordedStay, MapRecorderJournal};
use crate::modules::map::recorder::map_recorder_policy::{MapRecorderFix, MapRecorderPolicy};
use crate::modules::map::recorder::map_self_location_feed::{MapSelfHeading, MapSelfLocation, MapSelfLocationFeed};
use crate::modules::map::recorder::map_track_codec::MapTrackGeometry;
use crate::modules::map::shared::map_friend_status::MapSpeedFormat;
use crate::modules::map::stores::map_actions::map_types::{
    MapFix, MapMotion, MapSettings, MapVenueKind,
};

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapRecorderConfigurationHome {
    pub latitude: f64,
    pub longitude: f64,
    pub radius_meters: f64,
}

/// account facts the recorder needs
/// persisted since on the phone the system relaunches a killed app for a location event
/// long before any socket or screen exists, desktop reads it back each launch
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapRecorderConfiguration {
    #[serde(rename = "accountID")]
    pub account_id: String,
    pub tracks_enabled: bool,
    pub sharing_enabled: bool,
    pub battery_visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<MapRecorderConfigurationHome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_track_exclusion_radius_m: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place_track_zones: Option<Vec<MapRecorderConfigurationHome>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precise_recording: Option<bool>,
}

impl MapRecorderConfiguration {
    pub fn records_precisely(&self) -> bool {
        self.precise_recording == Some(true)
    }

    pub fn home_track_radius_meters(&self) -> f64 {
        MapSettings::clamped_home_track_radius(
            self.home_track_exclusion_radius_m
                .unwrap_or_else(MapSettings::default_home_track_radius),
        ) as f64
    }

    pub fn wants_location(&self) -> bool {
        self.tracks_enabled || self.sharing_enabled
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Regime {
    Off,
    Moving,
    Staying,
}

const EXIT_REGION_ID: &str = "riseonly.map.recorder.exit";
const MAP_RECORDER_ACTIVITY: &str = "MapRecorderCommit";

static STALE_FIX_MS: LazyLock<i64> = LazyLock::new(|| RiseTimingCanon::freshness_ms("map_friend_motion"));
static MAXIMUM_FUTURE_FIX_MS: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("map_max_fix_future_skew_ms"));
const DISPLAY_FILTER_METERS: f64 = 3.0;
const DISPLAY_ACCURACY_METERS: f64 = 100.0;

/// a journal step, runs on the account's journal and answers whether it sealed a segment
type JournalOperation = Box<dyn FnOnce(Arc<MapRecorderJournal>) -> BoxFuture<'static, bool> + Send>;

fn now_ms() -> i64 {
    RiseClock::now().ms()
}

fn millis(value: i64) -> Duration {
    Duration::from_millis(u64::try_from(value).unwrap_or(0))
}

fn utc_offset_minutes(time_ms: i64) -> i64 {
    i64::from(TimeZone::current().seconds_from_gmt(&Date::from_ms(time_ms))) / 60
}

/// what reaches the main thread from the location seam and the app event center
/// both call back on their own threads
enum RiseMapRecorderSignal {
    Location(LocationEvent),
    AppActive(bool),
    BatteryChanged,
}

/// the recorder's location manager
/// location seam subscription plus the standard service settings it keeps between calls
struct RiseMapRecorderLocationManager {
    _subscription: Option<LocationSubscription>,
    desired_accuracy: DesiredAccuracy,
    distance_filter: Option<f64>,
    is_updating_location: bool,
}

impl RiseMapRecorderLocationManager {
    fn is_authorized(&self) -> bool {
        location::permission().is_ok_and(|permission| permission.level.allows_foreground())
    }

    fn is_reduced_accuracy(&self) -> bool {
        location::permission().is_ok_and(|permission| !permission.is_precise)
    }

    /// the fix the system cached
    fn location(&self) -> Option<LocationFix> {
        location::last_fix().ok().flatten()
    }

    fn start_updating_location(&mut self) {
        self.is_updating_location = true;
        Self::perform(LocationCommand::StartUpdates(UpdateRequest {
            accuracy: self.desired_accuracy,
            distance_filter_meters: self.distance_filter,
        }));
    }

    fn stop_updating_location(&mut self) {
        self.is_updating_location = false;
        Self::perform(LocationCommand::StopUpdates);
    }

    fn set_distance_filter(&mut self, distance_filter: Option<f64>) {
        self.distance_filter = distance_filter;
        if self.is_updating_location {
            self.start_updating_location();
        }
    }

    fn perform(command: LocationCommand) {
        if let Err(reason) = location::perform(command) {
            tracing::debug!(target: "riseonly", %reason, "map recorder location command not run");
        }
    }
}

/// records movement and feeds the live layer, outside the engine so no socket needed
/// moving uses standard updates, staying reduced accuracy and nothing written at home
/// economy runs standard updates only in front, precise also when away
pub struct RiseMapRecorder {
    pub on_fix: Option<Rc<dyn Fn(MapFix, &mut App)>>,
    pub on_segment_sealed: Option<Rc<dyn Fn(&mut App)>>,
    pub on_moved: Option<Rc<dyn Fn(&mut App)>>,

    manager: Option<RiseMapRecorderLocationManager>,
    recovery_region: Option<CircularRegion>,
    configuration: Option<MapRecorderConfiguration>,
    regime: Regime,
    motion: MapMotion,
    anchor: Option<MapRecorderFix>,
    stay_since_ms: i64,
    home_since_ms: i64,
    restored_home_location: Option<MapSelfLocation>,
    allows_home_restore: bool,
    stay_timer: Option<Task<()>>,
    is_inside_home: bool,
    is_inside_home_track_zone: bool,
    place_kind: Option<MapVenueKind>,
    place_probe: Option<Task<()>>,
    last_presented: Option<LocationFix>,
    location_filter: MapLocationFilter,
    latest_fix: Option<MapRecorderFix>,
    is_map_on_screen: bool,
    is_app_active: bool,
    journal_tail: Option<Task<()>>,
    configuration_generation: u64,
    last_handled_ms: i64,
    _lifecycle_observers: Vec<RiseSubscription>,
    freshness_task: Option<Task<()>>,
    fresh_fix_task: Option<Task<()>>,
    fresh_fix_requested_at_ms: Option<i64>,

    live_demand_until_ms: i64,
    live_demand_expiry: Option<Task<()>>,

    is_battery_monitoring_enabled: bool,
    battery: BatteryReading,
    battery_refresh: Option<Task<()>>,

    signals: mpsc::UnboundedSender<RiseMapRecorderSignal>,
    _signal_loop: Task<()>,
}

impl RiseMapRecorder {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let (signals, mut inbox) = mpsc::unbounded::<RiseMapRecorderSignal>();
        let center = RiseAppEvents::default_center();
        let background = signals.clone();
        let active = signals.clone();
        let battery_level = signals.clone();
        let battery_state = signals.clone();
        let lifecycle_observers = vec![
            center.observe(UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION, move |_| {
                let _ = background.unbounded_send(RiseMapRecorderSignal::AppActive(false));
            }),
            center.observe(UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, move |_| {
                let _ = active.unbounded_send(RiseMapRecorderSignal::AppActive(true));
            }),
            center.observe(RiseNotificationName::new(BATTERY_LEVEL_DID_CHANGE), move |_| {
                let _ = battery_level.unbounded_send(RiseMapRecorderSignal::BatteryChanged);
            }),
            center.observe(RiseNotificationName::new(BATTERY_STATE_DID_CHANGE), move |_| {
                let _ = battery_state.unbounded_send(RiseMapRecorderSignal::BatteryChanged);
            }),
        ];
        let signal_loop = cx.spawn(async move |this, cx| {
            while let Some(signal) = inbox.next().await {
                let delivered = this.update(cx, |this, cx| match signal {
                    RiseMapRecorderSignal::Location(event) => this.location_event(event, cx),
                    RiseMapRecorderSignal::AppActive(active) => this.set_app_active(active, cx),
                    RiseMapRecorderSignal::BatteryChanged => this.refresh_battery(cx),
                });
                if delivered.is_err() {
                    return;
                }
            }
        });
        Self {
            on_fix: None,
            on_segment_sealed: None,
            on_moved: None,
            manager: None,
            recovery_region: None,
            configuration: None,
            regime: Regime::Off,
            motion: MapMotion::Unknown,
            anchor: None,
            stay_since_ms: 0,
            home_since_ms: 0,
            restored_home_location: None,
            allows_home_restore: false,
            stay_timer: None,
            is_inside_home: false,
            is_inside_home_track_zone: false,
            place_kind: None,
            place_probe: None,
            last_presented: None,
            location_filter: MapLocationFilter::new(),
            latest_fix: None,
            is_map_on_screen: false,
            is_app_active: RiseAppLifecycle::scene_phase(cx) == ScenePhase::Active,
            journal_tail: None,
            configuration_generation: 0,
            last_handled_ms: 0,
            _lifecycle_observers: lifecycle_observers,
            freshness_task: None,
            fresh_fix_task: None,
            fresh_fix_requested_at_ms: None,
            live_demand_until_ms: 0,
            live_demand_expiry: None,
            is_battery_monitoring_enabled: false,
            battery: BatteryReading::NO_BATTERY,
            battery_refresh: None,
            signals,
            _signal_loop: signal_loop,
        }
    }

    fn is_live_watched(&self) -> bool {
        self.live_demand_until_ms > now_ms()
    }

    fn is_map_visible(&self) -> bool {
        self.is_map_on_screen && self.is_app_active
    }

    fn records_continuously(&self) -> bool {
        self.configuration.as_ref().is_some_and(MapRecorderConfiguration::records_precisely) || self.is_app_active
    }

    fn wants_location(&self) -> bool {
        self.configuration.as_ref().is_some_and(MapRecorderConfiguration::wants_location)
    }

    fn sharing_enabled(&self) -> bool {
        self.configuration.as_ref().is_some_and(|configuration| configuration.sharing_enabled)
    }

    fn account_id(&self) -> Option<String> {
        self.configuration.as_ref().map(|configuration| configuration.account_id.clone())
    }

    fn is_authorized(&self) -> bool {
        self.manager.as_ref().is_some_and(RiseMapRecorderLocationManager::is_authorized)
    }

    fn is_reduced_accuracy(&self) -> bool {
        self.manager.as_ref().is_some_and(RiseMapRecorderLocationManager::is_reduced_accuracy)
    }

    pub fn set_map_on_screen(&mut self, on_screen: bool, cx: &mut Context<Self>) {
        if on_screen == self.is_map_on_screen {
            return;
        }
        let was_visible = self.is_map_visible();
        self.is_map_on_screen = on_screen;
        if was_visible != self.is_map_visible() {
            self.map_visibility_changed(cx);
        }
    }

    fn set_app_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if active == self.is_app_active {
            return;
        }
        let was_visible = self.is_map_visible();
        self.is_app_active = active;
        if was_visible != self.is_map_visible() {
            self.map_visibility_changed(cx);
        }
        if active {
            if self.wants_location() {
                self.start(cx);
            }
        } else if self.wants_location()
            && !self.configuration.as_ref().is_some_and(MapRecorderConfiguration::records_precisely)
        {
            self.sync_standard_updates();
        }
    }

    fn map_visibility_changed(&mut self, cx: &mut Context<Self>) {
        MapSelfLocationFeed::shared(cx).update(cx, |feed, cx| feed.publish_heading(None, cx));
        self.sync_standard_updates();
    }

    /// keeps the location session alive across stops
    /// where the host has no wake up service for a standing user
    /// coarse standard updates watch for the next walk
    fn sync_standard_updates(&mut self) {
        let regime = self.regime;
        let records_continuously = self.records_continuously();
        let is_map_visible = self.is_map_visible();
        let authorized = self.is_authorized();
        let urgent = self.is_live_watched() || self.fresh_fix_requested_at_ms.is_some();
        let moving_filter = self.moving_filter();
        let Some(manager) = self.manager.as_mut() else {
            return;
        };
        if regime != Regime::Off && records_continuously {
            manager.desired_accuracy = if regime == Regime::Moving || urgent {
                DesiredAccuracy::Best
            } else {
                DesiredAccuracy::NearestTenMeters
            };
            manager.distance_filter = if urgent {
                None
            } else if regime == Regime::Moving {
                moving_filter
            } else {
                Some(MapRecorderPolicy::distance_filter(MapMotion::Walk))
            };
            manager.start_updating_location();
        } else if is_map_visible && authorized {
            manager.desired_accuracy = DesiredAccuracy::Best;
            manager.distance_filter = Some(DISPLAY_FILTER_METERS);
            manager.start_updating_location();
        } else if let (true, StationaryWatch::CoarseUpdates(request)) = (
            regime != Regime::Off,
            location::recorder_stationary_watch(MapRecorderPolicy::exit_region_meters()),
        ) {
            manager.desired_accuracy = request.accuracy;
            manager.distance_filter = request.distance_filter_meters;
            manager.start_updating_location();
        } else {
            manager.stop_updating_location();
        }
    }

    /// none means no distance filter
    fn moving_filter(&self) -> Option<f64> {
        if self.is_live_watched() || self.fresh_fix_requested_at_ms.is_some() {
            return None;
        }
        let filter = MapRecorderPolicy::distance_filter(self.motion);
        Some(if self.is_map_visible() { filter.min(DISPLAY_FILTER_METERS) } else { filter })
    }

    fn update_location_availability(&mut self, available: bool, cx: &mut Context<Self>) {
        let feed = MapSelfLocationFeed::shared(cx);
        let before = feed.read(cx).location().cloned();
        feed.update(cx, |feed, cx| feed.set_location_available(available, cx));
        let Some(snapshot) = feed.read(cx).location().cloned() else {
            return;
        };
        if Some(&snapshot) == before.as_ref() {
            return;
        }
        let Some(account_id) = self.account_id() else {
            return;
        };
        self.enqueue_journal(account_id, true, Self::save_location_operation(snapshot), cx);
    }

    fn save_location_operation(snapshot: MapSelfLocation) -> JournalOperation {
        Box::new(move |journal: Arc<MapRecorderJournal>| {
            async move {
                journal.save_location(&snapshot);
                false
            }
            .boxed()
        })
    }

    fn receive(&mut self, location: LocationFix, cx: &mut Context<Self>) {
        if !(self.is_map_visible() || self.wants_location())
            || !self.is_authorized()
            || !location.is_usable()
            || !location.horizontal_accuracy_meters.is_finite()
            || location.time_ms > now_ms() + *MAXIMUM_FUTURE_FIX_MS
        {
            return;
        }
        if self.is_reduced_accuracy()
            || location.horizontal_accuracy_meters > MapRecorderPolicy::minimum_accuracy_meters()
        {
            self.receive_uncertain_location(location, cx);
            return;
        }
        let Some(filtered) = self.location_filter.accept(&location) else {
            return;
        };
        if filtered.horizontal_accuracy_meters > MapRecorderPolicy::minimum_accuracy_meters() {
            self.receive_uncertain_location(filtered, cx);
            return;
        }
        self.present(filtered, false, cx);
        self.handle(filtered, cx);
    }

    fn receive_uncertain_location(&mut self, location: LocationFix, cx: &mut Context<Self>) {
        let time_ms = location.time_ms;
        if time_ms <= self.last_handled_ms || now_ms() - time_ms > *STALE_FIX_MS {
            return;
        }
        self.last_handled_ms = time_ms;
        if self.is_reduced_accuracy() {
            self.complete_fresh_fix_request(time_ms);
        }
        self.present(location, false, cx);
        self.publish(
            MapRecorderFix {
                latitude: location.latitude,
                longitude: location.longitude,
                accuracy_meters: location.horizontal_accuracy_meters,
                speed_meters_per_second: 0.0,
                heading_degrees: -1.0,
                time_ms,
            },
            true,
            cx,
        );
    }

    fn present(&mut self, location: LocationFix, refreshing_status: bool, cx: &mut Context<Self>) {
        let time_ms = location.time_ms;
        let feed = MapSelfLocationFeed::shared(cx);
        let feed_time_ms = feed.read(cx).location().map(|current| current.time_ms);
        let refreshes_existing_fix = refreshing_status && feed_time_ms == Some(time_ms);
        let now = now_ms();
        if !(self.is_map_visible() || self.wants_location())
            || location.horizontal_accuracy_meters < 0.0
            || !location.is_usable()
            || !(refreshes_existing_fix || now - time_ms <= *STALE_FIX_MS)
            || time_ms > now + *MAXIMUM_FUTURE_FIX_MS
            || time_ms < feed_time_ms.unwrap_or(0)
        {
            return;
        }
        let approximate_access = self.is_reduced_accuracy();
        if !(approximate_access
            || location.horizontal_accuracy_meters <= DISPLAY_ACCURACY_METERS
            || feed_time_ms.is_none())
        {
            return;
        }
        let approximate =
            approximate_access || location.horizontal_accuracy_meters > MapRecorderPolicy::minimum_accuracy_meters();
        self.last_presented = Some(location);
        let inside_home = !approximate
            && self
                .configuration
                .as_ref()
                .and_then(|configuration| configuration.home.as_ref())
                .is_some_and(|home| {
                    MapTrackGeometry::haversine_meters(location.latitude, location.longitude, home.latitude, home.longitude)
                        <= home.radius_meters
                });
        if !approximate && !refreshes_existing_fix && location.horizontal_accuracy_meters <= DISPLAY_ACCURACY_METERS {
            self.update_home_stay(&location, inside_home);
        }
        let is_at_home = !approximate && (inside_home || self.home_since_ms > 0);
        let staying = self.regime == Regime::Staying;
        let stay_since_ms = if is_at_home && self.home_since_ms > 0 {
            Some(self.home_since_ms)
        } else if !approximate && staying && self.stay_since_ms > 0 {
            Some(self.stay_since_ms)
        } else {
            None
        };
        let next = MapSelfLocation {
            latitude: location.latitude,
            longitude: location.longitude,
            accuracy_meters: location.horizontal_accuracy_meters,
            speed_meters_per_second: if approximate || staying { 0.0 } else { location.speed_meters_per_second.max(0.0) },
            course_degrees: if approximate || staying { -1.0 } else { location.course_degrees },
            time_ms,
            is_at_home,
            place_kind: if !approximate && staying { self.place_kind } else { None },
            stay_since_ms,
            unavailable_since_ms: None,
        };
        feed.update(cx, |feed, cx| feed.publish(next, cx));
        let snapshot = feed.read(cx).location().cloned();
        if let (Some(snapshot), Some(account_id)) = (snapshot, self.account_id()) {
            self.enqueue_journal(account_id, false, Self::save_location_operation(snapshot), cx);
        }
    }

    fn update_home_stay(&mut self, location: &LocationFix, inside_home: bool) {
        let Some(home) = self.configuration.as_ref().and_then(|configuration| configuration.home.clone()) else {
            self.home_since_ms = 0;
            self.restored_home_location = None;
            return;
        };
        let time_ms = location.time_ms;
        if inside_home {
            if let Some(cached) = &self.restored_home_location {
                let resumable = cached.is_at_home
                    && cached.unavailable_since_ms.is_none()
                    && cached.stay_since_ms.is_some_and(|since| since > 0 && since <= cached.time_ms)
                    && time_ms >= cached.time_ms
                    && time_ms - cached.time_ms <= MapRecorderPolicy::segment_gap_ms()
                    && MapTrackGeometry::haversine_meters(cached.latitude, cached.longitude, home.latitude, home.longitude)
                        <= home.radius_meters;
                if let (true, Some(since)) = (resumable, cached.stay_since_ms) {
                    self.home_since_ms = if self.home_since_ms > 0 { self.home_since_ms.min(since) } else { since };
                }
            }
            self.restored_home_location = None;
            if self.home_since_ms == 0 {
                self.home_since_ms = time_ms;
            }
        } else {
            let distance =
                MapTrackGeometry::haversine_meters(location.latitude, location.longitude, home.latitude, home.longitude);
            if distance > home.radius_meters + location.horizontal_accuracy_meters {
                self.home_since_ms = 0;
                self.restored_home_location = None;
                self.allows_home_restore = false;
            }
        }
    }

    fn probe_place(&mut self, fix: MapRecorderFix, cx: &mut Context<Self>) {
        self.place_probe = None;
        self.place_kind = None;
        let Some(configuration) = &self.configuration else {
            return;
        };
        if !(configuration.sharing_enabled || self.is_map_visible()) || self.is_inside_home {
            return;
        }
        let probe = MapPlaceProbe::shared();
        let lookup = RiseAccountRuntime::current().spawn(async move { probe.kind(fix.latitude, fix.longitude).await });
        let lookup_abort = MapPlaceProbeAbortOnDrop(lookup.abort_handle());
        self.place_probe = Some(cx.spawn(async move |this, cx| {
            let _lookup_abort = lookup_abort;
            let Ok(Some(kind)) = lookup.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.regime != Regime::Staying {
                    return;
                }
                this.place_kind = Some(kind);
                this.publish(fix, false, cx);
                if let Some(last_presented) = this.last_presented {
                    this.present(last_presented, true, cx);
                }
            });
        }));
    }

    pub fn is_configured(&self, account_id: &str) -> bool {
        self.configuration
            .as_ref()
            .is_some_and(|configuration| configuration.account_id == account_id && configuration.wants_location())
    }

    pub fn home(&self, account_id: &str) -> Option<MapRecorderConfigurationHome> {
        self.configuration
            .as_ref()
            .filter(|configuration| configuration.account_id == account_id)
            .and_then(|configuration| configuration.home.clone())
    }

    pub fn resume_after_launch(&mut self, cx: &mut Context<Self>) {
        let recorder = cx.entity();
        AccountStateResetRegistry::register(&recorder, cx);
        let Some(data) = UserDefaults::standard().data(MAP_RECORDER_CONFIGURATION) else {
            return;
        };
        let Ok(saved) = serde_json::from_slice::<MapRecorderConfiguration>(&data) else {
            return;
        };
        self.apply(Some(saved), cx);
    }

    pub fn place_track_zones(&self, account_id: &str) -> Option<Vec<MapRecorderConfigurationHome>> {
        let configuration = self.configuration.as_ref()?;
        if configuration.account_id != account_id {
            return None;
        }
        configuration.place_track_zones.clone()
    }

    pub fn apply(&mut self, next: Option<MapRecorderConfiguration>, cx: &mut Context<Self>) {
        if next == self.configuration {
            if next.as_ref().is_some_and(MapRecorderConfiguration::wants_location) {
                self.start(cx);
            }
            return;
        }
        let previous_account = self.account_id();
        let next_account = next.as_ref().map(|configuration| configuration.account_id.clone());
        let current = self.configuration.as_ref();
        let home_recording_changed = current.map(|configuration| &configuration.home)
            != next.as_ref().map(|configuration| &configuration.home)
            || current.map(MapRecorderConfiguration::home_track_radius_meters)
                != next.as_ref().map(MapRecorderConfiguration::home_track_radius_meters)
            || current.map(|configuration| &configuration.place_track_zones)
                != next.as_ref().map(|configuration| &configuration.place_track_zones);
        let home_changed = current.map(|configuration| &configuration.home)
            != next.as_ref().map(|configuration| &configuration.home);
        if previous_account != next_account || home_changed {
            self.home_since_ms = 0;
            self.restored_home_location = None;
            self.allows_home_restore = previous_account.is_none() && next.is_some();
        }
        if previous_account != next_account {
            self.location_filter = MapLocationFilter::new();
            self.latest_fix = None;
        }
        self.configuration_generation = self.configuration_generation.wrapping_add(1);
        self.freshness_task = None;
        self.fresh_fix_task = None;
        self.fresh_fix_requested_at_ms = None;
        self.configuration = next.clone();
        if let Some(previous) = &previous_account
            && Some(previous) == next_account.as_ref()
            && home_recording_changed
        {
            self.is_inside_home_track_zone = false;
            self.enqueue_journal(previous.clone(), true, Self::seal_operation(false), cx);
        }

        let Some(next) = next else {
            UserDefaults::standard().remove_object(MAP_RECORDER_CONFIGURATION);
            self.stop(previous_account, cx);
            return;
        };
        match serde_json::to_vec(&next) {
            Ok(data) => UserDefaults::standard().set(MAP_RECORDER_CONFIGURATION, data),
            Err(error) => tracing::debug!(target: "riseonly", %error, "map recorder configuration not saved"),
        }
        if let Some(previous) = &previous_account
            && *previous != next.account_id
        {
            self.stop(Some(previous.clone()), cx);
        }
        if previous_account.as_deref() != Some(next.account_id.as_str()) {
            self.last_handled_ms = 0;
            self.last_presented = None;
            MapSelfLocationFeed::shared(cx).update(cx, |feed, cx| feed.reset(cx));
        }
        if next.wants_location() {
            self.start(cx);
        } else {
            self.stop(Some(next.account_id.clone()), cx);
            self.map_visibility_changed(cx);
        }
    }

    pub fn erase_journal(&mut self, account_id: &str, cx: &mut Context<Self>) {
        let operation: JournalOperation = Box::new(|journal: Arc<MapRecorderJournal>| {
            async move {
                journal.erase_all();
                false
            }
            .boxed()
        });
        self.enqueue_journal(account_id.to_owned(), true, operation, cx);
    }

    fn start(&mut self, _cx: &mut Context<Self>) {}

    pub fn request_fresh_position(&mut self, cx: &mut Context<Self>) {
        if self.fresh_fix_task.is_some() || self.regime == Regime::Off || !self.sharing_enabled() || !self.is_authorized()
        {
            return;
        }
        let generation = self.configuration_generation;
        self.fresh_fix_requested_at_ms = Some(now_ms());
        self.sync_standard_updates();
        let wait = RiseLimits::service_value("map_watched_update_interval_ms") * 2;
        self.fresh_fix_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(millis(wait)).await;
            let _ = this.update(cx, |this, _| {
                if this.configuration_generation != generation {
                    return;
                }
                this.fresh_fix_task = None;
                this.fresh_fix_requested_at_ms = None;
                this.sync_standard_updates();
            });
        }));
    }

    fn complete_fresh_fix_request(&mut self, received_at_ms: i64) {
        let Some(requested_at) = self.fresh_fix_requested_at_ms else {
            return;
        };
        if received_at_ms < requested_at {
            return;
        }
        self.fresh_fix_task = None;
        self.fresh_fix_requested_at_ms = None;
        self.sync_standard_updates();
    }

    fn start_freshness_updates(&mut self, cx: &mut Context<Self>) {
        if self.freshness_task.is_some() || !self.sharing_enabled() {
            return;
        }
        let generation = self.configuration_generation;
        let interval = RiseMapLiveRepository::unwatched_update_ms();
        self.freshness_task = Some(cx.spawn(async move |this, cx| {
            let mut next_attempt_at_ms = now_ms() + interval;
            loop {
                let wait = this.update(cx, |this, _| {
                    if this.configuration_generation != generation || this.regime == Regime::Off || !this.sharing_enabled()
                    {
                        return None;
                    }
                    let due_at_ms = next_attempt_at_ms.max(this.last_handled_ms + interval);
                    Some((due_at_ms - now_ms()).max(0))
                });
                let Ok(Some(wait_ms)) = wait else {
                    return;
                };
                cx.background_executor().timer(millis(wait_ms)).await;
                let attempted = this.update(cx, |this, cx| {
                    if this.configuration_generation != generation {
                        return None;
                    }
                    let now = now_ms();
                    if now - this.last_handled_ms < interval {
                        return Some(None);
                    }
                    this.request_fresh_position(cx);
                    Some(Some(now + interval))
                });
                match attempted {
                    Ok(Some(Some(next))) => next_attempt_at_ms = next,
                    Ok(Some(None)) => {}
                    _ => return,
                }
            }
        }));
    }

    fn stop(&mut self, sealing_account_id: Option<String>, cx: &mut Context<Self>) {
        self.freshness_task = None;
        self.fresh_fix_task = None;
        self.fresh_fix_requested_at_ms = None;
        self.live_demand_until_ms = 0;
        self.live_demand_expiry = None;
        self.stay_timer = None;
        self.place_probe = None;
        self.place_kind = None;
        self.regime = Regime::Off;
        self.latest_fix = None;
        self.anchor = None;
        self.stay_since_ms = 0;
        self.home_since_ms = 0;
        self.restored_home_location = None;
        self.allows_home_restore = false;
        self.is_inside_home = false;
        self.is_inside_home_track_zone = false;
        if let Some(manager) = self.manager.as_mut() {
            manager.stop_updating_location();
            RiseMapRecorderLocationManager::perform(LocationCommand::StopSignificantChanges);
            RiseMapRecorderLocationManager::perform(LocationCommand::StopVisits);
            self.clear_exit_region();
            self.sync_standard_updates();
        }
        let Some(account_id) = sealing_account_id else {
            return;
        };
        self.seal(account_id, false, cx);
    }

    fn enter_moving(&mut self, cx: &mut Context<Self>) {
        if self.manager.is_none() {
            return;
        }
        self.regime = Regime::Moving;
        self.stay_since_ms = 0;
        self.place_probe = None;
        self.place_kind = None;
        self.sync_standard_updates();
        self.arm_stay_timer(cx);
    }

    fn enter_staying(&mut self, fix: MapRecorderFix, cx: &mut Context<Self>) {
        if self.manager.is_none() || self.regime != Regime::Moving {
            return;
        }
        self.regime = Regime::Staying;
        self.motion = MapMotion::Still;
        self.location_filter.stop();
        self.stay_since_ms = self.anchor.map_or(fix.time_ms, |anchor| anchor.time_ms);
        self.stay_timer = None;
        self.sync_standard_updates();

        self.arm_recovery_region(&fix, true);

        if let Some(account_id) = self.account_id() {
            self.seal(account_id, true, cx);
        }
        self.publish(fix, false, cx);
        if let Some(last_presented) = self.last_presented {
            self.present(last_presented, true, cx);
        }
        let snapshot = MapSelfLocationFeed::shared(cx).read(cx).location().cloned();
        if let (Some(account_id), Some(snapshot)) = (self.account_id(), snapshot) {
            self.enqueue_journal(account_id, true, Self::save_location_operation(snapshot), cx);
        }
        self.probe_place(fix, cx);
    }

    /// under a distance filter a stationary person gives no fixes so silence is how a stop looks
    /// movement re-arms the timer and it fires when there was none
    fn arm_stay_timer(&mut self, cx: &mut Context<Self>) {
        let stay_after_ms = MapRecorderPolicy::stay_after_ms();
        self.stay_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(millis(stay_after_ms)).await;
            let _ = this.update(cx, |this, cx| {
                if this.regime != Regime::Moving {
                    return;
                }
                let Some(anchor) = this.anchor else {
                    this.arm_stay_timer(cx);
                    return;
                };
                let fix = this.latest_fix.unwrap_or(anchor);
                this.enter_staying(fix, cx);
            });
        }));
    }

    fn handle(&mut self, location: LocationFix, cx: &mut Context<Self>) {
        let Some(configuration) = self.configuration.clone() else {
            return;
        };
        if self.regime == Regime::Off {
            return;
        }
        let now = now_ms();
        let fix = MapRecorderFix {
            latitude: location.latitude,
            longitude: location.longitude,
            accuracy_meters: location.horizontal_accuracy_meters,
            speed_meters_per_second: location.speed_meters_per_second.max(0.0),
            heading_degrees: location.course_degrees,
            time_ms: location.time_ms,
        };
        if !location.is_usable()
            || fix.time_ms <= self.last_handled_ms
            || fix.time_ms > now + *MAXIMUM_FUTURE_FIX_MS
            || fix.accuracy_meters < 0.0
            || fix.accuracy_meters > MapRecorderPolicy::minimum_accuracy_meters()
        {
            return;
        }
        self.last_handled_ms = fix.time_ms;
        self.complete_fresh_fix_request(fix.time_ms);

        self.is_inside_home = configuration.home.as_ref().is_some_and(|home| {
            MapRecorderPolicy::is_inside(&fix, home.latitude, home.longitude, home.radius_meters)
        });
        let exit_margin = if self.is_inside_home_track_zone { MapRecorderPolicy::stay_radius_meters() } else { 0.0 };
        let excludes_track = configuration.home.as_ref().is_some_and(|home| {
            MapRecorderPolicy::may_be_inside(
                &fix,
                home.latitude,
                home.longitude,
                configuration.home_track_radius_meters() + exit_margin,
            )
        });
        let excludes_place = configuration.place_track_zones.as_deref().unwrap_or_default().iter().any(|zone| {
            MapRecorderPolicy::may_be_inside(&fix, zone.latitude, zone.longitude, zone.radius_meters + exit_margin)
        });
        let excludes_any_track = excludes_track || excludes_place;
        let entered_home_track_zone = excludes_any_track && !self.is_inside_home_track_zone;
        self.is_inside_home_track_zone = excludes_any_track;
        let offset = utc_offset_minutes(fix.time_ms);
        let left_stay = self
            .anchor
            .is_none_or(|anchor| MapRecorderPolicy::distance_meters(&fix, &anchor) > MapRecorderPolicy::stay_radius_meters());
        let records_movement = self.regime != Regime::Staying
            || left_stay
            || fix.speed_meters_per_second >= MapSpeedFormat::MOVING_METERS_PER_SECOND;
        let segment_gap_ms = if configuration.records_precisely() {
            MapRecorderPolicy::segment_gap_ms()
        } else {
            MapRecorderPolicy::economy_segment_gap_ms()
        };
        let account_id = configuration.account_id.clone();
        let operation: JournalOperation = Box::new(move |journal: Arc<MapRecorderJournal>| {
            async move {
                if !configuration.tracks_enabled {
                    return false;
                }
                if excludes_any_track {
                    return entered_home_track_zone && journal.seal_open_segment(false);
                }
                let mut zones = configuration.place_track_zones.clone().unwrap_or_default();
                if let Some(home) = &configuration.home {
                    zones.push(MapRecorderConfigurationHome {
                        latitude: home.latitude,
                        longitude: home.longitude,
                        radius_meters: configuration.home_track_radius_meters(),
                    });
                }
                journal.append(&fix, offset, records_movement, &zones, segment_gap_ms)
            }
            .boxed()
        });
        self.enqueue_journal(account_id, true, operation, cx);
        if now - fix.time_ms > *STALE_FIX_MS {
            return;
        }
        self.latest_fix = Some(fix);
        self.arm_recovery_region(&fix, false);

        if self.regime == Regime::Staying {
            let departed = self.anchor.is_some_and(|anchor| {
                let distance = MapRecorderPolicy::distance_meters(&fix, &anchor);
                distance > MapRecorderPolicy::stay_radius_meters()
                    || (fix.speed_meters_per_second >= MapSpeedFormat::MOVING_METERS_PER_SECOND
                        && distance >= MapRecorderPolicy::minimum_step_meters())
            });
            if !departed {
                self.publish(fix, false, cx);
                return;
            }
            self.enter_moving(cx);
            self.anchor = Some(fix);
            self.present(location, false, cx);
        }

        let anchor = self.anchor;
        match anchor {
            Some(anchor) if MapRecorderPolicy::distance_meters(&fix, &anchor) <= MapRecorderPolicy::stay_radius_meters() => {
                if fix.time_ms - anchor.time_ms >= MapRecorderPolicy::stay_after_ms() {
                    self.enter_staying(fix, cx);
                    return;
                }
            }
            _ => {
                self.anchor = Some(fix);
                self.arm_stay_timer(cx);
            }
        }

        let next_motion = MapRecorderPolicy::motion(fix.speed_meters_per_second);
        if next_motion != self.motion {
            self.motion = next_motion;
            let filter = self.moving_filter();
            if let Some(manager) = self.manager.as_mut() {
                manager.set_distance_filter(filter);
            }
        }

        self.publish(fix, false, cx);
    }

    fn handle_visit(&mut self, visit: LocationVisit, cx: &mut Context<Self>) {
        let Some(configuration) = &self.configuration else {
            return;
        };
        if !configuration.tracks_enabled {
            return;
        }
        let (Some(arrived_at), Some(departed_at)) = (visit.arrival_ms, visit.departure_ms) else {
            return;
        };
        let stay = MapRecordedStay {
            arrived_at,
            departed_at,
            latitude: visit.latitude,
            longitude: visit.longitude,
            utc_offset_minutes: utc_offset_minutes(arrived_at),
        };
        let account_id = configuration.account_id.clone();
        let operation: JournalOperation = Box::new(move |journal: Arc<MapRecorderJournal>| {
            async move {
                journal.add_stay(stay);
                true
            }
            .boxed()
        });
        self.enqueue_journal(account_id, true, operation, cx);
    }

    fn seal_operation(continuing: bool) -> JournalOperation {
        Box::new(move |journal: Arc<MapRecorderJournal>| async move { journal.seal_open_segment(continuing) }.boxed())
    }

    fn seal(&mut self, account_id: String, continuing: bool, cx: &mut Context<Self>) {
        self.enqueue_journal(account_id, true, Self::seal_operation(continuing), cx);
    }

    /// journal steps run one after another in the order the recorder asked for them
    fn enqueue_journal(
        &mut self,
        account_id: String,
        notifies_movement: bool,
        operation: JournalOperation,
        cx: &mut Context<Self>,
    ) {
        let previous = self.journal_tail.take();
        let generation = self.configuration_generation;
        let lease = begin_process_activity(MAP_RECORDER_ACTIVITY);
        self.journal_tail = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let journal = MapRecorderJournal::for_account(&account_id);
            let sealed = cx.background_executor().spawn(operation(journal)).await;
            drop(lease);
            let _ = this.update(cx, |this, cx| {
                if generation != this.configuration_generation
                    || this.account_id().as_deref() != Some(account_id.as_str())
                {
                    return;
                }
                let callback = if sealed {
                    this.on_segment_sealed.clone()
                } else if notifies_movement {
                    this.on_moved.clone()
                } else {
                    None
                };
                if let Some(callback) = callback {
                    cx.defer(move |cx| callback(cx));
                }
            });
        }));
    }

    fn publish(&mut self, fix: MapRecorderFix, approximate: bool, cx: &mut Context<Self>) {
        let Some(configuration) = &self.configuration else {
            return;
        };
        if !configuration.sharing_enabled {
            return;
        }
        let Some(on_fix) = self.on_fix.clone() else {
            return;
        };
        let battery_percent = if self.is_battery_monitoring_enabled { self.battery.percent } else { None };
        let has_battery = configuration.battery_visible && battery_percent.is_some();
        let staying = self.regime == Regime::Staying;
        let next = MapFix {
            latitude: fix.latitude,
            longitude: fix.longitude,
            accuracy_meters: fix.accuracy_meters,
            speed_meters_per_second: if approximate {
                -1.0
            } else if staying {
                0.0
            } else {
                fix.speed_meters_per_second
            },
            heading_degrees: if approximate { -1.0 } else { fix.heading_degrees },
            battery_level: match (has_battery, battery_percent) {
                (true, Some(percent)) => i64::from(percent),
                _ => -1,
            },
            is_charging: has_battery && matches!(self.battery.state, BatteryState::Charging | BatteryState::Full),
            motion: if approximate {
                MapMotion::Unknown
            } else if staying {
                MapMotion::Still
            } else {
                self.motion
            },
            recorded_at: fix.time_ms,
            stay_since: if approximate {
                0
            } else if self.home_since_ms > 0 {
                self.home_since_ms
            } else {
                self.stay_since_ms
            },
            place_kind: if !approximate && staying { self.place_kind } else { None },
        };
        cx.defer(move |cx| on_fix(next, cx));
    }

    fn set_battery_monitoring_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if enabled == self.is_battery_monitoring_enabled {
            return;
        }
        self.is_battery_monitoring_enabled = enabled;
        if enabled {
            self.refresh_battery(cx);
        } else {
            self.battery_refresh = None;
            self.battery = BatteryReading::NO_BATTERY;
        }
    }

    fn refresh_battery(&mut self, cx: &mut Context<Self>) {
        if !self.is_battery_monitoring_enabled {
            return;
        }
        let reading = cx.background_executor().spawn(async move { power::power_snapshot().reading.battery });
        self.battery_refresh = Some(cx.spawn(async move |this, cx| {
            let battery = reading.await;
            let _ = this.update(cx, |this, _| {
                if this.is_battery_monitoring_enabled {
                    this.battery = battery;
                }
                this.battery_refresh = None;
            });
        }));
    }

    /// keeps one exit region armed while moving as well as stopped, where the host monitors regions
    fn arm_recovery_region(&mut self, fix: &MapRecorderFix, recenter: bool) {
        if self.manager.is_none()
            || !self.wants_location()
            || location::supports(LocationService::RegionMonitoring).is_err()
            || now_ms() - fix.time_ms > *STALE_FIX_MS
        {
            return;
        }
        if let (false, Some(region)) = (recenter, &self.recovery_region)
            && MapRecorderPolicy::is_inside(fix, region.latitude, region.longitude, region.radius_meters)
        {
            return;
        }
        let region = CircularRegion {
            identifier: EXIT_REGION_ID.to_owned(),
            latitude: fix.latitude,
            longitude: fix.longitude,
            radius_meters: MapRecorderPolicy::exit_region_meters(),
        };
        self.recovery_region = Some(region.clone());
        RiseMapRecorderLocationManager::perform(LocationCommand::MonitorRegion(region));
    }

    fn recover_after_region_exit(&mut self, identifier: &str, cx: &mut Context<Self>) {
        if identifier != EXIT_REGION_ID || !self.wants_location() || !self.is_authorized() {
            return;
        }
        tracing::info!(target: "riseonly", "location recovery region exited; resuming standard updates");
        self.anchor = None;
        self.restored_home_location = None;
        self.allows_home_restore = false;
        match self.regime {
            Regime::Off => self.start(cx),
            Regime::Staying => self.enter_moving(cx),
            Regime::Moving => self.sync_standard_updates(),
        }
    }

    fn clear_exit_region(&mut self) {
        self.recovery_region = None;
        if location::supports(LocationService::RegionMonitoring).is_ok() {
            RiseMapRecorderLocationManager::perform(LocationCommand::StopRegion(EXIT_REGION_ID.to_owned()));
        }
    }

    fn resolved_manager(&mut self) {}

    fn location_event(&mut self, event: LocationEvent, cx: &mut Context<Self>) {
        match event {
            LocationEvent::Fixes(fixes) => {
                for fix in fixes {
                    self.receive(fix, cx);
                }
            }
            LocationEvent::Heading(heading) => self.did_update_heading(heading, cx),
            LocationEvent::Visit(visit) => self.handle_visit(visit, cx),
            LocationEvent::ExitedRegion(identifier) => self.recover_after_region_exit(&identifier, cx),
            LocationEvent::RegionState { identifier, state } => {
                if state == RegionState::Outside {
                    self.recover_after_region_exit(&identifier, cx);
                }
            }
            LocationEvent::Authorization(_) => self.did_change_authorization(cx),
            LocationEvent::Failed(LocationFailure::RegionMonitoring) => self.monitoring_did_fail(Some(EXIT_REGION_ID)),
            LocationEvent::Failed(failure) => self.did_fail(failure, cx),
        }
    }

    fn did_update_heading(&mut self, heading: Option<LocationHeading>, cx: &mut Context<Self>) {
        if !self.is_map_visible() {
            return;
        }
        let next = heading.map(|heading| MapSelfHeading {
            degrees: heading.degrees,
            accuracy_degrees: heading.accuracy_degrees,
        });
        MapSelfLocationFeed::shared(cx).update(cx, |feed, cx| feed.publish_heading(next, cx));
    }

    fn did_change_authorization(&mut self, cx: &mut Context<Self>) {
        self.location_filter = MapLocationFilter::new();
        if self.is_reduced_accuracy() {
            self.home_since_ms = 0;
            self.stay_since_ms = 0;
            self.restored_home_location = None;
            if let Some(account_id) = self.account_id() {
                self.seal(account_id, false, cx);
            }
        }
        if self.manager.is_some() {
            let authorized = self.is_authorized();
            self.update_location_availability(authorized, cx);
        }
        if self.is_map_visible() {
            self.map_visibility_changed(cx);
        }
        let Some(configuration) = &self.configuration else {
            return;
        };
        if !configuration.wants_location() {
            return;
        }
        let account_id = configuration.account_id.clone();
        if self.is_authorized() {
            self.start(cx);
        } else {
            self.stop(Some(account_id), cx);
        }
    }

    /// revoked access also arrives as an authorization change and that path stops recording
    /// an error alone never does since stopping here would disarm the services that bring recording back
    fn did_fail(&mut self, failure: LocationFailure, cx: &mut Context<Self>) {
        if !self.wants_location() || self.manager.is_none() {
            return;
        }
        tracing::warn!(target: "riseonly", ?failure, "location provider interrupted; monitoring remains armed");
        let authorized = self.is_authorized();
        self.update_location_availability(authorized, cx);
        self.sync_standard_updates();
    }

    fn monitoring_did_fail(&mut self, identifier: Option<&str>) {
        if !self.wants_location() {
            return;
        }
        if identifier == Some(EXIT_REGION_ID) {
            self.recovery_region = None;
        }
        tracing::warn!(target: "riseonly", "location region unavailable; standard updates remain active");
        self.sync_standard_updates();
    }
}

impl AccountResettable for RiseMapRecorder {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.apply(None, cx);
        MapSelfLocationFeed::shared(cx).update(cx, |feed, cx| feed.reset(cx));
        self.last_presented = None;
    }
}

struct MapPlaceProbeAbortOnDrop(tokio::task::AbortHandle);

impl Drop for MapPlaceProbeAbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
