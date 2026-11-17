use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{AnyWindowHandle, App, Context, Global, Subscription, Task, Window, WindowId};
use rise_platform::window_minimise::{self, MinimiseWatch};

use super::rise_app_events::{RiseAppEvents, RiseNotificationName, UIApplication};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScenePhase {
    Active,
    Inactive,
    Background,
}

/// what the shell observed about one window or the app
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleSignal {
    WindowFocus(WindowId, bool),
    WindowMinimised(WindowId, bool),
    WindowClosed(WindowId),
    Quit,
}

#[derive(Clone, Copy, Debug, Default)]
struct WindowPresence {
    focused: bool,
    minimised: bool,
}

/// pure half, window bookkeeping and the notification sequence for each phase change
#[derive(Debug)]
pub struct AppLifecycleState {
    windows: HashMap<WindowId, WindowPresence>,
    phase: ScenePhase,
    launched: bool,
    terminated: bool,
    terminate_posted: bool,
}

impl Default for AppLifecycleState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppLifecycleState {
    pub fn new() -> Self {
        Self {
            windows: HashMap::new(),
            phase: ScenePhase::Inactive,
            launched: false,
            terminated: false,
            terminate_posted: false,
        }
    }

    pub fn phase(&self) -> ScenePhase {
        self.phase
    }

    pub fn is_terminated(&self) -> bool {
        self.terminated
    }

    /// records a signal without changing the phase, settle publishes the result
    pub fn record(&mut self, signal: LifecycleSignal) {
        if self.terminated {
            return;
        }
        match signal {
            LifecycleSignal::WindowFocus(id, focused) => {
                self.launched = true;
                let presence = self.windows.entry(id).or_default();
                presence.focused = focused;
                if focused {
                    presence.minimised = false;
                }
            }
            LifecycleSignal::WindowMinimised(id, minimised) => {
                self.launched = true;
                let presence = self.windows.entry(id).or_default();
                presence.minimised = minimised;
                if minimised {
                    presence.focused = false;
                }
            }
            LifecycleSignal::WindowClosed(id) => {
                self.windows.remove(&id);
            }
            LifecycleSignal::Quit => {
                self.terminated = true;
            }
        }
    }

    /// moves to the phase the windows describe and returns the notifications to post in order
    pub fn settle(&mut self) -> Vec<RiseNotificationName> {
        if self.terminated {
            if self.terminate_posted {
                return Vec::new();
            }
            self.terminate_posted = true;
            let mut names = transition(self.phase, ScenePhase::Background);
            self.phase = ScenePhase::Background;
            names.push(UIApplication::WILL_TERMINATE_NOTIFICATION);
            return names;
        }
        if !self.launched {
            return Vec::new();
        }
        let target = self.target_phase();
        let names = transition(self.phase, target);
        self.phase = target;
        names
    }

    /// record then settle
    pub fn apply(&mut self, signal: LifecycleSignal) -> Vec<RiseNotificationName> {
        self.record(signal);
        self.settle()
    }

    fn target_phase(&self) -> ScenePhase {
        if self.windows.values().any(|w| w.focused) {
            ScenePhase::Active
        } else if self.windows.values().any(|w| !w.minimised) {
            ScenePhase::Inactive
        } else {
            ScenePhase::Background
        }
    }
}

/// application notifications posted between two scene phases
pub fn transition(from: ScenePhase, to: ScenePhase) -> Vec<RiseNotificationName> {
    use ScenePhase::{Active, Background, Inactive};
    match (from, to) {
        (Active, Inactive) => vec![UIApplication::WILL_RESIGN_ACTIVE_NOTIFICATION],
        (Active, Background) => vec![
            UIApplication::WILL_RESIGN_ACTIVE_NOTIFICATION,
            UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION,
        ],
        (Inactive, Background) => vec![UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION],
        (Background, Inactive) => vec![UIApplication::WILL_ENTER_FOREGROUND_NOTIFICATION],
        (Background, Active) => vec![
            UIApplication::WILL_ENTER_FOREGROUND_NOTIFICATION,
            UIApplication::DID_BECOME_ACTIVE_NOTIFICATION,
        ],
        (Inactive, Active) => vec![UIApplication::DID_BECOME_ACTIVE_NOTIFICATION],
        (Active, Active) | (Inactive, Inactive) | (Background, Background) => Vec::new(),
    }
}

static MINIMISE_GAP_LOGGED: AtomicBool = AtomicBool::new(false);

/// feeds one root window into the lifecycle until dropped, the root view keeps it
/// not send, the minimise observer lives on the window thread
#[must_use = "dropping the tracking stops lifecycle signals for the window"]
pub struct WindowLifecycleTracking {
    _activation: Subscription,
    _minimise: Option<MinimiseWatch>,
    _minimise_pump: Option<Task<()>>,
}

