use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll, Waker};

use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::{adding_percent_encoding, UrlAllowedCharacters};
use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LocationAuthorization {
    NotDetermined,
    Restricted,
    Denied,
    WhenInUse,
    Always,
}

impl LocationAuthorization {
    /// level from the raw status, 0 undetermined 1 restricted 2 denied 3 always 4 when in use
    /// mac has no when in use, unknown value counts as denied
    pub const fn from_cl_status(raw: i64) -> Self {
        match raw {
            0 => Self::NotDetermined,
            1 => Self::Restricted,
            3 => Self::Always,
            4 => Self::WhenInUse,
            _ => Self::Denied,
        }
    }

    /// true when the level allows foreground location
    pub const fn allows_foreground(self) -> bool {
        matches!(self, Self::WhenInUse | Self::Always)
    }
}

/// authorization level plus whether accuracy is full
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LocationPermission {
    pub level: LocationAuthorization,
    pub is_precise: bool,
}

impl LocationPermission {
    /// builds it from raw status and accuracy, 0 full and 1 reduced
    pub const fn from_cl(status: i64, accuracy_authorization: i64) -> Self {
        Self { level: LocationAuthorization::from_cl_status(status), is_precise: accuracy_authorization == 0 }
    }
}

/// string key of the map onboarding button for this level
pub const fn access_action_key(level: LocationAuthorization, did_ask_for_always: bool) -> &'static str {
    match level {
        LocationAuthorization::NotDetermined => "map_onboarding_allow_location",
        LocationAuthorization::WhenInUse if !did_ask_for_always => "map_onboarding_allow_always",
        LocationAuthorization::WhenInUse
        | LocationAuthorization::Always
        | LocationAuthorization::Denied
        | LocationAuthorization::Restricted => "map_onboarding_open_settings",
    }
}

/// which prompt to show
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AuthorizationScope {
    WhenInUse,
    Always,
}

/// when in use prompts only while undecided, always only from when in use
/// any other level shows nothing and the caller just rereads the state
pub const fn should_prompt(scope: AuthorizationScope, level: LocationAuthorization) -> bool {
    matches!(
        (scope, level),
        (AuthorizationScope::WhenInUse, LocationAuthorization::NotDetermined)
            | (AuthorizationScope::Always, LocationAuthorization::WhenInUse)
    )
}

/// accuracy choices the map uses
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DesiredAccuracy {
    BestForNavigation,
    Best,
    NearestTenMeters,
    HundredMeters,
    Kilometer,
    ThreeKilometers,
}

/// desired accuracy and distance filter of the standard service, none filter reports every movement
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct UpdateRequest {
    pub accuracy: DesiredAccuracy,
    pub distance_filter_meters: Option<f64>,
}

impl UpdateRequest {
    /// the filter the os gets, negative or non finite means none
    pub fn distance_filter(self) -> Option<f64> {
        self.distance_filter_meters.filter(|meters| meters.is_finite() && *meters > 0.0)
    }
}

/// one location fix, negative speed course or accuracies mean unknown
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LocationFix {
    pub latitude: f64,
    pub longitude: f64,
    pub horizontal_accuracy_meters: f64,
    pub altitude_meters: f64,
    pub vertical_accuracy_meters: f64,
    pub speed_meters_per_second: f64,
    pub speed_accuracy_meters_per_second: f64,
    pub course_degrees: f64,
    pub course_accuracy_degrees: f64,
    pub time_ms: i64,
}

impl LocationFix {
    /// true when the coordinate is valid and horizontal accuracy is not negative
    pub fn is_usable(&self) -> bool {
        self.latitude.is_finite()
            && self.longitude.is_finite()
            && (-90.0..=90.0).contains(&self.latitude)
            && (-180.0..=180.0).contains(&self.longitude)
            && self.horizontal_accuracy_meters >= 0.0
    }

    pub fn has_course(&self) -> bool {
        self.course_degrees >= 0.0
    }

    pub fn has_altitude(&self) -> bool {
        self.vertical_accuracy_meters >= 0.0
    }
}

/// where the device is pointing
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LocationHeading {
    pub degrees: f64,
    pub accuracy_degrees: f64,
}

