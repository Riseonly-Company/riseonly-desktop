use std::time::Duration;

use gpui::{
    AnyElement, Bounds, Context, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    TestAppContext, Window, div, point, px, size,
};

use crate::swiftui::environment::LayoutDirection;
use crate::swiftui::motion::{ease_in, ease_out, linear};
use crate::swiftui::transition::{
    ContentHeightMotion, DEFAULT_TRANSITION_SCALE, Edge, Presence, Transition, TransitionEffect, TransitionModifiers,
    TransitionPhase, UnitPoint, multiply_opacity,
};

const LTR: LayoutDirection = LayoutDirection::LeftToRight;
const RTL: LayoutDirection = LayoutDirection::RightToLeft;

fn at(seconds: f64) -> Duration {
    Duration::from_secs_f64(seconds)
}

fn assert_close(actual: f32, expected: f32, what: &str) {
    assert!((actual - expected).abs() <= 1e-4, "{what}: expected {expected}, got {actual}");
}

#[test]
fn opacity_fades_to_nothing_at_the_active_state() {
    let opacity = Transition::opacity();
    assert_close(opacity.effect(TransitionPhase::Removal, 0.0, LTR).opacity, 1.0, "in place");
    assert_close(opacity.effect(TransitionPhase::Removal, 0.25, LTR).opacity, 0.75, "a quarter");
    assert_close(opacity.effect(TransitionPhase::Insertion, 1.0, LTR).opacity, 0.0, "active");
}

#[test]
fn a_spring_past_the_target_does_not_drive_opacity_out_of_range() {
    let opacity = Transition::opacity();
    assert_close(opacity.effect(TransitionPhase::Insertion, -0.2, LTR).opacity, 1.0, "overshoot");
    assert_close(opacity.effect(TransitionPhase::Removal, 1.2, LTR).opacity, 0.0, "undershoot");
}

#[test]
fn move_travels_by_the_view_size_towards_the_edge() {
    let offset = |edge, direction| Transition::move_edge(edge).effect(TransitionPhase::Removal, 1.0, direction).offset;
    assert_eq!(offset(Edge::Top, LTR), point(0.0, -1.0));
    assert_eq!(offset(Edge::Bottom, LTR), point(0.0, 1.0));
    assert_eq!(offset(Edge::Leading, LTR), point(-1.0, 0.0));
    assert_eq!(offset(Edge::Trailing, LTR), point(1.0, 0.0));
    let half = Transition::move_edge(Edge::Bottom).effect(TransitionPhase::Insertion, 0.5, LTR);
    assert_eq!(half.offset, point(0.0, 0.5));
    assert_close(half.opacity, 1.0, "move alone keeps opacity");
}

#[test]
fn leading_and_trailing_follow_a_right_to_left_layout() {
    let offset = |edge| Transition::move_edge(edge).effect(TransitionPhase::Removal, 1.0, RTL).offset;
    assert_eq!(offset(Edge::Leading), point(1.0, 0.0));
    assert_eq!(offset(Edge::Trailing), point(-1.0, 0.0));
    let anchor = Transition::scale_anchored(0.5, UnitPoint::TOP_LEADING)
        .effect(TransitionPhase::Removal, 1.0, RTL)
        .anchor;
    assert_eq!(anchor, UnitPoint::TOP_TRAILING);
}

#[test]
fn scale_runs_from_the_given_scale_about_its_anchor() {
    let scale = Transition::scale_anchored(0.42, UnitPoint::TOP);
    let active = scale.effect(TransitionPhase::Insertion, 1.0, LTR);
    assert_close(active.scale, 0.42, "active scale");
    assert_eq!(active.anchor, UnitPoint::TOP);
    assert_close(scale.effect(TransitionPhase::Insertion, 0.5, LTR).scale, 0.71, "halfway");
    assert_close(
        Transition::scale_default().effect(TransitionPhase::Insertion, 1.0, LTR).scale,
        DEFAULT_TRANSITION_SCALE,
        "bare .scale",
    );
}

