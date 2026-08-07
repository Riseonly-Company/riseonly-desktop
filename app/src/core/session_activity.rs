//! What replaces iOS `scenePhase`, and the inversion at the centre of it.
//!
//! On the phone `.background` fails every pending request and blocks new ones.
//! A desktop app that did that would drop its socket the moment the user
//! alt-tabbed to a browser, then reconnect and resync on the way back — several
//! times a minute. So `setApplicationActive(false)` here changes presence and
//! speculation and nothing else: the connection and the in-flight requests are
//! not the OS's business.
//!
//! There is also not one signal but four — window focus, minimised, system idle,
//! screen locked — plus the window count, because on macOS an app with no window
//! at all is a normal running state. Pure policy: no gpui, no OS, no clock.

// This is a binary crate, so an entry point with no caller yet is dead code and
// fails the build. The policy lands before the first store by design; the
// allowance goes when the shell has wired every one of them.
#![allow(dead_code)]

/// Everything the shell can learn about the user's attention.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActivitySignal {
    WindowFocus(bool),
    Minimised(bool),
    SystemIdle(bool),
    ScreenLocked(bool),
    WindowCount(usize),
}

/// One variant, on purpose. No combination of activity signals may disconnect
/// the socket: realtime events are the product's freshness contract, and a
/// reconnect costs a handshake, a resume token and a resync of every open
/// resource. If a second variant ever appears here, it has to be justified by
/// something other than "the window is not in front".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SocketPolicy {
    KeepConnected,
}

/// One variant, on purpose, and it is the iOS inversion stated as a type. A
/// request in flight when the window loses focus is still a request the user
/// asked for; failing it would make the reply the user came back for depend on
/// where their mouse was.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PendingRequestPolicy {
    Keep,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PresenceState {
    Active,
    Away,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NotificationDelivery {
    Deliver,
    Suppress,
}

/// The default is the state the app launches in: one window, focused, awake.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SessionActivity {
    focused: bool,
    minimised: bool,
    idle: bool,
    locked: bool,
    windows: usize,
}

