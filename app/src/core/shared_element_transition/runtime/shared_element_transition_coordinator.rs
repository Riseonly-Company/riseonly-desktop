use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{App, Context, Entity, Task};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::environment::reduce_motion;

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementDirection, SharedElementExitStyle, SharedElementOpenStyle, SharedElementSessionPolicy,
    SharedElementSessionPolicyPhase, SharedElementSurface,
};
use crate::core::shared_element_transition::navigation::shared_element_navigation_transitioning::SharedElementNavigationOperation;
use crate::core::shared_element_transition::renderer::shared_element_bitmap_resolver::SharedElementBitmapResolver;
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCapture, SharedElementCover, SharedElementDestinationContainer, SharedElementOriginContainer,
};
use crate::core::shared_element_transition::runtime::shared_element_host_probe::SharedElementStackHost;
use crate::core::shared_element_transition::runtime::shared_element_lift_board::SharedElementLiftBoard;
use crate::core::shared_element_transition::runtime::shared_element_navigation_policy::SharedElementNavigationPolicy;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;

const LOG_NAME: &str = "SharedElement";

/// route stacks as the coordinator reads them, the route stack host installs it
/// without one every host is alive, alone in its stack and off window
pub trait SharedElementStackDirectory {
    fn stack_of(&self, host: SharedElementStackHost, cx: &App) -> Option<Vec<SharedElementStackHost>>;
    fn is_in_window(&self, host: SharedElementStackHost, cx: &App) -> bool;
}

/// one armed or running shared element navigation
pub struct SharedElementIntent {
    pub epoch: u64,
    pub direction: SharedElementDirection,
    pub interactive: bool,
    pub reduce_motion: bool,
    pub armed_at: f64,
    pub path_count_at_arm: usize,
    pub item_id: String,
    pub open_style: Option<SharedElementOpenStyle>,
    pub exit_style: Option<SharedElementExitStyle>,
    capture: RefCell<Option<SharedElementCapture>>,
    pub cover: Option<SharedElementCover>,
    source_host: Cell<Option<SharedElementStackHost>>,
    target_host: Cell<Option<SharedElementStackHost>>,
    origin: RefCell<Option<Weak<dyn SharedElementOriginContainer>>>,
    destination: RefCell<Option<Weak<dyn SharedElementDestinationContainer>>>,
}

impl SharedElementIntent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        epoch: u64,
        direction: SharedElementDirection,
        interactive: bool,
        reduce_motion: bool,
        armed_at: f64,
        path_count_at_arm: usize,
        item_id: String,
        open_style: Option<SharedElementOpenStyle>,
        exit_style: Option<SharedElementExitStyle>,
        capture: Option<SharedElementCapture>,
        cover: Option<SharedElementCover>,
        source_host: Option<SharedElementStackHost>,
        target_host: Option<SharedElementStackHost>,
        origin: Option<&Rc<dyn SharedElementOriginContainer>>,
        destination: Option<&Rc<dyn SharedElementDestinationContainer>>,
    ) -> Self {
        Self {
            epoch,
            direction,
            interactive,
            reduce_motion,
            armed_at,
            path_count_at_arm,
            item_id,
            open_style,
            exit_style,
            capture: RefCell::new(capture),
            cover,
            source_host: Cell::new(source_host),
            target_host: Cell::new(target_host),
            origin: RefCell::new(origin.map(Rc::downgrade)),
            destination: RefCell::new(destination.map(Rc::downgrade)),
        }
    }

    pub fn capture(&self) -> Option<SharedElementCapture> {
        self.capture.borrow().clone()
    }

    pub fn set_capture(&self, capture: Option<SharedElementCapture>) {
        *self.capture.borrow_mut() = capture;
    }

    pub fn source_host(&self) -> Option<SharedElementStackHost> {
        self.source_host.get()
    }

    pub fn set_source_host(&self, host: Option<SharedElementStackHost>) {
        self.source_host.set(host);
    }

    pub fn target_host(&self) -> Option<SharedElementStackHost> {
        self.target_host.get()
    }

    pub fn set_target_host(&self, host: Option<SharedElementStackHost>) {
        self.target_host.set(host);
    }

    pub fn origin(&self) -> Option<Rc<dyn SharedElementOriginContainer>> {
        self.origin.borrow().as_ref().and_then(Weak::upgrade)
    }

    pub fn set_origin(&self, origin: Option<&Rc<dyn SharedElementOriginContainer>>) {
        *self.origin.borrow_mut() = origin.map(Rc::downgrade);
    }

    pub fn destination(&self) -> Option<Rc<dyn SharedElementDestinationContainer>> {
        self.destination.borrow().as_ref().and_then(Weak::upgrade)
    }

    pub fn set_destination(&self, destination: Option<&Rc<dyn SharedElementDestinationContainer>>) {
        *self.destination.borrow_mut() = destination.map(Rc::downgrade);
    }
}