#[test]
fn a_shrinking_scale_fades_by_its_factor() {
    let active = Transition::scale_anchored(0.42, UnitPoint::TOP)
        .effect(TransitionPhase::Insertion, 1.0, LTR);
    assert_close(active.drawn_opacity(), 0.42, "scale 0.42");
    let both = Transition::scale_default()
        .combined(Transition::opacity())
        .effect(TransitionPhase::Removal, 0.5, LTR);
    assert_close(both.drawn_opacity(), 0.25, "scale and opacity halfway");
    let overshoot = TransitionEffect {
        scale: 1.1,
        ..TransitionEffect::IDENTITY
    };
    assert_close(overshoot.drawn_opacity(), 1.0, "overshoot");
}

#[test]
fn a_shrinking_scale_covers_the_rectangle_about_its_anchor() {
    let bounds = Bounds {
        origin: point(px(100.), px(50.)),
        size: size(px(200.), px(100.)),
    };
    let top = TransitionEffect {
        scale: 0.5,
        anchor: UnitPoint::TOP,
        ..TransitionEffect::IDENTITY
    };
    let covered = top.scaled_bounds(bounds);
    assert_eq!(covered.origin, point(px(150.), px(50.)));
    assert_eq!(covered.size, size(px(100.), px(50.)));
    let trailing = TransitionEffect {
        scale: 0.5,
        anchor: UnitPoint::BOTTOM_TRAILING,
        ..TransitionEffect::IDENTITY
    };
    assert_eq!(trailing.scaled_bounds(bounds).origin, point(px(200.), px(100.)));
    let grown = TransitionEffect {
        scale: 1.2,
        ..TransitionEffect::IDENTITY
    };
    assert_eq!(grown.scaled_bounds(bounds), bounds);
}

#[test]
fn combined_applies_both_modifiers() {
    let effect = Transition::move_edge(Edge::Bottom)
        .combined(Transition::opacity())
        .effect(TransitionPhase::Removal, 0.5, LTR);
    assert_eq!(effect.offset, point(0.0, 0.5));
    assert_close(effect.opacity, 0.5, "opacity");
    let scaled = Transition::opacity()
        .combined(Transition::scale_anchored(0.96, UnitPoint::TOP))
        .effect(TransitionPhase::Insertion, 1.0, LTR);
    assert_close(scaled.scale, 0.96, "scale");
    assert_eq!(scaled.anchor, UnitPoint::TOP);
    assert_close(scaled.opacity, 0.0, "opacity");
}

#[test]
fn asymmetric_picks_the_half_for_the_phase() {
    let pinned = Transition::asymmetric(
        Transition::move_edge(Edge::Top).combined(Transition::opacity()),
        Transition::move_edge(Edge::Bottom).combined(Transition::opacity()),
    );
    assert_eq!(pinned.effect(TransitionPhase::Insertion, 1.0, LTR).offset, point(0.0, -1.0));
    assert_eq!(pinned.effect(TransitionPhase::Removal, 1.0, LTR).offset, point(0.0, 1.0));
}

#[test]
fn identity_never_changes_the_view() {
    assert!(Transition::identity().effect(TransitionPhase::Removal, 1.0, LTR).is_identity());
    assert_eq!(Transition::default(), Transition::opacity());
}

#[test]
fn a_transition_animation_overrides_the_transaction_per_phase() {
    let filmstrip = Transition::asymmetric(
        Transition::opacity().animation(ease_out(0.24)),
        Transition::opacity().animation(ease_in(0.14)),
    );
    assert_eq!(filmstrip.animation_for(TransitionPhase::Insertion), Some(ease_out(0.24)));
    assert_eq!(filmstrip.animation_for(TransitionPhase::Removal), Some(ease_in(0.14)));
    assert_eq!(Transition::opacity().animation_for(TransitionPhase::Removal), None);
}

#[test]
fn the_first_value_appears_without_a_transition() {
    let presence = Presence::new(Some(7));
    assert_eq!(presence.value(at(0.0)), Some(&7));
    assert!(!presence.is_running(at(0.0)));
    assert!(presence.effect_in(at(0.0), LTR).is_identity());
    assert_eq!(Presence::<i32>::new(None).value(at(0.0)), None);
}

