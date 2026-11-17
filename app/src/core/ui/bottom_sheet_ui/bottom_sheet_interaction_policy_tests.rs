use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, Context, Empty, Entity, InteractiveElement, IntoElement, Modifiers,
    ParentElement, Pixels, Render, ScrollHandle, Size, StatefulInteractiveElement, Styled,
    TestAppContext, VisualTestContext, Window, div, point, px, size,
};
use rise_ui::bottom_sheet_ui::bottom_sheet_attached_motion_geometry::BottomSheetAttachedMotionGeometry;
use rise_ui::bottom_sheet_ui::bottom_sheet_content_pan_gate::{
    BottomSheetContentPanGate, bottom_sheet_nested_scroll,
};
use rise_ui::bottom_sheet_ui::bottom_sheet_interaction_policy::{
    BottomSheetInteractionStyle, BottomSheetReleaseDecision, BottomSheetReleasePolicy,
    BottomSheetSnapAnimationCurve,
};
use rise_ui::bottom_sheet_ui::bottom_sheet_motion_host::BottomSheetMotionHostingController;
use rise_ui::swiftui::binding::Binding;

use super::bottom_sheet_full_page_ui::BottomSheetDismissPolicy;
use super::experimental_bottom_sheet_presentation_host::ExperimentalBottomSheetPresentationHost;
use crate::modules::bot::pages::bot_mini_app_sheet::BotMiniAppSheetPolicy;

type RootView = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

const FRAME: Duration = Duration::from_millis(16);
const MAX_FRAMES: usize = 600;

fn phone_bounds() -> Bounds<Pixels> {
    Bounds::new(point(px(0.0), px(0.0)), size(px(390.0), px(844.0)))
}

fn empty_root() -> RootView {
    Rc::new(|_: &mut Window, _: &mut App| Empty.into_any_element())
}

/// host is the root of a window of that size so container bounds come from a real layout pass
fn motion_host(
    cx: &mut TestAppContext,
    frame: Size<Pixels>,
    root_view: RootView,
) -> (Entity<BottomSheetMotionHostingController>, &mut VisualTestContext) {
    let (host, cx) = cx.add_window_view(move |_, cx| {
        BottomSheetMotionHostingController::new(root_view, cx)
    });
    cx.simulate_resize(frame);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    (host, cx)
}

/// test platform never fires frame callbacks so the test steps the clock
/// and host frame by hand until the completion ran
fn run_motion_until_done(
    done: &Rc<Cell<bool>>,
    host: &Entity<BottomSheetMotionHostingController>,
    cx: &mut VisualTestContext,
) {
    for _ in 0..MAX_FRAMES {
        if done.get() {
            return;
        }
        cx.executor().advance_clock(FRAME);
        host.update(cx, |host, cx| host.frame_tick(cx));
        cx.run_until_parked();
    }
    panic!("the snap never completed");
}

fn completion_flag() -> (Rc<Cell<bool>>, Box<dyn FnOnce(&mut App)>) {
    let done = Rc::new(Cell::new(false));
    let flag = done.clone();
    (done, Box::new(move |_: &mut App| flag.set(true)))
}

fn surface_frame(
    host: &Entity<BottomSheetMotionHostingController>,
    cx: &VisualTestContext,
) -> Bounds<Pixels> {
    host.read_with(cx, |host, _| host.surface_frame())
        .expect("the sheet surface is laid out")
}

fn translation_y(host: &Entity<BottomSheetMotionHostingController>, cx: &VisualTestContext) -> Pixels {
    host.read_with(cx, |host, _| host.translation_y())
}

struct PresenterProbe {
    underlay_clicked: Rc<Cell<bool>>,
}

impl Render for PresenterProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let clicked = self.underlay_clicked.clone();
        div()
            .relative()
            .size_full()
            .child(
                div()
                    .id("presenter-underlay")
                    .absolute()
                    .inset_0()
                    .on_click(move |_, _, _| clicked.set(true)),
            )
            .child(div().absolute().inset_0().child(ExperimentalBottomSheetPresentationHost::new(
                "presenter",
                Binding::constant(false),
                |_: &mut Window, _: &mut App| div(),
            )))
    }
}

#[gpui::test]
fn inactive_experimental_presenter_passes_touches_through(cx: &mut TestAppContext) {
    let underlay_clicked = Rc::new(Cell::new(false));
    let probe_flag = underlay_clicked.clone();
    let (_, cx) = cx.add_window_view(move |_, _| PresenterProbe { underlay_clicked: probe_flag });
    cx.simulate_resize(size(px(390.0), px(844.0)));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    cx.simulate_click(point(px(195.0), px(422.0)), Modifiers::none());
    cx.run_until_parked();

    assert!(underlay_clicked.get());
}

