use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementDirection, SharedElementExitStyle, SharedElementOpenStyle, SharedElementPopPanMode,
    SharedElementSessionPolicy, SharedElementSessionPolicyLandingAction, SharedElementSessionPolicyPhase,
};

impl SharedElementSessionPolicy {
    /// one session at a time, arming is refused while anything is armed or running or settling
    pub fn can_arm(phase: SharedElementSessionPolicyPhase) -> bool {
        phase == SharedElementSessionPolicyPhase::Idle
    }

    /// fresh strictly before ttl has elapsed, a clock that went backwards is not fresh
    /// times are monotonic seconds
    pub fn is_fresh(armed_at: f64, now: f64, ttl: f64) -> bool {
        let elapsed = now - armed_at;
        elapsed >= 0.0 && elapsed < ttl
    }

    /// picks the open style, route_flies means the pushed route is one the delegate animators take
    pub fn open_style(
        flights_enabled: bool,
        route_flies: bool,
        has_capture: bool,
        reduce_motion: bool,
    ) -> SharedElementOpenStyle {
        if !flights_enabled || !route_flies {
            return SharedElementOpenStyle::Stock;
        }
        if !has_capture {
            return SharedElementOpenStyle::Crossfade;
        }
        if reduce_motion { SharedElementOpenStyle::Crossfade } else { SharedElementOpenStyle::Flight }
    }

    pub fn exit_style(
        flights_enabled: bool,
        has_capture: bool,
        has_origin: bool,
        can_return: bool,
        reduce_motion: bool,
    ) -> SharedElementExitStyle {
        if !flights_enabled || !has_capture {
            return SharedElementExitStyle::Stock;
        }
        if !has_origin || !can_return {
            return SharedElementExitStyle::StockWithFade;
        }
        if reduce_motion { SharedElementExitStyle::Crossfade } else { SharedElementExitStyle::Flight }
    }

    /// latched once per back pan from registrations only
    /// with flights disabled every pan is Stock so the kill switch never arms an exit
    #[allow(clippy::too_many_arguments)]
    pub fn pop_pan_mode(
        flights_enabled: bool,
        interactive_enabled: bool,
        has_destination: bool,
        can_begin: bool,
        has_origin: bool,
        phase_idle: bool,
        preferred_transition_nil: bool,
        reduce_motion: bool,
    ) -> SharedElementPopPanMode {
        if !flights_enabled {
            return SharedElementPopPanMode::Stock;
        }
        let can_drive = interactive_enabled
            && has_destination
            && can_begin
            && has_origin
            && phase_idle
            && preferred_transition_nil;
        if can_drive {
            return if reduce_motion { SharedElementPopPanMode::Crossfade } else { SharedElementPopPanMode::Flight };
        }
        if has_destination { SharedElementPopPanMode::StockArmed } else { SharedElementPopPanMode::Stock }
    }

    /// landing action from the mismatch, a NaN mismatch counts as a miss
    pub fn landing(mismatch: f32, threshold: f32) -> SharedElementSessionPolicyLandingAction {
        if mismatch <= threshold {
            SharedElementSessionPolicyLandingAction::Land
        } else {
            SharedElementSessionPolicyLandingAction::CrossfadeReveal
        }
    }

    /// only the navigation the intent was armed for keeps it
    /// exactly one pop for an exit or one push for an open
    /// any other path change abandons it even a same count replacement
    pub fn path_change_abandons_arm(direction: SharedElementDirection, count_at_arm: usize, new_count: usize) -> bool {
        match direction {
            SharedElementDirection::Open => count_at_arm.checked_add(1) != Some(new_count),
            SharedElementDirection::Exit => count_at_arm.checked_sub(1) != Some(new_count),
        }
    }
}
