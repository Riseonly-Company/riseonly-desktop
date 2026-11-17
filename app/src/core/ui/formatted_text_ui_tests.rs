use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyView, AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Pixels, Render, Size, Styled,
    TestAppContext, VisualTestContext, Window, black, blue, div, hsla, point, px, size,
};
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::{
    HoldContextMenuMetrics, HoldContextMenuSourceLiftPolicy,
};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use rise_ui::view_anchor_probe::{ViewAnchorProbe, ViewAnchorProbeReader};
use rise_ui::view_screen_snapshot::{
    HoldContextMenuLiveSourceLiftPolicy, HoldContextMenuSnapshotRenderRoot, ViewScreenSnapshotPolicy, ViewScreenSource,
};
use rise_ui::window_screen_frame_reader::WindowScreenFrameRefreshPolicy;

use crate::core::ui::formatted_text_ui::{
    FormattedText, FormattedTextOptions, FormattedTextPreparationCachePolicy, FormattedTextStyle,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuSnapshotCapturePolicy;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_overlay::HoldContextMenuOverflowPreviewPolicy;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuSourceLift;
use crate::modules::chat::components::chat::async_render::rise_manual_message_hosting_view::RiseHostedRowAuditPolicy;
use crate::modules::chat::components::chat::bubble::message_content::MessageContentKind;
use crate::modules::chat::components::chat::message_bubble::ChatMessageHoldPreviewPolicy;

const FRAME_TOLERANCE: f32 = 0.01;
const SCALE_TOLERANCE: f32 = 0.0001;

fn style() -> FormattedTextStyle {
    FormattedTextStyle::new(black(), blue(), hsla(0.0, 0.0, 0.5, 1.0))
}

fn options() -> FormattedTextOptions {
    FormattedTextOptions::default()
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}

fn assert_frames_match(actual: Bounds<Pixels>, expected: Bounds<Pixels>) {
    let pairs = [
        (actual.origin.x, expected.origin.x),
        (actual.origin.y, expected.origin.y),
        (actual.size.width, expected.size.width),
        (actual.size.height, expected.size.height),
    ];
    for (actual_value, expected_value) in pairs {
        assert!(
            (f32::from(actual_value) - f32::from(expected_value)).abs() < FRAME_TOLERANCE,
            "{actual:?} != {expected:?}"
        );
    }
}

#[test]
fn combined_link_and_mention_pass_preserves_formatted_text_semantics() {
    let text = "**Bold** @alice https://example.com/path?q=1#section";
    let first = FormattedText::format(text, &style(), &options(), &[]);
    let second = FormattedText::format(text, &style(), &options(), &[]);

    assert_eq!(first, second);
    assert_eq!(first.display_string, "Bold @alice https://example.com/path?q=1#section");
    assert!(first.has_url);
    assert!(first.has_mention);
    assert_eq!(first.subtitle.as_deref(), Some("example.com/path?q=1#section"));
}

#[test]
fn formatting_options_still_gate_only_their_own_token_work() {
    let text = "@alice https://example.com/path";
    let no_url = FormattedText::format(
        text,
        &style(),
        &FormattedTextOptions { no_url_format: true, ..Default::default() },
        &[],
    );
    let no_mention = FormattedText::format(
        text,
        &style(),
        &FormattedTextOptions { no_mention_format: true, ..Default::default() },
        &[],
    );

    assert!(!no_url.has_url);
    assert_eq!(no_url.subtitle, None);
    assert!(no_url.has_mention);
    assert!(no_mention.has_url);
    assert!(!no_mention.has_mention);
}

#[test]
fn inline_preparation_reuses_exact_formatted_and_geometry_values() {
    let text = "**Bold** text that is measured beside message metadata";
    let first = FormattedText::inline_preparation(text, &style(), &options(), &[]);
    let second = FormattedText::inline_preparation(text, &style(), &options(), &[]);

    assert_eq!(first.formatted, second.formatted);
    assert_eq!(first.formatted.display_string, first.geometry.display);
    assert_eq!(first.geometry.attributed.characters(), first.geometry.display);
    assert!(Arc::ptr_eq(&first.geometry, &second.geometry));
    assert!(first.geometry.cache_key == second.geometry.cache_key);
}

#[test]
fn interaction_preparation_preserves_mention_and_url_tokens() {
    let text = "**Bold** @alice https://example.com/path";
    let prepared = FormattedText::interaction_preparation(text, &style(), &options(), &[], None);

    assert_eq!(prepared.geometry.display, "Bold @alice https://example.com/path");
    let mentions: Vec<&str> = prepared.mentions.iter().map(|token| token.tag.as_str()).collect();
    let urls: Vec<&str> = prepared.urls.iter().map(|token| token.tag.as_str()).collect();
    assert_eq!(mentions, ["alice"]);
    assert_eq!(urls, ["https://example.com/path"]);
    assert_eq!(prepared.geometry.attributed.characters(), prepared.geometry.display);
}

#[test]
fn formatted_text_preparation_caches_are_bounded_to_the_chat_window() {
    assert_eq!(FormattedTextPreparationCachePolicy::entry_limit(), 768);
    assert_eq!(
        FormattedTextPreparationCachePolicy::line_measurement_entry_limit(),
        FormattedTextPreparationCachePolicy::entry_limit() * 2
    );
    assert!(FormattedTextPreparationCachePolicy::aggregate_cost_limit() <= 40 * 1_024 * 1_024);
    assert!(FormattedTextPreparationCachePolicy::maximum_cacheable_utf16_count() <= 32_768);
    assert!(!FormattedTextPreparationCachePolicy::can_cache(&"x".repeat(32_769)));
}

#[test]
fn hosted_row_height_audits_are_frame_budgeted() {
    let overflow_delay = RiseHostedRowAuditPolicy::OVERFLOW_DELAY;
    let scrolling_retry_delay = RiseHostedRowAuditPolicy::SCROLLING_RETRY_DELAY;

    assert_eq!(RiseHostedRowAuditPolicy::MAX_AUDITS_PER_TURN, 3);
    assert_eq!(RiseHostedRowAuditPolicy::MAX_VISIBLE_AUDITS_PER_SCROLLING_TURN, 1);
    assert!(overflow_delay >= 1.0 / 120.0);
    assert!(scrolling_retry_delay >= 0.1);
    assert_eq!(RiseHostedRowAuditPolicy::INITIAL_DELAY, 0.12);
    assert_eq!(RiseHostedRowAuditPolicy::VERIFICATION_DELAY, 0.08);
}

#[test]
fn passive_media_frame_readers_do_not_subscribe_to_every_scroll_frame() {
    assert!(!WindowScreenFrameRefreshPolicy::requires_scroll_subscription(false));
    assert!(WindowScreenFrameRefreshPolicy::requires_scroll_subscription(true));
}

#[test]
fn long_text_context_menu_uses_rasterized_preview_without_freezing_media() {
    let long_formatted_text = "**formatted** message ".repeat(24);

    assert!(ChatMessageHoldPreviewPolicy::prefers_rasterized_preview(MessageContentKind::Text, &long_formatted_text));
    assert!(!ChatMessageHoldPreviewPolicy::prefers_rasterized_preview(MessageContentKind::Text, "**short**"));
    assert!(!ChatMessageHoldPreviewPolicy::prefers_rasterized_preview(
        MessageContentKind::VideoNote,
        &long_formatted_text
    ));
}

#[test]
fn context_menu_snapshot_preserves_the_complete_tall_source() {
    let waits_for_screen_updates = ViewScreenSnapshotPolicy::WAITS_FOR_SCREEN_UPDATES;
    assert!(!waits_for_screen_updates);

    let full = rect(20.0, -400.0, 300.0, 1_600.0);
    let scale = 0.87_f32;
    let full_center = full.center();
    let captured = rect(
        f32::from(full_center.x) - 300.0 * scale / 2.0,
        f32::from(full_center.y) - 1_600.0 * scale / 2.0,
        300.0 * scale,
        1_600.0 * scale,
    );

    let presentation = HoldContextMenuSnapshotCapturePolicy::presentation_geometry(full, Some(captured), scale)
        .expect("the captured frame lies inside the scaled source");

    assert_frames_match(presentation.source_frame, full);
    let captured_center = captured.center();
    assert!(
        (f32::from(presentation.initial_source_center.x) - f32::from(captured_center.x)).abs() < FRAME_TOLERANCE
    );
    assert!(
        (f32::from(presentation.initial_source_center.y) - f32::from(captured_center.y)).abs() < FRAME_TOLERANCE
    );
    let coverage = HoldContextMenuSnapshotCapturePolicy::visible_coverage(captured, Some(captured));
    assert!((coverage - 1.0).abs() < SCALE_TOLERANCE);
}

#[test]
fn context_menu_press_scale_uses_telegram_width_based_activation() {
    let squish = HoldContextMenuMetrics::PRESS_SQUISH_AMOUNT;
    let wide = HoldContextMenuSourceLiftPolicy::activation_scale(300.0, squish, false);
    let compact = HoldContextMenuSourceLiftPolicy::activation_scale(50.0, squish, false);
    let reduced_motion = HoldContextMenuSourceLiftPolicy::activation_scale(300.0, squish, true);

    assert!((wide - 0.95).abs() < SCALE_TOLERANCE);
    assert!((compact - 0.70).abs() < SCALE_TOLERANCE);
    assert_eq!(reduced_motion, HoldContextMenuMetrics::REDUCED_MOTION_PRESS_SCALE);
}

/// source row, gpui has no superview to ask so the row reports where it was painted
struct LiftSourceView {
    probe: ViewAnchorProbe,
}

impl Render for LiftSourceView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().relative().size_full().child(ViewAnchorProbeReader::new(self.probe.clone()))
    }
}

