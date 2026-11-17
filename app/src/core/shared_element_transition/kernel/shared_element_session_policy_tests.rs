use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementDirection, SharedElementExitStyle, SharedElementOpenStyle, SharedElementPopPanMode,
    SharedElementSessionPolicy, SharedElementSessionPolicyLandingAction, SharedElementSessionPolicyPhase,
};

struct Exit {
    flights: bool,
    capture: bool,
    origin: bool,
    can_return: bool,
    reduce_motion: bool,
}

impl Default for Exit {
    fn default() -> Self {
        Self { flights: true, capture: true, origin: true, can_return: true, reduce_motion: false }
    }
}

fn exit(input: Exit) -> SharedElementExitStyle {
    SharedElementSessionPolicy::exit_style(
        input.flights,
        input.capture,
        input.origin,
        input.can_return,
        input.reduce_motion,
    )
}

struct Open {
    flights: bool,
    route_flies: bool,
    capture: bool,
    reduce_motion: bool,
}

impl Default for Open {
    fn default() -> Self {
        Self { flights: true, route_flies: true, capture: true, reduce_motion: false }
    }
}

fn open(input: Open) -> SharedElementOpenStyle {
    SharedElementSessionPolicy::open_style(input.flights, input.route_flies, input.capture, input.reduce_motion)
}

struct Pan {
    flights: bool,
    interactive: bool,
    destination: bool,
    can_begin: bool,
    origin: bool,
    idle: bool,
    preferred_transition_nil: bool,
    reduce_motion: bool,
}

impl Default for Pan {
    fn default() -> Self {
        Self {
            flights: true,
            interactive: true,
            destination: true,
            can_begin: true,
            origin: true,
            idle: true,
            preferred_transition_nil: true,
            reduce_motion: false,
        }
    }
}

fn pan(input: Pan) -> SharedElementPopPanMode {
    SharedElementSessionPolicy::pop_pan_mode(
        input.flights,
        input.interactive,
        input.destination,
        input.can_begin,
        input.origin,
        input.idle,
        input.preferred_transition_nil,
        input.reduce_motion,
    )
}

#[test]
fn arming_needs_an_idle_phase() {
    assert!(SharedElementSessionPolicy::can_arm(SharedElementSessionPolicyPhase::Idle));
    assert!(!SharedElementSessionPolicy::can_arm(SharedElementSessionPolicyPhase::Armed));
    assert!(!SharedElementSessionPolicy::can_arm(SharedElementSessionPolicyPhase::Running));
    assert!(!SharedElementSessionPolicy::can_arm(SharedElementSessionPolicyPhase::Settling));
}

#[test]
fn freshness_ends_at_the_ttl_boundary() {
    let ttl = SharedElementMotion::INTENT_TTL;
    assert!(SharedElementSessionPolicy::is_fresh(10.0, 10.0, ttl));
    assert!(SharedElementSessionPolicy::is_fresh(10.0, 10.25, ttl));
    assert!(SharedElementSessionPolicy::is_fresh(10.0, 10.4999, ttl));
    assert!(!SharedElementSessionPolicy::is_fresh(10.0, 10.5, ttl));
    assert!(!SharedElementSessionPolicy::is_fresh(10.0, 11.0, ttl));
    assert!(!SharedElementSessionPolicy::is_fresh(10.0, 9.9, ttl));
}

#[test]
fn exit_without_capture_is_stock() {
    assert_eq!(exit(Exit { capture: false, ..Exit::default() }), SharedElementExitStyle::Stock);
    assert_eq!(
        exit(Exit { capture: false, origin: false, can_return: false, ..Exit::default() }),
        SharedElementExitStyle::Stock
    );
}

#[test]
fn exit_without_return_target_is_stock_with_fade() {
    assert_eq!(exit(Exit { origin: false, ..Exit::default() }), SharedElementExitStyle::StockWithFade);
    assert_eq!(exit(Exit { can_return: false, ..Exit::default() }), SharedElementExitStyle::StockWithFade);
    assert_eq!(
        exit(Exit { origin: false, can_return: false, reduce_motion: true, ..Exit::default() }),
        SharedElementExitStyle::StockWithFade
    );
}

#[test]
fn exit_with_return_target_flies_or_crossfades() {
    assert_eq!(exit(Exit::default()), SharedElementExitStyle::Flight);
    assert_eq!(exit(Exit { reduce_motion: true, ..Exit::default() }), SharedElementExitStyle::Crossfade);
}

#[test]
fn exit_with_flights_disabled_is_stock() {
    assert_eq!(exit(Exit { flights: false, ..Exit::default() }), SharedElementExitStyle::Stock);
    assert_eq!(exit(Exit { flights: false, reduce_motion: true, ..Exit::default() }), SharedElementExitStyle::Stock);
}