#[test]
fn backdrop_dismiss_can_be_disabled_without_disabling_interactive_dismiss() {
    assert!(!BottomSheetDismissPolicy::allows_backdrop_dismiss(true, false, false));
    assert!(BottomSheetDismissPolicy::allows_backdrop_dismiss(true, true, false));
    assert!(!BottomSheetDismissPolicy::allows_backdrop_dismiss(true, true, true));
}

#[test]
fn configured_backdrop_consumes_tap_that_dismisses_keyboard() {
    assert!(BottomSheetDismissPolicy::should_dismiss_keyboard_first(true, 336.0, 0.0, 10.0));
    assert!(BottomSheetDismissPolicy::should_dismiss_keyboard_first(true, 0.0, 10.3, 10.0));
    assert!(!BottomSheetDismissPolicy::should_dismiss_keyboard_first(true, 0.0, 9.9, 10.0));
}

#[test]
fn attached_motion_covers_bounds_bottom_across_presentation_and_snap_heights() {
    let bounds = phone_bounds();

    for height in [286.0, 512.0, 742.0] {
        let geometry = BottomSheetAttachedMotionGeometry::resolve(bounds, px(height), px(0.0), false);

        assert_eq!(geometry.translation_y, px(0.0));
        assert_eq!(geometry.sheet_frame.bottom(), bounds.bottom());
        assert!(geometry.covers_container_bottom(bounds));
    }
}

#[test]
fn attached_motion_uses_container_bottom_instead_of_safe_area_bottom() {
    let bounds = phone_bounds();
    let safe_area_bottom = bounds.bottom() - px(34.0);
    let geometry = BottomSheetAttachedMotionGeometry::resolve(bounds, px(420.0), px(0.0), false);

    assert_eq!(geometry.sheet_frame.bottom(), bounds.bottom());
    assert_ne!(geometry.sheet_frame.bottom(), safe_area_bottom);
    assert!(geometry.covers_container_bottom(bounds));
}

#[test]
fn positive_interactive_translation_cannot_expose_container_bottom() {
    let bounds = phone_bounds();
    let ordinary_drag =
        BottomSheetAttachedMotionGeometry::resolve(bounds, px(420.0), px(96.0), false);
    let overdrag = BottomSheetAttachedMotionGeometry::resolve(bounds, px(420.0), px(900.0), false);

    assert_eq!(ordinary_drag.translation_y, px(96.0));
    assert!(ordinary_drag.covers_container_bottom(bounds));
    assert_eq!(overdrag.translation_y, px(419.0));
    assert!(overdrag.covers_container_bottom(bounds));
}

#[test]
fn committed_dismissal_may_detach_from_container_bottom() {
    let bounds = phone_bounds();
    let geometry = BottomSheetAttachedMotionGeometry::resolve(bounds, px(420.0), px(460.0), true);

    assert_eq!(geometry.translation_y, px(460.0));
    assert!(!geometry.covers_container_bottom(bounds));
}

#[gpui::test]
fn explicit_snap_supersedes_height_deferred_during_gesture(cx: &mut TestAppContext) {
    let (host, cx) = motion_host(cx, size(px(390.0), px(844.0)), empty_root());

    let collapsed_height = px(472.0);
    let expanded_height = px(776.0);
    host.update(cx, |host, cx| {
        host.update_configuration(
            collapsed_height,
            expanded_height,
            0.01,
            BottomSheetSnapAnimationCurve::EaseOut,
            false,
            false,
            cx,
        );
        host.begin_interaction(collapsed_height, collapsed_height, expanded_height, cx);

        host.update_configuration(
            collapsed_height,
            expanded_height,
            0.01,
            BottomSheetSnapAnimationCurve::EaseOut,
            false,
            true,
            cx,
        );
    });

    let (done, completion) = completion_flag();
    host.update(cx, |host, cx| {
        host.animate_snap(
            expanded_height,
            0.01,
            BottomSheetSnapAnimationCurve::EaseOut,
            false,
            completion,
            cx,
        );
    });
    run_motion_until_done(&done, &host, cx);

    assert!(!host.read_with(cx, |host, _| host.is_motion_active()));
}