#[test]
fn a_removal_keeps_the_value_until_it_has_run() {
    let mut presence = Presence::new(Some("toast"));
    presence.set(None, linear(1.0), at(10.0), false);
    assert!(!presence.is_present());
    assert!(presence.is_leaving(at(10.5)));
    assert_eq!(presence.value(at(10.5)), Some(&"toast"));
    let state = presence.state(at(10.5));
    assert_eq!(state.phase, TransitionPhase::Removal);
    assert_close(state.progress, 0.5, "halfway out");
    assert_close(presence.effect_in(at(10.5), LTR).opacity, 0.5, "halfway faded");
    assert_eq!(presence.value(at(11.0)), None);
    assert!(!presence.is_running(at(11.0)));
}

#[test]
fn an_insertion_runs_from_the_active_state() {
    let mut presence = Presence::new(None).with_transition(Transition::move_edge(Edge::Bottom));
    presence.set(Some(1), linear(1.0), at(0.0), false);
    assert_eq!(presence.value(at(0.0)), Some(&1));
    assert_close(presence.effect_in(at(0.0), LTR).offset.y, 1.0, "starts a full height down");
    assert_close(presence.effect_in(at(0.25), LTR).offset.y, 0.75, "a quarter in");
    assert!(presence.effect_in(at(1.0), LTR).is_identity());
}

#[test]
fn a_value_set_again_mid_removal_returns_from_where_it_is() {
    let mut presence = Presence::new(Some(1));
    presence.set(None, linear(1.0), at(0.0), false);
    let on_screen = presence.state(at(0.4)).progress;
    assert_close(on_screen, 0.6, "progress at the interruption");

    presence.set(Some(2), linear(1.0), at(0.4), false);
    assert!(presence.is_present());
    assert_eq!(presence.value(at(0.4)), Some(&2));
    let state = presence.state(at(0.4));
    assert_eq!(state.phase, TransitionPhase::Insertion);
    assert_close(state.progress, on_screen, "no jump on re-entry");
    assert_close(presence.state(at(0.9)).progress, 0.8, "rising again");
    assert_close(presence.state(at(1.4)).progress, 1.0, "back in place");
}

#[test]
fn without_an_animation_or_with_reduce_motion_the_change_lands_at_once() {
    let mut presence = Presence::new(Some(1));
    presence.set(None, None, at(0.0), false);
    assert_eq!(presence.value(at(0.0)), None);

    presence.set(Some(2), linear(1.0), at(1.0), true);
    assert!(!presence.is_running(at(1.0)));
    assert_close(presence.state(at(1.0)).progress, 1.0, "inserted at once");
}

#[test]
fn the_transition_animation_wins_over_the_call_site() {
    let mut presence = Presence::new(Some(1)).with_transition(Transition::opacity().animation(linear(0.2)));
    presence.set(None, linear(5.0), at(0.0), false);
    assert_eq!(presence.value(at(0.1)), Some(&1));
    assert_eq!(presence.value(at(0.2)), None);
}

#[test]
fn replacing_a_present_value_starts_no_transition() {
    let mut presence = Presence::new(Some(1));
    presence.set(Some(2), linear(1.0), at(0.0), false);
    assert_eq!(presence.value(at(0.0)), Some(&2));
    assert!(!presence.is_running(at(0.0)));
}

const ROOT: &str = "root";
const MOVED: &str = "moved";

struct Probe {
    build: fn() -> AnyElement,
}

impl Render for Probe {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| ROOT.to_string())
            .flex()
            .flex_col()
            .items_start()
            .w(px(400.))
            .h(px(400.))
            .child((self.build)())
            .child(div().debug_selector(|| "after".to_string()).w(px(10.)).h(px(10.)))
    }
}