impl LocationHeading {
    /// heading from the update, true north if known else magnetic
    /// none while accuracy is negative (uncalibrated)
    pub fn from_cl(true_heading: f64, magnetic_heading: f64, accuracy: f64) -> Option<Self> {
        if accuracy.is_nan() || accuracy < 0.0 {
            return None;
        }
        let degrees = if true_heading >= 0.0 { true_heading } else { magnetic_heading };
        Some(Self { degrees, accuracy_degrees: accuracy })
    }
}

/// a visit, arrival is unknown if only departure was seen
/// departure is unknown while the user still stays
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LocationVisit {
    pub latitude: f64,
    pub longitude: f64,
    pub horizontal_accuracy_meters: f64,
    pub arrival_ms: Option<i64>,
    pub departure_ms: Option<i64>,
}

const DATE_SENTINEL_SECONDS: f64 = 1.0e10;

/// seconds since 1970 to ms, none for the distant past or future sentinels
pub fn visit_date_ms(seconds_since_1970: f64) -> Option<i64> {
    if !seconds_since_1970.is_finite() || seconds_since_1970.abs() >= DATE_SENTINEL_SECONDS {
        return None;
    }
    Some((seconds_since_1970 * 1_000.0).round() as i64)
}

/// fix timestamp to ms, sentinel dates never come on a fix so they clamp to 0
pub fn fix_time_ms(seconds_since_1970: f64) -> i64 {
    visit_date_ms(seconds_since_1970).unwrap_or(0).max(0)
}

/// accuracy for a speed or course whose source reports none
/// 0 for a known value and -1 for unknown, so known values stay usable
pub fn unreported_accuracy(value: f64) -> f64 {
    if value.is_finite() && value >= 0.0 {
        0.0
    } else {
        -1.0
    }
}

/// circular region, the recorder exit region
#[derive(Clone, PartialEq, Debug)]
pub struct CircularRegion {
    pub identifier: String,
    pub latitude: f64,
    pub longitude: f64,
    pub radius_meters: f64,
}

impl CircularRegion {
    /// radius the os gets, min of wanted and the max monitoring distance
    /// none when the host monitors no regions
    pub fn clamped_radius(&self, maximum_monitoring_distance: f64) -> Option<f64> {
        if maximum_monitoring_distance.is_nan()
            || maximum_monitoring_distance <= 0.0
            || self.radius_meters.is_nan()
            || self.radius_meters <= 0.0
        {
            return None;
        }
        Some(self.radius_meters.min(maximum_monitoring_distance))
    }
}

/// region state, unknown 0 inside 1 outside 2
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RegionState {
    Unknown,
    Inside,
    Outside,
}

impl RegionState {
    pub const fn from_cl(raw: i64) -> Self {
        match raw {
            1 => Self::Inside,
            2 => Self::Outside,
            _ => Self::Unknown,
        }
    }
}

/// location errors the map tells apart
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LocationFailure {
    LocationUnknown,
    Denied,
    Network,
    RegionMonitoring,
    Other(i64),
}

impl LocationFailure {
    pub const fn from_cl_code(code: i64) -> Self {
        match code {
            0 => Self::LocationUnknown,
            1 => Self::Denied,
            2 => Self::Network,
            4 | 5 => Self::RegionMonitoring,
            other => Self::Other(other),
        }
    }

    /// true when the standard service keeps going after it, only a one shot request ends on it
    pub const fn is_transient(self) -> bool {
        matches!(self, Self::LocationUnknown | Self::Network)
    }
}

/// what the os reports, all delegate callbacks in one type
#[derive(Clone, PartialEq, Debug)]
pub enum LocationEvent {
    Authorization(LocationPermission),
    Fixes(Vec<LocationFix>),
    Heading(Option<LocationHeading>),
    Visit(LocationVisit),
    ExitedRegion(String),
    RegionState { identifier: String, state: RegionState },
    Failed(LocationFailure),
}

/// services beyond plain standard updates
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LocationService {
    SignificantChanges,
    Visits,
    RegionMonitoring,
    Heading,
    BackgroundRelaunch,
}

impl LocationService {
    pub const ALL: [Self; 5] =
        [Self::SignificantChanges, Self::Visits, Self::RegionMonitoring, Self::Heading, Self::BackgroundRelaunch];
}