/// test harness
/// a home slot that draws the row or its lifted placeholder and an overlay slot that draws the lift
struct LiftHarness {
    source: AnyView,
    home_frame: Bounds<Pixels>,
    overlay_frame: Bounds<Pixels>,
    lift: Option<Rc<HoldContextMenuSourceLift>>,
}

impl Render for LiftHarness {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let home = HoldContextMenuSourceLift::lifted_placeholder(&self.source)
            .unwrap_or_else(|| self.source.clone().into_any_element());
        let mut root = div().relative().size_full().child(
            div()
                .absolute()
                .left(self.home_frame.origin.x)
                .top(self.home_frame.origin.y)
                .w(self.home_frame.size.width)
                .h(self.home_frame.size.height)
                .child(home),
        );
        if let Some(host) = self.lift.as_ref().and_then(|lift| lift.layout(self.overlay_frame.size)) {
            root = root.child(
                div()
                    .absolute()
                    .left(self.overlay_frame.origin.x)
                    .top(self.overlay_frame.origin.y)
                    .w(self.overlay_frame.size.width)
                    .h(self.overlay_frame.size.height)
                    .child(host),
            );
        }
        root
    }
}

/// chat row containers opt in by returning their content, an unmarked container returns nothing
struct HoldContextMenuTestRenderRoot {
    source: Option<ViewScreenSource>,
}