struct OriginRecord {
    container: Weak<dyn SharedElementOriginContainer>,
    owner: Option<Weak<dyn Any>>,
}

impl OriginRecord {
    fn container(&self) -> Option<Rc<dyn SharedElementOriginContainer>> {
        self.container.upgrade()
    }

    fn is_released(&self) -> bool {
        self.container.strong_count() == 0
    }

    fn holds(&self, origin: &Rc<dyn SharedElementOriginContainer>) -> bool {
        !self.is_released() && std::ptr::addr_eq(self.container.as_ptr(), Rc::as_ptr(origin))
    }
}

struct DestinationRecord {
    container: Weak<dyn SharedElementDestinationContainer>,
    owner: Option<Weak<dyn Any>>,
}

impl DestinationRecord {
    fn container(&self) -> Option<Rc<dyn SharedElementDestinationContainer>> {
        self.container.upgrade()
    }

    fn is_released(&self) -> bool {
        self.container.strong_count() == 0
    }

    fn holds(&self, destination: &Rc<dyn SharedElementDestinationContainer>) -> bool {
        !self.is_released() && std::ptr::addr_eq(self.container.as_ptr(), Rc::as_ptr(destination))
    }
}

/// true if the record has no owner or the owner is this one, a released owner counts as none
fn owner_matches(record_owner: &Option<Weak<dyn Any>>, owner: &Rc<dyn Any>) -> bool {
    match record_owner {
        None => true,
        Some(record_owner) => {
            record_owner.strong_count() == 0 || std::ptr::addr_eq(record_owner.as_ptr(), Rc::as_ptr(owner))
        }
    }
}

struct HostRegistrations {
    host: SharedElementStackHost,
    origins: Vec<OriginRecord>,
    destinations: Vec<DestinationRecord>,
}

impl HostRegistrations {
    fn new(host: SharedElementStackHost) -> Self {
        Self { host, origins: Vec::new(), destinations: Vec::new() }
    }
}

struct DeferredWork {
    surface: SharedElementSurface,
    work: Box<dyn FnOnce(&mut App)>,
}

struct SettleWaiter {
    surface: SharedElementSurface,
    continuation: oneshot::Sender<()>,
}

struct TransitionHooks {
    epoch: u64,
    on_resign_active: Option<Rc<dyn Fn(&mut App)>>,
    on_force_end: Rc<dyn Fn(&mut App)>,
}

struct OpenLinger {
    owner: usize,
    abort: Box<dyn Fn(&mut App)>,
}

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// session owner of shared element navigation
/// holds registrations and epoch and intents and settle queue and playback hold
/// plus safety exits like ttl and path change and account reset, not observable
pub struct SharedElementTransitionCoordinator {
    epoch: u64,
    phase: SharedElementSessionPolicyPhase,
    intent: Option<Rc<SharedElementIntent>>,
    registration_generation: u64,
    hosts: Vec<HostRegistrations>,
    active_surfaces: HashSet<SharedElementSurface>,
    deferred_work: Vec<DeferredWork>,
    settle_waiters: HashMap<u64, SettleWaiter>,
    next_waiter_id: u64,
    held_playback_owners: HashSet<String>,
    transition_hooks: Option<TransitionHooks>,
    last_known_path_count: Option<usize>,
    open_linger: Option<OpenLinger>,
    overlay_epoch: Option<u64>,
    stack_directory: Option<Rc<dyn SharedElementStackDirectory>>,
    _resign_active_observer: RiseSubscription,
    _resign_active_signals: Task<()>,
}