/// why a host doesnt run a service
#[derive(Clone, Copy, PartialEq, Eq, Debug, Error)]
pub enum LocationUnavailable {
    #[error("the {0:?} layer does not read location yet")]
    BranchNotWritten(HostOs),
    #[error("the system framework class {0} is not available")]
    FrameworkMissing(&'static str),
    #[error("location calls must run on the main thread")]
    NotMainThread,
    #[error("{0:?} has no {1:?} service")]
    NoSuchService(HostOs, LocationService),
    #[error("this machine reports no {0:?} hardware or service")]
    ServiceOff(LocationService),
    #[error("no region with this identifier is monitored")]
    UnknownRegion,
}

/// static answer per host, the os file still says if this machine runs it
/// mac has no compass so heading is never started
/// linux gives fixes only
pub const fn host_offers(host: HostOs, service: LocationService) -> Result<(), LocationUnavailable> {
    let offered = match (host, service) {
        (_, LocationService::BackgroundRelaunch) => false,
        (HostOs::MacOs, LocationService::SignificantChanges)
        | (HostOs::MacOs, LocationService::Visits)
        | (HostOs::MacOs, LocationService::RegionMonitoring) => true,
        (HostOs::MacOs, LocationService::Heading) => false,
        (HostOs::Linux | HostOs::Windows, _) => false,
    };
    if offered { Ok(()) } else { Err(LocationUnavailable::NoSuchService(host, service)) }
}

/// what the os file is told to do
#[derive(Clone, PartialEq, Debug)]
pub enum LocationCommand {
    RequestAuthorization(AuthorizationScope),
    StartUpdates(UpdateRequest),
    StopUpdates,
    RequestFix,
    StartSignificantChanges,
    StopSignificantChanges,
    StartVisits,
    StopVisits,
    MonitorRegion(CircularRegion),
    StopRegion(String),
    RequestRegionState(String),
}

impl LocationCommand {
    /// the service a command needs beyond standard updates
    pub const fn service(&self) -> Option<LocationService> {
        match self {
            Self::StartSignificantChanges | Self::StopSignificantChanges => Some(LocationService::SignificantChanges),
            Self::StartVisits | Self::StopVisits => Some(LocationService::Visits),
            Self::MonitorRegion(_) | Self::StopRegion(_) | Self::RequestRegionState(_) => {
                Some(LocationService::RegionMonitoring)
            }
            Self::RequestAuthorization(_) | Self::StartUpdates(_) | Self::StopUpdates | Self::RequestFix => None,
        }
    }
}

/// how the recorder watches a user who stands still
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum StationaryWatch {
    SystemWakeups { significant_changes: bool, visits: bool, exit_region: bool },
    CoarseUpdates(UpdateRequest),
}

/// stationary plan for the services this host runs
pub fn stationary_watch(
    source: &dyn LocationSource,
    host: HostOs,
    exit_radius_meters: f64,
) -> StationaryWatch {
    let runs = |service| supports_on(source, host, service).is_ok();
    let significant_changes = runs(LocationService::SignificantChanges);
    let visits = runs(LocationService::Visits);
    let exit_region = runs(LocationService::RegionMonitoring);
    if significant_changes || exit_region {
        StationaryWatch::SystemWakeups { significant_changes, visits, exit_region }
    } else {
        StationaryWatch::CoarseUpdates(UpdateRequest {
            accuracy: DesiredAccuracy::HundredMeters,
            distance_filter_meters: Some(exit_radius_meters),
        })
    }
}

/// gets every event on the thread the os reports on (main thread on mac)
pub type LocationSink = Arc<dyn Fn(LocationEvent) + Send + Sync>;

/// the os side, every method defaults to branch not written
/// mac needs the main thread for all of them, the async driver is spawned once by the engine
pub type LocationDriver = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

pub trait LocationSource: Sync {
    fn permission(&self) -> Result<LocationPermission, LocationUnavailable> {
        Err(LocationUnavailable::BranchNotWritten(HostOs::current()))
    }

    /// last fix the os cached
    fn last_fix(&self) -> Result<Option<LocationFix>, LocationUnavailable> {
        Err(LocationUnavailable::BranchNotWritten(HostOs::current()))
    }

    fn service_available(&self, _service: LocationService) -> Result<(), LocationUnavailable> {
        Err(LocationUnavailable::BranchNotWritten(HostOs::current()))
    }

    fn install(&self, _sink: LocationSink) -> Result<(), LocationUnavailable> {
        Err(LocationUnavailable::BranchNotWritten(HostOs::current()))
    }

    fn perform(&self, _command: LocationCommand) -> Result<(), LocationUnavailable> {
        Err(LocationUnavailable::BranchNotWritten(HostOs::current()))
    }

