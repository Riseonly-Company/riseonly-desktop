use std::any::Any;
use std::fmt;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use parking_lot::Mutex;

/// name of a notification, declared as a const next to the code that posts it
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RiseNotificationName(&'static str);

impl RiseNotificationName {
    pub const fn new(raw_value: &'static str) -> Self {
        Self(raw_value)
    }

    pub const fn raw_value(&self) -> &'static str {
        self.0
    }
}

impl fmt::Display for RiseNotificationName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// notification names for app lifecycle events
pub struct UIApplication;

impl UIApplication {
    pub const DID_BECOME_ACTIVE_NOTIFICATION: RiseNotificationName =
        RiseNotificationName::new("UIApplicationDidBecomeActiveNotification");
    pub const WILL_RESIGN_ACTIVE_NOTIFICATION: RiseNotificationName =
        RiseNotificationName::new("UIApplicationWillResignActiveNotification");
    pub const DID_ENTER_BACKGROUND_NOTIFICATION: RiseNotificationName =
        RiseNotificationName::new("UIApplicationDidEnterBackgroundNotification");
    pub const WILL_ENTER_FOREGROUND_NOTIFICATION: RiseNotificationName =
        RiseNotificationName::new("UIApplicationWillEnterForegroundNotification");
    pub const WILL_TERMINATE_NOTIFICATION: RiseNotificationName =
        RiseNotificationName::new("UIApplicationWillTerminateNotification");
    pub const DID_RECEIVE_MEMORY_WARNING_NOTIFICATION: RiseNotificationName =
        RiseNotificationName::new("UIApplicationDidReceiveMemoryWarningNotification");
}

/// keyboard notification names
/// desktop keyboard is always attached so WILL_SHOW fires when the first text field gets focus
/// and WILL_HIDE when none has it
pub struct UIResponder;

impl UIResponder {
    pub const KEYBOARD_WILL_SHOW_NOTIFICATION: RiseNotificationName =
        RiseNotificationName::new("UIKeyboardWillShowNotification");
    pub const KEYBOARD_WILL_HIDE_NOTIFICATION: RiseNotificationName =
        RiseNotificationName::new("UIKeyboardWillHideNotification");
}

/// notification names posted when a system accessibility setting changes
pub struct UIAccessibility;

impl UIAccessibility {
    pub const REDUCE_MOTION_STATUS_DID_CHANGE_NOTIFICATION: RiseNotificationName =
        RiseNotificationName::new("UIAccessibilityReduceMotionStatusDidChangeNotification");
}

/// one delivered notification, name plus optional object and payload
#[derive(Clone)]
pub struct RiseNotification {
    pub name: RiseNotificationName,
    payload: Option<Arc<dyn Any + Send + Sync>>,
}

impl RiseNotification {
    pub fn new(name: RiseNotificationName) -> Self {
        Self {
            name,
            payload: None,
        }
    }

    pub fn with_payload<P: Any + Send + Sync>(name: RiseNotificationName, payload: P) -> Self {
        Self {
            name,
            payload: Some(Arc::new(payload)),
        }
    }

    /// payload as type P, none if absent or another type
    pub fn payload<P: Any + Send + Sync>(&self) -> Option<&P> {
        self.payload.as_deref().and_then(|p| p.downcast_ref::<P>())
    }

    pub fn has_payload(&self) -> bool {
        self.payload.is_some()
    }
}

impl fmt::Debug for RiseNotification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RiseNotification")
            .field("name", &self.name)
            .field("has_payload", &self.payload.is_some())
            .finish()
    }
}

type Callback = Arc<dyn Fn(&RiseNotification) + Send + Sync>;

struct Observer {
    id: u64,
    name: Option<RiseNotificationName>,
    alive: Arc<AtomicBool>,
    callback: Callback,
}

struct Registry {
    next_id: AtomicU64,
    observers: Mutex<Vec<Observer>>,
}

impl Registry {
    fn remove(&self, id: u64) {
        self.observers.lock().retain(|o| o.id != id);
    }
}

/// removes its observer on drop
#[must_use = "dropping the subscription removes the observer at once"]
pub struct RiseSubscription {
    id: u64,
    alive: Arc<AtomicBool>,
    registry: Arc<Registry>,
}

impl RiseSubscription {
    /// keeps the observer for the whole process life
    pub fn detach(self) {
        std::mem::forget(self);
    }
}

impl Drop for RiseSubscription {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        self.registry.remove(self.id);
    }
}

impl fmt::Debug for RiseSubscription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RiseSubscription").field("id", &self.id).finish()
    }
}

static DEFAULT_CENTER: LazyLock<RiseAppEvents> = LazyLock::new(RiseAppEvents::new);

/// notification center, product code uses default_center and tests build their own
#[derive(Clone)]
pub struct RiseAppEvents {
    registry: Arc<Registry>,
}

impl Default for RiseAppEvents {
    fn default() -> Self {
        Self::new()
    }
}

impl RiseAppEvents {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(Registry {
                next_id: AtomicU64::new(1),
                observers: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn default_center() -> &'static RiseAppEvents {
        &DEFAULT_CENTER
    }

    pub fn post(&self, name: RiseNotificationName) {
        self.post_notification(&RiseNotification::new(name));
    }

    pub fn post_with<P: Any + Send + Sync>(&self, name: RiseNotificationName, payload: P) {
        self.post_notification(&RiseNotification::with_payload(name, payload));
    }

    pub fn post_notification(&self, notification: &RiseNotification) {
        let targets: Vec<(Arc<AtomicBool>, Callback)> = self
            .registry
            .observers
            .lock()
            .iter()
            .filter(|o| o.name.is_none_or(|n| n == notification.name))
            .map(|o| (Arc::clone(&o.alive), Arc::clone(&o.callback)))
            .collect();
        for (alive, callback) in targets {
            if alive.load(Ordering::Acquire) {
                callback(notification);
            }
        }
    }

    /// observes one name, the callback runs on the posting thread
    pub fn observe(
        &self,
        name: RiseNotificationName,
        callback: impl Fn(&RiseNotification) + Send + Sync + 'static,
    ) -> RiseSubscription {
        self.insert(Some(name), Arc::new(callback))
    }

    /// observes every notification posted to this center
    pub fn observe_all(
        &self,
        callback: impl Fn(&RiseNotification) + Send + Sync + 'static,
    ) -> RiseSubscription {
        self.insert(None, Arc::new(callback))
    }

    pub fn observer_count(&self, name: RiseNotificationName) -> usize {
        self.registry
            .observers
            .lock()
            .iter()
            .filter(|o| o.name.is_none_or(|n| n == name))
            .count()
    }

    fn insert(&self, name: Option<RiseNotificationName>, callback: Callback) -> RiseSubscription {
        let id = self.registry.next_id.fetch_add(1, Ordering::Relaxed);
        let alive = Arc::new(AtomicBool::new(true));
        self.registry.observers.lock().push(Observer {
            id,
            name,
            alive: Arc::clone(&alive),
            callback,
        });
        RiseSubscription {
            id,
            alive,
            registry: Arc::clone(&self.registry),
        }
    }
}