const FOCUS_LOSS_SETTLE: Duration = Duration::from_millis(150);

/// gpui half, a global installed once by the shell and fed by every root window
pub struct RiseAppLifecycle {
    state: AppLifecycleState,
    events: RiseAppEvents,
    pending_settle: Option<Task<()>>,
    _quit: Subscription,
    _window_closed: Subscription,
}

impl Global for RiseAppLifecycle {}

impl RiseAppLifecycle {
    pub fn install(cx: &mut App) {
        Self::install_with(RiseAppEvents::default_center().clone(), cx);
    }

    pub fn install_with(events: RiseAppEvents, cx: &mut App) {
        if cx.has_global::<Self>() {
            return;
        }
        let quit = cx.on_app_quit(|cx| {
            Self::apply(LifecycleSignal::Quit, cx);
            async {}
        });
        let window_closed =
            cx.on_window_closed(|cx, id| Self::apply(LifecycleSignal::WindowClosed(id), cx));
        cx.set_global(Self {
            state: AppLifecycleState::new(),
            events,
            pending_settle: None,
            _quit: quit,
            _window_closed: window_closed,
        });
    }

    pub fn track_window<V: 'static>(
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> WindowLifecycleTracking {
        let id = window.window_handle().window_id();
        Self::apply(LifecycleSignal::WindowFocus(id, window.is_window_active()), cx);
        let activation = cx.observe_window_activation(window, move |_, window, cx| {
            Self::apply(LifecycleSignal::WindowFocus(id, window.is_window_active()), cx);
        });
        let (sender, mut receiver) = mpsc::unbounded::<bool>();
        let watched = window_minimise::watch(&*window, move |minimised| {
            let _ = sender.unbounded_send(minimised);
        });
        let (minimise, minimise_pump) = match watched {
            Ok(watch) => {
                let pump = cx.spawn(async move |_, cx| {
                    while let Some(minimised) = receiver.next().await {
                        cx.update(|cx| {
                            Self::apply(LifecycleSignal::WindowMinimised(id, minimised), cx);
                        });
                    }
                });
                (Some(watch), Some(pump))
            }
            Err(error) => {
                if !MINIMISE_GAP_LOGGED.swap(true, Ordering::Relaxed) {
                    tracing::warn!(target: "riseonly", %error, "window minimise is not observed");
                }
                (None, None)
            }
        };
        WindowLifecycleTracking {
            _activation: activation,
            _minimise: minimise,
            _minimise_pump: minimise_pump,
        }
    }

    /// current scene phase, inactive before the shell installed the lifecycle
    pub fn scene_phase(cx: &App) -> ScenePhase {
        cx.try_global::<Self>()
            .map_or(ScenePhase::Inactive, |this| this.state.phase())
    }

    /// true when the app is active and this window is the key one
    /// so a post in a window behind another stops counting
    pub fn is_foreground_active(window: AnyWindowHandle, cx: &App) -> bool {
        let phase = Self::scene_phase(cx);
        if phase == ScenePhase::Active && cx.active_window() == Some(window) {
            return true;
        }
        phase == ScenePhase::Inactive
            && crate::app::dev_drive::DevDrive::is_stealth(cx)
            && crate::app::dev_drive::target_window(None, cx).is_ok_and(|(_, target)| target == window)
    }

    /// feeds one signal
    /// focus loss or minimise settles after a short delay, every other signal at once
    pub fn apply(signal: LifecycleSignal, cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        let deferred = matches!(
            signal,
            LifecycleSignal::WindowFocus(_, false) | LifecycleSignal::WindowMinimised(_, true)
        );
        let this = cx.global_mut::<Self>();
        this.state.record(signal);
        tracing::warn!(target: "riseonly::X1291", ?signal, deferred, state = ?this.state, "lifecycle signal");
        if !deferred {
            this.pending_settle = None;
            Self::settle_now(cx);
            return;
        }
        let timer = cx.background_executor().timer(FOCUS_LOSS_SETTLE);
        let task = cx.spawn(async move |cx| {
            timer.await;
            cx.update(|cx| {
                if cx.has_global::<Self>() {
                    cx.global_mut::<Self>().pending_settle = None;
                    Self::settle_now(cx);
                }
            });
        });
        cx.global_mut::<Self>().pending_settle = Some(task);
    }

    fn settle_now(cx: &mut App) {
        let this = cx.global_mut::<Self>();
        let names = this.state.settle();
        if !names.is_empty() {
            tracing::warn!(target: "riseonly::X1291", ?names, state = ?this.state, "lifecycle settled");
        }
        let events = this.events.clone();
        for name in names {
            events.post(name);
        }
    }
}

#[cfg(test)]
#[path = "rise_app_lifecycle_tests.rs"]
mod tests;