    fn take_driver(&self) -> Option<LocationDriver> {
        None
    }
}

/// checks host policy first and then the machine
pub fn supports_on(source: &dyn LocationSource, host: HostOs, service: LocationService) -> Result<(), LocationUnavailable> {
    host_offers(host, service)?;
    source.service_available(service)
}

/// runs the command if its service exists, a missing service never reaches the os
pub fn perform_on(source: &dyn LocationSource, host: HostOs, command: LocationCommand) -> Result<(), LocationUnavailable> {
    if let Some(service) = command.service() {
        supports_on(source, host, service)?;
    }
    source.perform(command)
}

/// subscribers of one process, fed by the sink the os file got
#[derive(Default)]
pub struct LocationHub {
    subscribers: Mutex<HashMap<u64, LocationSink>>,
    next: Mutex<u64>,
}

impl LocationHub {
    pub fn dispatch(&self, event: LocationEvent) {
        let subscribers: Vec<LocationSink> = self.subscribers.lock().values().cloned().collect();
        for subscriber in subscribers {
            subscriber(event.clone());
        }
    }

    fn add(&self, sink: LocationSink) -> u64 {
        let id = {
            let mut next = self.next.lock();
            *next += 1;
            *next
        };
        self.subscribers.lock().insert(id, sink);
        id
    }

    fn remove(&self, id: u64) {
        self.subscribers.lock().remove(&id);
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers.lock().len()
    }
}

/// removes its subscriber on drop
pub struct LocationSubscription {
    hub: Arc<LocationHub>,
    id: u64,
}

impl Drop for LocationSubscription {
    fn drop(&mut self) {
        self.hub.remove(self.id);
    }
}

/// installs the hub as the source sink the first time, then adds the sink
pub fn subscribe_to(
    source: &dyn LocationSource,
    hub: &Arc<LocationHub>,
    installed: &OnceLock<Result<(), LocationUnavailable>>,
    sink: LocationSink,
) -> Result<LocationSubscription, LocationUnavailable> {
    let forward = Arc::clone(hub);
    if let Err(reason) = installed.get_or_init(|| source.install(Arc::new(move |event| forward.dispatch(event)))) {
        return Err(*reason);
    }
    let id = hub.add(sink);
    Ok(LocationSubscription { hub: Arc::clone(hub), id })
}

#[derive(Default)]
struct FixSlot {
    answer: Option<Option<LocationFix>>,
    waker: Option<Waker>,
}

/// last fix of the answer, none on failure or no permission
/// holds its subscription until answered
pub struct FixRequest {
    slot: Arc<Mutex<FixSlot>>,
    subscription: Option<LocationSubscription>,
}

impl Future for FixRequest {
    type Output = Option<LocationFix>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let answer = {
            let mut slot = self.slot.lock();
            match slot.answer.take() {
                Some(answer) => answer,
                None => {
                    slot.waker = Some(context.waker().clone());
                    return Poll::Pending;
                }
            }
        };
        self.subscription = None;
        Poll::Ready(answer)
    }
}

fn answer_fix(slot: &Mutex<FixSlot>, answer: Option<LocationFix>) {
    let waker = {
        let mut slot = slot.lock();
        if slot.answer.is_some() {
            return;
        }
        slot.answer = Some(answer);
        slot.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}

/// one shot fix against the source, the first fix or failure settles it
/// transient failures included
pub fn current_fix_from(
    source: &dyn LocationSource,
    hub: &Arc<LocationHub>,
    installed: &OnceLock<Result<(), LocationUnavailable>>,
) -> FixRequest {
    let slot = Arc::new(Mutex::new(FixSlot::default()));
    let allowed = source.permission().map(|permission| permission.level.allows_foreground());
    if !matches!(allowed, Ok(true)) {
        answer_fix(&slot, None);
        return FixRequest { slot, subscription: None };
    }
    let answered = Arc::clone(&slot);
    let sink: LocationSink = Arc::new(move |event| match event {
        LocationEvent::Fixes(fixes) => {
            answer_fix(&answered, fixes.into_iter().rev().find(LocationFix::is_usable))
        }
        LocationEvent::Failed(_) => answer_fix(&answered, None),
        _ => {}
    });
    let subscription = match subscribe_to(source, hub, installed, sink) {
        Ok(subscription) => subscription,
        Err(reason) => {
            tracing::debug!(%reason, "location fix unavailable");
            answer_fix(&slot, None);
            return FixRequest { slot, subscription: None };
        }
    };
    if let Err(reason) = source.perform(LocationCommand::RequestFix) {
        tracing::debug!(%reason, "location fix request refused");
        answer_fix(&slot, None);
    }
    FixRequest { slot, subscription: Some(subscription) }
}

static NATIVE_LOCATION: super::os::NativeLocation = super::os::NativeLocation;

fn source() -> &'static dyn LocationSource {
    match TESTER_LOCATION.get() {
        Some(tester) => tester as &dyn LocationSource,
        None => &NATIVE_LOCATION,
    }
}