#[gpui::test]
fn the_effect_moves_the_drawing_but_not_the_layout_slot(cx: &mut TestAppContext) {
    let (_, cx) = cx.add_window_view(|_, _| Probe {
        build: || {
            let effect = TransitionEffect {
                offset: point(0.5, -0.25),
                ..TransitionEffect::IDENTITY
            };
            div()
                .debug_selector(|| MOVED.to_string())
                .w(px(80.))
                .h(px(40.))
                .transition_effect(effect)
                .into_any_element()
        },
    });
    cx.run_until_parked();
    let root = cx.debug_bounds(ROOT).expect("probe root was laid out");
    let moved = cx.debug_bounds(MOVED).expect("moved element was laid out");
    let after = cx.debug_bounds("after").expect("sibling was laid out");
    assert_eq!(moved.origin.x - root.origin.x, px(40.));
    assert_eq!(moved.origin.y - root.origin.y, px(-10.));
    assert_eq!(moved.size.width, px(80.));
    assert_eq!(after.origin.y - root.origin.y, px(40.));
}

#[test]
fn the_effect_multiplies_the_element_opacity() {
    let mut faded = div().opacity(0.5);
    multiply_opacity(&mut faded, 0.5);
    assert_eq!(faded.style().opacity, Some(0.25));
    let mut plain = div();
    multiply_opacity(&mut plain, 0.3);
    assert_eq!(plain.style().opacity, Some(0.3));
}

#[gpui::test]
fn a_scale_effect_keeps_the_layout_slot_and_fades(cx: &mut TestAppContext) {
    let (_, cx) = cx.add_window_view(|_, _| Probe {
        build: || {
            div()
                .debug_selector(|| MOVED.to_string())
                .w(px(80.))
                .h(px(40.))
                .scale_effect_anchored(0.5, UnitPoint::TOP_LEADING)
                .into_any_element()
        },
    });
    cx.run_until_parked();
    let root = cx.debug_bounds(ROOT).expect("probe root was laid out");
    let scaled = cx.debug_bounds(MOVED).expect("scaled element was laid out");
    let after = cx.debug_bounds("after").expect("sibling was laid out");
    assert_eq!(scaled.origin, root.origin);
    assert_eq!(scaled.size.width, px(80.));
    assert_eq!(after.origin.y - root.origin.y, px(40.));
}

#[test]
fn a_scale_effect_carries_its_scale_as_data() {
    let element = div().w(px(10.)).scale_effect(0.96);
    assert_close(element.effect().scale, 0.96, "scale");
    assert_eq!(element.effect().anchor, UnitPoint::CENTER);
}

#[test]
fn a_new_key_sends_the_old_value_out_while_the_new_one_comes_in() {
    let mut presence = Presence::new(Some((1, "old"))).with_transition(Transition::opacity());
    presence.set_keyed(Some((2, "new")), |value| value.0, linear(1.0), at(10.0), false);
    assert_eq!(presence.value(at(10.5)), Some(&(2, "new")));
    let state = presence.state(at(10.5));
    assert_eq!(state.phase, TransitionPhase::Insertion);
    assert_close(state.progress, 0.5, "new value halfway in");
    let departing: Vec<_> = presence.departing_in(at(10.5), LTR).collect();
    assert_eq!(departing.len(), 1);
    assert_eq!(departing[0].0, &(1, "old"));
    assert_close(departing[0].1.opacity, 0.5, "old value halfway out");
    assert!(presence.is_running(at(10.5)));
    assert_eq!(presence.departing_in(at(11.0), LTR).count(), 0);
    assert!(!presence.is_running(at(11.0)));
}

#[test]
fn the_same_key_replaces_the_value_without_a_swap() {
    let mut presence = Presence::new(Some((1, "old"))).with_transition(Transition::opacity());
    presence.set_keyed(Some((1, "new")), |value| value.0, linear(1.0), at(0.0), false);
    assert_eq!(presence.value(at(0.0)), Some(&(1, "new")));
    assert_eq!(presence.departing_in(at(0.0), LTR).count(), 0);
    assert!(!presence.is_running(at(0.0)));
}

#[test]
fn a_new_key_without_a_spec_lands_at_once() {
    let mut presence = Presence::new(Some(1)).with_transition(Transition::opacity());
    presence.set_keyed(Some(2), |value| *value, None, at(0.0), false);
    assert_eq!(presence.departing_in(at(0.0), LTR).count(), 0);
    assert_close(presence.state(at(0.0)).progress, 1.0, "no spec");
    presence.set_keyed(Some(3), |value| *value, linear(1.0), at(0.0), true);
    assert_eq!(presence.departing_in(at(0.0), LTR).count(), 0);
    assert_close(presence.state(at(0.0)).progress, 1.0, "reduce motion");
    assert_eq!(presence.value(at(0.0)), Some(&3));
}

