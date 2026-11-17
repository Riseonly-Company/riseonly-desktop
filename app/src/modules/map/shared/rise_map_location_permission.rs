use std::future::Future;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, Entity, Task};
use rise_platform::location::{
    self, AuthorizationScope, LocationAuthorization, LocationEvent, LocationFix, LocationPermission,
    LocationSubscription, LocationUnavailable,
};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::MAP_LOCATION_DID_REQUEST_ALWAYS;

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RiseMapLocationPermissionLevel {
    NotDetermined,
    Denied,
    Restricted,
    WhenInUse,
    Always,
}

impl RiseMapLocationPermissionLevel {
    pub fn allows_foreground(self) -> bool {
        self == Self::WhenInUse || self == Self::Always
    }

    fn from_authorization(status: LocationAuthorization) -> Self {
        match status {
            LocationAuthorization::NotDetermined => Self::NotDetermined,
            LocationAuthorization::Restricted => Self::Restricted,
            LocationAuthorization::Denied => Self::Denied,
            LocationAuthorization::WhenInUse => Self::WhenInUse,
            LocationAuthorization::Always => Self::Always,
        }
    }

    fn authorization(self) -> LocationAuthorization {
        match self {
            Self::NotDetermined => LocationAuthorization::NotDetermined,
            Self::Denied => LocationAuthorization::Denied,
            Self::Restricted => LocationAuthorization::Restricted,
            Self::WhenInUse => LocationAuthorization::WhenInUse,
            Self::Always => LocationAuthorization::Always,
        }
    }
}

pub struct RiseMapLocationPermission {
    level: RiseMapLocationPermissionLevel,
    is_precise: bool,
    did_ask_for_always: bool,
    manager: Option<LocationSubscription>,
    _foreground_observer: RiseSubscription,
    _signals: Task<()>,
    events: mpsc::UnboundedSender<Signal>,
}

/// what reaches the main thread from the OS and from the activation observer
enum Signal {
    DidBecomeActive,
    Authorization(LocationPermission),
}

impl RiseMapLocationPermission {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let (events, mut inbox) = mpsc::unbounded::<Signal>();
        let activation = events.clone();
        let foreground_observer = RiseAppEvents::default_center()
            .observe(UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, move |_| {
                let _ = activation.unbounded_send(Signal::DidBecomeActive);
            });
        let signals = cx.spawn(async move |this, cx| {
            while let Some(signal) = inbox.next().await {
                let updated = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| match signal {
                    Signal::DidBecomeActive => this.refresh(cx),
                    Signal::Authorization(permission) => this.apply_status(permission, cx),
                });
                if updated.is_err() {
                    break;
                }
            }
        });
        Self {
            level: RiseMapLocationPermissionLevel::NotDetermined,
            is_precise: true,
            did_ask_for_always: UserDefaults::standard().bool(MAP_LOCATION_DID_REQUEST_ALWAYS),
            manager: None,
            _foreground_observer: foreground_observer,
            _signals: signals,
            events,
        }
    }

    pub fn level(&self) -> RiseMapLocationPermissionLevel {
        self.level
    }

    pub fn is_precise(&self) -> bool {
        self.is_precise
    }

    pub fn did_ask_for_always(&self) -> bool {
        self.did_ask_for_always
    }

    pub fn access_action_key(&self) -> &'static str {
        location::access_action_key(self.level.authorization(), self.did_ask_for_always)
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let permission = self.resolved_manager();
        self.apply(permission, cx);
    }

    pub fn request_when_in_use(&mut self, cx: &mut Context<Self>) {
        let permission = self.resolved_manager();
        if !matches!(permission, Ok(current) if current.level == LocationAuthorization::NotDetermined) {
            self.apply(permission, cx);
            return;
        }
        if let Err(reason) = location::request_authorization(AuthorizationScope::WhenInUse) {
            tracing::debug!(target: "riseonly", %reason, "location prompt not shown");
        }
    }

    pub fn request_always(&mut self, cx: &mut Context<Self>) {
        let permission = self.resolved_manager();
        if !matches!(permission, Ok(current) if current.level == LocationAuthorization::WhenInUse) {
            self.apply(permission, cx);
            return;
        }
        self.did_ask_for_always = true;
        UserDefaults::standard().set(MAP_LOCATION_DID_REQUEST_ALWAYS, true);
        cx.notify();
        if let Err(reason) = location::request_authorization(AuthorizationScope::Always) {
            tracing::debug!(target: "riseonly", %reason, "location prompt not shown");
        }
    }

    pub fn current_location(&mut self) -> impl Future<Output = Option<LocationFix>> + use<> {
        let permission = self.resolved_manager();
        let allowed = self.level.allows_foreground()
            || matches!(permission, Ok(current) if current.level.allows_foreground());
        let request = allowed.then(location::current_fix);
        async move {
            match request {
                Some(request) => request.await,
                None => None,
            }
        }
    }

    pub fn open_system_settings(&self, cx: &mut App) {
        if !location::open_settings(cx).is_supported() {
            tracing::debug!(target: "riseonly", "this host has no location settings pane");
        }
    }

    fn resolved_manager(&mut self) -> Result<LocationPermission, LocationUnavailable> {
        if self.manager.is_none() {
            let events = self.events.clone();
            let subscribed = location::subscribe(std::sync::Arc::new(move |event: LocationEvent| {
                if let LocationEvent::Authorization(permission) = event {
                    let _ = events.unbounded_send(Signal::Authorization(permission));
                }
            }));
            match subscribed {
                Ok(subscription) => self.manager = Some(subscription),
                Err(reason) => tracing::debug!(target: "riseonly", %reason, "location events unavailable"),
            }
        }
        location::permission()
    }

    fn apply(&mut self, permission: Result<LocationPermission, LocationUnavailable>, cx: &mut Context<Self>) {
        let permission = permission.unwrap_or(LocationPermission {
            level: LocationAuthorization::Restricted,
            is_precise: true,
        });
        self.apply_status(permission, cx);
    }

    fn apply_status(&mut self, permission: LocationPermission, cx: &mut Context<Self>) {
        let next = RiseMapLocationPermissionLevel::from_authorization(permission.level);
        let mut changed = false;
        if self.level != next {
            self.level = next;
            changed = true;
        }
        if self.is_precise != permission.is_precise {
            self.is_precise = permission.is_precise;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }
}