#[gpui::test]
fn inertial_snap_between_detents_keeps_bottom_edge_docked(cx: &mut TestAppContext) {
    let (host, cx) = motion_host(cx, size(px(390.0), px(844.0)), empty_root());
    let bounds = phone_bounds();

    let collapsed_height = px(472.0);
    let expanded_height = px(776.0);
    host.update(cx, |host, cx| {
        host.update_configuration(
            expanded_height,
            expanded_height,
            0.01,
            BottomSheetSnapAnimationCurve::EaseOut,
            false,
            false,
            cx,
        );
        host.begin_interaction(expanded_height, collapsed_height, expanded_height, cx);
        host.update_interactive_translation(px(84.0), cx);
        host.finish_interactive_updates(cx);
    });

    assert_eq!(translation_y(&host, cx), px(0.0));
    assert!((surface_frame(&host, cx).bottom() - bounds.bottom()).abs() < px(0.5));

    let settle_frames: Rc<RefCell<Vec<Bounds<Pixels>>>> = Rc::default();
    let recorded = settle_frames.clone();
    host.update(cx, |host, _| {
        host.attached_layout_observer =
            Some(Rc::new(move |frame: Bounds<Pixels>, _: &mut App| recorded.borrow_mut().push(frame)));
    });

    let (done, completion) = completion_flag();
    host.update(cx, |host, cx| {
        host.animate_snap(
            collapsed_height,
            0.05,
            BottomSheetSnapAnimationCurve::EaseOut,
            false,
            completion,
            cx,
        );
    });
    run_motion_until_done(&done, &host, cx);

    let settle_frames = settle_frames.borrow();
    assert!(settle_frames.len() > 1);
    assert!(settle_frames.iter().all(|frame| (frame.bottom() - bounds.bottom()).abs() < px(0.5)));
    assert_eq!(translation_y(&host, cx), px(0.0));
    let sheet_frame = surface_frame(&host, cx);
    assert!((sheet_frame.bottom() - bounds.bottom()).abs() < px(0.5));
    assert!((sheet_frame.size.height - collapsed_height).abs() < px(0.5));
}

#[test]
fn detent_interaction_resizes_top_while_keeping_footer_attached() {
    let bounds = phone_bounds();
    let collapsed_height = px(472.0);
    let expanded_height = px(776.0);

    let expanding = BottomSheetAttachedMotionGeometry::resolve_detent_interaction(
        bounds,
        collapsed_height,
        collapsed_height,
        expanded_height,
        px(-140.0),
        false,
    );
    let collapsing = BottomSheetAttachedMotionGeometry::resolve_detent_interaction(
        bounds,
        expanded_height,
        collapsed_height,
        expanded_height,
        px(140.0),
        false,
    );

    assert_eq!(expanding.sheet_frame.size.height, px(612.0));
    assert_eq!(collapsing.sheet_frame.size.height, px(636.0));
    assert_eq!(expanding.translation_y, px(0.0));
    assert_eq!(collapsing.translation_y, px(0.0));
    assert_eq!(expanding.sheet_frame.bottom(), bounds.bottom());
    assert_eq!(collapsing.sheet_frame.bottom(), bounds.bottom());
}

#[test]
fn lowest_detent_remains_attached_until_dismissal_is_committed() {
    let bounds = phone_bounds();
    let collapsed_height = px(472.0);
    let geometry = BottomSheetAttachedMotionGeometry::resolve_detent_interaction(
        bounds,
        collapsed_height,
        collapsed_height,
        px(776.0),
        px(96.0),
        false,
    );

    assert_eq!(geometry.sheet_frame.size.height, collapsed_height);
    assert_eq!(geometry.translation_y, px(0.0));
    assert_eq!(geometry.sheet_frame.bottom(), bounds.bottom());
    assert!(geometry.covers_container_bottom(bounds));
}

#[test]
fn telegram_web_app_release_policy_matches_compact_and_expanded_thresholds() {
    let detents = [634.0, 797.0];
    let style = BottomSheetInteractionStyle::TelegramWebApp;

    assert_eq!(
        BottomSheetReleasePolicy::resolve(style, &detents, 0, detents[0], 61.0, 0.0),
        BottomSheetReleaseDecision::Dismiss
    );
    assert_eq!(
        BottomSheetReleasePolicy::resolve(style, &detents, 0, detents[0], -90.0, 0.0),
        BottomSheetReleaseDecision::Snap { index: 1 }
    );
    assert_eq!(
        BottomSheetReleasePolicy::resolve(style, &detents, 1, detents[1], 90.0, 0.0),
        BottomSheetReleaseDecision::Snap { index: 0 }
    );
    assert_eq!(
        BottomSheetReleasePolicy::resolve(style, &detents, 1, detents[1], 1.0, 1_801.0),
        BottomSheetReleaseDecision::Dismiss
    );
}