#[test]
fn a_new_key_during_a_removal_keeps_the_run_of_the_leaving_value() {
    let mut presence = Presence::new(Some(1)).with_transition(Transition::opacity());
    presence.set(None, linear(1.0), at(0.0), false);
    presence.set_keyed(Some(2), |value| *value, linear(4.0), at(0.5), false);
    let departing: Vec<_> = presence.departing_in(at(0.75), LTR).collect();
    assert_eq!(departing[0].0, &1);
    assert_close(departing[0].1.opacity, 0.25, "still on its first curve");
    assert_close(presence.state(at(0.75)).progress, 0.0625, "new value from nothing");
}

#[test]
fn removing_the_value_lets_a_replaced_one_finish_leaving() {
    let mut presence = Presence::new(Some(1)).with_transition(Transition::opacity());
    presence.set_keyed(Some(2), |value| *value, linear(1.0), at(0.0), false);
    presence.set_keyed(None, |value| *value, linear(1.0), at(0.5), false);
    assert_eq!(presence.departing_in(at(0.6), LTR).count(), 1);
    assert!(presence.is_leaving(at(0.6)));
    assert!(!presence.is_running(at(2.0)));
}

fn measured_motion(height: f32) -> ContentHeightMotion {
    let mut motion = ContentHeightMotion::new();
    motion.frame(false);
    motion.measured(height, linear(1.0), at(0.0), false);
    motion
}

#[test]
fn content_height_first_measure_lands_without_a_clamp() {
    let mut motion = ContentHeightMotion::new();
    motion.frame(false);
    assert_eq!(motion.clamp(at(0.0)), None, "nothing measured yet");
    assert!(!motion.measured(100.0, linear(1.0), at(0.0), false));
    assert_eq!(motion.clamp(at(0.0)), None);
}

#[test]
fn content_height_swap_holds_the_old_height_then_animates_to_the_new_one() {
    let mut motion = measured_motion(100.0);
    motion.frame(true);
    assert_eq!(motion.clamp(at(0.0)), Some(100.0), "the frame before the new height is known");
    assert!(motion.measured(300.0, linear(1.0), at(0.0), false));
    assert_close(motion.clamp(at(0.5)).unwrap_or(-1.0), 200.0, "halfway");
    assert!(motion.is_running(at(0.5)));
    assert_eq!(motion.clamp(at(1.5)), None, "follows the content again");
}

#[test]
fn content_height_swap_to_the_same_height_releases_the_clamp() {
    let mut motion = measured_motion(100.0);
    motion.frame(true);
    assert!(!motion.measured(100.0, linear(1.0), at(0.0), false));
    assert_eq!(motion.clamp(at(0.0)), None);
}

#[test]
fn content_height_change_without_a_swap_lands_at_once() {
    let mut motion = measured_motion(100.0);
    motion.frame(false);
    assert_eq!(motion.clamp(at(0.0)), None, "the same branch arms nothing");
    assert!(!motion.measured(150.0, linear(1.0), at(0.0), false));
    assert_eq!(motion.clamp(at(0.0)), None);
    motion.frame(true);
    assert_eq!(motion.clamp(at(0.0)), Some(150.0), "the next swap starts from the landed height");
}

#[test]
fn content_height_retargets_mid_flight_from_the_height_on_screen() {
    let mut motion = measured_motion(100.0);
    motion.frame(true);
    motion.measured(300.0, linear(1.0), at(0.0), false);
    motion.frame(false);
    assert!(motion.measured(100.0, linear(1.0), at(0.5), false));
    assert_close(motion.clamp(at(0.5)).unwrap_or(-1.0), 200.0, "no jump at the turn");
}

#[test]
fn content_height_reduce_motion_lands_at_once() {
    let mut motion = measured_motion(100.0);
    motion.frame(true);
    assert!(motion.measured(300.0, linear(1.0), at(0.0), true));
    assert_eq!(motion.clamp(at(0.0)), None);
}