impl Default for SessionActivity {
    fn default() -> Self {
        Self {
            focused: true,
            minimised: false,
            idle: false,
            locked: false,
            windows: 1,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ActivityPolicy {
    pub socket: SocketPolicy,
    pub pending_requests: PendingRequestPolicy,
    pub presence: PresenceState,
    pub speculative_work: bool,
    pub decorative_animation: bool,
}

impl SessionActivity {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, signal: ActivitySignal) -> bool {
        let before = *self;
        match signal {
            ActivitySignal::WindowFocus(focused) => self.focused = focused,
            ActivitySignal::Minimised(minimised) => self.minimised = minimised,
            ActivitySignal::SystemIdle(idle) => self.idle = idle,
            ActivitySignal::ScreenLocked(locked) => self.locked = locked,
            ActivitySignal::WindowCount(windows) => self.windows = windows,
        }
        *self != before
    }

    pub fn policy(&self) -> ActivityPolicy {
        ActivityPolicy {
            socket: SocketPolicy::KeepConnected,
            pending_requests: PendingRequestPolicy::Keep,
            presence: self.presence(),
            speculative_work: self.is_user_present(),
            decorative_animation: self.is_user_present(),
        }
    }

    /// What other people are told. Focus is part of it — a window sitting behind
    /// a browser is not somewhere the user is reading — and so is the window
    /// count: closing the last window on macOS leaves the app frontmost with its
    /// menu bar and nothing to look at, so focus alone would report a lie.
    pub fn presence(&self) -> PresenceState {
        if self.focused && !self.idle && !self.locked && self.windows > 0 {
            PresenceState::Active
        } else {
            PresenceState::Away
        }
    }

    /// Whether the app has pixels a human could be looking at. Deliberately not
    /// a question about focus: an unfocused window on a second monitor is still
    /// on screen, and freezing its animations would be visible.
    pub fn is_user_present(&self) -> bool {
        self.windows > 0 && !self.minimised && !self.idle && !self.locked
    }

    /// Suppression asks whether this window is in front with the resource on
    /// screen — the desktop restatement of "no push for the chat you are
    /// reading". `target_is_visible` is `ActiveScreens::suppresses_notification`,
    /// composed here rather than inside either half.
    pub fn notification_delivery(&self, target_is_visible: bool) -> NotificationDelivery {
        if target_is_visible && self.focused && !self.minimised && !self.locked && self.windows > 0
        {
            NotificationDelivery::Suppress
        } else {
            NotificationDelivery::Deliver
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activity(focused: bool, minimised: bool, idle: bool, locked: bool) -> SessionActivity {
        let mut activity = SessionActivity::new();
        activity.apply(ActivitySignal::WindowFocus(focused));
        activity.apply(ActivitySignal::Minimised(minimised));
        activity.apply(ActivitySignal::SystemIdle(idle));
        activity.apply(ActivitySignal::ScreenLocked(locked));
        activity
    }

    fn every_combination() -> Vec<SessionActivity> {
        let mut all = Vec::new();
        for focused in [false, true] {
            for minimised in [false, true] {
                for idle in [false, true] {
                    for locked in [false, true] {
                        all.push(activity(focused, minimised, idle, locked));
                    }
                }
            }
        }
        all
    }

    #[test]
    fn the_socket_is_kept_in_every_combination_of_the_four_signals() {
        let all = every_combination();
        assert_eq!(all.len(), 16);
        for activity in all {
            for windows in [0, 1, 3] {
                let mut activity = activity;
                activity.apply(ActivitySignal::WindowCount(windows));
                assert_eq!(
                    activity.policy().socket,
                    SocketPolicy::KeepConnected,
                    "{activity:?} dropped the socket; a desktop app keeps it while unfocused"
                );
            }
        }
    }

    #[test]
    fn pending_requests_are_never_failed_by_an_activity_change() {
        for activity in every_combination() {
            assert_eq!(
                activity.policy().pending_requests,
                PendingRequestPolicy::Keep,
                "{activity:?} failed in-flight requests, which is the iOS behaviour, not ours"
            );
        }
    }

    #[test]
    fn presence_is_active_only_when_focused_and_awake_and_unlocked() {
        assert_eq!(
            activity(true, false, false, false).presence(),
            PresenceState::Active
        );
        assert_eq!(
            activity(false, false, false, false).presence(),
            PresenceState::Away
        );
        assert_eq!(
            activity(true, false, true, false).presence(),
            PresenceState::Away
        );
        assert_eq!(
            activity(true, false, false, true).presence(),
            PresenceState::Away
        );
        assert_eq!(
            activity(true, false, false, false).policy().presence,
            PresenceState::Active
        );
    }

    #[test]
    fn speculative_work_and_decorative_animation_stop_when_the_user_is_not_present() {
        for (state, present) in [
            (activity(true, false, false, false), true),
            (activity(true, true, false, false), false),
            (activity(true, false, true, false), false),
            (activity(true, false, false, true), false),
        ] {
            let policy = state.policy();
            assert_eq!(state.is_user_present(), present, "{state:?}");
            assert_eq!(policy.speculative_work, present, "{state:?}");
            assert_eq!(policy.decorative_animation, present, "{state:?}");
        }
    }

    #[test]
    fn an_unfocused_but_visible_window_keeps_animating() {
        let state = activity(false, false, false, false);
        assert!(state.is_user_present());
        assert!(state.policy().decorative_animation);
        assert_eq!(state.presence(), PresenceState::Away);
    }

    #[test]
    fn a_notification_is_suppressed_only_while_its_target_is_on_a_focused_window() {
        let focused = activity(true, false, false, false);
        assert_eq!(
            focused.notification_delivery(true),
            NotificationDelivery::Suppress
        );
        assert_eq!(
            focused.notification_delivery(false),
            NotificationDelivery::Deliver
        );

        for state in [
            activity(false, false, false, false),
            activity(true, true, false, false),
            activity(true, false, false, true),
        ] {
            assert_eq!(
                state.notification_delivery(true),
                NotificationDelivery::Deliver,
                "{state:?} is not a user reading the conversation"
            );
        }
    }

    #[test]
    fn an_app_with_no_windows_is_away_and_still_connected() {
        let mut state = SessionActivity::new();
        state.apply(ActivitySignal::WindowCount(0));

        assert!(!state.is_user_present());
        assert_eq!(state.presence(), PresenceState::Away);
        assert_eq!(state.policy().socket, SocketPolicy::KeepConnected);
        assert_eq!(
            state.notification_delivery(true),
            NotificationDelivery::Deliver
        );
    }

    #[test]
    fn apply_reports_whether_anything_changed() {
        let mut state = SessionActivity::new();
        assert!(!state.apply(ActivitySignal::WindowFocus(true)));
        assert!(state.apply(ActivitySignal::WindowFocus(false)));
        assert!(!state.apply(ActivitySignal::WindowFocus(false)));
        assert!(state.apply(ActivitySignal::WindowCount(2)));
        assert!(!state.apply(ActivitySignal::WindowCount(2)));
    }
}