impl SharedElementTransitionCoordinator {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    /// unique per process, safe from any thread
    pub fn next_session() -> u64 {
        SESSION_COUNTER.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let (sender, mut inbox) = mpsc::unbounded::<()>();
        let observer = RiseAppEvents::default_center().observe(UIApplication::WILL_RESIGN_ACTIVE_NOTIFICATION, move |_| {
            let _ = sender.unbounded_send(());
        });
        let signals = cx.spawn(async move |this, cx| {
            while inbox.next().await.is_some() {
                let updated = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                    this.application_will_resign_active(cx);
                });
                if updated.is_err() {
                    break;
                }
            }
        });
        Self {
            epoch: 0,
            phase: SharedElementSessionPolicyPhase::Idle,
            intent: None,
            registration_generation: 0,
            hosts: Vec::new(),
            active_surfaces: HashSet::new(),
            deferred_work: Vec::new(),
            settle_waiters: HashMap::new(),
            next_waiter_id: 0,
            held_playback_owners: HashSet::new(),
            transition_hooks: None,
            last_known_path_count: None,
            open_linger: None,
            overlay_epoch: None,
            stack_directory: None,
            _resign_active_observer: observer,
            _resign_active_signals: signals,
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn phase(&self) -> SharedElementSessionPolicyPhase {
        self.phase
    }

    pub fn intent(&self) -> Option<Rc<SharedElementIntent>> {
        self.intent.clone()
    }

    pub fn registration_generation(&self) -> u64 {
        self.registration_generation
    }

    pub fn set_stack_directory(&mut self, directory: Option<Rc<dyn SharedElementStackDirectory>>) {
        self.stack_directory = directory;
    }

    pub fn register_origin_unowned(
        &mut self,
        origin: &Rc<dyn SharedElementOriginContainer>,
        host: SharedElementStackHost,
        cx: &mut Context<Self>,
    ) {
        self.insert_origin(origin, host, None, cx);
    }

    pub fn register_origin(
        &mut self,
        origin: &Rc<dyn SharedElementOriginContainer>,
        host: SharedElementStackHost,
        owner: &Rc<dyn Any>,
        cx: &mut Context<Self>,
    ) {
        self.insert_origin(origin, host, Some(Rc::downgrade(owner)), cx);
    }

    fn insert_origin(
        &mut self,
        origin: &Rc<dyn SharedElementOriginContainer>,
        host: SharedElementStackHost,
        owner: Option<Weak<dyn Any>>,
        cx: &mut Context<Self>,
    ) {
        for entry in &mut self.hosts {
            entry.origins.retain(|record| !(record.is_released() || record.holds(origin)));
        }
        self.registrations(host).origins.push(OriginRecord { container: Rc::downgrade(origin), owner });
        self.compact_registrations(cx);
    }

    pub fn unregister_origin_any_owner(&mut self, origin: &Rc<dyn SharedElementOriginContainer>, cx: &mut Context<Self>) {
        for entry in &mut self.hosts {
            entry.origins.retain(|record| !(record.is_released() || record.holds(origin)));
        }
        self.compact_registrations(cx);
    }

    pub fn unregister_origin(
        &mut self,
        origin: &Rc<dyn SharedElementOriginContainer>,
        owner: &Rc<dyn Any>,
        cx: &mut Context<Self>,
    ) {
        for entry in &mut self.hosts {
            entry.origins.retain(|record| {
                if record.is_released() {
                    return false;
                }
                !(record.holds(origin) && owner_matches(&record.owner, owner))
            });
        }
        self.compact_registrations(cx);
    }

    pub fn register_destination_unowned(
        &mut self,
        destination: &Rc<dyn SharedElementDestinationContainer>,
        host: SharedElementStackHost,
        cx: &mut Context<Self>,
    ) {
        self.insert_destination(destination, host, None, cx);
    }

    pub fn register_destination(
        &mut self,
        destination: &Rc<dyn SharedElementDestinationContainer>,
        host: SharedElementStackHost,
        owner: &Rc<dyn Any>,
        cx: &mut Context<Self>,
    ) {
        self.insert_destination(destination, host, Some(Rc::downgrade(owner)), cx);
    }

    fn insert_destination(
        &mut self,
        destination: &Rc<dyn SharedElementDestinationContainer>,
        host: SharedElementStackHost,
        owner: Option<Weak<dyn Any>>,
        cx: &mut Context<Self>,
    ) {
        for entry in &mut self.hosts {
            entry.destinations.retain(|record| !(record.is_released() || record.holds(destination)));
        }
        self.registrations(host)
            .destinations
            .push(DestinationRecord { container: Rc::downgrade(destination), owner });
        self.compact_registrations(cx);
    }

    pub fn unregister_destination_any_owner(
        &mut self,
        destination: &Rc<dyn SharedElementDestinationContainer>,
        cx: &mut Context<Self>,
    ) {
        for entry in &mut self.hosts {
            entry.destinations.retain(|record| !(record.is_released() || record.holds(destination)));
        }
        self.compact_registrations(cx);
    }

    pub fn unregister_destination(
        &mut self,
        destination: &Rc<dyn SharedElementDestinationContainer>,
        owner: &Rc<dyn Any>,
        cx: &mut Context<Self>,
    ) {
        for entry in &mut self.hosts {
            entry.destinations.retain(|record| {
                if record.is_released() {
                    return false;
                }
                !(record.holds(destination) && owner_matches(&record.owner, owner))
            });
        }
        self.compact_registrations(cx);
    }

    /// most recently registered live destination of host
    pub fn destination_hosted_by(
        &self,
        host: Option<SharedElementStackHost>,
    ) -> Option<Rc<dyn SharedElementDestinationContainer>> {
        let host = host?;
        let entry = self.hosts.iter().find(|entry| entry.host == host)?;
        entry.destinations.iter().rev().find_map(DestinationRecord::container)
    }

    /// registered origin of the page right below host whose surface matches
    pub fn origin_below(
        &self,
        host: SharedElementStackHost,
        surface: &SharedElementSurface,
        cx: &App,
    ) -> Option<Rc<dyn SharedElementOriginContainer>> {
        let stack = self.navigation_stack_of(host, cx)?;
        let index = stack.iter().position(|candidate| *candidate == host)?;
        if index == 0 {
            return None;
        }
        let below = stack[index - 1];
        let entry = self.hosts.iter().find(|entry| entry.host == below)?;
        entry
            .origins
            .iter()
            .rev()
            .filter_map(OriginRecord::container)
            .find(|container| container.surface() == *surface)
    }

    /// registered origin of surface under any host, for overlays with no page below
    /// host on screen wins, otherwise the latest host entry
    pub fn origin(&self, surface: &SharedElementSurface, cx: &App) -> Option<Rc<dyn SharedElementOriginContainer>> {
        let mut fallback: Option<Rc<dyn SharedElementOriginContainer>> = None;
        for entry in self.hosts.iter().rev() {
            if !self.host_is_alive(entry.host, cx) {
                continue;
            }
            let Some(origin) = entry
                .origins
                .iter()
                .rev()
                .filter_map(OriginRecord::container)
                .find(|container| container.surface() == *surface)
            else {
                continue;
            };
            if self.host_is_in_window(entry.host, cx) {
                return Some(origin);
            }
            if fallback.is_none() {
                fallback = Some(origin);
            }
        }
        fallback
    }

    /// host a probe registered for origin, containers may answer host_view_controller with it
    pub fn registered_host_of_origin(&self, origin: &Rc<dyn SharedElementOriginContainer>) -> Option<SharedElementStackHost> {
        self.hosts
            .iter()
            .find(|entry| entry.origins.iter().any(|record| record.holds(origin)))
            .map(|entry| entry.host)
    }

    pub fn registered_host_of_destination(
        &self,
        destination: &Rc<dyn SharedElementDestinationContainer>,
    ) -> Option<SharedElementStackHost> {
        self.hosts
            .iter()
            .find(|entry| entry.destinations.iter().any(|record| record.holds(destination)))
            .map(|entry| entry.host)
    }

    pub fn arm_open(
        origin: &Rc<dyn SharedElementOriginContainer>,
        item_id: &str,
        route_flies: bool,
        cx: &mut App,
    ) -> bool {
        let coordinator = Self::shared(cx);
        if !SharedElementSessionPolicy::can_arm(coordinator.read(cx).phase) {
            return false;
        }
        let flights_enabled = SharedElementNavigationPolicy::flights_enabled();
        let reduce_motion = reduce_motion(cx);
        if SharedElementSessionPolicy::open_style(flights_enabled, route_flies, true, reduce_motion)
            == SharedElementOpenStyle::Stock
        {
            return false;
        }
        let Some(source_host) = origin
            .host_view_controller(cx)
            .or_else(|| coordinator.read(cx).registered_host_of_origin(origin))
        else {
            return false;
        };

        let capture = if reduce_motion { None } else { origin.open_capture(item_id, cx) };
        let style = SharedElementSessionPolicy::open_style(flights_enabled, route_flies, capture.is_some(), reduce_motion);
        if style == SharedElementOpenStyle::Stock {
            settle_capture(capture.as_ref());
            return false;
        }
        let surface = origin.surface();
        let item_id = item_id.to_owned();
        let origin = origin.clone();
        coordinator.update(cx, move |this, cx| {
            this.abort_open_linger(cx);
            this.epoch = this.epoch.wrapping_add(1);
            let mut surfaces = HashSet::from([surface]);
            if let Some(capture) = &capture {
                surfaces.insert(capture.source_key.surface.clone());
            }
            this.active_surfaces = surfaces;
            let path_count = this.path_count_near(source_host, cx);
            this.intent = Some(Rc::new(SharedElementIntent::new(
                this.epoch,
                SharedElementDirection::Open,
                false,
                reduce_motion,
                RiseClock::monotonic().as_secs_f64(),
                path_count,
                item_id,
                Some(style),
                None,
                capture,
                None,
                Some(source_host),
                None,
                Some(&origin),
                None,
            )));
            this.phase = SharedElementSessionPolicyPhase::Armed;
            let epoch = this.epoch;
            this.schedule_arm_expiry(epoch, cx);
        });
        true
    }

    /// arms the exit, none when nothing is armed including the plain stock exit
    pub fn arm_exit(
        destination: &Rc<dyn SharedElementDestinationContainer>,
        interactive: bool,
        cx: &mut App,
    ) -> Option<Rc<SharedElementIntent>> {
        let coordinator = Self::shared(cx);
        let (phase, epoch) = {
            let this = coordinator.read(cx);
            (this.phase, this.epoch)
        };
        if !SharedElementSessionPolicy::can_arm(phase) {
            log(move || format!("armExit refused: phase={phase:?} epoch={epoch}"), cx);
            return None;
        }
        let flights_enabled = SharedElementNavigationPolicy::flights_enabled();
        let source_host = if flights_enabled {
            destination
                .host_view_controller(cx)
                .or_else(|| coordinator.read(cx).registered_host_of_destination(destination))
        } else {
            None
        };
        let Some(source_host) = source_host else {
            log(|| "armExit refused: no host".to_owned(), cx);
            return None;
        };
        let Some(capture) = destination.current_capture(cx) else {
            log(|| "armExit refused: no visible capture".to_owned(), cx);
            return None;
        };

        let reduce_motion = reduce_motion(cx);
        let origin_surface = destination.origin_surface();
        let origin = coordinator.read(cx).origin_below(source_host, &origin_surface, cx);
        let can_return = origin.as_ref().is_some_and(|origin| origin.can_return(&capture.item_id, cx));
        let style = SharedElementSessionPolicy::exit_style(
            flights_enabled,
            true,
            origin.is_some(),
            can_return,
            reduce_motion,
        );
        let has_origin = origin.is_some();
        log(
            move || format!("armExit style={style:?} origin={has_origin} canReturn={can_return} interactive={interactive}"),
            cx,
        );
        if style == SharedElementExitStyle::Stock {
            settle_capture(Some(&capture));
            return None;
        }
        if style != SharedElementExitStyle::Flight {
            settle_capture(Some(&capture));
        }
        let cover = if style == SharedElementExitStyle::Flight {
            origin.as_ref().and_then(|origin| origin.return_cover(&capture.item_id, cx))
        } else {
            None
        };
        let target_host = origin.as_ref().and_then(|origin| {
            origin
                .host_view_controller(cx)
                .or_else(|| coordinator.read(cx).registered_host_of_origin(origin))
        });

        let destination = destination.clone();
        let armed = coordinator.update(cx, move |this, cx| {
            this.abort_open_linger(cx);
            this.epoch = this.epoch.wrapping_add(1);
            let mut surfaces = HashSet::from([destination.surface(), capture.source_key.surface.clone()]);
            if let Some(origin) = &origin {
                surfaces.insert(origin.surface());
            }
            this.active_surfaces = surfaces;
            let path_count = this.path_count_near(source_host, cx);
            let item_id = capture.item_id.clone();
            let armed = Rc::new(SharedElementIntent::new(
                this.epoch,
                SharedElementDirection::Exit,
                interactive,
                reduce_motion,
                RiseClock::monotonic().as_secs_f64(),
                path_count,
                item_id,
                None,
                Some(style),
                Some(capture),
                cover,
                Some(source_host),
                target_host,
                origin.as_ref(),
                Some(&destination),
            ));
            this.intent = Some(armed.clone());
            this.phase = SharedElementSessionPolicyPhase::Armed;
            let epoch = this.epoch;
            this.schedule_arm_expiry(epoch, cx);
            armed
        });
        Some(armed)
    }

    /// hands the armed intent to the transition that matches it by page identity
    pub fn consume(
        &mut self,
        operation: SharedElementNavigationOperation,
        from: SharedElementStackHost,
        to: SharedElementStackHost,
        cx: &mut Context<Self>,
    ) -> Option<Rc<SharedElementIntent>> {
        if self.phase != SharedElementSessionPolicyPhase::Armed {
            return None;
        }
        let armed = self.intent.clone()?;
        if !SharedElementSessionPolicy::is_fresh(
            armed.armed_at,
            RiseClock::monotonic().as_secs_f64(),
            SharedElementMotion::INTENT_TTL,
        ) {
            self.abandon(armed.epoch, "stale", cx);
            return None;
        }

        match operation {
            SharedElementNavigationOperation::Push => {
                if armed.direction != SharedElementDirection::Open || armed.source_host() != Some(from) {
                    self.abandon(armed.epoch, "push_mismatch", cx);
                    return None;
                }
                armed.set_target_host(Some(to));
                if armed.destination().is_none() {
                    let destination = self.destination_hosted_by(Some(to));
                    armed.set_destination(destination.as_ref());
                }
            }
            SharedElementNavigationOperation::Pop => {
                if armed.direction != SharedElementDirection::Exit || armed.source_host() != Some(from) {
                    self.abandon(armed.epoch, "pop_mismatch", cx);
                    return None;
                }
                match armed.exit_style {
                    Some(SharedElementExitStyle::Flight) | Some(SharedElementExitStyle::Crossfade) => {
                        if armed.target_host() != Some(to) {
                            self.abandon(armed.epoch, "pop_target_mismatch", cx);
                            return None;
                        }
                    }
                    Some(SharedElementExitStyle::StockWithFade) | Some(SharedElementExitStyle::Stock) | None => {}
                }
            }
            SharedElementNavigationOperation::None => return None,
        }

        self.phase = SharedElementSessionPolicyPhase::Running;
        Some(armed)
    }

    /// drops the session of epoch with no landing
    /// nothing on the scroller is undone because arming never changes it
    pub fn abandon(&mut self, epoch: u64, reason: &str, cx: &mut Context<Self>) {
        let Some(current) = self.current_intent_of(epoch) else { return };
        let reason = reason.to_owned();
        log(move || format!("abandon epoch={epoch} reason={reason}"), cx);
        self.end_session(&current, cx);
    }

    pub fn transition_did_end(&mut self, epoch: u64, cx: &mut Context<Self>) {
        let Some(current) = self.current_intent_of(epoch) else { return };
        self.end_session(&current, cx);
    }

    /// registered by the running animator
    /// on_force_end runs on account reset after the epoch bump so it has to finish the transition
    /// context without an epoch check
    pub fn attach_transition_hooks(
        &mut self,
        epoch: u64,
        on_resign_active: Option<Box<dyn Fn(&mut App)>>,
        on_force_end: Box<dyn Fn(&mut App)>,
    ) {
        if epoch != self.epoch || self.phase == SharedElementSessionPolicyPhase::Idle {
            return;
        }
        self.transition_hooks = Some(TransitionHooks {
            epoch,
            on_resign_active: on_resign_active.map(Rc::from),
            on_force_end: Rc::from(on_force_end),
        });
    }

    pub fn detach_transition_hooks(&mut self, epoch: u64) {
        if self.transition_hooks.as_ref().map(|hooks| hooks.epoch) != Some(epoch) {
            return;
        }
        self.transition_hooks = None;
    }

    pub fn attach_open_linger(&mut self, owner: usize, abort: Box<dyn Fn(&mut App)>) {
        self.open_linger = Some(OpenLinger { owner, abort });
    }

    pub fn detach_open_linger(&mut self, owner: usize) {
        let is_owner = self.open_linger.as_ref().is_some_and(|linger| linger.owner == owner);
        if !is_owner {
            return;
        }
        self.open_linger = None;
    }

    /// starts a session for a flight over something that is not a route like the story player
    /// no intent and no ttl and no page match, none unless idle
    /// every path must end it with end_overlay_session
    pub fn begin_overlay_session(
        &mut self,
        item_id: &str,
        surfaces: HashSet<SharedElementSurface>,
        interactive: bool,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        if !SharedElementNavigationPolicy::flights_enabled() || !SharedElementSessionPolicy::can_arm(self.phase) {
            let (phase, epoch) = (self.phase, self.epoch);
            log(move || format!("overlay refused: phase={phase:?} epoch={epoch}"), cx);
            return None;
        }
        self.abort_open_linger(cx);
        self.epoch = self.epoch.wrapping_add(1);
        self.active_surfaces = surfaces;
        self.overlay_epoch = Some(self.epoch);
        self.phase = SharedElementSessionPolicyPhase::Running;
        let epoch = self.epoch;
        let item_id = item_id.to_owned();
        log(move || format!("overlay begin epoch={epoch} item={item_id} interactive={interactive}"), cx);
        Some(epoch)
    }

    pub fn end_overlay_session(&mut self, epoch: u64, cx: &mut Context<Self>) {
        if self.overlay_epoch != Some(epoch) || epoch != self.epoch || self.phase == SharedElementSessionPolicyPhase::Idle {
            return;
        }
        self.phase = SharedElementSessionPolicyPhase::Settling;
        if self.transition_hooks.as_ref().map(|hooks| hooks.epoch) == Some(epoch) {
            self.transition_hooks = None;
        }
        SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.land(epoch, cx));
        self.release_inline_playback(epoch, cx);
        self.overlay_epoch = None;
        self.active_surfaces.clear();
        self.phase = SharedElementSessionPolicyPhase::Idle;
        log(move || format!("overlay end epoch={epoch}"), cx);
        self.flush_settled_work(cx);
    }

    pub fn is_flight_active(&self, surface: &SharedElementSurface) -> bool {
        if self.phase == SharedElementSessionPolicyPhase::Idle {
            return false;
        }
        let Some(current) = &self.intent else {
            return self.overlay_epoch.is_some() && self.active_surfaces.contains(surface);
        };
        if self.active_surfaces.contains(surface) {
            return true;
        }
        match current.destination() {
            Some(destination) => destination.surface() == *surface,
            None => false,
        }
    }

    /// runs work now if no flight touches surface, else once it settles
    pub fn run_or_defer_until_settled(
        surface: SharedElementSurface,
        work: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) {
        let coordinator = Self::shared(cx);
        if !coordinator.read(cx).is_flight_active(&surface) {
            work(cx);
            return;
        }
        coordinator.update(cx, move |this, _| {
            this.deferred_work.push(DeferredWork { surface, work: Box::new(work) });
        });
    }

    /// returns at once when idle, else on settle
    /// timeout or account reset
    /// dropping the future cancels the task
    pub fn wait_until_settled(
        surface: SharedElementSurface,
        timeout: f64,
        cx: &mut App,
    ) -> impl Future<Output = ()> + 'static + use<> {
        let coordinator = Self::shared(cx);
        let receiver = coordinator.update(cx, move |this, cx| {
            if !this.is_flight_active(&surface) {
                return None;
            }
            this.next_waiter_id = this.next_waiter_id.wrapping_add(1);
            let waiter_id = this.next_waiter_id;
            let (continuation, receiver) = oneshot::channel();
            this.settle_waiters.insert(waiter_id, SettleWaiter { surface, continuation });
            let delay = Duration::from_secs_f64(timeout.max(0.0));
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(delay).await;
                let _ = this.update(cx, |this: &mut Self, _| this.resume_waiter(waiter_id));
            })
            .detach();
            Some(receiver)
        });
        async move {
            if let Some(receiver) = receiver {
                let _ = receiver.await;
            }
        }
    }

    /// holds inline reels through the full screen owner set so the player own gate pauses and resumes it
    pub fn hold_inline_playback(&mut self, epoch: u64, cx: &mut Context<Self>) {
        if epoch != self.epoch {
            return;
        }
        let owner = playback_owner(epoch);
        if !self.held_playback_owners.insert(owner.clone()) {
            return;
        }
        ShortsServicesStore::shared(cx).update(cx, |store, cx| store.begin_full_screen_playback(owner, cx));
    }

    /// not epoch guarded, a hold from an older epoch still has to be released
    pub fn release_inline_playback(&mut self, epoch: u64, cx: &mut Context<Self>) {
        let owner = playback_owner(epoch);
        if !self.held_playback_owners.remove(&owner) {
            return;
        }
        ShortsServicesStore::shared(cx).update(cx, |store, cx| store.end_full_screen_playback(owner, cx));
    }

    pub fn navigation_path_did_change(&mut self, count: usize, cx: &mut Context<Self>) {
        if self.phase == SharedElementSessionPolicyPhase::Armed
            && let Some(current) = self.intent.clone()
            && SharedElementSessionPolicy::path_change_abandons_arm(current.direction, current.path_count_at_arm, count)
        {
            self.abandon(current.epoch, "path_change", cx);
        }
        self.last_known_path_count = Some(count);
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.abort_open_linger(cx);
        self.epoch = self.epoch.wrapping_add(1);
        let hooks = self.transition_hooks.take();
        let current = self.intent.take();
        self.overlay_epoch = None;
        if let Some(hooks) = hooks {
            let on_force_end = hooks.on_force_end;
            cx.defer(move |cx| on_force_end(cx));
        }

        SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.clear_all(cx));
        SharedElementBitmapResolver::reset_account_state(cx);
        settle_capture(current.as_ref().and_then(|current| current.capture()).as_ref());
        let held: Vec<String> = self.held_playback_owners.drain().collect();
        if !held.is_empty() {
            ShortsServicesStore::shared(cx).update(cx, |store, cx| {
                for owner in held {
                    store.end_full_screen_playback(owner, cx);
                }
            });
        }

        self.phase = SharedElementSessionPolicyPhase::Idle;
        self.active_surfaces.clear();
        self.deferred_work.clear();
        for (_, waiter) in self.settle_waiters.drain() {
            let _ = waiter.continuation.send(());
        }

        self.hosts.clear();
        self.registration_generation = self.registration_generation.wrapping_add(1);
        self.last_known_path_count = None;
    }

    fn current_intent_of(&self, epoch: u64) -> Option<Rc<SharedElementIntent>> {
        if epoch != self.epoch || self.phase == SharedElementSessionPolicyPhase::Idle {
            return None;
        }
        self.intent.clone().filter(|current| current.epoch == epoch)
    }

    fn abort_open_linger(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.open_linger.take() {
            cx.defer(pending.abort);
        }
    }

    fn end_session(&mut self, current: &Rc<SharedElementIntent>, cx: &mut Context<Self>) {
        let ended_epoch = current.epoch;
        self.phase = SharedElementSessionPolicyPhase::Settling;
        if self.transition_hooks.as_ref().map(|hooks| hooks.epoch) == Some(ended_epoch) {
            self.transition_hooks = None;
        }
        settle_capture(current.capture().as_ref());
        SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.land(ended_epoch, cx));
        self.release_inline_playback(ended_epoch, cx);

        let settled_destination = if current.direction == SharedElementDirection::Open {
            current.destination().or_else(|| self.destination_hosted_by(current.target_host()))
        } else {
            None
        };

        self.intent = None;
        self.active_surfaces.clear();
        self.phase = SharedElementSessionPolicyPhase::Idle;
        if let Some(destination) = settled_destination {
            cx.defer(move |cx| destination.did_settle_open(ended_epoch, cx));
        }
        self.flush_settled_work(cx);
    }

    fn flush_settled_work(&mut self, cx: &mut Context<Self>) {
        if !self.deferred_work.is_empty() {
            let pending = std::mem::take(&mut self.deferred_work);
            for item in pending {
                if self.is_flight_active(&item.surface) {
                    self.deferred_work.push(item);
                } else {
                    cx.defer(item.work);
                }
            }
        }
        if self.settle_waiters.is_empty() {
            return;
        }
        let ready_ids: Vec<u64> = self
            .settle_waiters
            .iter()
            .filter(|(_, waiter)| !self.is_flight_active(&waiter.surface))
            .map(|(id, _)| *id)
            .collect();
        for id in ready_ids {
            self.resume_waiter(id);
        }
    }

    fn resume_waiter(&mut self, id: u64) {
        if let Some(waiter) = self.settle_waiters.remove(&id) {
            let _ = waiter.continuation.send(());
        }
    }

    fn schedule_arm_expiry(&mut self, armed_epoch: u64, cx: &mut Context<Self>) {
        let ttl = Duration::from_secs_f64(SharedElementMotion::INTENT_TTL);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ttl).await;
            let _ = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                if this.epoch != armed_epoch || this.phase != SharedElementSessionPolicyPhase::Armed {
                    return;
                }
                this.abandon(armed_epoch, "ttl", cx);
            });
        })
        .detach();
    }

    fn application_will_resign_active(&mut self, cx: &mut Context<Self>) {
        if self.phase != SharedElementSessionPolicyPhase::Running {
            return;
        }
        let Some(hooks) = &self.transition_hooks else { return };
        if hooks.epoch != self.epoch {
            return;
        }
        if let Some(on_resign_active) = hooks.on_resign_active.clone() {
            cx.defer(move |cx| on_resign_active(cx));
        }
    }

    fn registrations(&mut self, host: SharedElementStackHost) -> &mut HostRegistrations {
        let index = match self.hosts.iter().position(|entry| entry.host == host) {
            Some(index) => index,
            None => {
                self.hosts.push(HostRegistrations::new(host));
                self.hosts.len() - 1
            }
        };
        &mut self.hosts[index]
    }

    fn compact_registrations(&mut self, cx: &App) {
        let directory = self.stack_directory.clone();
        self.hosts.retain(|entry| {
            let host_alive = directory.as_ref().is_none_or(|directory| directory.stack_of(entry.host, cx).is_some());
            host_alive && !(entry.origins.is_empty() && entry.destinations.is_empty())
        });
    }

    fn host_is_alive(&self, host: SharedElementStackHost, cx: &App) -> bool {
        self.stack_directory.as_ref().is_none_or(|directory| directory.stack_of(host, cx).is_some())
    }

    fn host_is_in_window(&self, host: SharedElementStackHost, cx: &App) -> bool {
        self.stack_directory.as_ref().is_some_and(|directory| directory.is_in_window(host, cx))
    }

    fn navigation_stack_of(&self, host: SharedElementStackHost, cx: &App) -> Option<Vec<SharedElementStackHost>> {
        self.stack_directory.as_ref().and_then(|directory| directory.stack_of(host, cx))
    }

    /// count navigation_path_did_change compares against, stack depth until one arrives
    fn path_count_near(&self, host: SharedElementStackHost, cx: &App) -> usize {
        if let Some(count) = self.last_known_path_count {
            return count;
        }
        self.navigation_stack_of(host, cx).map_or(0, |stack| stack.len().saturating_sub(1))
    }
}

/// cancels the stop frame and the live readiness of the capture
fn settle_capture(capture: Option<&SharedElementCapture>) {
    let Some(capture) = capture else { return };
    if let Some(stop_frame) = &capture.stop_frame {
        stop_frame.cancel();
    }
    if let Some(live_readiness) = &capture.live_readiness {
        live_readiness.cancel();
    }
}

fn playback_owner(epoch: u64) -> String {
    format!("se.{epoch}")
}

#[track_caller]
fn log(message: impl FnOnce() -> String, cx: &mut App) {
    Sai::log(LOG_NAME, message, SaiLogKind::Ui, cx);
}