#[test]
fn open_on_a_route_that_does_not_fly_or_with_flights_disabled_is_stock() {
    assert_eq!(open(Open { route_flies: false, ..Open::default() }), SharedElementOpenStyle::Stock);
    assert_eq!(open(Open { flights: false, ..Open::default() }), SharedElementOpenStyle::Stock);
    assert_eq!(
        open(Open { flights: false, capture: false, reduce_motion: true, ..Open::default() }),
        SharedElementOpenStyle::Stock
    );
}

#[test]
fn open_without_capture_or_under_reduce_motion_crossfades() {
    assert_eq!(open(Open { capture: false, ..Open::default() }), SharedElementOpenStyle::Crossfade);
    assert_eq!(open(Open { reduce_motion: true, ..Open::default() }), SharedElementOpenStyle::Crossfade);
}

#[test]
fn open_with_capture_flies() {
    assert_eq!(open(Open::default()), SharedElementOpenStyle::Flight);
}

#[test]
fn pan_flies_only_when_every_input_holds() {
    assert_eq!(pan(Pan::default()), SharedElementPopPanMode::Flight);
    assert_eq!(pan(Pan { reduce_motion: true, ..Pan::default() }), SharedElementPopPanMode::Crossfade);
}

#[test]
fn pan_with_a_destination_but_a_missing_input_is_stock_armed() {
    assert_eq!(pan(Pan { interactive: false, ..Pan::default() }), SharedElementPopPanMode::StockArmed);
    assert_eq!(pan(Pan { can_begin: false, ..Pan::default() }), SharedElementPopPanMode::StockArmed);
    assert_eq!(pan(Pan { origin: false, ..Pan::default() }), SharedElementPopPanMode::StockArmed);
    assert_eq!(pan(Pan { idle: false, ..Pan::default() }), SharedElementPopPanMode::StockArmed);
    assert_eq!(pan(Pan { preferred_transition_nil: false, ..Pan::default() }), SharedElementPopPanMode::StockArmed);
    assert_eq!(
        pan(Pan { interactive: false, reduce_motion: true, ..Pan::default() }),
        SharedElementPopPanMode::StockArmed
    );
}

#[test]
fn pan_without_a_destination_is_stock() {
    assert_eq!(pan(Pan { destination: false, ..Pan::default() }), SharedElementPopPanMode::Stock);
    assert_eq!(pan(Pan { destination: false, reduce_motion: true, ..Pan::default() }), SharedElementPopPanMode::Stock);
}

#[test]
fn pan_with_flights_disabled_is_stock() {
    assert_eq!(pan(Pan { flights: false, ..Pan::default() }), SharedElementPopPanMode::Stock);
    assert_eq!(pan(Pan { flights: false, interactive: false, ..Pan::default() }), SharedElementPopPanMode::Stock);
}

#[test]
fn landing_crossfades_only_past_the_threshold() {
    let threshold = SharedElementMotion::LANDING_MISMATCH;
    assert_eq!(SharedElementSessionPolicy::landing(0.0, threshold), SharedElementSessionPolicyLandingAction::Land);
    assert_eq!(SharedElementSessionPolicy::landing(1.0, threshold), SharedElementSessionPolicyLandingAction::Land);
    assert_eq!(
        SharedElementSessionPolicy::landing(1.01, threshold),
        SharedElementSessionPolicyLandingAction::CrossfadeReveal
    );
    assert_eq!(
        SharedElementSessionPolicy::landing(f32::INFINITY, threshold),
        SharedElementSessionPolicyLandingAction::CrossfadeReveal
    );
    assert_eq!(
        SharedElementSessionPolicy::landing(f32::NAN, threshold),
        SharedElementSessionPolicyLandingAction::CrossfadeReveal
    );
}

#[test]
fn exit_arm_survives_only_a_single_pop() {
    let direction = SharedElementDirection::Exit;
    assert!(!SharedElementSessionPolicy::path_change_abandons_arm(direction, 3, 2));
    assert!(SharedElementSessionPolicy::path_change_abandons_arm(direction, 3, 3));
    assert!(SharedElementSessionPolicy::path_change_abandons_arm(direction, 3, 4));
    assert!(SharedElementSessionPolicy::path_change_abandons_arm(direction, 3, 1));
    assert!(SharedElementSessionPolicy::path_change_abandons_arm(direction, 3, 0));
}

#[test]
fn open_arm_survives_only_a_single_push() {
    let direction = SharedElementDirection::Open;
    assert!(!SharedElementSessionPolicy::path_change_abandons_arm(direction, 2, 3));
    assert!(SharedElementSessionPolicy::path_change_abandons_arm(direction, 2, 2));
    assert!(SharedElementSessionPolicy::path_change_abandons_arm(direction, 2, 1));
    assert!(SharedElementSessionPolicy::path_change_abandons_arm(direction, 2, 4));
}