#[test]
fn telegram_web_app_lowest_detent_can_follow_dismissal_pull() {
    let bounds = phone_bounds();
    let collapsed_height = px(634.0);
    let geometry = BottomSheetAttachedMotionGeometry::resolve_detent_interaction(
        bounds,
        collapsed_height,
        collapsed_height,
        px(797.0),
        px(96.0),
        true,
    );

    assert_eq!(geometry.sheet_frame.size.height, collapsed_height);
    assert_eq!(geometry.translation_y, px(96.0));
    assert_eq!(geometry.sheet_frame.bottom(), bounds.bottom() + px(96.0));
}

#[gpui::test]
fn cancelled_telegram_web_app_dismissal_pull_settles_without_transform(cx: &mut TestAppContext) {
    let (host, cx) = motion_host(cx, size(px(390.0), px(844.0)), empty_root());

    let collapsed_height = px(634.0);
    host.update(cx, |host, cx| {
        host.update_configuration(
            collapsed_height,
            px(797.0),
            0.01,
            BottomSheetSnapAnimationCurve::EaseOut,
            true,
            false,
            cx,
        );
        host.begin_interaction(collapsed_height, collapsed_height, px(797.0), cx);
        host.update_interactive_translation(px(42.0), cx);
        host.finish_interactive_updates(cx);
    });

    assert!((translation_y(&host, cx) - px(42.0)).abs() < px(0.5));

    let (done, completion) = completion_flag();
    host.update(cx, |host, cx| {
        host.animate_snap(
            collapsed_height,
            0.01,
            BottomSheetSnapAnimationCurve::EaseOut,
            false,
            completion,
            cx,
        );
    });
    run_motion_until_done(&done, &host, cx);

    assert_eq!(translation_y(&host, cx), px(0.0));
    assert!((surface_frame(&host, cx).size.height - collapsed_height).abs() < px(0.5));
}

#[test]
fn telegram_web_app_snap_curve_uses_telegram_spring_parameters() {
    let curve = BottomSheetSnapAnimationCurve::TelegramSpring { initial_velocity: 0.0 };

    assert_eq!(curve.solve(0.0, 0.45), 0.0);
    assert_eq!(curve.solve(1.0, 0.45), 1.0);
    assert!(curve.solve(0.5, 0.45) > 0.75);
}

#[test]
fn telegram_web_app_uses_full_height_in_regular_width_and_landscape() {
    assert_eq!(
        BotMiniAppSheetPolicy::detent_fractions(size(px(768.0), px(1_024.0))),
        vec![1.0]
    );
    assert_eq!(
        BotMiniAppSheetPolicy::detent_fractions(size(px(844.0), px(390.0))),
        vec![1.0]
    );

    let phone_detents = BotMiniAppSheetPolicy::detent_fractions(size(px(390.0), px(844.0)));
    assert_eq!(phone_detents.len(), 2);
    assert!(phone_detents[0] > 0.74 && phone_detents[0] < 0.77);
    assert_eq!(phone_detents[1], 1.0);
}

#[test]
fn standard_release_dismisses_from_upper_detent_in_one_pull() {
    let detents = [477.0, 784.0];
    let style = BottomSheetInteractionStyle::Standard;

    assert_eq!(
        BottomSheetReleasePolicy::resolve(style, &detents, 1, detents[1], 560.0, 0.0),
        BottomSheetReleaseDecision::Dismiss
    );
    assert_eq!(
        BottomSheetReleasePolicy::resolve(style, &detents, 1, detents[1], 120.0, 2_400.0),
        BottomSheetReleaseDecision::Dismiss
    );
}

#[test]
fn standard_release_keeps_upper_detent_on_small_pull() {
    let detents = [477.0, 784.0];
    let style = BottomSheetInteractionStyle::Standard;

    assert_eq!(
        BottomSheetReleasePolicy::resolve(style, &detents, 1, detents[1], 30.0, 200.0),
        BottomSheetReleaseDecision::Snap { index: 1 }
    );
    assert_eq!(
        BottomSheetReleasePolicy::resolve(style, &detents, 1, detents[1], 350.0, 500.0),
        BottomSheetReleaseDecision::Snap { index: 0 }
    );
}