pub const TESTER_ACCURACY_METERS: f64 = 5.0;

static TESTER_LOCATION: OnceLock<TesterLocation> = OnceLock::new();

/// tester source for stealth mode, reads one fixed allowed point so no consent prompt shows
/// only standard updates exist
pub struct TesterLocation {
    point: Mutex<Option<(f64, f64)>>,
    sink: Mutex<Option<LocationSink>>,
}

impl TesterLocation {
    const PERMISSION: LocationPermission =
        LocationPermission { level: LocationAuthorization::Always, is_precise: true };
    const REFUSED: LocationPermission =
        LocationPermission { level: LocationAuthorization::Denied, is_precise: false };

    fn current_permission(&self) -> LocationPermission {
        if self.point.lock().is_some() { Self::PERMISSION } else { Self::REFUSED }
    }

    fn fix(&self) -> Option<LocationFix> {
        let (latitude, longitude) = (*self.point.lock())?;
        Some(Self::fix_at(latitude, longitude))
    }

    fn fix_at(latitude: f64, longitude: f64) -> LocationFix {
        let time_ms = fix_time_ms(RiseClock::now().time_interval_since_1970());
        LocationFix {
            latitude,
            longitude,
            horizontal_accuracy_meters: TESTER_ACCURACY_METERS,
            altitude_meters: 0.0,
            vertical_accuracy_meters: -1.0,
            speed_meters_per_second: -1.0,
            speed_accuracy_meters_per_second: -1.0,
            course_degrees: -1.0,
            course_accuracy_degrees: -1.0,
            time_ms,
        }
    }

    fn dispatch(&self, event: LocationEvent) {
        let sink = self.sink.lock().clone();
        if let Some(sink) = sink {
            sink(event);
        }
    }
}

impl LocationSource for TesterLocation {
    fn permission(&self) -> Result<LocationPermission, LocationUnavailable> {
        Ok(self.current_permission())
    }

    fn last_fix(&self) -> Result<Option<LocationFix>, LocationUnavailable> {
        Ok(self.fix())
    }

    fn service_available(&self, service: LocationService) -> Result<(), LocationUnavailable> {
        Err(LocationUnavailable::ServiceOff(service))
    }

    fn install(&self, sink: LocationSink) -> Result<(), LocationUnavailable> {
        *self.sink.lock() = Some(sink);
        Ok(())
    }

    fn perform(&self, command: LocationCommand) -> Result<(), LocationUnavailable> {
        match command {
            LocationCommand::RequestAuthorization(_) => {
                self.dispatch(LocationEvent::Authorization(self.current_permission()));
            }
            LocationCommand::StartUpdates(_) | LocationCommand::RequestFix => match self.fix() {
                Some(fix) => self.dispatch(LocationEvent::Fixes(vec![fix])),
                None => self.dispatch(LocationEvent::Failed(LocationFailure::Denied)),
            },
            _ => {}
        }
        Ok(())
    }
}

pub fn install_tester_location(latitude: f64, longitude: f64) -> bool {
    if !TesterLocation::fix_at(latitude, longitude).is_usable() {
        return false;
    }
    let tester = TesterLocation { point: Mutex::new(Some((latitude, longitude))), sink: Mutex::new(None) };
    TESTER_LOCATION.set(tester).is_ok()
}

pub fn move_tester_location(latitude: f64, longitude: f64) -> bool {
    let Some(tester) = TESTER_LOCATION.get() else {
        return false;
    };
    let fix = TesterLocation::fix_at(latitude, longitude);
    if !fix.is_usable() {
        return false;
    }
    let was_refused = tester.point.lock().replace((latitude, longitude)).is_none();
    if was_refused {
        tester.dispatch(LocationEvent::Authorization(TesterLocation::PERMISSION));
    }
    tester.dispatch(LocationEvent::Fixes(vec![fix]));
    true
}

