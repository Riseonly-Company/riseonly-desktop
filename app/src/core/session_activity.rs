#![allow(dead_code)]

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActivitySignal {
    WindowFocus(bool),
    Minimised(bool),
    SystemIdle(bool),
    ScreenLocked(bool),
    WindowCount(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SocketPolicy {
    KeepConnected,
}

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

    // macOS keeps an app frontmost with zero windows, so focus alone would report presence.
    pub fn presence(&self) -> PresenceState {
        if self.focused && !self.idle && !self.locked && self.windows > 0 {
            PresenceState::Active
        } else {
            PresenceState::Away
        }
    }

    pub fn is_user_present(&self) -> bool {
        self.windows > 0 && !self.minimised && !self.idle && !self.locked
    }

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