#[test]
fn telegram_web_app_release_dismisses_from_upper_detent_past_smallest_detent() {
    let detents = [634.0, 797.0];

    assert_eq!(
        BottomSheetReleasePolicy::resolve(
            BottomSheetInteractionStyle::TelegramWebApp,
            &detents,
            1,
            detents[1],
            500.0,
            0.0,
        ),
        BottomSheetReleaseDecision::Dismiss
    );
}

#[gpui::test]
fn interrupting_gesture_adopts_rendered_height_over_stale_resting_detent(cx: &mut TestAppContext) {
    let (host, cx) = motion_host(cx, size(px(390.0), px(844.0)), empty_root());

    let collapsed_height = px(477.0);
    let expanded_height = px(784.0);
    let travel = px(100.0);
    let initial = host.update(cx, |host, cx| {
        host.update_configuration(
            collapsed_height,
            expanded_height,
            0.01,
            BottomSheetSnapAnimationCurve::EaseOut,
            false,
            false,
            cx,
        );
        host.begin_interaction(collapsed_height, collapsed_height, expanded_height, cx)
    });
    assert!((initial - collapsed_height).abs() < px(0.5));

    let (adopted, expansion_room) = host.update(cx, |host, cx| {
        host.update_interactive_translation(-travel, cx);
        host.finish_interactive_updates(cx);

        let adopted = host.begin_interaction(collapsed_height, collapsed_height, expanded_height, cx);
        (adopted, host.interaction_expansion_room())
    });

    assert!((adopted - (collapsed_height + travel)).abs() < px(0.5));
    assert!((expansion_room - (expanded_height - collapsed_height - travel)).abs() < px(0.5));
}

/// root view with one scroll view in it, the host surface at window size
/// scroll areas are declared to the gate like sheet content does
fn scroll_root(scroll: ScrollHandle, vertical: bool, frame: Size<Pixels>, content: Size<Pixels>) -> RootView {
    Rc::new(move |_: &mut Window, _: &mut App| {
        let mut area = div().id("probe-scroll").w(frame.width).h(frame.height);
        area = if vertical { area.overflow_y_scroll() } else { area.overflow_x_scroll() };
        let area = area
            .track_scroll(&scroll)
            .child(div().flex_none().w(content.width).h(content.height));
        div()
            .size_full()
            .child(bottom_sheet_nested_scroll(&scroll, area))
            .into_any_element()
    })
}

fn configure_full_surface(
    host: &Entity<BottomSheetMotionHostingController>,
    height: Pixels,
    cx: &mut VisualTestContext,
) {
    host.update(cx, |host, cx| {
        host.update_configuration(
            height,
            height,
            0.01,
            BottomSheetSnapAnimationCurve::EaseOut,
            false,
            false,
            cx,
        );
    });
    cx.run_until_parked();
}

#[gpui::test]
fn content_pan_gate_ignores_scroll_view_parked_below_its_top(cx: &mut TestAppContext) {
    let root_frame = size(px(390.0), px(600.0));
    let scroll_view = ScrollHandle::new();
    let (host, cx) = motion_host(
        cx,
        root_frame,
        scroll_root(scroll_view.clone(), true, root_frame, size(px(390.0), px(4_000.0))),
    );
    configure_full_surface(&host, root_frame.height, cx);

    let probe = point(px(195.0), px(300.0));
    let resolved = cx.update(|_, cx| BottomSheetContentPanGate::vertically_scrollable_view(probe, &host, cx));
    assert_eq!(resolved.map(|scroll| scroll.bounds()), Some(scroll_view.bounds()));
    assert!(BottomSheetContentPanGate::is_at_top(Some(&scroll_view)));

    scroll_view.set_offset(point(px(0.0), px(-240.0)));
    assert!(!BottomSheetContentPanGate::is_at_top(Some(&scroll_view)));
}

#[gpui::test]
fn content_pan_gate_treats_horizontal_strip_as_sheet_surface(cx: &mut TestAppContext) {
    let root_frame = size(px(390.0), px(600.0));
    let strip = ScrollHandle::new();
    let (host, cx) = motion_host(
        cx,
        root_frame,
        scroll_root(strip, false, size(px(390.0), px(44.0)), size(px(1_200.0), px(44.0))),
    );
    configure_full_surface(&host, root_frame.height, cx);

    let resolved = cx.update(|_, cx| {
        BottomSheetContentPanGate::vertically_scrollable_view(point(px(195.0), px(20.0)), &host, cx)
    });

    assert!(resolved.is_none());
    assert!(BottomSheetContentPanGate::is_at_top(resolved.as_ref()));
}