impl HoldContextMenuSnapshotRenderRoot for HoldContextMenuTestRenderRoot {
    fn hold_context_menu_live_source_view(&self) -> Option<ViewScreenSource> {
        self.source.clone()
    }
}

fn window_size() -> Size<Pixels> {
    size(px(800.0), px(1_000.0))
}

fn redraw(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn open_lift_window(
    cx: &mut TestAppContext,
    home_frame: Bounds<Pixels>,
    overlay_frame: Bounds<Pixels>,
) -> (Entity<LiftHarness>, ViewScreenSource, &mut VisualTestContext) {
    let probe = ViewAnchorProbe::new();
    let source_probe = probe.clone();
    let view: AnyView = cx.update(|cx| cx.new(|_: &mut Context<LiftSourceView>| LiftSourceView { probe: source_probe }).into());
    let source = ViewScreenSource::new(view.clone(), probe);
    let (harness, cx) =
        cx.add_window_view(move |_, _| LiftHarness { source: view, home_frame, overlay_frame, lift: None });
    cx.simulate_resize(window_size());
    redraw(cx);
    (harness, source, cx)
}

fn show_lift(harness: &Entity<LiftHarness>, lift: &Rc<HoldContextMenuSourceLift>, cx: &mut VisualTestContext) {
    let shown = Rc::clone(lift);
    harness.update(cx, |harness, cx| {
        harness.lift = Some(shown);
        cx.notify();
    });
}

#[gpui::test]
fn context_menu_live_source_restores_its_original_hierarchy_and_state(cx: &mut TestAppContext) {
    let home_frame = rect(40.0, 60.0, 240.0, 72.0);
    let overlay_frame = rect(100.0, 300.0, 240.0, 72.0);
    let (harness, source, cx) = open_lift_window(cx, home_frame, overlay_frame);
    assert_eq!(source.window_frame(), Some(home_frame));

    let lift = Rc::new(HoldContextMenuSourceLift::new(source.clone(), None).expect("the source is drawn in a window"));
    assert!(!lift.clips_source_to_host_bounds());
    assert!(lift.layout(overlay_frame.size).is_none());
    assert!(lift.lifted_center_in_screen().is_none());

    show_lift(&harness, &lift, cx);
    cx.update(|_, cx| lift.attach(cx));
    redraw(cx);
    assert_eq!(source.window_frame(), Some(overlay_frame));
    assert_eq!(lift.lifted_center_in_screen(), Some(overlay_frame.center()));
    assert!(HoldContextMenuSourceLift::lifted_placeholder(&source.view).is_some());

    cx.update(|_, cx| {
        lift.restore(cx);
        lift.restore(cx);
    });
    redraw(cx);
    assert_eq!(source.window_frame(), Some(home_frame));
    assert!(HoldContextMenuSourceLift::lifted_placeholder(&source.view).is_none());
    assert!(lift.layout(overlay_frame.size).is_none());
    assert!(lift.lifted_center_in_screen().is_none());
}

#[gpui::test]
fn fallback_message_live_source_uses_marked_row_and_exact_bubble_crop(cx: &mut TestAppContext) {
    let row_frame = rect(0.0, 220.0, 390.0, 140.0);
    let (_harness, source, _cx) = open_lift_window(cx, row_frame, rect(0.0, 0.0, 390.0, 140.0));
    let plate_frame = rect(104.0, 18.0, 248.0, 82.0);
    let bubble_rect = Bounds::new(row_frame.origin + plate_frame.origin, plate_frame.size);

    let row = HoldContextMenuTestRenderRoot { source: Some(source.clone()) };
    let candidate = HoldContextMenuLiveSourceLiftPolicy::candidate(&[&row], bubble_rect, 3.0)
        .expect("the marked row encloses the bubble");
    assert_eq!(candidate.source_view.view.entity_id(), source.view.entity_id());
    assert_eq!(candidate.crop_rect_in_source, plate_frame);

    let unmarked_row = HoldContextMenuTestRenderRoot { source: None };
    assert!(HoldContextMenuLiveSourceLiftPolicy::candidate(&[&unmarked_row], bubble_rect, 3.0).is_none());
}

#[gpui::test]
fn cropped_live_source_fills_host_and_restores_row_placeholder(cx: &mut TestAppContext) {
    let home_frame = rect(0.0, 40.0, 390.0, 140.0);
    let crop = rect(104.0, 18.0, 248.0, 82.0);
    let overlay_frame = rect(150.0, 400.0, 248.0, 82.0);
    let (harness, source, cx) = open_lift_window(cx, home_frame, overlay_frame);

    let lift =
        Rc::new(HoldContextMenuSourceLift::new(source.clone(), Some(crop)).expect("the crop lies inside the source"));

    show_lift(&harness, &lift, cx);
    cx.update(|_, cx| lift.attach(cx));
    redraw(cx);
    assert!(lift.clips_source_to_host_bounds());
    let lifted = source.window_frame().expect("the overlay draws the source");
    assert_eq!(lifted.size, home_frame.size);
    assert_eq!(Bounds::new(lifted.origin + crop.origin, crop.size), overlay_frame);
    assert!(HoldContextMenuSourceLift::lifted_placeholder(&source.view).is_some());

    cx.update(|_, cx| lift.restore(cx));
    redraw(cx);
    assert_eq!(source.window_frame(), Some(home_frame));
    assert!(HoldContextMenuSourceLift::lifted_placeholder(&source.view).is_none());
}

#[test]
fn context_menu_full_raster_is_sharp_but_memory_bounded() {
    let normal = ViewScreenSnapshotPolicy::full_source_raster_scale(size(px(300.0), px(1_600.0)), 3.0)
        .expect("a normal source keeps a raster scale");
    assert!(normal <= 1.3);
    assert!(300.0 * 1_600.0 * normal * normal <= 1_000_000.0);

    let tall = ViewScreenSnapshotPolicy::full_source_raster_scale(size(px(800.0), px(8_000.0)), 3.0)
        .expect("a tall source keeps a raster scale");
    assert!(tall <= ViewScreenSnapshotPolicy::MAXIMUM_FULL_SOURCE_SCALE);
    assert!(800.0 * 8_000.0 * tall * tall <= ViewScreenSnapshotPolicy::MAXIMUM_FULL_SOURCE_PIXEL_COUNT + 1.0);
    assert!(8_000.0 * tall <= ViewScreenSnapshotPolicy::MAXIMUM_FULL_SOURCE_PIXEL_DIMENSION + 1.0);

    assert_eq!(ViewScreenSnapshotPolicy::full_source_raster_scale(size(px(300.0), px(30_000.0)), 3.0), None);
}

#[test]
fn overflowing_context_menu_starts_at_message_bottom_beside_actions() {
    let actions: Vec<HoldContextMenuAction> = (0..7)
        .map(|index| {
            HoldContextMenuAction::new(format!("action-{index}"), format!("Action {index}"))
                .section_break(index == 4)
        })
        .collect();
    let estimated_menu_height = HoldContextMenuOverflowPreviewPolicy::resolved_menu_height(0.0, &actions, 0.0);

    assert_eq!(HoldContextMenuOverflowPreviewPolicy::DEFAULT_SCROLL_ANCHOR, rise_ui::swiftui::transition::UnitPoint::BOTTOM);
    assert_eq!(estimated_menu_height, 10.0 + 7.0 * HoldContextMenuMetrics::MENU_ROW_HEIGHT + 0.66);
    assert_eq!(HoldContextMenuOverflowPreviewPolicy::initial_scroll_offset(1_940.0, 844.0), 1_096.0);
    assert_eq!(HoldContextMenuOverflowPreviewPolicy::initial_scroll_offset(700.0, 844.0), 0.0);
    assert_eq!(HoldContextMenuOverflowPreviewPolicy::safe_scroll_inset(32.0, 34.0), 40.0);
}