pub fn refuse_tester_location() -> bool {
    let Some(tester) = TESTER_LOCATION.get() else {
        return false;
    };
    if tester.point.lock().take().is_some() {
        tester.dispatch(LocationEvent::Authorization(TesterLocation::REFUSED));
        tester.dispatch(LocationEvent::Failed(LocationFailure::Denied));
    }
    true
}

static HUB: OnceLock<Arc<LocationHub>> = OnceLock::new();
static INSTALLED: OnceLock<Result<(), LocationUnavailable>> = OnceLock::new();

fn hub() -> &'static Arc<LocationHub> {
    HUB.get_or_init(Arc::default)
}

pub fn permission() -> Result<LocationPermission, LocationUnavailable> {
    source().permission()
}

pub fn last_fix() -> Result<Option<LocationFix>, LocationUnavailable> {
    source().last_fix()
}

pub fn request_authorization(scope: AuthorizationScope) -> Result<(), LocationUnavailable> {
    let level = permission()?.level;
    if !should_prompt(scope, level) {
        return Ok(());
    }
    source().perform(LocationCommand::RequestAuthorization(scope))
}

pub fn subscribe(sink: LocationSink) -> Result<LocationSubscription, LocationUnavailable> {
    subscribe_to(source(), hub(), &INSTALLED, sink)
}

/// main thread only, subscribe first so no event is lost
pub fn perform(command: LocationCommand) -> Result<(), LocationUnavailable> {
    perform_on(source(), HostOs::current(), command)
}

pub fn supports(service: LocationService) -> Result<(), LocationUnavailable> {
    supports_on(source(), HostOs::current(), service)
}

pub fn take_location_driver() -> Option<LocationDriver> {
    source().take_driver()
}

pub fn current_fix() -> FixRequest {
    current_fix_from(source(), hub(), &INSTALLED)
}

pub fn recorder_stationary_watch(exit_radius_meters: f64) -> StationaryWatch {
    stationary_watch(source(), HostOs::current(), exit_radius_meters)
}

pub fn location_support() -> PlatformSupport {
    match permission() {
        Ok(_) => PlatformSupport::Performed,
        Err(reason) => {
            tracing::debug!(%reason, "location unreadable");
            PlatformSupport::Unsupported
        }
    }
}

/// url of the location privacy pane, none on linux since there is no common one
pub const fn settings_url(host: HostOs) -> Option<&'static str> {
    match host {
        HostOs::MacOs => Some("x-apple.systempreferences:com.apple.preference.security?Privacy_LocationServices"),
        HostOs::Windows => Some("ms-settings:privacy-location"),
        HostOs::Linux => None,
    }
}

pub fn open_settings(app: &mut gpui::App) -> PlatformSupport {
    match settings_url(HostOs::current()) {
        Some(url) => {
            app.open_url(url);
            PlatformSupport::Performed
        }
        None => ask_again_without_settings(source()),
    }
}

pub fn ask_again_without_settings(source: &dyn LocationSource) -> PlatformSupport {
    if let Err(reason) = source.perform(LocationCommand::RequestAuthorization(AuthorizationScope::WhenInUse)) {
        tracing::debug!(%reason, "location could not be asked again");
    }
    PlatformSupport::Unsupported
}

const DIRECTIONS_NAME_ALLOWED: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";

/// link for directions in the host navigator
/// mac opens apple maps, linux and windows open the osm route planner in the browser
pub fn directions_url(host: HostOs, latitude: f64, longitude: f64, name: &str) -> String {
    match host {
        HostOs::MacOs => {
            let mut url = format!("maps://?daddr={latitude},{longitude}");
            if !name.is_empty() {
                let allowed = UrlAllowedCharacters::characters_in(DIRECTIONS_NAME_ALLOWED);
                url.push_str("&q=");
                url.push_str(&adding_percent_encoding(name, allowed));
            }
            url
        }
        HostOs::Linux | HostOs::Windows => {
            format!("https://www.openstreetmap.org/directions?route=%3B{latitude}%2C{longitude}")
        }
    }
}

pub fn open_directions(latitude: f64, longitude: f64, name: &str, app: &mut gpui::App) -> PlatformSupport {
    if !latitude.is_finite() || !longitude.is_finite() {
        return PlatformSupport::Unsupported;
    }
    app.open_url(&directions_url(HostOs::current(), latitude, longitude, name));
    PlatformSupport::Performed
}
